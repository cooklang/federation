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

#[tokio::test]
async fn rebuilding_with_backfill_indexes_tags_and_feed_titles() {
    let pool = pool().await;
    let recipe_id = seed(&pool).await;
    let dir = tempfile::tempdir().unwrap();
    let index = SearchIndex::new(dir.path()).unwrap();

    let stats = federation::cli::commands::backfill_locales(&pool, &index, true)
        .await
        .unwrap();
    assert_eq!(stats.scanned, 1);

    let results = index
        .search_with(&everything(), &dessert_filter(), SortOrder::Relevance, 100)
        .unwrap();
    assert_eq!(results.results.len(), 1);
    assert_eq!(results.results[0].recipe_id, recipe_id);
    assert_eq!(
        results.results[0].feed_title.as_deref(),
        Some("Jane's Kitchen"),
        "a rebuild must fill the card's feed title"
    );
}

/// A recipe row with `content` and the given stored servings/time/difficulty,
/// like a GitHub recipe indexed before servings and time were extracted.
async fn seed_with_facts(
    pool: &DbPool,
    content: &str,
    servings: Option<i64>,
    total_time_minutes: Option<i64>,
    difficulty: Option<&str>,
) -> i64 {
    let feed = feeds::create_feed(
        pool,
        &NewFeed {
            url: "https://github.com/alice/recipes".to_string(),
            title: Some("alice/recipes".to_string()),
        },
    )
    .await
    .unwrap();

    recipes::create_recipe(
        pool,
        &NewRecipe {
            feed_id: feed.id,
            external_id: "Lentil Soup.cook".to_string(),
            title: "Lentil Soup".to_string(),
            source_url: None,
            enclosure_url: "https://example.com/Lentil%20Soup.cook".to_string(),
            content: Some(content.to_string()),
            summary: None,
            servings,
            total_time_minutes,
            active_time_minutes: None,
            difficulty: difficulty.map(str::to_string),
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
    .unwrap()
    .id
}

const LENTIL_SOUP: &str =
    "---\nservings: 2-4\ntime: 1h 15min\ndifficulty: Easy\n---\nSimmer @lentils{200%g}.\n";

#[tokio::test]
async fn rebuilding_fills_servings_time_and_difficulty_from_cooklang_metadata() {
    let pool = pool().await;
    let recipe_id = seed_with_facts(&pool, LENTIL_SOUP, None, None, None).await;
    let dir = tempfile::tempdir().unwrap();
    let index = SearchIndex::new(dir.path()).unwrap();

    let stats = federation::cli::commands::backfill_locales(&pool, &index, true)
        .await
        .unwrap();
    assert_eq!(stats.facts_filled, 1);

    let stored = recipes::get_recipe(&pool, recipe_id).await.unwrap();
    assert_eq!(stored.servings, Some(2));
    assert_eq!(stored.total_time_minutes, Some(75));
    assert_eq!(stored.difficulty.as_deref(), Some("easy"));

    let filters = SearchFilters {
        max_time: Some(90),
        min_servings: Some(2),
        difficulty: Some("easy".to_string()),
        ..SearchFilters::default()
    };
    assert_eq!(ids(&index, &everything(), &filters), vec![recipe_id]);
}

#[tokio::test]
async fn rebuilding_keeps_servings_time_and_difficulty_already_stored() {
    let pool = pool().await;
    let recipe_id = seed_with_facts(&pool, LENTIL_SOUP, Some(8), None, Some("hard")).await;
    let dir = tempfile::tempdir().unwrap();
    let index = SearchIndex::new(dir.path()).unwrap();

    let stats = federation::cli::commands::backfill_locales(&pool, &index, true)
        .await
        .unwrap();
    assert_eq!(stats.facts_filled, 1, "only the missing time was filled");

    let stored = recipes::get_recipe(&pool, recipe_id).await.unwrap();
    assert_eq!(stored.servings, Some(8));
    assert_eq!(stored.total_time_minutes, Some(75));
    assert_eq!(stored.difficulty.as_deref(), Some("hard"));

    // Running it again changes nothing.
    let again = federation::cli::commands::backfill_locales(&pool, &index, true)
        .await
        .unwrap();
    assert_eq!(again.facts_filled, 0);
}
