use std::env;

/// GitHub integration configuration
#[derive(Debug, Clone)]
pub struct GitHubConfig {
    /// Optional GitHub personal access token for increased rate limits
    pub token: Option<String>,

    /// Update interval in seconds (default: 6 hours)
    pub update_interval_secs: u64,

    /// Rate limit buffer - reserve this many requests
    pub rate_limit_buffer: u32,

    /// Maximum file size to download (in bytes)
    pub max_file_size_bytes: u64,

    /// Recipe processing concurrency (default: 10)
    pub recipe_concurrency: usize,

    /// Base URL of the GitHub REST API (overridable for tests)
    pub api_base_url: String,

    /// Base URL for raw file downloads (overridable for tests)
    pub raw_base_url: String,
}

pub const DEFAULT_API_BASE_URL: &str = "https://api.github.com";
pub const DEFAULT_RAW_BASE_URL: &str = "https://raw.githubusercontent.com";

impl GitHubConfig {
    /// Create a new GitHubConfig from environment variables
    pub fn from_env() -> Self {
        Self {
            token: env::var("GITHUB_TOKEN").ok(),
            update_interval_secs: env::var("GITHUB_UPDATE_INTERVAL")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(21600), // 6 hours
            rate_limit_buffer: env::var("GITHUB_RATE_LIMIT_BUFFER")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(500),
            max_file_size_bytes: env::var("GITHUB_MAX_FILE_SIZE")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1_048_576), // 1MB
            recipe_concurrency: env::var("RECIPE_CONCURRENCY")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(10),
            api_base_url: DEFAULT_API_BASE_URL.to_string(),
            raw_base_url: DEFAULT_RAW_BASE_URL.to_string(),
        }
    }

    /// Check if GitHub integration is enabled
    pub fn is_enabled(&self) -> bool {
        // GitHub integration can work without token, just with lower rate limits
        true
    }

    /// Get the base API URL
    pub fn api_base_url(&self) -> &str {
        &self.api_base_url
    }

    /// URL of a file's raw content on the default branch
    pub fn raw_url(&self, owner: &str, repo: &str, branch: &str, path: &str) -> String {
        format!("{}/{owner}/{repo}/{branch}/{path}", self.raw_base_url)
    }
}

impl Default for GitHubConfig {
    fn default() -> Self {
        Self {
            token: None,
            update_interval_secs: 21600, // 6 hours
            rate_limit_buffer: 500,
            max_file_size_bytes: 1_048_576, // 1MB
            recipe_concurrency: 10,
            api_base_url: DEFAULT_API_BASE_URL.to_string(),
            raw_base_url: DEFAULT_RAW_BASE_URL.to_string(),
        }
    }
}
