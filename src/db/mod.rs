pub mod feeds;
pub mod github;
pub mod ingredients;
pub mod models;
pub mod recipes;
pub mod tags;

use crate::config::DatabaseConfig;
use crate::error::Result;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{Pool, Sqlite, SqlitePool};
use std::path::Path;
use std::time::Duration;

pub type DbPool = Pool<Sqlite>;

/// Initialize database connection pool
pub async fn init_pool(database_url: &str) -> Result<DbPool> {
    // Create data directory if it doesn't exist (for SQLite)
    if database_url.starts_with("sqlite:") {
        if let Some(path) = database_url.strip_prefix("sqlite:") {
            if let Some(parent) = Path::new(path).parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
        }
    }

    let pool = SqlitePool::connect(database_url).await?;
    Ok(pool)
}

/// Initialize database connection pool with custom configuration
pub async fn init_pool_with_config(config: &DatabaseConfig) -> Result<DbPool> {
    // Create data directory if it doesn't exist (for SQLite)
    if config.url.starts_with("sqlite:") {
        if let Some(path) = config.url.strip_prefix("sqlite:") {
            if let Some(parent) = Path::new(path).parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
        }
    }

    let pool = SqlitePoolOptions::new()
        .max_connections(config.max_connections)
        .min_connections(config.min_connections)
        .acquire_timeout(Duration::from_secs(config.connection_timeout_seconds))
        .idle_timeout(Duration::from_secs(config.idle_timeout_seconds))
        .connect(&config.url)
        .await?;

    Ok(pool)
}

/// Run database migrations
pub async fn run_migrations(pool: &DbPool) -> Result<()> {
    sqlx::migrate!("./migrations").run(pool).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_init_pool() {
        let pool = init_pool("sqlite::memory:").await;
        assert!(pool.is_ok());
    }
}
