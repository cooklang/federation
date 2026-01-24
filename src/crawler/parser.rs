use crate::error::{Error, Result};
use chrono::{DateTime, Utc};
use feed_rs::parser;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedFeed {
    pub title: Option<String>,
    pub author: Option<String>,
    pub updated: Option<DateTime<Utc>>,
    pub entries: Vec<ParsedEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedEntry {
    pub id: String,
    pub title: String,
    pub summary: Option<String>,
    pub source_url: Option<String>,
    pub enclosure_url: Option<String>,
    pub image_url: Option<String>,
    pub published: Option<DateTime<Utc>>,
    pub updated: Option<DateTime<Utc>>,
    pub tags: Vec<String>,
    pub metadata: RecipeMetadata,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecipeMetadata {
    pub servings: Option<i64>,
    pub total_time: Option<i64>,
    pub active_time: Option<i64>,
    pub difficulty: Option<String>,
}

/// Parse an RSS or Atom feed
pub fn parse_feed(content: &str) -> Result<ParsedFeed> {
    let feed = parser::parse(content.as_bytes())
        .map_err(|e| Error::FeedParse(format!("Failed to parse feed: {e}")))?;

    debug!(
        "Parsed feed: {}",
        feed.title
            .as_ref()
            .map(|t| t.content.as_str())
            .unwrap_or("Untitled")
    );

    // Extract feed metadata
    let title = feed.title.map(|t| t.content);
    let author = feed
        .authors
        .first()
        .map(|a| a.name.clone())
        .or_else(|| feed.contributors.first().map(|c| c.name.clone()));

    let updated = feed.updated;

    // Extract cooklang:image URLs from raw XML (feed_rs doesn't parse custom namespaces)
    let cooklang_images = extract_cooklang_images(content);

    // Parse entries
    let entries: Vec<ParsedEntry> = feed
        .entries
        .into_iter()
        .filter_map(|entry| match parse_entry(entry, &cooklang_images) {
            Ok(parsed) => Some(parsed),
            Err(e) => {
                warn!("Failed to parse entry: {}", e);
                None
            }
        })
        .collect();

    Ok(ParsedFeed {
        title,
        author,
        updated,
        entries,
    })
}

fn parse_entry(
    entry: feed_rs::model::Entry,
    cooklang_images: &HashMap<String, String>,
) -> Result<ParsedEntry> {
    // Get entry ID
    let id = entry.id;

    // Get title
    let title = entry
        .title
        .map(|t| t.content)
        .ok_or_else(|| Error::FeedParse("Entry missing title".to_string()))?;

    // Get summary
    let summary = entry
        .summary
        .map(|s| s.content)
        .or_else(|| entry.content.and_then(|c| c.body));

    // Get source URL (link to the recipe page)
    let source_url = entry
        .links
        .iter()
        .find(|l| l.rel.as_deref() == Some("alternate") || l.rel.is_none())
        .map(|l| l.href.clone());

    // Get enclosure URL (link to .cook file)
    let enclosure_url = entry
        .links
        .iter()
        .find(|l| {
            l.rel.as_deref() == Some("enclosure")
                && (l.media_type.as_deref() == Some("text/plain") || l.href.ends_with(".cook"))
        })
        .map(|l| l.href.clone());

    // Get image URL: prefer cooklang:image from feed, then media elements, then image enclosure
    let image_url = cooklang_images
        .get(&id)
        .cloned()
        .or_else(|| {
            entry
                .media
                .first()
                .and_then(|m| {
                    m.content
                        .first()
                        .and_then(|c| c.url.as_ref().map(|u| u.to_string()))
                        .or_else(|| m.thumbnails.first().map(|t| t.image.uri.to_string()))
                })
        })
        // Fallback: check for image enclosure
        .or_else(|| {
            entry.links.iter().find_map(|l| {
                if l.rel.as_deref() == Some("enclosure") {
                    if let Some(ref media_type) = l.media_type {
                        if media_type.starts_with("image/") {
                            return Some(l.href.clone());
                        }
                    }
                }
                None
            })
        });

    // Get published date
    let published = entry.published;
    let updated = entry.updated;

    // Extract tags from categories
    let tags: Vec<String> = entry.categories.into_iter().map(|c| c.term).collect();

    // Parse Cooklang-specific metadata from extensions (placeholder for now)
    let metadata = RecipeMetadata::default();

    Ok(ParsedEntry {
        id,
        title,
        summary,
        source_url,
        enclosure_url,
        image_url,
        published,
        updated,
        tags,
        metadata,
    })
}

/// Extract cooklang:image URLs from raw feed XML, keyed by entry ID.
/// feed_rs doesn't parse custom namespace extensions, so we extract them with regex.
fn extract_cooklang_images(content: &str) -> HashMap<String, String> {
    let mut images = HashMap::new();

    // Match Atom entries: <entry>...</entry>
    let entry_re = Regex::new(r"(?s)<entry[^>]*>(.*?)</entry>").unwrap();
    let atom_id_re = Regex::new(r"<id>([^<]+)</id>").unwrap();

    for cap in entry_re.captures_iter(content) {
        let block = &cap[1];
        if let (Some(id_cap), Some(img)) = (
            atom_id_re.captures(block),
            extract_cooklang_image_from_block(block),
        ) {
            images.insert(id_cap[1].trim().to_string(), img);
        }
    }

    // Match RSS items: <item>...</item>
    let item_re = Regex::new(r"(?s)<item[^>]*>(.*?)</item>").unwrap();
    let guid_re = Regex::new(r"<guid[^>]*>([^<]+)</guid>").unwrap();

    for cap in item_re.captures_iter(content) {
        let block = &cap[1];
        if let (Some(id_cap), Some(img)) = (
            guid_re.captures(block),
            extract_cooklang_image_from_block(block),
        ) {
            images.insert(id_cap[1].trim().to_string(), img);
        }
    }

    images
}

fn extract_cooklang_image_from_block(block: &str) -> Option<String> {
    let re = Regex::new(r"<cooklang:image>([^<]+)</cooklang:image>").unwrap();
    re.captures(block)
        .map(|cap| cap[1].trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_atom_feed() {
        let atom = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Jane's Recipes</title>
  <author>
    <name>Jane Doe</name>
  </author>
  <updated>2025-01-01T00:00:00Z</updated>
  <entry>
    <id>recipe-1</id>
    <title>Chocolate Chip Cookies</title>
    <summary>Classic chocolate chip cookies</summary>
    <link rel="alternate" href="https://example.com/cookies"/>
    <link rel="enclosure" href="https://example.com/cookies.cook" type="text/plain"/>
    <published>2025-01-01T00:00:00Z</published>
    <category term="dessert"/>
    <category term="cookies"/>
  </entry>
</feed>"#;

        let result = parse_feed(atom);
        assert!(result.is_ok());

        let feed = result.unwrap();
        assert_eq!(feed.title, Some("Jane's Recipes".to_string()));
        assert_eq!(feed.author, Some("Jane Doe".to_string()));
        assert_eq!(feed.entries.len(), 1);

        let entry = &feed.entries[0];
        assert_eq!(entry.id, "recipe-1");
        assert_eq!(entry.title, "Chocolate Chip Cookies");
        assert_eq!(entry.tags, vec!["dessert", "cookies"]);
    }

    #[test]
    fn test_parse_rss_feed() {
        let rss = r#"<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0">
  <channel>
    <title>Jane's Recipes</title>
    <item>
      <guid>recipe-1</guid>
      <title>Chocolate Chip Cookies</title>
      <description>Classic chocolate chip cookies</description>
      <link>https://example.com/cookies</link>
      <enclosure url="https://example.com/cookies.cook" type="text/plain"/>
      <pubDate>Wed, 01 Jan 2025 00:00:00 +0000</pubDate>
      <category>dessert</category>
      <category>cookies</category>
    </item>
  </channel>
</rss>"#;

        let result = parse_feed(rss);
        assert!(result.is_ok());

        let feed = result.unwrap();
        assert_eq!(feed.title, Some("Jane's Recipes".to_string()));
        assert_eq!(feed.entries.len(), 1);
    }

    #[test]
    fn test_parse_invalid_feed() {
        let invalid = "not a valid feed";
        let result = parse_feed(invalid);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_atom_feed_with_cooklang_image() {
        let atom = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom" xmlns:cooklang="https://cooklang.org/feed/">
  <title>Cocktails</title>
  <entry>
    <id>https://example.com/r/manhattan</id>
    <title>The Manhattan Project</title>
    <link rel="alternate" href="https://example.com/r/manhattan"/>
    <link rel="enclosure" href="https://example.com/r/manhattan/cooklang" type="text/plain"/>
    <cooklang:recipe>
      <cooklang:image>https://images.example.com/manhattan.jpeg</cooklang:image>
    </cooklang:recipe>
  </entry>
</feed>"#;

        let result = parse_feed(atom);
        assert!(result.is_ok());

        let feed = result.unwrap();
        let entry = &feed.entries[0];
        assert_eq!(
            entry.image_url,
            Some("https://images.example.com/manhattan.jpeg".to_string())
        );
    }

    #[test]
    fn test_extract_cooklang_images() {
        let content = r#"
        <entry>
            <id>entry-1</id>
            <cooklang:recipe>
                <cooklang:image>https://cdn.example.com/photo1.jpg</cooklang:image>
            </cooklang:recipe>
        </entry>
        <entry>
            <id>entry-2</id>
        </entry>
        <entry>
            <id>entry-3</id>
            <cooklang:recipe>
                <cooklang:image>https://cdn.example.com/photo3.jpg</cooklang:image>
            </cooklang:recipe>
        </entry>"#;

        let images = extract_cooklang_images(content);
        assert_eq!(images.len(), 2);
        assert_eq!(
            images.get("entry-1"),
            Some(&"https://cdn.example.com/photo1.jpg".to_string())
        );
        assert_eq!(images.get("entry-2"), None);
        assert_eq!(
            images.get("entry-3"),
            Some(&"https://cdn.example.com/photo3.jpg".to_string())
        );
    }
}
