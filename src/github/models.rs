use serde::{Deserialize, Serialize};

/// GitHub API rate limit information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimit {
    pub limit: u32,
    pub remaining: u32,
    pub reset: i64,
}

/// GitHub repository information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repository {
    pub id: u64,
    pub name: String,
    pub full_name: String,
    pub owner: Owner,
    pub default_branch: String,
    pub description: Option<String>,
    pub html_url: String,
    pub archived: bool,
}

/// Repository owner information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Owner {
    pub login: String,
    pub id: u64,
}

/// File content from GitHub API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileContent {
    pub name: String,
    pub path: String,
    pub sha: String,
    pub size: u64,
    pub url: String,
    pub html_url: String,
    pub git_url: String,
    pub download_url: Option<String>,
    #[serde(rename = "type")]
    pub file_type: String,
    pub content: Option<String>,
    pub encoding: Option<String>,
}

/// Directory tree entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeEntry {
    pub path: String,
    pub mode: String,
    pub sha: String,
    pub size: Option<u64>,
    #[serde(rename = "type")]
    pub entry_type: String,
    pub url: String,
}

/// Git tree API response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tree {
    pub sha: String,
    pub url: String,
    pub tree: Vec<TreeEntry>,
    pub truncated: bool,
}

/// Commit information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commit {
    pub sha: String,
    pub url: String,
    pub html_url: String,
    pub commit: CommitDetails,
}

/// Detailed commit information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitDetails {
    pub message: String,
    pub tree: TreeReference,
}

/// Tree reference in commit
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeReference {
    pub sha: String,
    pub url: String,
}

/// Repository reference (branch, tag, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reference {
    #[serde(rename = "ref")]
    pub ref_name: String,
    pub node_id: String,
    pub url: String,
    pub object: RefObject,
}

/// Object reference (commit, tag, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefObject {
    pub sha: String,
    #[serde(rename = "type")]
    pub object_type: String,
    pub url: String,
}
