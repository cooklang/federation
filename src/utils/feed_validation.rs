// Feed validation utilities

use crate::error::{Error, Result};
use crate::utils::validation::validate_url;
use reqwest::Client;
use std::time::Duration;

/// Information about a validated feed
#[derive(Debug, Clone)]
pub struct FeedInfo {
    pub title: String,
    pub feed_type: String,
    pub entry_count: usize,
    pub sample_entries: Vec<String>,
}

/// Validate a feed URL by fetching and parsing it
pub async fn validate_feed_url(url: &str) -> Result<FeedInfo> {
    // First validate the URL format and security
    validate_url(url)?;

    // Fetch the feed
    let client = Client::builder()
        .user_agent("Cooklang-Federation-Validator/1.0")
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(Error::Http)?;

    let response = client.get(url).send().await.map_err(Error::Http)?;

    if !response.status().is_success() {
        return Err(Error::FeedParse(format!(
            "HTTP error: {}",
            response.status()
        )));
    }

    let content = response.text().await.map_err(Error::Http)?;

    // Parse the feed
    let feed = feed_rs::parser::parse(content.as_bytes())
        .map_err(|e| Error::FeedParse(format!("Failed to parse feed: {e}")))?;

    // Determine feed type
    let feed_type = match feed.feed_type {
        feed_rs::model::FeedType::Atom => "Atom".to_string(),
        feed_rs::model::FeedType::RSS0 => "RSS 0.x".to_string(),
        feed_rs::model::FeedType::RSS1 => "RSS 1.0".to_string(),
        feed_rs::model::FeedType::RSS2 => "RSS 2.0".to_string(),
        feed_rs::model::FeedType::JSON => "JSON Feed".to_string(),
    };

    // Extract title
    let title = feed
        .title
        .map(|t| t.content)
        .unwrap_or_else(|| "Untitled Feed".to_string());

    // Get entry count and sample titles
    let entry_count = feed.entries.len();
    let sample_entries: Vec<String> = feed
        .entries
        .iter()
        .take(3)
        .filter_map(|e| e.title.as_ref().map(|t| t.content.clone()))
        .collect();

    Ok(FeedInfo {
        title,
        feed_type,
        entry_count,
        sample_entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_validate_feed_url_invalid_url() {
        let result = validate_feed_url("not-a-url").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_validate_feed_url_localhost_blocked() {
        let result = validate_feed_url("http://localhost/feed.xml").await;
        assert!(result.is_err());
    }
}
