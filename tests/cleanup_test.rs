use federation::cli::commands::cleanup_recipes;
use federation::db::models::{NewFeed, NewGitHubFeed, NewGitHubRecipe, NewRecipe};
use federation::db::{self, init_pool, run_migrations};
use federation::indexer::{SearchIndex, SearchQuery};

fn new_recipe(feed_id: i64, external_id: &str, title: &str, content: &str) -> NewRecipe {
    NewRecipe {
        feed_id,
        external_id: external_id.to_string(),
        title: title.to_string(),
        source_url: None,
        enclosure_url: format!("https://example.com/{external_id}"),
        content: Some(content.to_string()),
        summary: None,
        servings: None,
        total_time_minutes: None,
        active_time_minutes: None,
        difficulty: None,
        image_url: None,
        published_at: None,
        content_hash: Some(db::recipes::calculate_content_hash(title, Some(content))),
        content_etag: None,
        content_last_modified: None,
        feed_entry_updated: None,
        locale: None,
        locale_source: None,
    }
}

fn search(index: &SearchIndex, q: &str) -> Vec<i64> {
    let query = SearchQuery {
        q: q.to_string(),
        page: 1,
        limit: 10,
        locale: None,
    };
    let mut ids: Vec<i64> = index
        .search(&query, 100)
        .unwrap()
        .results
        .iter()
        .map(|r| r.recipe_id)
        .collect();
    ids.sort();
    ids
}

async fn index_all(pool: &db::DbPool, index: &SearchIndex) {
    let mut writer = index.writer().unwrap();
    for recipe in db::recipes::list_all_recipes(pool, 100, 0).await.unwrap() {
        index
            .index_recipe(&mut writer, &recipe, None, &[], &[])
            .unwrap();
    }
    index.commit(&mut writer).unwrap();
}

#[tokio::test]
async fn cleanup_retitles_github_recipes_and_removes_duplicates() {
    let pool = init_pool("sqlite::memory:").await.unwrap();
    run_migrations(&pool).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let index = SearchIndex::new(dir.path()).unwrap();

    let feed = db::feeds::create_feed(
        &pool,
        &NewFeed {
            url: "https://github.com/alice/recipes".to_string(),
            title: None,
        },
    )
    .await
    .unwrap();
    let github_feed = db::github::create_github_feed(
        &pool,
        &NewGitHubFeed {
            feed_id: feed.id,
            repository_url: "https://github.com/alice/recipes".to_string(),
            owner: "alice".to_string(),
            repo_name: "recipes".to_string(),
            default_branch: "main".to_string(),
        },
    )
    .await
    .unwrap();

    // A GitHub recipe whose title was taken from a dated slug file name.
    let declared = "---\ntitle: Tiramisu Brownies\n---\nLayer @mascarpone{}.\n";
    let slug = db::recipes::create_recipe(
        &pool,
        &new_recipe(
            feed.id,
            "recipes/2025-12-01-tiramisu-brownies.cook",
            "2025-12-01-tiramisu-brownies",
            declared,
        ),
    )
    .await
    .unwrap();
    // The same file also indexed from a build directory under the same slug title.
    let copy = db::recipes::create_recipe(
        &pool,
        &new_recipe(
            feed.id,
            "public/recipes/2025-12-01-tiramisu-brownies.cook",
            "2025-12-01-tiramisu-brownies",
            declared,
        ),
    )
    .await
    .unwrap();
    for (recipe, path) in [
        (&slug, "recipes/2025-12-01-tiramisu-brownies.cook"),
        (&copy, "public/recipes/2025-12-01-tiramisu-brownies.cook"),
    ] {
        db::github::create_github_recipe(
            &pool,
            &NewGitHubRecipe {
                recipe_id: recipe.id,
                github_feed_id: github_feed.id,
                file_path: path.to_string(),
                file_sha: "abc".to_string(),
                raw_url: String::new(),
                html_url: String::new(),
            },
        )
        .await
        .unwrap();
    }

    // A recipe with no declared title whose file name should be humanized.
    let gnocchi = db::recipes::create_recipe(
        &pool,
        &new_recipe(
            feed.id,
            "recipes/2025-12-04-cottage-cheese-gnocchi.cook",
            "2025-12-04-cottage-cheese-gnocchi",
            "Boil @gnocchi{}.\n",
        ),
    )
    .await
    .unwrap();
    db::github::create_github_recipe(
        &pool,
        &NewGitHubRecipe {
            recipe_id: gnocchi.id,
            github_feed_id: github_feed.id,
            file_path: "recipes/2025-12-04-cottage-cheese-gnocchi.cook".to_string(),
            file_sha: "def".to_string(),
            raw_url: String::new(),
            html_url: String::new(),
        },
    )
    .await
    .unwrap();

    // An RSS recipe whose title must be left alone, plus a mirror of it in
    // another feed that must go.
    let rss = db::feeds::create_feed(
        &pool,
        &NewFeed {
            url: "https://example.com/feed.xml".to_string(),
            title: None,
        },
    )
    .await
    .unwrap();
    let mirror = db::feeds::create_feed(
        &pool,
        &NewFeed {
            url: "https://mirror.example.com/feed.xml".to_string(),
            title: None,
        },
    )
    .await
    .unwrap();
    let soup = db::recipes::create_recipe(
        &pool,
        &new_recipe(rss.id, "soup", "lentil-soup", "Simmer @lentils{}.\n"),
    )
    .await
    .unwrap();
    let soup_mirror = db::recipes::create_recipe(
        &pool,
        &new_recipe(mirror.id, "soup", "lentil-soup", "Simmer @lentils{}.\n"),
    )
    .await
    .unwrap();

    index_all(&pool, &index).await;
    assert_eq!(search(&index, "lentils"), vec![soup.id, soup_mirror.id]);

    let stats = cleanup_recipes(&pool, &index).await.unwrap();

    assert_eq!(
        stats.retitled, 3,
        "slug, its copy and gnocchi titles are rewritten"
    );
    assert_eq!(
        stats.duplicates_removed, 2,
        "the build-dir copy and the mirror go"
    );

    let mut titles: Vec<String> = db::recipes::list_all_recipes(&pool, 100, 0)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.title)
        .collect();
    titles.sort();
    assert_eq!(
        titles,
        vec!["Cottage Cheese Gnocchi", "Tiramisu Brownies", "lentil-soup"]
    );

    assert!(db::recipes::get_recipe(&pool, copy.id).await.is_err());
    assert!(db::recipes::get_recipe(&pool, soup_mirror.id)
        .await
        .is_err());
    assert_eq!(search(&index, "lentils"), vec![soup.id]);
    assert_eq!(search(&index, "title:tiramisu"), vec![slug.id]);
    assert_eq!(search(&index, "title:gnocchi"), vec![gnocchi.id]);

    // Rerunning changes nothing.
    let again = cleanup_recipes(&pool, &index).await.unwrap();
    assert_eq!((again.retitled, again.duplicates_removed), (0, 0));
}
