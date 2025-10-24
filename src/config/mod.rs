pub mod feeds;
pub mod sync;

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub database: DatabaseConfig,
    pub server: ServerConfig,
    pub crawler: CrawlerConfig,
    pub search: SearchConfig,
    pub pagination: PaginationConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
    pub min_connections: u32,
    pub connection_timeout_seconds: u64,
    pub idle_timeout_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub external_url: Option<String>,
    pub api_rate_limit: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlerConfig {
    pub interval_seconds: u64,
    pub max_feed_size: usize,
    pub max_recipe_size: usize,
    pub rate_limit: u64,
    pub user_agent: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchConfig {
    pub index_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginationConfig {
    pub api_max_limit: usize,
    pub web_default_limit: usize,
    pub feed_page_size: usize,
    pub max_search_results: usize,
    pub max_request_body_size: usize,
    pub max_pages: usize, // Maximum pages to prevent overflow
}

impl Settings {
    /// Load settings from environment variables
    pub fn from_env() -> Result<Self> {
        let database_url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "sqlite:./data/federation.db".to_string());

        let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port = std::env::var("PORT")
            .unwrap_or_else(|_| "3000".to_string())
            .parse()
            .map_err(|_| Error::Config("Invalid PORT value".to_string()))?;

        let crawler_interval = std::env::var("CRAWLER_INTERVAL")
            .unwrap_or_else(|_| "3600".to_string())
            .parse()
            .map_err(|_| Error::Config("Invalid CRAWLER_INTERVAL value".to_string()))?;

        let max_feed_size = std::env::var("MAX_FEED_SIZE")
            .unwrap_or_else(|_| "5242880".to_string())
            .parse()
            .map_err(|_| Error::Config("Invalid MAX_FEED_SIZE value".to_string()))?;

        let max_recipe_size = std::env::var("MAX_RECIPE_SIZE")
            .unwrap_or_else(|_| "1048576".to_string())
            .parse()
            .map_err(|_| Error::Config("Invalid MAX_RECIPE_SIZE value".to_string()))?;

        let rate_limit = std::env::var("RATE_LIMIT")
            .unwrap_or_else(|_| "1".to_string())
            .parse()
            .map_err(|_| Error::Config("Invalid RATE_LIMIT value".to_string()))?;

        let index_path = std::env::var("INDEX_PATH")
            .unwrap_or_else(|_| "./data/index".to_string())
            .into();

        let external_url = std::env::var("EXTERNAL_URL").ok();

        let api_rate_limit = std::env::var("API_RATE_LIMIT")
            .unwrap_or_else(|_| "100".to_string())
            .parse()
            .map_err(|_| Error::Config("Invalid API_RATE_LIMIT value".to_string()))?;

        let max_connections = std::env::var("DATABASE_MAX_CONNECTIONS")
            .unwrap_or_else(|_| "25".to_string())
            .parse()
            .map_err(|_| Error::Config("Invalid DATABASE_MAX_CONNECTIONS value".to_string()))?;

        let min_connections = std::env::var("DATABASE_MIN_CONNECTIONS")
            .unwrap_or_else(|_| "5".to_string())
            .parse()
            .map_err(|_| Error::Config("Invalid DATABASE_MIN_CONNECTIONS value".to_string()))?;

        let connection_timeout_seconds = std::env::var("DATABASE_CONNECTION_TIMEOUT")
            .unwrap_or_else(|_| "30".to_string())
            .parse()
            .map_err(|_| Error::Config("Invalid DATABASE_CONNECTION_TIMEOUT value".to_string()))?;

        let idle_timeout_seconds = std::env::var("DATABASE_IDLE_TIMEOUT")
            .unwrap_or_else(|_| "600".to_string())
            .parse()
            .map_err(|_| Error::Config("Invalid DATABASE_IDLE_TIMEOUT value".to_string()))?;

        let api_max_limit = std::env::var("API_MAX_LIMIT")
            .unwrap_or_else(|_| "100".to_string())
            .parse()
            .map_err(|_| Error::Config("Invalid API_MAX_LIMIT value".to_string()))?;

        let web_default_limit = std::env::var("WEB_DEFAULT_LIMIT")
            .unwrap_or_else(|_| "12".to_string())
            .parse()
            .map_err(|_| Error::Config("Invalid WEB_DEFAULT_LIMIT value".to_string()))?;

        let feed_page_size = std::env::var("FEED_PAGE_SIZE")
            .unwrap_or_else(|_| "20".to_string())
            .parse()
            .map_err(|_| Error::Config("Invalid FEED_PAGE_SIZE value".to_string()))?;

        let max_search_results = std::env::var("MAX_SEARCH_RESULTS")
            .unwrap_or_else(|_| "1000".to_string())
            .parse()
            .map_err(|_| Error::Config("Invalid MAX_SEARCH_RESULTS value".to_string()))?;

        let max_request_body_size = std::env::var("MAX_REQUEST_BODY_SIZE")
            .unwrap_or_else(|_| "10485760".to_string())
            .parse()
            .map_err(|_| Error::Config("Invalid MAX_REQUEST_BODY_SIZE value".to_string()))?;

        let max_pages = std::env::var("MAX_PAGES")
            .unwrap_or_else(|_| "10000".to_string())
            .parse()
            .map_err(|_| Error::Config("Invalid MAX_PAGES value".to_string()))?;

        Ok(Settings {
            database: DatabaseConfig {
                url: database_url,
                max_connections,
                min_connections,
                connection_timeout_seconds,
                idle_timeout_seconds,
            },
            server: ServerConfig {
                host,
                port,
                external_url,
                api_rate_limit,
            },
            crawler: CrawlerConfig {
                interval_seconds: crawler_interval,
                max_feed_size,
                max_recipe_size,
                rate_limit,
                user_agent: format!("Cooklang-Federation/{}", env!("CARGO_PKG_VERSION")),
            },
            search: SearchConfig { index_path },
            pagination: PaginationConfig {
                api_max_limit,
                web_default_limit,
                feed_page_size,
                max_search_results,
                max_request_body_size,
                max_pages,
            },
        })
    }

    /// Validate configuration
    pub fn validate(&self) -> Result<()> {
        if self.server.port == 0 {
            return Err(Error::Config("Port must be non-zero".to_string()));
        }

        if self.crawler.rate_limit == 0 {
            return Err(Error::Config("Rate limit must be non-zero".to_string()));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_settings_validation() {
        let mut settings = Settings {
            database: DatabaseConfig {
                url: "sqlite::memory:".to_string(),
                max_connections: 5,
                min_connections: 2,
                connection_timeout_seconds: 30,
                idle_timeout_seconds: 600,
            },
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 3000,
                external_url: None,
                api_rate_limit: 100,
            },
            crawler: CrawlerConfig {
                interval_seconds: 3600,
                max_feed_size: 5242880,
                max_recipe_size: 1048576,
                rate_limit: 1,
                user_agent: "test".to_string(),
            },
            search: SearchConfig {
                index_path: "/tmp/index".into(),
            },
            pagination: PaginationConfig {
                api_max_limit: 100,
                web_default_limit: 50,
                feed_page_size: 20,
                max_search_results: 1000,
                max_request_body_size: 10485760,
                max_pages: 10000,
            },
        };

        assert!(settings.validate().is_ok());

        settings.server.port = 0;
        assert!(settings.validate().is_err());
    }
}
