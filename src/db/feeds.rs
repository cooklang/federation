use crate::db::{models::*, DbPool};
use crate::error::{Error, Result};
use chrono::Utc;

/// Create a new feed
pub async fn create_feed(pool: &DbPool, new_feed: &NewFeed) -> Result<Feed> {
    let now = Utc::now();

    let feed = sqlx::query_as::<_, Feed>(
        r#"
        INSERT INTO feeds (url, title, status, error_count, created_at, updated_at)
        VALUES (?, ?, 'active', 0, ?, ?)
        RETURNING *
        "#,
    )
    .bind(&new_feed.url)
    .bind(&new_feed.title)
    .bind(now)
    .bind(now)
    .fetch_one(pool)
    .await?;

    Ok(feed)
}

/// Get feed by ID
pub async fn get_feed(pool: &DbPool, feed_id: i64) -> Result<Feed> {
    let feed = sqlx::query_as::<_, Feed>("SELECT * FROM feeds WHERE id = ?")
        .bind(feed_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| Error::NotFound(format!("Feed {feed_id} not found")))?;

    Ok(feed)
}

/// Get feed by URL
pub async fn get_feed_by_url(pool: &DbPool, url: &str) -> Result<Option<Feed>> {
    let feed = sqlx::query_as::<_, Feed>("SELECT * FROM feeds WHERE url = ?")
        .bind(url)
        .fetch_optional(pool)
        .await?;

    Ok(feed)
}

/// List all feeds
/// By default, excludes GitHub feeds (use exclude_github = false to include them)
pub async fn list_feeds(
    pool: &DbPool,
    status: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<Feed>> {
    list_feeds_with_filter(pool, status, limit, offset, true).await
}

/// List feeds with optional GitHub feed exclusion
pub async fn list_feeds_with_filter(
    pool: &DbPool,
    status: Option<&str>,
    limit: i64,
    offset: i64,
    exclude_github: bool,
) -> Result<Vec<Feed>> {
    let feeds = if exclude_github {
        // Exclude feeds that have a corresponding entry in github_feeds
        if let Some(status) = status {
            sqlx::query_as::<_, Feed>(
                r#"
                SELECT f.* FROM feeds f
                LEFT JOIN github_feeds gf ON f.id = gf.feed_id
                WHERE f.status = ? AND gf.id IS NULL
                ORDER BY f.created_at DESC
                LIMIT ? OFFSET ?
                "#,
            )
            .bind(status)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?
        } else {
            sqlx::query_as::<_, Feed>(
                r#"
                SELECT f.* FROM feeds f
                LEFT JOIN github_feeds gf ON f.id = gf.feed_id
                WHERE gf.id IS NULL
                ORDER BY f.created_at DESC
                LIMIT ? OFFSET ?
                "#,
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?
        }
    } else {
        // Include all feeds (original behavior)
        if let Some(status) = status {
            sqlx::query_as::<_, Feed>(
                "SELECT * FROM feeds WHERE status = ? ORDER BY created_at DESC LIMIT ? OFFSET ?",
            )
            .bind(status)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?
        } else {
            sqlx::query_as::<_, Feed>(
                "SELECT * FROM feeds ORDER BY created_at DESC LIMIT ? OFFSET ?",
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?
        }
    };

    Ok(feeds)
}

/// List the feeds the crawler should poll: everything except the ones that were
/// deliberately switched off.
///
/// `error` feeds are included on purpose. A fetch failure is usually transient
/// (server hiccup, timeout), so excluding them would make a single bad response
/// permanent - the feed would never be retried and would go stale forever.
/// A feed that recovers is flipped back to `active` by the crawler.
pub async fn list_crawlable_feeds(pool: &DbPool, limit: i64, offset: i64) -> Result<Vec<Feed>> {
    let feeds = sqlx::query_as::<_, Feed>(
        r#"
        SELECT f.* FROM feeds f
        LEFT JOIN github_feeds gf ON f.id = gf.feed_id
        WHERE f.status IN ('active', 'error') AND gf.id IS NULL
        ORDER BY f.created_at DESC
        LIMIT ? OFFSET ?
        "#,
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok(feeds)
}

/// Count feeds
pub async fn count_feeds(pool: &DbPool, status: Option<&str>) -> Result<i64> {
    let count = if let Some(status) = status {
        sqlx::query_scalar("SELECT COUNT(*) FROM feeds WHERE status = ?")
            .bind(status)
            .fetch_one(pool)
            .await?
    } else {
        sqlx::query_scalar("SELECT COUNT(*) FROM feeds")
            .fetch_one(pool)
            .await?
    };

    Ok(count)
}

/// Update feed status
pub async fn update_feed_status(
    pool: &DbPool,
    feed_id: i64,
    status: &str,
    error_count: i64,
    error_message: Option<String>,
) -> Result<()> {
    let now = Utc::now();

    sqlx::query(
        r#"
        UPDATE feeds
        SET status = ?, error_count = ?, error_message = ?, updated_at = ?
        WHERE id = ?
        "#,
    )
    .bind(status)
    .bind(error_count)
    .bind(error_message)
    .bind(now)
    .bind(feed_id)
    .execute(pool)
    .await?;

    Ok(())
}

/// Update feed metadata after crawling
pub async fn update_feed_metadata(pool: &DbPool, feed_id: i64, update: &UpdateFeed) -> Result<()> {
    let now = Utc::now();

    sqlx::query(
        r#"
        UPDATE feeds
        SET title = ?, author = ?, last_fetched_at = ?, last_modified = ?,
            etag = ?, error_count = ?, error_message = ?, updated_at = ?
        WHERE id = ?
        "#,
    )
    .bind(&update.title)
    .bind(&update.author)
    .bind(update.last_fetched_at)
    .bind(update.last_modified)
    .bind(&update.etag)
    .bind(update.error_count)
    .bind(&update.error_message)
    .bind(now)
    .bind(feed_id)
    .execute(pool)
    .await?;

    Ok(())
}

/// Update feed fetch time and ETag
pub async fn update_feed_fetch_info(
    pool: &DbPool,
    feed_id: i64,
    etag: Option<&str>,
    last_modified: Option<chrono::DateTime<Utc>>,
) -> Result<()> {
    let now = Utc::now();

    sqlx::query(
        r#"
        UPDATE feeds
        SET last_fetched_at = ?, etag = ?, last_modified = ?, updated_at = ?
        WHERE id = ?
        "#,
    )
    .bind(now)
    .bind(etag)
    .bind(last_modified)
    .bind(now)
    .bind(feed_id)
    .execute(pool)
    .await?;

    Ok(())
}

/// Record a successful poll that returned 304 Not Modified.
///
/// The feed is healthy and unchanged, so this clears any earlier error state but
/// leaves `etag`/`last_modified` untouched - those validators are what produced
/// the 304, and clearing them would make the next request unconditional.
pub async fn mark_feed_unchanged(pool: &DbPool, feed_id: i64) -> Result<()> {
    let now = Utc::now();

    sqlx::query(
        r#"
        UPDATE feeds
        SET last_fetched_at = ?, status = 'active', error_count = 0,
            error_message = NULL, updated_at = ?
        WHERE id = ?
        "#,
    )
    .bind(now)
    .bind(now)
    .bind(feed_id)
    .execute(pool)
    .await?;

    Ok(())
}

/// Increment feed error count
pub async fn increment_error_count(pool: &DbPool, feed_id: i64) -> Result<()> {
    sqlx::query("UPDATE feeds SET error_count = error_count + 1 WHERE id = ?")
        .bind(feed_id)
        .execute(pool)
        .await?;

    Ok(())
}

/// Delete a feed
pub async fn delete_feed(pool: &DbPool, feed_id: i64) -> Result<()> {
    sqlx::query("DELETE FROM feeds WHERE id = ?")
        .bind(feed_id)
        .execute(pool)
        .await?;

    Ok(())
}

/// Get feed with recipe count
pub async fn get_feed_with_count(pool: &DbPool, feed_id: i64) -> Result<FeedWithCount> {
    let feed = get_feed(pool, feed_id).await?;

    let recipe_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM recipes WHERE feed_id = ?")
        .bind(feed_id)
        .fetch_one(pool)
        .await?;

    Ok(FeedWithCount { feed, recipe_count })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{init_pool, run_migrations};

    #[tokio::test]
    async fn test_feed_crud() {
        let pool = init_pool("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();

        // Create
        let new_feed = NewFeed {
            url: "https://example.com/feed.xml".to_string(),
            title: Some("Test Feed".to_string()),
        };
        let feed = create_feed(&pool, &new_feed).await.unwrap();
        assert_eq!(feed.url, new_feed.url);

        // Get
        let retrieved = get_feed(&pool, feed.id).await.unwrap();
        assert_eq!(retrieved.id, feed.id);

        // List
        let feeds = list_feeds(&pool, None, 10, 0).await.unwrap();
        assert_eq!(feeds.len(), 1);

        // Count
        let count = count_feeds(&pool, None).await.unwrap();
        assert_eq!(count, 1);

        // Delete
        delete_feed(&pool, feed.id).await.unwrap();
        let count = count_feeds(&pool, None).await.unwrap();
        assert_eq!(count, 0);
    }

    async fn seed_feed(pool: &DbPool, url: &str, status: &str) -> Feed {
        let feed = create_feed(
            pool,
            &NewFeed {
                url: url.to_string(),
                title: None,
            },
        )
        .await
        .unwrap();

        update_feed_status(pool, feed.id, status, 3, Some("boom".to_string()))
            .await
            .unwrap();

        get_feed(pool, feed.id).await.unwrap()
    }

    /// A feed that failed once must be retried on the next scheduler tick.
    /// Previously the scheduler only listed `active` feeds, so `error` was a
    /// terminal state and the feed was never crawled again (issue #9).
    #[tokio::test]
    async fn list_crawlable_feeds_retries_errored_feeds_but_skips_disabled() {
        let pool = init_pool("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();

        seed_feed(&pool, "https://example.com/active.xml", "active").await;
        seed_feed(&pool, "https://example.com/errored.xml", "error").await;
        seed_feed(&pool, "https://example.com/disabled.xml", "disabled").await;

        let feeds = list_crawlable_feeds(&pool, 50, 0).await.unwrap();

        let urls: Vec<&str> = feeds.iter().map(|f| f.url.as_str()).collect();
        assert!(urls.contains(&"https://example.com/active.xml"));
        assert!(
            urls.contains(&"https://example.com/errored.xml"),
            "errored feeds must be retried, otherwise a single failure is permanent"
        );
        assert!(
            !urls.contains(&"https://example.com/disabled.xml"),
            "disabled feeds are switched off deliberately and must stay off"
        );
    }

    /// A 304 Not Modified means the feed is healthy and unchanged: record the
    /// fetch attempt, clear any previous error state, and keep the cached
    /// validators so the next conditional request still works.
    #[tokio::test]
    async fn mark_feed_unchanged_clears_error_state_and_keeps_validators() {
        let pool = init_pool("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();

        let feed = seed_feed(&pool, "https://example.com/feed.xml", "error").await;
        let modified = Utc::now();
        update_feed_fetch_info(&pool, feed.id, Some("\"etag-v1\""), Some(modified))
            .await
            .unwrap();

        mark_feed_unchanged(&pool, feed.id).await.unwrap();

        let feed = get_feed(&pool, feed.id).await.unwrap();
        assert_eq!(feed.status, "active");
        assert_eq!(feed.error_count, 0);
        assert_eq!(feed.error_message, None);
        assert_eq!(
            feed.etag.as_deref(),
            Some("\"etag-v1\""),
            "the cached ETag must survive a 304, or the next request can't be conditional"
        );
        assert!(feed.last_fetched_at.is_some());
    }
}
