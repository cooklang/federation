use chrono::{DateTime, Utc};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, warn};

/// Rate limiter for GitHub API
#[derive(Clone)]
pub struct RateLimiter {
    state: Arc<RwLock<RateLimitState>>,
    buffer: u32,
}

#[derive(Debug, Clone)]
struct RateLimitState {
    /// Total rate limit
    limit: u32,

    /// Remaining requests
    remaining: u32,

    /// Unix timestamp when rate limit resets
    reset_at: i64,
}

impl RateLimiter {
    /// Create a new rate limiter with a buffer
    pub fn new(buffer: u32) -> Self {
        Self {
            state: Arc::new(RwLock::new(RateLimitState {
                limit: 60, // Default for unauthenticated requests
                remaining: 60,
                reset_at: Utc::now().timestamp() + 3600,
            })),
            buffer,
        }
    }

    /// Update rate limit from GitHub API response headers
    pub async fn update_from_headers(&self, headers: &reqwest::header::HeaderMap) {
        let mut state = self.state.write().await;

        if let Some(limit) = headers
            .get("x-ratelimit-limit")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse().ok())
        {
            state.limit = limit;
        }

        if let Some(remaining) = headers
            .get("x-ratelimit-remaining")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse().ok())
        {
            state.remaining = remaining;
        }

        if let Some(reset) = headers
            .get("x-ratelimit-reset")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse().ok())
        {
            state.reset_at = reset;
        }

        debug!(
            "Rate limit updated: {}/{} (resets at {})",
            state.remaining, state.limit, state.reset_at
        );
    }

    /// Check if we should wait before making the next request
    pub async fn should_wait(&self) -> bool {
        let state = self.state.read().await;
        // Use the minimum of buffer or 10% of limit to handle low rate limits
        let threshold = std::cmp::min(self.buffer, (state.limit / 10).max(5));
        state.remaining <= threshold
    }

    /// Wait if necessary before making a request
    pub async fn wait_if_needed(&self) {
        let should_wait = self.should_wait().await;

        if should_wait {
            let state = self.state.read().await;
            let now = Utc::now().timestamp();

            if now < state.reset_at {
                let wait_secs = (state.reset_at - now) as u64;
                warn!(
                    "Rate limit approaching ({}/{}), waiting {} seconds until reset",
                    state.remaining, state.limit, wait_secs
                );
                drop(state); // Release the lock before sleeping
                tokio::time::sleep(tokio::time::Duration::from_secs(wait_secs)).await;
            }
        }
    }

    /// Get current rate limit status
    pub async fn get_status(&self) -> (u32, u32, DateTime<Utc>) {
        let state = self.state.read().await;
        (
            state.remaining,
            state.limit,
            DateTime::from_timestamp(state.reset_at, 0).unwrap_or_else(Utc::now),
        )
    }
}
