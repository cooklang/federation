use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Feed {
    pub id: i64,
    pub url: String,
    pub title: Option<String>,
    pub author: Option<String>,
    pub last_fetched_at: Option<DateTime<Utc>>,
    pub last_modified: Option<DateTime<Utc>>,
    pub etag: Option<String>,
    pub status: String,
    pub error_count: i64,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewFeed {
    pub url: String,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateFeed {
    pub title: Option<String>,
    pub author: Option<String>,
    pub last_fetched_at: Option<DateTime<Utc>>,
    pub last_modified: Option<DateTime<Utc>>,
    pub etag: Option<String>,
    pub error_count: i64,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Recipe {
    pub id: i64,
    pub feed_id: i64,
    pub external_id: String,
    pub title: String,
    pub source_url: Option<String>,
    pub enclosure_url: String,
    pub content: Option<String>,
    pub summary: Option<String>,
    pub servings: Option<i64>,
    pub total_time_minutes: Option<i64>,
    pub active_time_minutes: Option<i64>,
    pub difficulty: Option<String>,
    pub image_url: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub indexed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewRecipe {
    pub feed_id: i64,
    pub external_id: String,
    pub title: String,
    pub source_url: Option<String>,
    pub enclosure_url: String,
    pub content: Option<String>,
    pub summary: Option<String>,
    pub servings: Option<i64>,
    pub total_time_minutes: Option<i64>,
    pub active_time_minutes: Option<i64>,
    pub difficulty: Option<String>,
    pub image_url: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateRecipe {
    pub title: Option<String>,
    pub source_url: Option<String>,
    pub content: Option<String>,
    pub summary: Option<String>,
    pub servings: Option<i64>,
    pub total_time_minutes: Option<i64>,
    pub active_time_minutes: Option<i64>,
    pub difficulty: Option<String>,
    pub image_url: Option<String>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Tag {
    pub id: i64,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Ingredient {
    pub id: i64,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipeIngredient {
    pub name: String,
    pub quantity: Option<f64>,
    pub unit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipeWithDetails {
    #[serde(flatten)]
    pub recipe: Recipe,
    pub tags: Vec<String>,
    pub ingredients: Vec<IngredientWithQuantity>,
    pub feed: FeedInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct IngredientWithQuantity {
    pub name: String,
    pub quantity: Option<f64>,
    pub unit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct FeedInfo {
    pub id: i64,
    pub title: Option<String>,
    pub author: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedWithCount {
    #[serde(flatten)]
    pub feed: Feed,
    pub recipe_count: i64,
}

// GitHub integration models

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct GitHubFeed {
    pub id: i64,
    pub feed_id: i64,
    pub repository_url: String,
    pub owner: String,
    pub repo_name: String,
    pub default_branch: String,
    pub last_commit_sha: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewGitHubFeed {
    pub feed_id: i64,
    pub repository_url: String,
    pub owner: String,
    pub repo_name: String,
    pub default_branch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct GitHubRecipe {
    pub id: i64,
    pub recipe_id: i64,
    pub github_feed_id: i64,
    pub file_path: String,
    pub file_sha: String,
    pub raw_url: String,
    pub html_url: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewGitHubRecipe {
    pub recipe_id: i64,
    pub github_feed_id: i64,
    pub file_path: String,
    pub file_sha: String,
    pub raw_url: String,
    pub html_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct GitHubFeedWithStats {
    // Flatten all GitHubFeed fields manually for FromRow
    pub id: i64,
    pub feed_id: i64,
    pub repository_url: String,
    pub owner: String,
    pub repo_name: String,
    pub default_branch: String,
    pub last_commit_sha: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    // Additional stats fields
    pub recipe_count: i64,
    pub feed_title: Option<String>,
}
