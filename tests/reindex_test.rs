use federation::cli::commands::reindex_feed;
use federation::db::models::{NewFeed, NewRecipe};
use federation::db::{feeds, recipes};
use sqlx::SqlitePool;

#[tokio::test]
async fn test_reindex_deletes_recipes_and_resets_feed() {
    // Create in-memory database
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("Failed to create in-memory database");

    // Run migrations
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    // Create test feed with caching headers set
    let feed = feeds::create_feed(
        &pool,
        &NewFeed {
            url: "https://example.com/feed.xml".to_string(),
            title: Some("Test Feed".to_string()),
        },
    )
    .await
    .expect("Failed to create feed");

    // Set some caching headers on the feed
    feeds::update_feed_fetch_info(
        &pool,
        feed.id,
        Some("etag-123"),
        Some(chrono::Utc::now()),
    )
    .await
    .expect("Failed to update feed fetch info");

    // Create some recipes
    for i in 1..=3 {
        let recipe = NewRecipe {
            feed_id: feed.id,
            external_id: format!("recipe-{}", i),
            title: format!("Test Recipe {}", i),
            source_url: None,
            enclosure_url: format!("https://example.com/recipe{}.cook", i),
            content: Some(format!("Recipe {} content", i)),
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
        };

        recipes::get_or_create_recipe(&pool, &recipe)
            .await
            .expect("Failed to create recipe");
    }

    // Verify recipes exist
    let recipe_count = recipes::count_recipes_by_feed(&pool, feed.id)
        .await
        .expect("Failed to count recipes");
    assert_eq!(recipe_count, 3, "Should have 3 recipes before reindex");

    // Call reindex (without crawling - just the delete/reset part)
    let deleted_count = reindex_feed(&pool, "https://example.com/feed.xml")
        .await
        .expect("Failed to reindex feed");

    assert_eq!(deleted_count, 3, "Should have deleted 3 recipes");

    // Verify recipes are deleted
    let recipe_count = recipes::count_recipes_by_feed(&pool, feed.id)
        .await
        .expect("Failed to count recipes after reindex");
    assert_eq!(recipe_count, 0, "Should have 0 recipes after reindex");

    // Verify feed caching headers are reset
    let updated_feed = feeds::get_feed_by_url(&pool, "https://example.com/feed.xml")
        .await
        .expect("Failed to get feed")
        .expect("Feed should still exist");

    assert!(updated_feed.etag.is_none(), "Feed etag should be reset");
    assert!(
        updated_feed.last_modified.is_none(),
        "Feed last_modified should be reset"
    );
    assert!(
        updated_feed.last_fetched_at.is_none(),
        "Feed last_fetched_at should be reset"
    );
}

#[tokio::test]
async fn test_reindex_returns_error_for_unknown_feed() {
    // Create in-memory database
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("Failed to create in-memory database");

    // Run migrations
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    // Try to reindex a feed that doesn't exist
    let result = reindex_feed(&pool, "https://nonexistent.com/feed.xml").await;

    assert!(result.is_err(), "Should return error for unknown feed");
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("not found"),
        "Error should mention feed not found"
    );
}
