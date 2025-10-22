use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedConfig {
    pub version: u32,
    pub feeds: Vec<FeedEntry>,
    #[serde(default)]
    pub validation: ValidationConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedEntry {
    pub url: String,
    pub title: String,
    #[serde(default = "default_feed_type")]
    pub feed_type: FeedType,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub notes: Option<String>,
    pub added_by: String,
    pub added_at: String,
    #[serde(default)]
    pub disabled_at: Option<String>,
    #[serde(default)]
    pub disabled_by: Option<String>,
    #[serde(default)]
    pub disabled_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum FeedType {
    #[default]
    Web,
    GitHub,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationConfig {
    #[serde(default = "default_max_feeds")]
    pub max_feeds: usize,
    #[serde(default = "default_protocols")]
    pub allowed_protocols: Vec<String>,
    #[serde(default)]
    pub url_patterns: UrlPatterns,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            max_feeds: default_max_feeds(),
            allowed_protocols: default_protocols(),
            url_patterns: UrlPatterns::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UrlPatterns {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

fn default_true() -> bool {
    true
}

fn default_feed_type() -> FeedType {
    FeedType::Web
}

fn default_max_feeds() -> usize {
    1000
}

fn default_protocols() -> Vec<String> {
    vec!["https".to_string(), "http".to_string()]
}

impl FeedConfig {
    /// Load feed configuration from a YAML file
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = fs::read_to_string(path.as_ref()).map_err(|e| {
            Error::Config(format!(
                "Failed to read feed config from {}: {}",
                path.as_ref().display(),
                e
            ))
        })?;

        let config: FeedConfig = serde_yaml::from_str(&content).map_err(|e| {
            Error::Config(format!(
                "Failed to parse feed config from {}: {}",
                path.as_ref().display(),
                e
            ))
        })?;

        config.validate()?;
        Ok(config)
    }

    /// Validate the entire configuration
    pub fn validate(&self) -> Result<()> {
        // Version check
        if self.version != 1 {
            return Err(Error::Config(format!(
                "Unsupported config version: {}. Expected version 1",
                self.version
            )));
        }

        // Feed count check
        if self.feeds.len() > self.validation.max_feeds {
            return Err(Error::Config(format!(
                "Too many feeds: {} > {}",
                self.feeds.len(),
                self.validation.max_feeds
            )));
        }

        // Duplicate URL check
        let mut seen = HashSet::new();
        for feed in &self.feeds {
            if !seen.insert(&feed.url) {
                return Err(Error::Config(format!("Duplicate feed URL: {}", feed.url)));
            }
        }

        // Validate each feed
        for (index, feed) in self.feeds.iter().enumerate() {
            self.validate_feed(feed)
                .map_err(|e| Error::Config(format!("Feed #{} ({}): {}", index + 1, feed.url, e)))?;
        }

        Ok(())
    }

    /// Validate a single feed entry
    fn validate_feed(&self, feed: &FeedEntry) -> Result<()> {
        // Basic field validation
        if feed.url.trim().is_empty() {
            return Err(Error::Config("Feed URL cannot be empty".to_string()));
        }

        if feed.title.trim().is_empty() {
            return Err(Error::Config("Feed title cannot be empty".to_string()));
        }

        if feed.added_by.trim().is_empty() {
            return Err(Error::Config("Feed added_by cannot be empty".to_string()));
        }

        if feed.added_at.trim().is_empty() {
            return Err(Error::Config("Feed added_at cannot be empty".to_string()));
        }

        // Parse URL
        let url = Url::parse(&feed.url)
            .map_err(|e| Error::Config(format!("Invalid URL '{}': {}", feed.url, e)))?;

        // Protocol check
        if !self
            .validation
            .allowed_protocols
            .contains(&url.scheme().to_string())
        {
            return Err(Error::Config(format!(
                "Invalid protocol '{}'. Allowed protocols: {}",
                url.scheme(),
                self.validation.allowed_protocols.join(", ")
            )));
        }

        // Host check
        if url.host_str().is_none() {
            return Err(Error::Config("URL must have a valid host".to_string()));
        }

        // Feed type-specific validation
        match feed.feed_type {
            FeedType::GitHub => {
                self.validate_github_url(&url)?;
            }
            FeedType::Web => {
                // Regular RSS/Atom feed - no additional validation needed
            }
        }

        // Deny pattern check
        for pattern in &self.validation.url_patterns.deny {
            if self.matches_pattern(&feed.url, pattern) {
                return Err(Error::Config(format!(
                    "URL matches deny pattern: {pattern}"
                )));
            }
        }

        // Allow pattern check (if any allow patterns are specified)
        if !self.validation.url_patterns.allow.is_empty() {
            let matches_allow = self
                .validation
                .url_patterns
                .allow
                .iter()
                .any(|pattern| self.matches_pattern(&feed.url, pattern));

            if !matches_allow {
                return Err(Error::Config(format!(
                    "URL does not match any allow patterns: {}",
                    self.validation.url_patterns.allow.join(", ")
                )));
            }
        }

        // Disabled feed validation
        if !feed.enabled {
            if feed.disabled_at.is_none() {
                return Err(Error::Config(
                    "Disabled feed must have disabled_at field".to_string(),
                ));
            }
            if feed.disabled_by.is_none() {
                return Err(Error::Config(
                    "Disabled feed must have disabled_by field".to_string(),
                ));
            }
        }

        Ok(())
    }

    /// Validate GitHub repository URL
    fn validate_github_url(&self, url: &Url) -> Result<()> {
        // Check that it's a GitHub URL
        let host = url
            .host_str()
            .ok_or_else(|| Error::Config("GitHub URL must have a valid host".to_string()))?;

        if host != "github.com" && host != "www.github.com" {
            return Err(Error::Config(format!(
                "GitHub feed must be from github.com, got: {host}"
            )));
        }

        // Parse owner and repo from path
        let path = url.path();
        let parts: Vec<&str> = path
            .trim_start_matches('/')
            .trim_end_matches('/')
            .split('/')
            .collect();

        if parts.len() < 2 {
            return Err(Error::Config(
                "GitHub URL must be in format: https://github.com/owner/repo".to_string(),
            ));
        }

        let owner = parts[0];
        let repo = parts[1];

        if owner.is_empty() || repo.is_empty() {
            return Err(Error::Config(
                "GitHub owner and repository name cannot be empty".to_string(),
            ));
        }

        Ok(())
    }

    /// Check if a URL matches a simple glob pattern
    fn matches_pattern(&self, url: &str, pattern: &str) -> bool {
        // Convert glob pattern to regex
        // Replace . with \. and * with .*
        let regex_pattern = pattern
            .replace(".", "\\.")
            .replace("*", ".*")
            .replace("?", ".");

        regex::Regex::new(&format!("^{regex_pattern}$"))
            .map(|re| re.is_match(url))
            .unwrap_or(false)
    }

    /// Get an iterator over enabled feeds
    pub fn enabled_feeds(&self) -> impl Iterator<Item = &FeedEntry> {
        self.feeds.iter().filter(|f| f.enabled)
    }

    /// Get an iterator over disabled feeds
    pub fn disabled_feeds(&self) -> impl Iterator<Item = &FeedEntry> {
        self.feeds.iter().filter(|f| !f.enabled)
    }

    /// Get the total number of feeds
    pub fn total_feeds(&self) -> usize {
        self.feeds.len()
    }

    /// Get the number of enabled feeds
    pub fn enabled_count(&self) -> usize {
        self.enabled_feeds().count()
    }

    /// Get the number of disabled feeds
    pub fn disabled_count(&self) -> usize {
        self.disabled_feeds().count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn create_test_config(content: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file.flush().unwrap();
        file
    }

    #[test]
    fn test_load_valid_config() {
        let config_content = r#"
version: 1
feeds:
  - url: "https://example.com/feed.xml"
    title: "Example Feed"
    feed_type: web
    enabled: true
    tags:
      - test
    notes: "Test feed"
    added_by: "@tester"
    added_at: "2025-10-13"
  - url: "https://github.com/owner/repo"
    title: "GitHub Repo"
    feed_type: github
    enabled: true
    tags:
      - test
    added_by: "@tester"
    added_at: "2025-10-13"
validation:
  max_feeds: 1000
  allowed_protocols:
    - https
    - http
"#;

        let file = create_test_config(config_content);
        let config = FeedConfig::from_file(file.path()).unwrap();

        assert_eq!(config.version, 1);
        assert_eq!(config.feeds.len(), 2);
        assert_eq!(config.feeds[0].url, "https://example.com/feed.xml");
        assert_eq!(config.feeds[0].title, "Example Feed");
        assert_eq!(config.feeds[0].feed_type, FeedType::Web);
        assert!(config.feeds[0].enabled);
        assert_eq!(config.feeds[1].url, "https://github.com/owner/repo");
        assert_eq!(config.feeds[1].feed_type, FeedType::GitHub);
    }

    #[test]
    fn test_reject_duplicate_urls() {
        let config_content = r#"
version: 1
feeds:
  - url: "https://example.com/feed.xml"
    title: "Feed 1"
    feed_type: web
    enabled: true
    tags: []
    added_by: "@tester"
    added_at: "2025-10-13"
  - url: "https://example.com/feed.xml"
    title: "Feed 2"
    feed_type: web
    enabled: true
    tags: []
    added_by: "@tester"
    added_at: "2025-10-13"
"#;

        let file = create_test_config(config_content);
        let result = FeedConfig::from_file(file.path());

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Duplicate feed URL"));
    }

    #[test]
    fn test_reject_private_ips() {
        let config_content = r#"
version: 1
feeds:
  - url: "http://localhost/feed.xml"
    title: "Local Feed"
    feed_type: web
    enabled: true
    tags: []
    added_by: "@tester"
    added_at: "2025-10-13"
validation:
  url_patterns:
    deny:
      - "*localhost*"
      - "*127.0.0.1*"
      - "*192.168.*"
"#;

        let file = create_test_config(config_content);
        let result = FeedConfig::from_file(file.path());

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("matches deny pattern"));
    }

    #[test]
    fn test_reject_invalid_protocol() {
        let config_content = r#"
version: 1
feeds:
  - url: "ftp://example.com/feed.xml"
    title: "FTP Feed"
    feed_type: web
    enabled: true
    tags: []
    added_by: "@tester"
    added_at: "2025-10-13"
validation:
  allowed_protocols:
    - https
    - http
"#;

        let file = create_test_config(config_content);
        let result = FeedConfig::from_file(file.path());

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid protocol"));
    }

    #[test]
    fn test_reject_too_many_feeds() {
        let config_content = r#"
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
validation:
  max_feeds: 1
"#;

        let file = create_test_config(config_content);
        let result = FeedConfig::from_file(file.path());

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Too many feeds"));
    }

    #[test]
    fn test_disabled_feed_validation() {
        let config_content = r#"
version: 1
feeds:
  - url: "https://example.com/feed.xml"
    title: "Disabled Feed"
    feed_type: web
    enabled: false
    tags: []
    added_by: "@tester"
    added_at: "2025-10-13"
    disabled_at: "2025-10-14"
    disabled_by: "@admin"
    disabled_reason: "Site unavailable"
"#;

        let file = create_test_config(config_content);
        let config = FeedConfig::from_file(file.path()).unwrap();

        assert_eq!(config.feeds.len(), 1);
        assert!(!config.feeds[0].enabled);
        assert_eq!(config.enabled_count(), 0);
        assert_eq!(config.disabled_count(), 1);
    }

    #[test]
    fn test_pattern_matching() {
        let config = FeedConfig {
            version: 1,
            feeds: vec![],
            validation: ValidationConfig::default(),
        };

        assert!(config.matches_pattern("localhost", "localhost"));
        assert!(config.matches_pattern("http://localhost/feed", "*localhost*"));
        assert!(config.matches_pattern("http://192.168.1.1/feed", "*192.168.*"));
        assert!(config.matches_pattern("something.local", "*.local"));
        assert!(!config.matches_pattern("example.com", "*.local"));
    }

    #[test]
    fn test_github_url_validation() {
        // Valid GitHub URL
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
        assert_eq!(config.feeds[0].feed_type, FeedType::GitHub);

        // Invalid GitHub URL - not github.com
        let config_content = r#"
version: 1
feeds:
  - url: "https://gitlab.com/owner/repo"
    title: "GitLab Repo"
    feed_type: github
    enabled: true
    tags: []
    added_by: "@tester"
    added_at: "2025-10-13"
"#;

        let file = create_test_config(config_content);
        let result = FeedConfig::from_file(file.path());
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("must be from github.com"));

        // Invalid GitHub URL - missing repo name
        let config_content = r#"
version: 1
feeds:
  - url: "https://github.com/owner"
    title: "GitHub User"
    feed_type: github
    enabled: true
    tags: []
    added_by: "@tester"
    added_at: "2025-10-13"
"#;

        let file = create_test_config(config_content);
        let result = FeedConfig::from_file(file.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("owner/repo"));
    }

    #[test]
    fn test_enabled_feeds_iterator() {
        let config_content = r#"
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
    enabled: false
    tags: []
    added_by: "@tester"
    added_at: "2025-10-13"
    disabled_at: "2025-10-14"
    disabled_by: "@admin"
  - url: "https://example3.com/feed.xml"
    title: "Feed 3"
    feed_type: web
    enabled: true
    tags: []
    added_by: "@tester"
    added_at: "2025-10-13"
"#;

        let file = create_test_config(config_content);
        let config = FeedConfig::from_file(file.path()).unwrap();

        assert_eq!(config.total_feeds(), 3);
        assert_eq!(config.enabled_count(), 2);
        assert_eq!(config.disabled_count(), 1);

        let enabled_urls: Vec<_> = config.enabled_feeds().map(|f| &f.url).collect();
        assert_eq!(enabled_urls.len(), 2);
        assert!(enabled_urls.contains(&&"https://example1.com/feed.xml".to_string()));
        assert!(enabled_urls.contains(&&"https://example3.com/feed.xml".to_string()));
    }
}
