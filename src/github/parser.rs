use crate::{Error, Result};

/// Parsed GitHub repository information
#[derive(Debug, Clone)]
pub struct RepositoryInfo {
    pub owner: String,
    pub repo: String,
}

/// Parse a GitHub repository URL
/// Accepts formats:
/// - https://github.com/owner/repo
/// - https://github.com/owner/repo/
/// - https://github.com/owner/repo.git
/// - github.com/owner/repo
/// - owner/repo
pub fn parse_repository_url(url: &str) -> Result<RepositoryInfo> {
    let url = url.trim();

    // Remove trailing slashes and .git suffix
    let url = url.trim_end_matches('/').trim_end_matches(".git");

    // Remove protocol if present
    let url = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);

    // Remove github.com if present
    let url = url.strip_prefix("github.com/").unwrap_or(url);

    // Now we should have owner/repo or owner/repo/something
    let parts: Vec<&str> = url.split('/').collect();

    if parts.len() < 2 {
        return Err(Error::Validation(
            "Invalid GitHub repository URL format. Expected: owner/repo".to_string(),
        ));
    }

    let owner = parts[0].trim();
    let repo = parts[1].trim();

    if owner.is_empty() || repo.is_empty() {
        return Err(Error::Validation(
            "Repository owner and name cannot be empty".to_string(),
        ));
    }

    Ok(RepositoryInfo {
        owner: owner.to_string(),
        repo: repo.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_full_https_url() {
        let info = parse_repository_url("https://github.com/cooklang/cooklang-rs").unwrap();
        assert_eq!(info.owner, "cooklang");
        assert_eq!(info.repo, "cooklang-rs");
    }

    #[test]
    fn test_parse_url_with_trailing_slash() {
        let info = parse_repository_url("https://github.com/cooklang/cooklang-rs/").unwrap();
        assert_eq!(info.owner, "cooklang");
        assert_eq!(info.repo, "cooklang-rs");
    }

    #[test]
    fn test_parse_url_with_git_suffix() {
        let info = parse_repository_url("https://github.com/cooklang/cooklang-rs.git").unwrap();
        assert_eq!(info.owner, "cooklang");
        assert_eq!(info.repo, "cooklang-rs");
    }

    #[test]
    fn test_parse_without_protocol() {
        let info = parse_repository_url("github.com/cooklang/cooklang-rs").unwrap();
        assert_eq!(info.owner, "cooklang");
        assert_eq!(info.repo, "cooklang-rs");
    }

    #[test]
    fn test_parse_short_format() {
        let info = parse_repository_url("cooklang/cooklang-rs").unwrap();
        assert_eq!(info.owner, "cooklang");
        assert_eq!(info.repo, "cooklang-rs");
    }

    #[test]
    fn test_parse_invalid_single_part() {
        let result = parse_repository_url("cooklang");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_empty_owner() {
        let result = parse_repository_url("/cooklang-rs");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_empty_repo() {
        let result = parse_repository_url("cooklang/");
        assert!(result.is_err());
    }
}
