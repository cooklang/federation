use federation::crawler::parser;

#[test]
fn test_parse_sample_feed() {
    let feed_content = include_str!("fixtures/sample_feed.xml");
    let result = parser::parse_feed(feed_content);

    assert!(result.is_ok());

    let feed = result.unwrap();
    assert_eq!(feed.title, Some("Jane's Recipe Collection".to_string()));
    assert_eq!(feed.author, Some("Jane Doe".to_string()));
    assert_eq!(feed.entries.len(), 2);

    // Check first entry
    let entry = &feed.entries[0];
    assert_eq!(entry.id, "recipe-chocolate-cookies");
    assert_eq!(entry.title, "Chocolate Chip Cookies");
    // Note: Custom cooklang namespace metadata parsing is simplified for now
    assert!(entry.tags.contains(&"dessert".to_string()));
    assert!(entry.tags.contains(&"cookies".to_string()));

    // Check second entry
    let entry = &feed.entries[1];
    assert_eq!(entry.id, "recipe-pasta-carbonara");
    assert_eq!(entry.title, "Spaghetti Carbonara");
}
