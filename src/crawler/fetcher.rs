use crate::error::{Error, Result};
use reqwest::{header, Client, Response, StatusCode};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, warn};

/// HTTP fetcher with retry logic and rate limiting
pub struct Fetcher {
    client: Client,
    max_retries: u32,
    initial_backoff: Duration,
    max_feed_size: usize,
}

#[derive(Debug)]
pub struct FetchResult {
    pub content: String,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub content_type: Option<String>,
}

/// Result of fetching recipe content with conditional request support
#[derive(Debug)]
pub enum RecipeContentResult {
    /// Content was fetched (new or modified)
    Fetched {
        content: String,
        etag: Option<String>,
        last_modified: Option<String>,
    },
    /// Content not modified (304 response)
    NotModified,
}

impl Fetcher {
    pub fn new(user_agent: String, max_feed_size: usize) -> Result<Self> {
        let client = Client::builder()
            .user_agent(user_agent)
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .map_err(Error::Http)?;

        Ok(Self {
            client,
            max_retries: 3,
            initial_backoff: Duration::from_secs(1),
            max_feed_size,
        })
    }

    /// Fetch a URL with retry logic and exponential backoff
    pub async fn fetch(&self, url: &str) -> Result<FetchResult> {
        self.fetch_with_conditions(url, None, None).await
    }

    /// Fetch a URL with conditional request headers (ETag, Last-Modified)
    pub async fn fetch_with_conditions(
        &self,
        url: &str,
        etag: Option<&str>,
        last_modified: Option<&str>,
    ) -> Result<FetchResult> {
        let mut retries = 0;
        let mut backoff = self.initial_backoff;

        loop {
            match self.fetch_once(url, etag, last_modified).await {
                Ok(result) => return Ok(result),
                Err(e) if retries < self.max_retries && Self::is_retryable(&e) => {
                    retries += 1;
                    warn!(
                        "Fetch failed (attempt {}/{}): {}. Retrying in {:?}",
                        retries, self.max_retries, e, backoff
                    );
                    sleep(backoff).await;
                    backoff *= 2; // Exponential backoff
                }
                Err(e) => return Err(e),
            }
        }
    }

    async fn fetch_once(
        &self,
        url: &str,
        etag: Option<&str>,
        last_modified: Option<&str>,
    ) -> Result<FetchResult> {
        debug!("Fetching: {}", url);

        let mut request = self.client.get(url);

        // Add conditional request headers
        if let Some(etag) = etag {
            request = request.header(header::IF_NONE_MATCH, etag);
        }

        if let Some(last_modified) = last_modified {
            request = request.header(header::IF_MODIFIED_SINCE, last_modified);
        }

        let response = request.send().await?;

        // Handle 304 Not Modified
        if response.status() == StatusCode::NOT_MODIFIED {
            return Err(Error::FeedParse("304 Not Modified".to_string()));
        }

        // Check for success status
        if !response.status().is_success() {
            return Err(Error::FeedParse(format!("HTTP {}", response.status())));
        }

        // Extract headers before consuming response
        let new_etag = response
            .headers()
            .get(header::ETAG)
            .and_then(|v| v.to_str().ok())
            .map(String::from);

        let new_last_modified = response
            .headers()
            .get(header::LAST_MODIFIED)
            .and_then(|v| v.to_str().ok())
            .map(String::from);

        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(String::from);

        // Validate content type for feeds (allow text/xml, application/xml, application/atom+xml, application/rss+xml)
        // Also allow text/plain for .cook files and no content-type (some servers don't set it)
        if let Some(ref ct) = content_type {
            let ct_lower = ct.to_lowercase();
            let valid_types = [
                "text/xml",
                "application/xml",
                "application/atom+xml",
                "application/rss+xml",
                "text/plain",
                "text/html", // Some feeds are served as HTML
            ];

            if !valid_types
                .iter()
                .any(|&valid_type| ct_lower.starts_with(valid_type))
            {
                warn!("Unexpected content type: {} for {}", ct, url);
                // Don't fail, just warn - some servers return incorrect content types
            }
        }

        // Check content length
        if let Some(content_length) = response.content_length() {
            if content_length > self.max_feed_size as u64 {
                return Err(Error::Validation(format!(
                    "Feed size {} exceeds maximum {}",
                    content_length, self.max_feed_size
                )));
            }
        }

        // Read response body with size limit
        let content = self.read_with_limit(response).await?;

        Ok(FetchResult {
            content,
            etag: new_etag,
            last_modified: new_last_modified,
            content_type,
        })
    }

    async fn read_with_limit(&self, response: Response) -> Result<String> {
        let bytes = response.bytes().await?;

        if bytes.len() > self.max_feed_size {
            return Err(Error::Validation(format!(
                "Feed size {} exceeds maximum {}",
                bytes.len(),
                self.max_feed_size
            )));
        }

        let content = String::from_utf8(bytes.to_vec())
            .map_err(|e| Error::FeedParse(format!("Invalid UTF-8 in response: {e}")))?;

        Ok(content)
    }

    fn is_retryable(error: &Error) -> bool {
        match error {
            Error::Http(e) => {
                // Retry on network errors, timeouts, server errors
                e.is_timeout() || e.is_connect() || e.is_request()
            }
            _ => false,
        }
    }

    /// Fetch recipe content with conditional request support
    /// Returns NotModified if the content hasn't changed (304 response)
    pub async fn fetch_recipe_content(
        &self,
        url: &str,
        etag: Option<&str>,
        last_modified: Option<&str>,
    ) -> Result<RecipeContentResult> {
        match self.fetch_with_conditions(url, etag, last_modified).await {
            Ok(result) => Ok(RecipeContentResult::Fetched {
                content: result.content,
                etag: result.etag,
                last_modified: result.last_modified,
            }),
            Err(Error::FeedParse(msg)) if msg == "304 Not Modified" => {
                Ok(RecipeContentResult::NotModified)
            }
            Err(e) => Err(e),
        }
    }
}

/// Rate limiter for respecting crawl delays
pub struct RateLimiter {
    delay: Duration,
    last_request: tokio::sync::Mutex<Option<tokio::time::Instant>>,
}

impl RateLimiter {
    pub fn new(requests_per_second: u64) -> Self {
        let delay = Duration::from_millis(1000 / requests_per_second.max(1));
        Self {
            delay,
            last_request: tokio::sync::Mutex::new(None),
        }
    }

    /// Wait if necessary to respect rate limit
    pub async fn wait(&self) {
        // Calculate wait time inside lock scope, then release lock before sleeping
        let wait_time = {
            let last = self.last_request.lock().await;

            if let Some(last_time) = *last {
                let elapsed = last_time.elapsed();
                if elapsed < self.delay {
                    Some(self.delay - elapsed)
                } else {
                    None
                }
            } else {
                None
            }
        }; // Lock released here

        // Sleep outside lock scope to avoid blocking other requests
        if let Some(wait) = wait_time {
            debug!("Rate limiting: waiting {:?}", wait);
            sleep(wait).await;
        }

        // Re-acquire lock only to update timestamp
        let mut last = self.last_request.lock().await;
        *last = Some(tokio::time::Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_rate_limiter() {
        let limiter = RateLimiter::new(2); // 2 requests per second

        let start = tokio::time::Instant::now();
        limiter.wait().await; // First request - no wait
        limiter.wait().await; // Second request - should wait ~500ms
        let elapsed = start.elapsed();

        // Should take at least 500ms for the second request
        assert!(elapsed >= Duration::from_millis(400));
    }

    #[test]
    fn test_fetcher_creation() {
        let fetcher = Fetcher::new("TestBot/1.0".to_string(), 5_242_880);
        assert!(fetcher.is_ok());
    }
}
