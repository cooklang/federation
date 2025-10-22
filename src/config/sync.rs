use crate::config::feeds::{FeedConfig, FeedEntry, FeedType};
use crate::db::{self, models::*, DbPool};
use crate::error::Result;
use crate::github::parse_repository_url;
use chrono::Utc;
use std::collections::HashMap;
use tracing::{debug, info, warn};

/// Report of feed synchronization results
#[derive(Debug, Clone)]
pub struct SyncReport {
    pub added: usize,
    pub updated: usize,
    pub disabled: usize,
    pub re_enabled: usize,
    pub unchanged: usize,
    pub errors: Vec<String>,
}

impl Default for SyncReport {
    fn default() -> Self {
        Self::new()
    }
}

impl SyncReport {
    pub fn new() -> Self {
        Self {
            added: 0,
            updated: 0,
            disabled: 0,
            re_enabled: 0,
            unchanged: 0,
            errors: Vec::new(),
        }
    }

    pub fn log_summary(&self) {
        info!(
            "Feed sync completed: {} added, {} updated, {} disabled, {} re-enabled, {} unchanged, {} errors",
            self.added, self.updated, self.disabled, self.re_enabled, self.unchanged, self.errors.len()
        );

        if !self.errors.is_empty() {
            warn!("Sync errors:");
            for error in &self.errors {
                warn!("  - {}", error);
            }
        }
    }
}

/// Synchronize feeds from configuration to database
pub async fn sync_feeds_from_config(pool: &DbPool, config: &FeedConfig) -> Result<SyncReport> {
    let mut report = SyncReport::new();

    info!(
        "Starting feed sync: {} total feeds, {} enabled",
        config.total_feeds(),
        config.enabled_count()
    );

    // Load all existing feeds from database
    let existing_feeds = db::feeds::list_feeds(pool, None, 10000, 0).await?;
    let mut existing_by_url: HashMap<String, Feed> = existing_feeds
        .into_iter()
        .map(|f| (f.url.clone(), f))
        .collect();

    debug!(
        "Loaded {} existing feeds from database",
        existing_by_url.len()
    );

    // Process each feed in the config
    for feed_entry in &config.feeds {
        match sync_feed(pool, feed_entry, &mut existing_by_url, &mut report).await {
            Ok(_) => {}
            Err(e) => {
                let error_msg = format!("Failed to sync feed '{}': {}", feed_entry.url, e);
                warn!("{}", error_msg);
                report.errors.push(error_msg);
            }
        }
    }

    // Disable feeds that are no longer in the config
    let config_urls: HashMap<String, &FeedEntry> =
        config.feeds.iter().map(|f| (f.url.clone(), f)).collect();

    for (url, feed) in existing_by_url {
        // Only disable feeds that came from config (source='config')
        // and are currently active
        if feed.status == "active" && !config_urls.contains_key(&url) {
            match disable_feed(pool, &feed).await {
                Ok(_) => {
                    info!("Disabled feed no longer in config: {}", url);
                    report.disabled += 1;
                }
                Err(e) => {
                    let error_msg = format!("Failed to disable feed '{url}': {e}");
                    warn!("{}", error_msg);
                    report.errors.push(error_msg);
                }
            }
        }
    }

    report.log_summary();
    Ok(report)
}

/// Synchronize a single feed entry
async fn sync_feed(
    pool: &DbPool,
    feed_entry: &FeedEntry,
    existing_by_url: &mut HashMap<String, Feed>,
    report: &mut SyncReport,
) -> Result<()> {
    match existing_by_url.remove(&feed_entry.url) {
        Some(existing_feed) => {
            // Feed exists, update if needed
            sync_existing_feed(pool, feed_entry, &existing_feed, report).await?;
        }
        None => {
            // New feed, create it
            sync_new_feed(pool, feed_entry, report).await?;
        }
    }

    Ok(())
}

/// Sync a new feed (doesn't exist in database)
async fn sync_new_feed(
    pool: &DbPool,
    feed_entry: &FeedEntry,
    report: &mut SyncReport,
) -> Result<()> {
    debug!("Creating new feed: {}", feed_entry.url);

    // Create feed entry
    let new_feed = NewFeed {
        url: feed_entry.url.clone(),
        title: Some(feed_entry.title.clone()),
    };

    let status = if feed_entry.enabled {
        "active"
    } else {
        "disabled"
    };

    let now = Utc::now();
    let feed = sqlx::query_as::<_, Feed>(
        r#"
        INSERT INTO feeds (url, title, status, source, error_count, created_at, updated_at)
        VALUES (?, ?, ?, 'config', 0, ?, ?)
        RETURNING *
        "#,
    )
    .bind(&new_feed.url)
    .bind(&new_feed.title)
    .bind(status)
    .bind(now)
    .bind(now)
    .fetch_one(pool)
    .await?;

    // If GitHub feed, create github_feeds entry
    if feed_entry.feed_type == FeedType::GitHub {
        create_github_feed_entry(pool, &feed, feed_entry).await?;
    }

    info!(
        "Created new {} feed: {} (id: {})",
        if feed_entry.feed_type == FeedType::GitHub {
            "GitHub"
        } else {
            "Web"
        },
        feed_entry.title,
        feed.id
    );

    report.added += 1;
    Ok(())
}

/// Sync an existing feed (already exists in database)
async fn sync_existing_feed(
    pool: &DbPool,
    feed_entry: &FeedEntry,
    existing_feed: &Feed,
    report: &mut SyncReport,
) -> Result<()> {
    let mut updated = false;

    // Check if title needs updating
    if existing_feed.title.as_ref() != Some(&feed_entry.title) {
        debug!(
            "Updating feed title: {} -> {}",
            existing_feed
                .title
                .as_ref()
                .unwrap_or(&"(none)".to_string()),
            feed_entry.title
        );
        updated = true;
    }

    // Check if status needs updating (enabled/disabled changed)
    let desired_status = if feed_entry.enabled {
        "active"
    } else {
        "disabled"
    };
    let status_changed = existing_feed.status != desired_status;

    if status_changed {
        debug!(
            "Feed status changed: {} -> {}",
            existing_feed.status, desired_status
        );
        updated = true;

        if feed_entry.enabled && existing_feed.status == "disabled" {
            report.re_enabled += 1;
        }
    }

    if updated {
        let now = Utc::now();
        sqlx::query(
            r#"
            UPDATE feeds
            SET title = ?, status = ?, source = 'config', updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(&feed_entry.title)
        .bind(desired_status)
        .bind(now)
        .bind(existing_feed.id)
        .execute(pool)
        .await?;

        info!(
            "Updated feed: {} (id: {})",
            feed_entry.title, existing_feed.id
        );

        // Don't count re-enabled feeds in the updated count
        if !status_changed || !feed_entry.enabled {
            report.updated += 1;
        }
    } else {
        report.unchanged += 1;
    }

    // Ensure GitHub feed entry exists and is up to date
    if feed_entry.feed_type == FeedType::GitHub {
        if let Some(github_feed) = db::github::get_github_feed_by_url(pool, &feed_entry.url).await?
        {
            // Check if branch needs updating
            let desired_branch = feed_entry
                .branch
                .clone()
                .unwrap_or_else(|| "main".to_string());

            if github_feed.default_branch != desired_branch {
                debug!(
                    "Updating GitHub feed branch: {} -> {}",
                    github_feed.default_branch, desired_branch
                );
                db::github::update_github_feed_branch(pool, github_feed.id, &desired_branch)
                    .await?;
            }
        } else {
            // Create missing GitHub feed entry
            create_github_feed_entry(pool, existing_feed, feed_entry).await?;
            debug!("Created missing GitHub feed entry for: {}", feed_entry.url);
        }
    }

    Ok(())
}

/// Disable a feed that's no longer in the config
async fn disable_feed(pool: &DbPool, feed: &Feed) -> Result<()> {
    let now = Utc::now();
    sqlx::query(
        r#"
        UPDATE feeds
        SET status = 'disabled', source = 'disabled', updated_at = ?
        WHERE id = ?
        "#,
    )
    .bind(now)
    .bind(feed.id)
    .execute(pool)
    .await?;

    Ok(())
}

/// Create a GitHub feed entry
async fn create_github_feed_entry(
    pool: &DbPool,
    feed: &Feed,
    feed_entry: &FeedEntry,
) -> Result<()> {
    let repo_info = parse_repository_url(&feed_entry.url)?;

    // Use branch from config if specified, otherwise default to "main"
    let default_branch = feed_entry
        .branch
        .clone()
        .unwrap_or_else(|| "main".to_string());

    let new_github_feed = NewGitHubFeed {
        feed_id: feed.id,
        repository_url: feed_entry.url.clone(),
        owner: repo_info.owner.clone(),
        repo_name: repo_info.repo.clone(),
        default_branch,
    };

    db::github::create_github_feed(pool, &new_github_feed).await?;
    debug!(
        "Created GitHub feed entry for {}/{}",
        repo_info.owner, repo_info.repo
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::feeds::FeedConfig;
    use crate::db::{init_pool, run_migrations};
    use std::io::Write;
    use tempfile::NamedTempFile;

    async fn create_test_pool() -> DbPool {
        let pool = init_pool("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        pool
    }

    fn create_test_config(content: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file.flush().unwrap();
        file
    }

    #[tokio::test]
    async fn test_sync_new_feeds() {
        let pool = create_test_pool().await;

        let config_content = r#"
version: 1
feeds:
  - url: "https://example.com/feed.xml"
    title: "Example Feed"
    feed_type: web
    enabled: true
    tags: []
    added_by: "@tester"
    added_at: "2025-10-13"
"#;

        let file = create_test_config(config_content);
        let config = FeedConfig::from_file(file.path()).unwrap();

        let report = sync_feeds_from_config(&pool, &config).await.unwrap();

        assert_eq!(report.added, 1);
        assert_eq!(report.updated, 0);
        assert_eq!(report.disabled, 0);
        assert_eq!(report.unchanged, 0);

        // Verify feed was created
        let feeds = db::feeds::list_feeds(&pool, None, 10, 0).await.unwrap();
        assert_eq!(feeds.len(), 1);
        assert_eq!(feeds[0].url, "https://example.com/feed.xml");
        assert_eq!(feeds[0].status, "active");
    }

    #[tokio::test]
    async fn test_sync_idempotent() {
        let pool = create_test_pool().await;

        let config_content = r#"
version: 1
feeds:
  - url: "https://example.com/feed.xml"
    title: "Example Feed"
    feed_type: web
    enabled: true
    tags: []
    added_by: "@tester"
    added_at: "2025-10-13"
"#;

        let file = create_test_config(config_content);
        let config = FeedConfig::from_file(file.path()).unwrap();

        // First sync
        let report1 = sync_feeds_from_config(&pool, &config).await.unwrap();
        assert_eq!(report1.added, 1);

        // Second sync - should be unchanged
        let report2 = sync_feeds_from_config(&pool, &config).await.unwrap();
        assert_eq!(report2.added, 0);
        assert_eq!(report2.unchanged, 1);
    }

    #[tokio::test]
    async fn test_sync_disable_removed_feeds() {
        let pool = create_test_pool().await;

        // First config with two feeds
        let config_content1 = r#"
version: 1
feeds:
  - url: "https://example1.com/feed.xml"
    title: "Feed 1"
    feed_type: web
    enabled: true
    tags: []
    added_by: "@tester"
    added_at: "2025-10-13"
  - url: "https://example2.com/feed.xml"
    title: "Feed 2"
    feed_type: web
    enabled: true
    tags: []
    added_by: "@tester"
    added_at: "2025-10-13"
"#;

        let file1 = create_test_config(config_content1);
        let config1 = FeedConfig::from_file(file1.path()).unwrap();
        sync_feeds_from_config(&pool, &config1).await.unwrap();

        // Second config with one feed removed
        let config_content2 = r#"
version: 1
feeds:
  - url: "https://example1.com/feed.xml"
    title: "Feed 1"
    feed_type: web
    enabled: true
    tags: []
    added_by: "@tester"
    added_at: "2025-10-13"
"#;

        let file2 = create_test_config(config_content2);
        let config2 = FeedConfig::from_file(file2.path()).unwrap();
        let report = sync_feeds_from_config(&pool, &config2).await.unwrap();

        assert_eq!(report.disabled, 1);

        // Verify second feed is disabled
        let feed2 = db::feeds::get_feed_by_url(&pool, "https://example2.com/feed.xml")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(feed2.status, "disabled");
    }

    #[tokio::test]
    async fn test_sync_github_feed() {
        let pool = create_test_pool().await;

        let config_content = r#"
version: 1
feeds:
  - url: "https://github.com/owner/repo"
    title: "GitHub Repo"
    feed_type: github
    enabled: true
    tags: []
    added_by: "@tester"
    added_at: "2025-10-13"
"#;

        let file = create_test_config(config_content);
        let config = FeedConfig::from_file(file.path()).unwrap();

        let report = sync_feeds_from_config(&pool, &config).await.unwrap();

        assert_eq!(report.added, 1);

        // Verify feed was created
        let feed = db::feeds::get_feed_by_url(&pool, "https://github.com/owner/repo")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(feed.status, "active");

        // Verify GitHub feed entry was created
        let github_feed =
            db::github::get_github_feed_by_url(&pool, "https://github.com/owner/repo")
                .await
                .unwrap()
                .unwrap();
        assert_eq!(github_feed.owner, "owner");
        assert_eq!(github_feed.repo_name, "repo");
    }
}
