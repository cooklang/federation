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
pub async fn list_feeds(
    pool: &DbPool,
    status: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<Feed>> {
    let feeds = if let Some(status) = status {
        sqlx::query_as::<_, Feed>(
            "SELECT * FROM feeds WHERE status = ? ORDER BY created_at DESC LIMIT ? OFFSET ?",
        )
        .bind(status)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, Feed>("SELECT * FROM feeds ORDER BY created_at DESC LIMIT ? OFFSET ?")
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?
    };

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
}
