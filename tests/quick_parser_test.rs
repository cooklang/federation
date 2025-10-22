// Quick test to verify parser integration
use federation::indexer::parse_cooklang_full;

#[test]
fn test_parser_quick() {
    let recipe_content = r#"
Mix @flour{2%cups} with @sugar{1%cup}.
Bake in #oven{} for ~{25%minutes}.
"#;

    let parsed = parse_cooklang_full(recipe_content).unwrap();

    println!("✅ Parser works!");
    println!("   - Ingredients: {}", parsed.ingredients.len());
    println!("   - Cookware: {}", parsed.cookware.len());
    println!("   - Timers: {}", parsed.timers.len());
    println!("   - Sections: {}", parsed.sections.len());

    assert!(parsed.ingredients.len() >= 2);
    assert!(parsed.cookware.len() >= 1);
    assert!(parsed.timers.len() >= 1);
    assert!(!parsed.sections.is_empty());

    // Verify JSON serialization works
    let json = serde_json::to_string(&parsed).unwrap();
    let deserialized: federation::indexer::ParsedRecipeData = serde_json::from_str(&json).unwrap();

    assert_eq!(deserialized.ingredients.len(), parsed.ingredients.len());
    println!("✅ Serialization works!");
}
