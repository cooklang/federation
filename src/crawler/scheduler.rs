use crate::{crawler::Crawler, db, Result};
use sqlx::SqlitePool;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::interval;
use tracing::{debug, error, info, warn};

/// Background scheduler for feed crawling
pub struct Scheduler {
    pool: SqlitePool,
    crawler: Arc<Crawler>,
    interval_seconds: u64,
}

impl Scheduler {
    /// Create a new scheduler
    pub fn new(pool: SqlitePool, crawler: Arc<Crawler>, interval_seconds: u64) -> Self {
        Self {
            pool,
            crawler,
            interval_seconds,
        }
    }

    /// Start the scheduler in the background
    pub fn start(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            info!(
                "Crawler scheduler started with interval: {}s",
                self.interval_seconds
            );

            let mut ticker = interval(Duration::from_secs(self.interval_seconds));

            loop {
                ticker.tick().await;

                debug!("Scheduler tick: checking feeds for updates");

                if let Err(e) = self.process_feeds().await {
                    error!("Error processing feeds: {}", e);
                }
            }
        })
    }

    /// Process all active feeds with batched pagination to avoid memory issues
    async fn process_feeds(&self) -> Result<()> {
        const BATCH_SIZE: i64 = 50;
        let mut offset = 0;
        let mut total_success = 0;
        let mut total_errors = 0;
        let mut total_feeds = 0;

        loop {
            // Fetch feeds in batches
            let feeds =
                db::feeds::list_feeds(&self.pool, Some("active"), BATCH_SIZE, offset).await?;

            if feeds.is_empty() {
                break;
            }

            let batch_size = feeds.len();
            total_feeds += batch_size;

            debug!(
                "Processing batch of {} feeds (offset: {})",
                batch_size, offset
            );

            let mut success_count = 0;
            let mut error_count = 0;

            for feed in feeds {
                match self.crawler.crawl_feed(&self.pool, &feed.url).await {
                    Ok(result) => {
                        success_count += 1;
                        debug!(
                            "Successfully processed feed {}: {} new, {} updated, {} skipped",
                            feed.url,
                            result.new_recipes,
                            result.updated_recipes,
                            result.skipped_recipes
                        );
                    }
                    Err(e) => {
                        error_count += 1;
                        warn!("Error processing feed {}: {}", feed.url, e);
                    }
                }
            }

            total_success += success_count;
            total_errors += error_count;

            info!(
                "Batch complete: {} success, {} errors (batch offset: {})",
                success_count, error_count, offset
            );

            offset += BATCH_SIZE;

            // If we got fewer feeds than the batch size, we've reached the end
            if (batch_size as i64) < BATCH_SIZE {
                break;
            }
        }

        info!(
            "Feed processing complete: {} total feeds, {} success, {} errors",
            total_feeds, total_success, total_errors
        );

        Ok(())
    }

    /// Clean up unused tags
    pub async fn cleanup_unused_tags(&self) -> Result<()> {
        info!("Starting cleanup of unused tags");

        let deleted = db::tags::delete_unused_tags(&self.pool).await?;

        info!("Deleted {} unused tags", deleted);

        Ok(())
    }

    /// Clean up unused ingredients
    pub async fn cleanup_unused_ingredients(&self) -> Result<()> {
        info!("Starting cleanup of unused ingredients");

        let deleted = db::ingredients::delete_unused_ingredients(&self.pool).await?;

        info!("Deleted {} unused ingredients", deleted);

        Ok(())
    }
}
