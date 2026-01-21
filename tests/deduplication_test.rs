use federation::db::models::{NewFeed, NewRecipe};
use federation::db::{feeds, recipes};
use sqlx::SqlitePool;

#[tokio::test]
async fn test_duplicate_detection_by_hash() {
    // Create in-memory database
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("Failed to create in-memory database");

    // Run migrations
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    // Create two test feeds
    let feed1 = feeds::create_feed(
        &pool,
        &NewFeed {
            url: "https://example.com/feed1.xml".to_string(),
            title: Some("Test Feed 1".to_string()),
        },
    )
    .await
    .expect("Failed to create feed1");

    let feed2 = feeds::create_feed(
        &pool,
        &NewFeed {
            url: "https://example.com/feed2.xml".to_string(),
            title: Some("Test Feed 2".to_string()),
        },
    )
    .await
    .expect("Failed to create feed2");

    // Create identical recipe from two different feeds
    let content = "@flour{500%g}\n@sugar{200%g}\n\nMix ingredients.";
    let hash = recipes::calculate_content_hash("Chocolate Cake", Some(content));

    let recipe1 = NewRecipe {
        feed_id: feed1.id,
        external_id: "recipe1".to_string(),
        title: "Chocolate Cake".to_string(),
        source_url: Some("https://example.com/recipe1".to_string()),
        enclosure_url: "https://example.com/recipe1.cook".to_string(),
        content: Some(content.to_string()),
        summary: Some("A delicious chocolate cake".to_string()),
        servings: Some(8),
        total_time_minutes: Some(60),
        active_time_minutes: Some(30),
        difficulty: Some("medium".to_string()),
        image_url: None,
        published_at: None,
        content_hash: Some(hash.clone()),
        content_etag: None,
        content_last_modified: None,
        feed_entry_updated: None,
    };

    let recipe2 = NewRecipe {
        feed_id: feed2.id,
        external_id: "recipe2".to_string(),
        title: "Chocolate Cake".to_string(), // Same title
        source_url: Some("https://example.com/recipe2".to_string()),
        enclosure_url: "https://example.com/recipe2.cook".to_string(),
        content: Some(content.to_string()), // Same content
        summary: Some("A delicious chocolate cake".to_string()),
        servings: Some(8),
        total_time_minutes: Some(60),
        active_time_minutes: Some(30),
        difficulty: Some("medium".to_string()),
        image_url: None,
        published_at: None,
        content_hash: Some(hash.clone()),
        content_etag: None,
        content_last_modified: None,
        feed_entry_updated: None,
    };

    // Create both recipes
    let (r1, is_new1) = recipes::get_or_create_recipe(&pool, &recipe1)
        .await
        .expect("Failed to create recipe1");
    assert!(is_new1, "First recipe should be new");

    let (r2, is_new2) = recipes::get_or_create_recipe(&pool, &recipe2)
        .await
        .expect("Failed to create recipe2");
    assert!(is_new2, "Second recipe should be new");

    // They should have different IDs (different feeds)
    assert_ne!(
        r1.id, r2.id,
        "Recipes from different feeds should have different IDs"
    );

    // But the same content hash
    assert_eq!(
        r1.content_hash, r2.content_hash,
        "Recipes with identical content should have the same hash"
    );

    // Find duplicates by hash
    let duplicates = recipes::find_duplicate_recipes(&pool, &hash)
        .await
        .expect("Failed to query duplicates");

    // Should find both recipes
    assert_eq!(
        duplicates.len(),
        2,
        "Should find 2 recipes with the same content hash"
    );
    assert!(
        duplicates.iter().any(|r| r.id == r1.id),
        "Should include recipe1 in duplicates"
    );
    assert!(
        duplicates.iter().any(|r| r.id == r2.id),
        "Should include recipe2 in duplicates"
    );
}

#[tokio::test]
async fn test_different_recipes_have_different_hashes() {
    // Create in-memory database
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("Failed to create in-memory database");

    // Run migrations
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    // Create test feed
    let feed = feeds::create_feed(
        &pool,
        &NewFeed {
            url: "https://example.com/feed.xml".to_string(),
            title: Some("Test Feed".to_string()),
        },
    )
    .await
    .expect("Failed to create feed");

    // Create two different recipes
    let hash1 =
        recipes::calculate_content_hash("Chocolate Cake", Some("@flour{500%g}\n@sugar{200%g}"));
    let hash2 =
        recipes::calculate_content_hash("Vanilla Cake", Some("@flour{400%g}\n@sugar{300%g}"));

    let recipe1 = NewRecipe {
        feed_id: feed.id,
        external_id: "recipe1".to_string(),
        title: "Chocolate Cake".to_string(),
        source_url: None,
        enclosure_url: "https://example.com/recipe1.cook".to_string(),
        content: Some("@flour{500%g}\n@sugar{200%g}".to_string()),
        summary: None,
        servings: None,
        total_time_minutes: None,
        active_time_minutes: None,
        difficulty: None,
        image_url: None,
        published_at: None,
        content_hash: Some(hash1.clone()),
        content_etag: None,
        content_last_modified: None,
        feed_entry_updated: None,
    };

    let recipe2 = NewRecipe {
        feed_id: feed.id,
        external_id: "recipe2".to_string(),
        title: "Vanilla Cake".to_string(), // Different title
        source_url: None,
        enclosure_url: "https://example.com/recipe2.cook".to_string(),
        content: Some("@flour{400%g}\n@sugar{300%g}".to_string()), // Different content
        summary: None,
        servings: None,
        total_time_minutes: None,
        active_time_minutes: None,
        difficulty: None,
        image_url: None,
        published_at: None,
        content_hash: Some(hash2.clone()),
        content_etag: None,
        content_last_modified: None,
        feed_entry_updated: None,
    };

    // Hashes should be different
    assert_ne!(
        hash1, hash2,
        "Different recipes should have different content hashes"
    );

    // Create both recipes
    let (r1, _) = recipes::get_or_create_recipe(&pool, &recipe1)
        .await
        .expect("Failed to create recipe1");

    let (r2, _) = recipes::get_or_create_recipe(&pool, &recipe2)
        .await
        .expect("Failed to create recipe2");

    // Different recipes should have different content hashes
    assert_ne!(
        r1.content_hash, r2.content_hash,
        "Different recipes should have different content hashes"
    );

    // Verify they have different IDs
    assert_ne!(r1.id, r2.id);
}

#[tokio::test]
async fn test_find_recipe_by_content_hash() {
    // Create in-memory database
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("Failed to create in-memory database");

    // Run migrations
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    // Create test feed
    let feed = feeds::create_feed(
        &pool,
        &NewFeed {
            url: "https://example.com/feed.xml".to_string(),
            title: Some("Test Feed".to_string()),
        },
    )
    .await
    .expect("Failed to create feed");

    let content = "@flour{500%g}\n@sugar{200%g}";
    let hash = recipes::calculate_content_hash("Test Recipe", Some(content));

    // Before creating recipe, should find nothing
    let found = recipes::find_recipe_by_content_hash(&pool, &hash)
        .await
        .expect("Failed to query by hash");
    assert!(
        found.is_none(),
        "Should not find recipe before it's created"
    );

    // Create recipe
    let new_recipe = NewRecipe {
        feed_id: feed.id,
        external_id: "test-recipe".to_string(),
        title: "Test Recipe".to_string(),
        source_url: None,
        enclosure_url: "https://example.com/test.cook".to_string(),
        content: Some(content.to_string()),
        summary: None,
        servings: None,
        total_time_minutes: None,
        active_time_minutes: None,
        difficulty: None,
        image_url: None,
        published_at: None,
        content_hash: Some(hash.clone()),
        content_etag: None,
        content_last_modified: None,
        feed_entry_updated: None,
    };

    let (recipe, _) = recipes::get_or_create_recipe(&pool, &new_recipe)
        .await
        .expect("Failed to create recipe");

    // After creating, should find it
    let found = recipes::find_recipe_by_content_hash(&pool, &hash)
        .await
        .expect("Failed to query by hash");

    assert!(found.is_some(), "Should find recipe after creation");
    let found_recipe = found.unwrap();
    assert_eq!(
        found_recipe.id, recipe.id,
        "Found recipe should match created recipe"
    );
}
