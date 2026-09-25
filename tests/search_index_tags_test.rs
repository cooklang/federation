//! Tags stored in the database must be searchable both through the `tags:`
//! query syntax and through the structured `tags` filter, for every recipe
//! that goes through the shared indexing path.

use std::sync::Arc;

use federation::db::models::{NewFeed, NewRecipe};
use federation::db::{self, feeds, recipes, tags, DbPool};
use federation::indexer::extras::reindex_recipes;
use federation::indexer::filters::{SearchFilters, SortOrder};
use federation::indexer::{SearchIndex, SearchQuery};

async fn pool() -> DbPool {
    let pool = db::init_pool("sqlite::memory:").await.unwrap();
    db::run_migrations(&pool).await.unwrap();
    pool
}

/// A feed titled "Jane's Kitchen" with one recipe tagged Desserts + chocolate.
async fn seed(pool: &DbPool) -> i64 {
    let feed = feeds::create_feed(
        pool,
        &NewFeed {
            url: "https://example.com/feed.xml".to_string(),
            title: Some("Jane's Kitchen".to_string()),
        },
    )
    .await
    .unwrap();

    let recipe = recipes::create_recipe(
        pool,
        &NewRecipe {
            feed_id: feed.id,
            external_id: "brownies".to_string(),
            title: "Brownies".to_string(),
            source_url: None,
            enclosure_url: "https://example.com/brownies.cook".to_string(),
            content: Some("Melt the @chocolate{200%g}.".to_string()),
            summary: None,
            servings: None,
            total_time_minutes: None,
            active_time_minutes: None,
            difficulty: None,
            image_url: None,
            published_at: None,
            content_hash: None,
            content_etag: None,
            content_last_modified: None,
            feed_entry_updated: None,
            locale: None,
            locale_source: None,
        },
    )
    .await
    .unwrap();

    tags::set_recipe_tags(
        pool,
        recipe.id,
        &["Desserts".to_string(), "chocolate".to_string()],
    )
    .await
    .unwrap();

    recipe.id
}

fn everything() -> SearchQuery {
    SearchQuery {
        q: String::new(),
        page: 1,
        limit: 10,
        locale: None,
    }
}

fn ids(index: &SearchIndex, query: &SearchQuery, filters: &SearchFilters) -> Vec<i64> {
    index
        .search_with(query, filters, SortOrder::Relevance, 100)
        .unwrap()
        .results
        .iter()
        .map(|r| r.recipe_id)
        .collect()
}

fn dessert_filter() -> SearchFilters {
    SearchFilters {
        tags: vec!["dessert".to_string()],
        ..SearchFilters::default()
    }
}

#[tokio::test]
async fn database_tags_are_found_by_tags_query_and_tags_filter() {
    let pool = pool().await;
    let recipe_id = seed(&pool).await;
    let dir = tempfile::tempdir().unwrap();
    let index = SearchIndex::new(dir.path()).unwrap();

    let indexed = reindex_recipes(&pool, &index, &[recipe_id]).await.unwrap();
    assert_eq!(indexed, 1);

    let by_query = SearchQuery {
        q: "tags:dessert".to_string(),
        ..everything()
    };
    assert_eq!(
        ids(&index, &by_query, &SearchFilters::default()),
        vec![recipe_id]
    );
    assert_eq!(
        ids(&index, &everything(), &dessert_filter()),
        vec![recipe_id]
    );

    let card = index
        .search_with(&everything(), &dessert_filter(), SortOrder::Relevance, 100)
        .unwrap()
        .results
        .remove(0);
    assert_eq!(card.feed_title.as_deref(), Some("Jane's Kitchen"));
}

#[tokio::test]
async fn reindexing_nothing_is_a_no_op() {
    let pool = pool().await;
    let dir = tempfile::tempdir().unwrap();
    let index = SearchIndex::new(dir.path()).unwrap();
    assert_eq!(reindex_recipes(&pool, &index, &[]).await.unwrap(), 0);
}

#[tokio::test]
async fn locked_writers_are_handed_out_one_at_a_time() {
    let dir = tempfile::tempdir().unwrap();
    let index = Arc::new(SearchIndex::new(dir.path()).unwrap());

    let first = index.locked_writer().await.unwrap();
    let second = {
        let index = index.clone();
        tokio::spawn(async move { index.locked_writer().await.map(|_| ()) })
    };

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(
        !second.is_finished(),
        "a second writer must wait for the first instead of failing on the index lock"
    );

    drop(first);
    second.await.unwrap().unwrap();
}
