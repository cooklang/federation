// Phase 2: Feed crawling module
// This module handles fetching and parsing RSS/Atom feeds

pub mod fetcher;
pub mod parser;
pub mod scheduler;

use crate::config::CrawlerConfig;
use crate::db::{self, models::*, DbPool};
use crate::error::{Error, Result};
use crate::utils::validation;
use fetcher::{Fetcher, RateLimiter};
use parser::{parse_feed, ParsedEntry};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

/// Main crawler that orchestrates feed fetching and parsing
pub struct Crawler {
    fetcher: Fetcher,
    rate_limiters: Arc<Mutex<HashMap<String, Arc<RateLimiter>>>>,
    config: CrawlerConfig,
}

impl Crawler {
    pub fn new(config: CrawlerConfig) -> Result<Self> {
        let fetcher = Fetcher::new(config.user_agent.clone(), config.max_feed_size)?;

        Ok(Self {
            fetcher,
            rate_limiters: Arc::new(Mutex::new(HashMap::new())),
            config,
        })
    }

    /// Crawl a single feed by URL
    pub async fn crawl_feed(&self, pool: &DbPool, feed_url: &str) -> Result<CrawlResult> {
        info!("Crawling feed: {}", feed_url);

        // Validate URL
        let url = validation::validate_url(feed_url)?;
        let domain = url
            .host_str()
            .ok_or_else(|| Error::Validation("Invalid URL: no host".to_string()))?;

        // Get or create feed in database
        let feed = match db::feeds::get_feed_by_url(pool, feed_url).await? {
            Some(feed) => feed,
            None => {
                let new_feed = NewFeed {
                    url: feed_url.to_string(),
                    title: None,
                };
                db::feeds::create_feed(pool, &new_feed).await?
            }
        };

        // Apply rate limiting per domain
        self.apply_rate_limit(domain).await;

        // Fetch feed with conditional requests
        let fetch_result = match self
            .fetcher
            .fetch_with_conditions(
                feed_url,
                feed.etag.as_deref(),
                feed.last_modified
                    .as_ref()
                    .map(|dt| dt.to_rfc3339())
                    .as_deref(),
            )
            .await
        {
            Ok(result) => result,
            Err(e) => {
                error!("Failed to fetch feed {}: {}", feed_url, e);
                db::feeds::increment_error_count(pool, feed.id).await?;
                db::feeds::update_feed_status(
                    pool,
                    feed.id,
                    "error",
                    feed.error_count + 1,
                    Some(e.to_string()),
                )
                .await?;
                return Err(e);
            }
        };

        // Parse feed
        let parsed_feed = match parse_feed(&fetch_result.content) {
            Ok(parsed) => parsed,
            Err(e) => {
                error!("Failed to parse feed {}: {}", feed_url, e);
                db::feeds::increment_error_count(pool, feed.id).await?;
                db::feeds::update_feed_status(
                    pool,
                    feed.id,
                    "error",
                    feed.error_count + 1,
                    Some(e.to_string()),
                )
                .await?;
                return Err(e);
            }
        };

        // Update feed metadata
        let last_modified = fetch_result
            .last_modified
            .and_then(|s| chrono::DateTime::parse_from_rfc2822(&s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc));

        db::feeds::update_feed_fetch_info(
            pool,
            feed.id,
            fetch_result.etag.as_deref(),
            last_modified,
        )
        .await?;

        // Update feed title and author if not set
        // Note: Using parameterized queries with bind() which are safe from SQL injection
        if feed.title.is_none() {
            if let Some(title) = &parsed_feed.title {
                sqlx::query("UPDATE feeds SET title = ? WHERE id = ?")
                    .bind(title)
                    .bind(feed.id)
                    .execute(pool)
                    .await?;
            }
        }
        if feed.author.is_none() {
            if let Some(author) = &parsed_feed.author {
                sqlx::query("UPDATE feeds SET author = ? WHERE id = ?")
                    .bind(author)
                    .bind(feed.id)
                    .execute(pool)
                    .await?;
            }
        }

        // Process entries
        let mut new_recipes = 0;
        let mut updated_recipes = 0;

        for entry in parsed_feed.entries {
            match self.process_entry(pool, feed.id, &entry).await {
                Ok(is_new) => {
                    if is_new {
                        new_recipes += 1;
                    } else {
                        updated_recipes += 1;
                    }
                }
                Err(e) => {
                    warn!("Failed to process entry {}: {}", entry.id, e);
                }
            }
        }

        // Mark feed as active (reset error count)
        db::feeds::update_feed_status(pool, feed.id, "active", 0, None).await?;

        info!(
            "Completed crawl of {}: {} new, {} updated",
            feed_url, new_recipes, updated_recipes
        );

        Ok(CrawlResult {
            feed_id: feed.id,
            new_recipes,
            updated_recipes,
        })
    }

    async fn process_entry(
        &self,
        pool: &DbPool,
        feed_id: i64,
        entry: &ParsedEntry,
    ) -> Result<bool> {
        // Skip entries without enclosure URL (no .cook file)
        let enclosure_url = entry
            .enclosure_url
            .as_ref()
            .ok_or_else(|| Error::Validation(format!("Entry {} has no enclosure URL", entry.id)))?;

        // Fetch the actual .cook file content from the enclosure URL
        let content = match self.fetch_recipe_content(enclosure_url).await {
            Ok(c) => Some(c),
            Err(e) => {
                warn!("Failed to fetch recipe content from {}: {}", enclosure_url, e);
                None
            }
        };

        // Calculate content hash for deduplication
        let content_hash = content
            .as_ref()
            .map(|c| db::recipes::calculate_content_hash(&entry.title, Some(c)));

        // Create or update recipe
        let new_recipe = NewRecipe {
            feed_id,
            external_id: entry.id.clone(),
            title: entry.title.clone(),
            source_url: entry.source_url.clone(),
            enclosure_url: enclosure_url.clone(),
            content,
            summary: entry.summary.clone(),
            servings: entry.metadata.servings,
            total_time_minutes: entry.metadata.total_time,
            active_time_minutes: entry.metadata.active_time,
            difficulty: entry.metadata.difficulty.clone(),
            image_url: entry.image_url.clone(),
            published_at: entry.published,
            content_hash,
        };

        let (recipe, is_new) = db::recipes::get_or_create_recipe(pool, &new_recipe).await?;

        // Add tags
        if !entry.tags.is_empty() {
            db::tags::clear_recipe_tags(pool, recipe.id).await?;
            db::tags::add_recipe_tags(pool, recipe.id, &entry.tags).await?;
        }

        debug!(
            "Processed recipe {}: {} ({})",
            recipe.id,
            recipe.title,
            if is_new { "new" } else { "updated" }
        );

        Ok(is_new)
    }

    async fn apply_rate_limit(&self, domain: &str) {
        let mut limiters = self.rate_limiters.lock().await;

        let limiter = limiters
            .entry(domain.to_string())
            .or_insert_with(|| Arc::new(RateLimiter::new(self.config.rate_limit)));

        let limiter = Arc::clone(limiter);
        drop(limiters);

        limiter.wait().await;
    }

    /// Fetch .cook file content for a recipe
    pub async fn fetch_recipe_content(&self, recipe_url: &str) -> Result<String> {
        debug!("Fetching recipe content: {}", recipe_url);

        // Validate URL
        validation::validate_url(recipe_url)?;

        // Fetch with size limit
        let fetcher = Fetcher::new(self.config.user_agent.clone(), self.config.max_recipe_size)?;

        let result = fetcher.fetch(recipe_url).await?;

        // Verify content type
        if let Some(content_type) = &result.content_type {
            if !content_type.contains("text/plain") && !content_type.contains("text/") {
                warn!("Unexpected content type for recipe: {}", content_type);
            }
        }

        Ok(result.content)
    }

    /// Fetch feed with conditional requests for scheduler
    pub async fn fetch_feed(
        &self,
        url: &str,
        etag: Option<&str>,
        last_modified: Option<&chrono::DateTime<chrono::Utc>>,
    ) -> Result<FetchedFeed> {
        let last_modified_str = last_modified.map(|dt| dt.to_rfc2822());

        match self
            .fetcher
            .fetch_with_conditions(url, etag, last_modified_str.as_deref())
            .await
        {
            Ok(result) => {
                let last_modified = result
                    .last_modified
                    .and_then(|s| chrono::DateTime::parse_from_rfc2822(&s).ok())
                    .map(|dt| dt.with_timezone(&chrono::Utc));

                Ok(FetchedFeed {
                    content: result.content,
                    etag: result.etag,
                    last_modified,
                    modified: true,
                })
            }
            Err(Error::FeedParse(msg)) if msg == "304 Not Modified" => Ok(FetchedFeed {
                content: String::new(),
                etag: None,
                last_modified: None,
                modified: false,
            }),
            Err(e) => Err(e),
        }
    }

    /// Parse feed content
    pub fn parse_feed(&self, content: &str) -> Result<parser::ParsedFeed> {
        parser::parse_feed(content)
    }

    /// Fetch recipe content (alias for scheduler)
    pub async fn fetch_recipe(&self, url: &str) -> Result<String> {
        self.fetch_recipe_content(url).await
    }
}

/// Feed fetch result for scheduler
#[derive(Debug)]
pub struct FetchedFeed {
    pub content: String,
    pub etag: Option<String>,
    pub last_modified: Option<chrono::DateTime<chrono::Utc>>,
    pub modified: bool,
}

#[derive(Debug)]
pub struct CrawlResult {
    pub feed_id: i64,
    pub new_recipes: usize,
    pub updated_recipes: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CrawlerConfig;

    #[test]
    fn test_crawler_creation() {
        let config = CrawlerConfig {
            interval_seconds: 3600,
            max_feed_size: 5_242_880,
            max_recipe_size: 1_048_576,
            rate_limit: 1,
            user_agent: "TestBot/1.0".to_string(),
        };

        let crawler = Crawler::new(config);
        assert!(crawler.is_ok());
    }
}
