use serde::{Deserialize, Serialize};

/// Search request parameters
#[derive(Debug, Clone, Deserialize)]
pub struct SearchParams {
    #[serde(default)]
    pub q: String, // Unified query string
    #[serde(default = "default_page")]
    pub page: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_page() -> usize {
    1
}

fn default_limit() -> usize {
    20
}

/// Search response
#[derive(Debug, Clone, Serialize)]
pub struct SearchResponse {
    pub results: Vec<RecipeCard>,
    pub pagination: Pagination,
}

/// Recipe card for search results
#[derive(Debug, Clone, Serialize)]
pub struct RecipeCard {
    pub id: i64,
    pub title: String,
    pub summary: Option<String>,
    pub tags: Vec<String>,
}

/// Pagination metadata
#[derive(Debug, Clone, Serialize)]
pub struct Pagination {
    pub page: usize,
    pub limit: usize,
    pub total: usize,
    pub total_pages: usize,
}

/// Full recipe details
#[derive(Debug, Clone, Serialize)]
pub struct RecipeDetail {
    pub id: i64,
    pub title: String,
    pub summary: Option<String>,
    pub content: Option<String>,
    pub ingredients: Vec<IngredientDetail>,
    pub tags: Vec<String>,
    pub servings: Option<i64>,
    pub total_time_minutes: Option<i64>,
    pub active_time_minutes: Option<i64>,
    pub difficulty: Option<String>,
    pub image_url: Option<String>,
    pub source_url: Option<String>,
    pub enclosure_url: String,
    pub feed: FeedInfo,
}

/// Ingredient with quantity
#[derive(Debug, Clone, Serialize)]
pub struct IngredientDetail {
    pub name: String,
    pub quantity: Option<f64>,
    pub unit: Option<String>,
}

/// Feed info for recipe details
#[derive(Debug, Clone, Serialize)]
pub struct FeedInfo {
    pub id: i64,
    pub title: Option<String>,
    pub author: Option<String>,
}

/// Feed list response
#[derive(Debug, Clone, Serialize)]
pub struct FeedsResponse {
    pub feeds: Vec<FeedCard>,
    pub pagination: Pagination,
}

/// Feed card
#[derive(Debug, Clone, Serialize)]
pub struct FeedCard {
    pub id: i64,
    pub url: String,
    pub title: Option<String>,
    pub author: Option<String>,
    pub status: String,
    pub recipe_count: i64,
    pub last_fetched_at: Option<String>,
    pub created_at: String,
}

/// Feed list query parameters
#[derive(Debug, Clone, Deserialize)]
pub struct FeedListParams {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default = "default_page")]
    pub page: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

/// System statistics
#[derive(Debug, Clone, Serialize)]
pub struct Stats {
    pub total_recipes: i64,
    pub total_feeds: i64,
    pub total_tags: i64,
    pub total_ingredients: i64,
    pub active_feeds: i64,
}

/// Health check response
#[derive(Debug, Clone, Serialize)]
pub struct HealthResponse {
    pub status: String,
}

/// Readiness check response
#[derive(Debug, Clone, Serialize)]
pub struct ReadinessResponse {
    pub ready: bool,
    pub database: String,
    pub search_index: String,
}
