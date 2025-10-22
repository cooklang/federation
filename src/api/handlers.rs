use axum::{
    extract::{Path, Query, State},
    Json,
};
use std::sync::Arc;
use tracing::debug;

use crate::{api::models::*, db, indexer::search::SearchQuery, Error, Result};

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    pub pool: sqlx::SqlitePool,
    pub search_index: Arc<crate::indexer::search::SearchIndex>,
    pub github_indexer: Option<crate::github::GitHubIndexer>,
    pub settings: crate::config::Settings,
}

/// GET /api/search - Search recipes
pub async fn search_recipes(
    State(state): State<AppState>,
    Query(params): Query<SearchParams>,
) -> Result<Json<SearchResponse>> {
    debug!("Search request: {:?}", params);

    // Build search query
    let query = SearchQuery {
        q: params.q,
        page: params.page,
        limit: params.limit.min(state.settings.pagination.api_max_limit),
    };

    // Execute search
    let results = state.search_index.search(&query)?;

    // Batch fetch tags for all recipes (avoid N+1 query problem)
    let recipe_ids: Vec<i64> = results.results.iter().map(|r| r.recipe_id).collect();
    let tags_map = db::tags::get_tags_for_recipes(&state.pool, &recipe_ids).await?;

    // Build recipe cards
    let mut recipe_cards = Vec::new();
    for result in results.results {
        let tags = tags_map.get(&result.recipe_id).cloned().unwrap_or_default();

        recipe_cards.push(RecipeCard {
            id: result.recipe_id,
            title: result.title,
            summary: result.summary,
            tags,
        });
    }

    Ok(Json(SearchResponse {
        results: recipe_cards,
        pagination: Pagination {
            page: results.page,
            limit: query.limit,
            total: results.total,
            total_pages: results.total_pages,
        },
    }))
}

/// GET /api/recipes/:id - Get recipe details
pub async fn get_recipe(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<RecipeDetail>> {
    debug!("Get recipe request: {}", id);

    // Fetch recipe from database
    let recipe = db::recipes::get_recipe(&state.pool, id).await?;

    // Fetch feed info
    let feed = db::feeds::get_feed(&state.pool, recipe.feed_id).await?;

    // Fetch tags
    let tags = db::tags::get_tags_for_recipe(&state.pool, id).await?;

    // Fetch ingredients
    let ingredients_raw = db::ingredients::get_ingredients_for_recipe(&state.pool, id).await?;
    let ingredients = ingredients_raw
        .into_iter()
        .map(|i| IngredientDetail {
            name: i.name,
            quantity: i.quantity,
            unit: i.unit,
        })
        .collect();

    Ok(Json(RecipeDetail {
        id: recipe.id,
        title: recipe.title,
        summary: recipe.summary,
        content: recipe.content,
        ingredients,
        tags,
        servings: recipe.servings,
        total_time_minutes: recipe.total_time_minutes,
        active_time_minutes: recipe.active_time_minutes,
        difficulty: recipe.difficulty,
        image_url: recipe.image_url,
        source_url: recipe.source_url,
        enclosure_url: recipe.enclosure_url,
        feed: FeedInfo {
            id: feed.id,
            title: feed.title,
            author: feed.author,
        },
    }))
}

/// GET /api/recipes/:id/download - Download .cook file
pub async fn download_recipe(State(state): State<AppState>, Path(id): Path<i64>) -> Result<String> {
    debug!("Download recipe request: {}", id);

    // Fetch recipe from database
    let recipe = db::recipes::get_recipe(&state.pool, id).await?;

    // Return content as plain text
    let content = recipe
        .content
        .ok_or_else(|| Error::NotFound("Recipe content not found".to_string()))?;

    Ok(content)
}

/// GET /api/feeds - List all feeds
pub async fn list_feeds(
    State(state): State<AppState>,
    Query(params): Query<FeedListParams>,
) -> Result<Json<FeedsResponse>> {
    debug!("List feeds request: {:?}", params);

    let limit = params.limit.min(state.settings.pagination.api_max_limit);
    let offset = (params.page.saturating_sub(1)) * limit;

    // Fetch feeds from database
    let feeds = db::feeds::list_feeds(
        &state.pool,
        params.status.as_deref(),
        limit as i64,
        offset as i64,
    )
    .await?;

    let total = db::feeds::count_feeds(&state.pool, params.status.as_deref()).await?;

    // Convert to feed cards
    let mut feed_cards = Vec::new();
    for feed in feeds {
        let recipe_count = db::recipes::count_recipes_by_feed(&state.pool, feed.id).await?;

        feed_cards.push(FeedCard {
            id: feed.id,
            url: feed.url,
            title: feed.title,
            author: feed.author,
            status: feed.status,
            recipe_count,
            last_fetched_at: feed.last_fetched_at.map(|dt| dt.to_rfc3339()),
            created_at: feed.created_at.to_rfc3339(),
        });
    }

    let total_pages = (total as usize)
        .div_ceil(limit)
        .min(state.settings.pagination.max_pages);

    Ok(Json(FeedsResponse {
        feeds: feed_cards,
        pagination: Pagination {
            page: params.page,
            limit,
            total: total as usize,
            total_pages,
        },
    }))
}

/// GET /api/feeds/:id - Get feed details
pub async fn get_feed(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<FeedCard>> {
    debug!("Get feed request: {}", id);

    let feed = db::feeds::get_feed(&state.pool, id).await?;
    let recipe_count = db::recipes::count_recipes_by_feed(&state.pool, id).await?;

    Ok(Json(FeedCard {
        id: feed.id,
        url: feed.url,
        title: feed.title,
        author: feed.author,
        status: feed.status,
        recipe_count,
        last_fetched_at: feed.last_fetched_at.map(|dt| dt.to_rfc3339()),
        created_at: feed.created_at.to_rfc3339(),
    }))
}

/// GET /api/stats - Get system statistics
pub async fn get_stats(State(state): State<AppState>) -> Result<Json<Stats>> {
    debug!("Get stats request");

    let total_recipes = db::recipes::count_all_recipes(&state.pool).await?;
    let total_feeds = db::feeds::count_feeds(&state.pool, None).await?;
    let total_tags = db::tags::count_tags(&state.pool).await?;
    let total_ingredients = db::ingredients::count_ingredients(&state.pool).await?;
    let active_feeds = db::feeds::count_feeds(&state.pool, Some("active")).await?;

    Ok(Json(Stats {
        total_recipes,
        total_feeds,
        total_tags,
        total_ingredients,
        active_feeds,
    }))
}

/// GET /health - Health check endpoint
pub async fn health_check() -> Result<Json<HealthResponse>> {
    Ok(Json(HealthResponse {
        status: "ok".to_string(),
    }))
}

/// GET /ready - Readiness check endpoint
pub async fn readiness_check(State(state): State<AppState>) -> Result<Json<ReadinessResponse>> {
    // Check database connectivity
    let db_healthy = sqlx::query("SELECT 1").fetch_one(&state.pool).await.is_ok();

    // Search index is always ready if it was initialized
    let index_healthy = true;

    let ready = db_healthy && index_healthy;

    Ok(Json(ReadinessResponse {
        ready,
        database: if db_healthy { "ok" } else { "error" }.to_string(),
        search_index: if index_healthy { "ok" } else { "error" }.to_string(),
    }))
}
