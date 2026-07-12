// Phase 2: Feed crawling module
// This module handles fetching and parsing RSS/Atom feeds

pub mod fetcher;
pub mod parser;
pub mod scheduler;

use crate::config::CrawlerConfig;
use crate::db::{self, models::*, DbPool};
use crate::error::{Error, Result};
use crate::indexer::parse_cooklang_full;
use crate::utils::validation;
use fetcher::{http_date, FetchOutcome, Fetcher, RateLimiter};
use parser::{parse_feed, ParsedEntry};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

/// Result of processing a single feed entry
#[derive(Debug)]
enum ProcessResult {
    /// New recipe was created
    New,
    /// Existing recipe was updated
    Updated,
    /// Recipe was skipped (no changes detected)
    Skipped,
}

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
                feed.last_modified.as_ref().map(http_date).as_deref(),
            )
            .await
        {
            Ok(FetchOutcome::Fetched(result)) => result,
            Ok(FetchOutcome::NotModified) => {
                // The feed is healthy, it just has nothing new for us.
                info!("Feed {} unchanged since last crawl (304)", feed_url);
                db::feeds::mark_feed_unchanged(pool, feed.id).await?;

                return Ok(CrawlResult {
                    feed_id: feed.id,
                    new_recipes: 0,
                    updated_recipes: 0,
                    skipped_recipes: 0,
                });
            }
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
        let mut skipped_recipes = 0;

        for entry in parsed_feed.entries {
            match self.process_entry(pool, feed.id, &entry).await {
                Ok(ProcessResult::New) => new_recipes += 1,
                Ok(ProcessResult::Updated) => updated_recipes += 1,
                Ok(ProcessResult::Skipped) => skipped_recipes += 1,
                Err(e) => {
                    warn!("Failed to process entry {}: {}", entry.id, e);
                }
            }
        }

        // Mark feed as active (reset error count)
        db::feeds::update_feed_status(pool, feed.id, "active", 0, None).await?;

        info!(
            "Completed crawl of {}: {} new, {} updated, {} skipped (cached)",
            feed_url, new_recipes, updated_recipes, skipped_recipes
        );

        Ok(CrawlResult {
            feed_id: feed.id,
            new_recipes,
            updated_recipes,
            skipped_recipes,
        })
    }

    async fn process_entry(
        &self,
        pool: &DbPool,
        feed_id: i64,
        entry: &ParsedEntry,
    ) -> Result<ProcessResult> {
        // Skip entries without enclosure URL (no .cook file)
        let enclosure_url = entry
            .enclosure_url
            .as_ref()
            .ok_or_else(|| Error::Validation(format!("Entry {} has no enclosure URL", entry.id)))?;

        // Check if recipe already exists
        let existing_recipe =
            db::recipes::find_by_feed_and_external_id(pool, feed_id, &entry.id).await?;

        // Determine if we need to fetch content based on entry's updated timestamp
        let should_fetch = match &existing_recipe {
            Some(recipe) => {
                // If entry has updated timestamp, compare with stored feed_entry_updated
                match (entry.updated, recipe.feed_entry_updated) {
                    (Some(entry_updated), Some(stored_updated)) => {
                        // Only fetch if entry has been updated since last fetch
                        if entry_updated > stored_updated {
                            debug!(
                                "Entry {} updated ({} > {}), will fetch",
                                entry.id, entry_updated, stored_updated
                            );
                            true
                        } else {
                            debug!(
                                "Entry {} unchanged ({} <= {}), skipping fetch",
                                entry.id, entry_updated, stored_updated
                            );
                            false
                        }
                    }
                    (Some(_entry_updated), None) => {
                        // First time we're tracking entry updated timestamp
                        debug!(
                            "Entry {} has updated timestamp, stored doesn't - will fetch",
                            entry.id
                        );
                        true
                    }
                    (None, _) => {
                        // Entry doesn't have updated timestamp, use conditional HTTP request
                        debug!(
                            "Entry {} has no updated timestamp, will use conditional fetch",
                            entry.id
                        );
                        true
                    }
                }
            }
            None => {
                // New recipe, definitely need to fetch
                debug!("Entry {} is new, will fetch", entry.id);
                true
            }
        };

        if !should_fetch {
            // Entry hasn't changed based on feed timestamp, skip fetch entirely
            return Ok(ProcessResult::Skipped);
        }

        // Extract domain for rate limiting
        let url = validation::validate_url(enclosure_url)?;
        let domain = url
            .host_str()
            .ok_or_else(|| Error::Validation("Invalid enclosure URL: no host".to_string()))?;

        // Apply rate limiting before fetching recipe content
        self.apply_rate_limit(domain).await;

        // Fetch content with conditional request if we have cached ETag/Last-Modified
        let (content, content_etag, content_last_modified) = match &existing_recipe {
            Some(recipe) => {
                // Use conditional request with stored caching headers
                let result = self
                    .fetcher
                    .fetch_with_conditions(
                        enclosure_url,
                        recipe.content_etag.as_deref(),
                        recipe
                            .content_last_modified
                            .as_ref()
                            .map(http_date)
                            .as_deref(),
                    )
                    .await;

                match result {
                    Ok(FetchOutcome::Fetched(fetched)) => {
                        debug!("Fetched updated content for {}", entry.id);
                        (Some(fetched.content), fetched.etag, fetched.last_modified)
                    }
                    Ok(FetchOutcome::NotModified) => {
                        // Content unchanged, just update the feed_entry_updated timestamp
                        debug!("Content not modified for {} (304)", entry.id);
                        db::recipes::update_feed_entry_timestamp(
                            pool,
                            recipe.id,
                            entry.updated.as_ref(),
                        )
                        .await?;
                        return Ok(ProcessResult::Skipped);
                    }
                    Err(e) => {
                        warn!(
                            "Failed to fetch recipe content from {}: {}",
                            enclosure_url, e
                        );
                        (None, None, None)
                    }
                }
            }
            None => {
                // New recipe, fetch without conditional headers
                match self
                    .fetcher
                    .fetch_with_conditions(enclosure_url, None, None)
                    .await
                {
                    Ok(FetchOutcome::Fetched(fetched)) => {
                        (Some(fetched.content), fetched.etag, fetched.last_modified)
                    }
                    Ok(FetchOutcome::NotModified) => {
                        // Shouldn't happen without conditional headers, but handle gracefully
                        warn!(
                            "Server returned 304 for {} despite no conditional headers",
                            enclosure_url
                        );
                        (None, None, None)
                    }
                    Err(e) => {
                        warn!(
                            "Failed to fetch recipe content from {}: {}",
                            enclosure_url, e
                        );
                        (None, None, None)
                    }
                }
            }
        };

        // Parse Last-Modified string to DateTime
        let content_last_modified_dt = content_last_modified
            .as_ref()
            .and_then(|s| chrono::DateTime::parse_from_rfc2822(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc));

        // Calculate content hash for deduplication
        let content_hash = content
            .as_ref()
            .map(|c| db::recipes::calculate_content_hash(&entry.title, Some(c)));

        let result = match existing_recipe {
            Some(recipe) => {
                // Update existing recipe with new content
                if let Some(ref content_str) = content {
                    db::recipes::update_recipe_with_content(
                        pool,
                        recipe.id,
                        content_str,
                        content_hash.as_deref(),
                        content_etag.as_deref(),
                        content_last_modified_dt.as_ref(),
                        entry.updated.as_ref(),
                        None,
                        None,
                    )
                    .await?;
                }

                // Update tags
                if !entry.tags.is_empty() {
                    db::tags::clear_recipe_tags(pool, recipe.id).await?;
                    db::tags::add_recipe_tags(pool, recipe.id, &entry.tags).await?;
                }

                debug!("Updated recipe {}: {}", recipe.id, recipe.title);
                ProcessResult::Updated
            }
            None => {
                // Determine image URL: prefer feed entry image, fallback to Cooklang metadata
                let metadata_image = content.as_ref().and_then(|c| {
                    parse_cooklang_full(c)
                        .ok()
                        .and_then(|parsed| parsed.metadata.and_then(|m| m.image))
                });
                let image_url = entry
                    .image_url
                    .clone()
                    .or(metadata_image)
                    .and_then(|img| resolve_image_url(&img, enclosure_url));

                // Create new recipe
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
                    image_url,
                    published_at: entry.published,
                    content_hash,
                    content_etag,
                    content_last_modified: content_last_modified_dt,
                    feed_entry_updated: entry.updated,
                    locale: None,
                    locale_source: None,
                };

                let (recipe, _) = db::recipes::get_or_create_recipe(pool, &new_recipe).await?;

                // Add tags
                if !entry.tags.is_empty() {
                    db::tags::clear_recipe_tags(pool, recipe.id).await?;
                    db::tags::add_recipe_tags(pool, recipe.id, &entry.tags).await?;
                }

                debug!("Created new recipe {}: {}", recipe.id, recipe.title);
                ProcessResult::New
            }
        };

        Ok(result)
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

    /// Parse feed content
    pub fn parse_feed(&self, content: &str) -> Result<parser::ParsedFeed> {
        parser::parse_feed(content)
    }
}

#[derive(Debug)]
pub struct CrawlResult {
    pub feed_id: i64,
    pub new_recipes: usize,
    pub updated_recipes: usize,
    pub skipped_recipes: usize,
}

use crate::utils::resolve_image_url;

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

    #[test]
    fn test_resolve_image_url_absolute() {
        let result = resolve_image_url(
            "https://example.com/images/photo.jpg",
            "https://example.com/recipes/cake.cook",
        );
        assert_eq!(
            result,
            Some("https://example.com/images/photo.jpg".to_string())
        );
    }

    #[test]
    fn test_resolve_image_url_relative_filename() {
        let result = resolve_image_url(
            "Lemon Drop.jpeg",
            "https://example.com/recipes/Lemon Drop.cook",
        );
        assert_eq!(
            result,
            Some("https://example.com/recipes/Lemon%20Drop.jpeg".to_string())
        );
    }

    #[test]
    fn test_resolve_image_url_relative_path() {
        let result = resolve_image_url(
            "../images/photo.jpg",
            "https://example.com/recipes/cake.cook",
        );
        assert_eq!(
            result,
            Some("https://example.com/images/photo.jpg".to_string())
        );
    }

    #[test]
    fn test_resolve_image_url_absolute_path() {
        let result =
            resolve_image_url("/images/photo.jpg", "https://example.com/recipes/cake.cook");
        assert_eq!(
            result,
            Some("https://example.com/images/photo.jpg".to_string())
        );
    }
}
