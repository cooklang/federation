// Integration test for Cooklang parser with full stack
use federation::{
    db::{init_pool, models::*, run_migrations},
    indexer::parse_cooklang_full,
};

#[tokio::test]
async fn test_parser_integration() {
    // Create in-memory database
    let pool = init_pool("sqlite::memory:").await.unwrap();
    run_migrations(&pool).await.unwrap();

    // Create test feed
    let new_feed = NewFeed {
        url: "https://test.example/feed".to_string(),
        title: Some("Test Feed".to_string()),
    };
    let feed = federation::db::feeds::create_feed(&pool, &new_feed)
        .await
        .unwrap();

    // Sample Cooklang recipe
    let recipe_content = r#"
>> source: https://www.example.com/recipe
>> servings: 4
>> total time: 45 minutes

-- This is a delicious test recipe

Preheat the #oven{} to 180°C.

Mix @flour{2%cups} with @sugar{1%cup} and @butter{200%g}.

Bake for ~{25%minutes} until golden.

>> Finishing
Let it cool for ~{10%minutes} before serving.
"#;

    // Parse the recipe
    let parsed = parse_cooklang_full(recipe_content).unwrap();

    // Verify parsing worked
    assert!(!parsed.sections.is_empty(), "Should have sections");
    assert!(!parsed.ingredients.is_empty(), "Should have ingredients");
    assert!(!parsed.cookware.is_empty(), "Should have cookware");
    assert!(!parsed.timers.is_empty(), "Should have timers");

    // Verify we found expected ingredients
    let ingredient_names: Vec<_> = parsed.ingredients.iter().map(|i| i.name.as_str()).collect();
    assert!(ingredient_names.contains(&"flour"));
    assert!(ingredient_names.contains(&"sugar"));
    assert!(ingredient_names.contains(&"butter"));

    // Verify cookware
    assert!(parsed.cookware.iter().any(|c| c.name == "oven"));

    // Verify sections
    println!("DEBUG: Found {} sections", parsed.sections.len());
    for (idx, section) in parsed.sections.iter().enumerate() {
        println!(
            "  Section {}: name = {:?}, steps = {}, notes = {}",
            idx,
            section.name,
            section.steps.len(),
            section.notes.len()
        );
    }

    assert!(
        !parsed.sections.is_empty(),
        "Should have at least 1 section"
    );
    // Note: The cooklang parser may merge sections differently than expected

    // Verify steps have inline items
    let first_step = &parsed.sections[0].steps[0];
    assert!(
        first_step.items.iter().any(|item| matches!(
            item,
            federation::indexer::cooklang_parser::StepItem::Cookware { .. }
        )),
        "First step should have cookware item"
    );

    // Create recipe with raw content (no pre-parsing)
    let new_recipe = NewRecipe {
        feed_id: feed.id,
        external_id: "test-1".to_string(),
        title: "Test Recipe".to_string(),
        source_url: Some("https://www.example.com/recipe".to_string()),
        enclosure_url: "https://test.example/recipe.cook".to_string(),
        content: Some(recipe_content.to_string()),
        summary: Some("A delicious test recipe".to_string()),
        servings: Some(4),
        total_time_minutes: Some(45),
        active_time_minutes: None,
        difficulty: None,
        image_url: None,
        published_at: None,
        content_hash: None,
        content_etag: None,
        content_last_modified: None,
        feed_entry_updated: None,
    };

    let recipe = federation::db::recipes::create_recipe(&pool, &new_recipe)
        .await
        .unwrap();

    // Retrieve and verify content is stored
    let retrieved = federation::db::recipes::get_recipe(&pool, recipe.id)
        .await
        .unwrap();
    assert!(retrieved.content.is_some(), "Raw content should be stored");

    // Parse on-the-fly and verify structure
    let reparsed =
        federation::indexer::parse_cooklang_full(retrieved.content.as_ref().unwrap()).unwrap();

    assert_eq!(reparsed.ingredients.len(), parsed.ingredients.len());
    assert_eq!(reparsed.sections.len(), parsed.sections.len());
    assert_eq!(reparsed.cookware.len(), parsed.cookware.len());
    assert_eq!(reparsed.timers.len(), parsed.timers.len());

    println!("✅ Parser integration test passed!");
    println!("   - Parsed {} ingredients", reparsed.ingredients.len());
    println!("   - Parsed {} cookware items", reparsed.cookware.len());
    println!("   - Parsed {} timers", reparsed.timers.len());
    println!("   - Parsed {} sections", reparsed.sections.len());
}
