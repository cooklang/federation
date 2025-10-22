use crate::github::{
    config::GitHubConfig,
    models::{Commit, FileContent, Reference, Repository, Tree},
    rate_limiter::RateLimiter,
};
use crate::{Error, Result};
use reqwest::{header, Client, StatusCode};
use tracing::{debug, error};

/// GitHub API client
#[derive(Clone)]
pub struct GitHubClient {
    client: Client,
    config: GitHubConfig,
    rate_limiter: RateLimiter,
}

impl GitHubClient {
    /// Create a new GitHub client
    pub fn new(config: GitHubConfig) -> Result<Self> {
        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::USER_AGENT,
            header::HeaderValue::from_static("Cooklang-Federation/0.1"),
        );
        headers.insert(
            header::ACCEPT,
            header::HeaderValue::from_static("application/vnd.github.v3+json"),
        );

        // Add authentication if token is provided
        if let Some(token) = &config.token {
            let auth_value = format!("Bearer {token}");
            headers.insert(
                header::AUTHORIZATION,
                header::HeaderValue::from_str(&auth_value)
                    .map_err(|e| Error::Internal(format!("Invalid GitHub token: {e}")))?,
            );
        }

        let client = Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| Error::Internal(format!("Failed to create HTTP client: {e}")))?;

        let rate_limiter = RateLimiter::new(config.rate_limit_buffer);

        Ok(Self {
            client,
            config,
            rate_limiter,
        })
    }

    /// Make a GET request to GitHub API
    async fn get<T>(&self, path: &str) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        // Wait if we're approaching rate limit
        self.rate_limiter.wait_if_needed().await;

        let url = format!("{}{}", self.config.api_base_url(), path);
        debug!("GitHub API request: GET {}", url);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| Error::Internal(format!("GitHub API request failed: {e}")))?;

        // Update rate limit from headers
        self.rate_limiter
            .update_from_headers(response.headers())
            .await;

        let status = response.status();

        if !status.is_success() {
            let error_body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unable to read error response".to_string());
            error!("GitHub API error: {} - {}", status, error_body);

            return Err(match status {
                StatusCode::NOT_FOUND => Error::NotFound("GitHub resource not found".to_string()),
                StatusCode::FORBIDDEN => {
                    Error::Internal("GitHub API rate limit exceeded".to_string())
                }
                StatusCode::UNAUTHORIZED => {
                    Error::Internal("GitHub authentication failed".to_string())
                }
                _ => Error::Internal(format!("GitHub API error: {status}")),
            });
        }

        response
            .json::<T>()
            .await
            .map_err(|e| Error::Internal(format!("Failed to parse GitHub API response: {e}")))
    }

    /// Get repository information
    pub async fn get_repository(&self, owner: &str, repo: &str) -> Result<Repository> {
        let path = format!("/repos/{owner}/{repo}");
        self.get(&path).await
    }

    /// Get the latest commit SHA for a branch
    pub async fn get_branch_commit(&self, owner: &str, repo: &str, branch: &str) -> Result<String> {
        let path = format!("/repos/{owner}/{repo}/git/refs/heads/{branch}");
        let reference: Reference = self.get(&path).await?;
        Ok(reference.object.sha)
    }

    /// Get commit information
    pub async fn get_commit(&self, owner: &str, repo: &str, sha: &str) -> Result<Commit> {
        let path = format!("/repos/{owner}/{repo}/commits/{sha}");
        self.get(&path).await
    }

    /// Get repository tree (file listing)
    pub async fn get_tree(&self, owner: &str, repo: &str, tree_sha: &str) -> Result<Tree> {
        let path = format!("/repos/{owner}/{repo}/git/trees/{tree_sha}?recursive=1");
        self.get(&path).await
    }

    /// Get file content
    pub async fn get_file_content(
        &self,
        owner: &str,
        repo: &str,
        path: &str,
        ref_name: Option<&str>,
    ) -> Result<FileContent> {
        let api_path = if let Some(r) = ref_name {
            format!("/repos/{owner}/{repo}/contents/{path}?ref={r}")
        } else {
            format!("/repos/{owner}/{repo}/contents/{path}")
        };
        self.get(&api_path).await
    }

    /// Download raw file content from raw.githubusercontent.com
    /// This doesn't count against rate limits
    pub async fn download_raw_content(&self, url: &str) -> Result<String> {
        debug!("Downloading raw content from: {}", url);

        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| Error::Internal(format!("Failed to download file: {e}")))?;

        if !response.status().is_success() {
            return Err(Error::Internal(format!(
                "Failed to download file: HTTP {}",
                response.status()
            )));
        }

        // Check content length
        if let Some(content_length) = response.content_length() {
            if content_length > self.config.max_file_size_bytes {
                return Err(Error::Validation(format!(
                    "File too large: {} bytes (max: {})",
                    content_length, self.config.max_file_size_bytes
                )));
            }
        }

        response
            .text()
            .await
            .map_err(|e| Error::Internal(format!("Failed to read file content: {e}")))
    }

    /// Get current rate limit status
    pub async fn get_rate_limit_status(&self) -> (u32, u32, chrono::DateTime<chrono::Utc>) {
        self.rate_limiter.get_status().await
    }
}
