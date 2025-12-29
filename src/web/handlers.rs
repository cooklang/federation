use askama::Template;
use axum::{
    extract::{Path, Query, State},
    response::{Html, IntoResponse},
};
use serde::{Deserialize, Deserializer};

use crate::{api::handlers::AppState, db, error::Error, indexer::search::SearchQuery, Result};

/// Deserialize optional string, treating empty strings as None
fn deserialize_optional_string<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let opt = Option::<String>::deserialize(deserializer)?;
    match opt.as_deref() {
        None | Some("") => Ok(None),
        Some(s) => Ok(Some(s.to_string())),
    }
}

/// Search page template
#[derive(Template)]
#[template(path = "search.html")]
struct SearchTemplate {
    query: String,
    results: Vec<RecipeCardData>,
    total: usize,
    page: usize,
    total_pages: usize,
    recent_recipes: Vec<RecipeCardData>,
}

#[derive(Clone)]
#[allow(dead_code)] // Fields are used by Askama templates
struct RecipeCardData {
    id: i64,
    title: String,
    summary: String,
    tags: Vec<String>,
    servings: String,
    total_time_minutes: String,
    difficulty: String,
    image_url: String,
    source_url: String,
}

#[derive(Deserialize)]
pub struct SearchParams {
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    q: Option<String>,
    #[serde(default = "default_page")]
    page: usize,
}

fn default_page() -> usize {
    1
}

/// GET / - Search page
pub async fn index(
    State(state): State<AppState>,
    Query(params): Query<SearchParams>,
) -> Result<impl IntoResponse> {
    let query = params.q.clone().unwrap_or_default();

    // If query is empty, show no results
    let (results, total, total_pages) = if query.is_empty() {
        (vec![], 0, 0)
    } else {
        // Build search query
        let search_query = SearchQuery {
            q: query.clone(),
            page: params.page,
            limit: state.settings.pagination.web_default_limit,
        };

        // Execute search
        let search_results = state
            .search_index
            .search(&search_query, state.settings.pagination.max_search_results)?;
        let total = search_results.total;
        let total_pages = search_results.total_pages;

        // Batch fetch tags for all recipes (avoid N+1 query problem)
        let recipe_ids: Vec<i64> = search_results.results.iter().map(|r| r.recipe_id).collect();
        let tags_map = db::tags::get_tags_for_recipes(&state.pool, &recipe_ids).await?;

        let mut results = vec![];

        // Fetch details for each result
        for result in search_results.results {
            let recipe = db::recipes::get_recipe(&state.pool, result.recipe_id)
                .await
                .ok();
            let tags = tags_map.get(&result.recipe_id).cloned().unwrap_or_default();

            if let Some(r) = recipe {
                results.push(RecipeCardData {
                    id: r.id,
                    title: r.title,
                    summary: r.summary.unwrap_or_default(),
                    tags,
                    servings: r.servings.map(|s| s.to_string()).unwrap_or_default(),
                    total_time_minutes: r
                        .total_time_minutes
                        .map(|t| t.to_string())
                        .unwrap_or_default(),
                    difficulty: r.difficulty.unwrap_or_default(),
                    image_url: r.image_url.unwrap_or_default(),
                    source_url: r.source_url.unwrap_or_default(),
                });
            }
        }

        (results, total, total_pages)
    };

    // Fetch recently indexed recipes for the homepage
    let recent_recipes = if query.is_empty() {
        let recipes = db::recipes::list_recently_indexed(&state.pool, 6).await?;
        let recipe_ids: Vec<i64> = recipes.iter().map(|r| r.id).collect();
        let tags_map = db::tags::get_tags_for_recipes(&state.pool, &recipe_ids).await?;

        recipes
            .into_iter()
            .map(|r| {
                let tags = tags_map.get(&r.id).cloned().unwrap_or_default();
                RecipeCardData {
                    id: r.id,
                    title: r.title,
                    summary: r.summary.unwrap_or_default(),
                    tags,
                    servings: r.servings.map(|s| s.to_string()).unwrap_or_default(),
                    total_time_minutes: r
                        .total_time_minutes
                        .map(|t| t.to_string())
                        .unwrap_or_default(),
                    difficulty: r.difficulty.unwrap_or_default(),
                    image_url: r.image_url.unwrap_or_default(),
                    source_url: r.source_url.unwrap_or_default(),
                }
            })
            .collect()
    } else {
        vec![]
    };

    let template = SearchTemplate {
        query,
        results,
        total,
        page: params.page,
        total_pages,
        recent_recipes,
    };

    Ok(Html(template.render().map_err(|e| {
        Error::Internal(format!("Template render failed: {e}"))
    })?))
}

/// Recipe detail page template
#[derive(Template)]
#[template(path = "recipe.html")]
struct RecipeTemplate {
    recipe: RecipeData,
    schema_json: String,
}

#[derive(Clone)]
pub struct RecipeData {
    pub id: i64,
    pub title: String,
    pub summary: String,
    pub parsed_sections: Option<Vec<crate::indexer::cooklang_parser::RecipeSection>>,
    pub ingredients: Vec<IngredientData>,
    pub cookware: Vec<String>,
    pub tags: Vec<String>,
    pub servings: String,
    pub total_time_minutes: String,
    pub active_time_minutes: String,
    pub difficulty: String,
    pub image_url: String,
    pub source_url: String,
    pub feed: FeedData,
    pub metadata: Option<crate::indexer::cooklang_parser::RecipeMetadata>,
}

#[derive(Clone)]
pub struct IngredientData {
    pub name: String,
    pub quantity: String,
    pub unit: String,
}

#[derive(Clone)]
pub struct FeedData {
    pub id: i64,
    pub title: String,
    pub author: String,
}

/// GET /recipes/:id - Recipe detail page
pub async fn recipe_detail(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse> {
    // Fetch recipe
    let recipe = db::recipes::get_recipe(&state.pool, id).await?;

    // Fetch feed
    let feed = db::feeds::get_feed(&state.pool, recipe.feed_id).await?;

    // Fetch tags (still from database for search/filtering purposes)
    let tags = db::tags::get_tags_for_recipe(&state.pool, id).await?;

    // Parse recipe content on-the-fly - all recipe data comes from here!
    let parsed_recipe = recipe
        .content
        .as_ref()
        .and_then(|content| crate::indexer::parse_cooklang_full(content).ok());

    // Extract everything from parsed recipe (like cookcli does)
    let (parsed_sections, ingredients, cookware, metadata) = if let Some(ref parsed) = parsed_recipe
    {
        let ingredients = parsed
            .ingredients
            .iter()
            .map(|i| IngredientData {
                name: i.name.clone(),
                quantity: i.quantity.clone().unwrap_or_default(),
                unit: i.unit.clone().unwrap_or_default(),
            })
            .collect();

        let cookware = parsed.cookware.iter().map(|c| c.name.clone()).collect();

        (
            Some(parsed.sections.clone()),
            ingredients,
            cookware,
            parsed.metadata.clone(),
        )
    } else {
        (None, vec![], vec![], None)
    };

    let recipe_data = RecipeData {
        id: recipe.id,
        title: recipe.title,
        summary: recipe.summary.unwrap_or_default(),
        parsed_sections,
        ingredients,
        cookware,
        tags,
        servings: recipe.servings.map(|s| s.to_string()).unwrap_or_default(),
        total_time_minutes: recipe
            .total_time_minutes
            .map(|t| t.to_string())
            .unwrap_or_default(),
        active_time_minutes: recipe
            .active_time_minutes
            .map(|t| t.to_string())
            .unwrap_or_default(),
        difficulty: recipe.difficulty.unwrap_or_default(),
        image_url: recipe.image_url.unwrap_or_default(),
        source_url: recipe.source_url.unwrap_or_default(),
        feed: FeedData {
            id: feed.id,
            title: feed.title.unwrap_or_else(|| "Unknown Feed".to_string()),
            author: feed.author.unwrap_or_default(),
        },
        metadata,
    };

    // Generate Schema.org JSON-LD
    let schema = super::schema::recipe_to_schema_json(&recipe_data);
    let schema_json = serde_json::to_string_pretty(&schema).unwrap_or_else(|_| "{}".to_string());

    let template = RecipeTemplate {
        recipe: recipe_data,
        schema_json,
    };

    Ok(Html(template.render().map_err(|e| {
        Error::Internal(format!("Template render failed: {e}"))
    })?))
}

/// Feeds page template
#[derive(Template)]
#[template(path = "feeds.html")]
struct FeedsTemplate {
    feeds: Vec<FeedCardData>,
    page: usize,
    total_pages: usize,
    stats: StatsData,
}

#[derive(Clone)]
struct FeedCardData {
    id: i64,
    url: String,
    title: String,
    author: String,
    status: String,
    recipe_count: i64,
    last_fetched_at: String,
}

#[derive(Clone)]
struct StatsData {
    total_feeds: i64,
    active_feeds: i64,
    total_recipes: i64,
    total_tags: i64,
}

#[derive(Deserialize)]
pub struct FeedListParams {
    #[serde(default = "default_page")]
    page: usize,
}

/// GET /feeds - Feeds management page
pub async fn feeds_page(
    State(state): State<AppState>,
    Query(params): Query<FeedListParams>,
) -> Result<impl IntoResponse> {
    let limit = state.settings.pagination.feed_page_size;
    let offset = (params.page.saturating_sub(1)) * limit;

    // Fetch feeds (include GitHub feeds for web display)
    let feeds =
        db::feeds::list_feeds_with_filter(&state.pool, None, limit as i64, offset as i64, false)
            .await?;
    let total = db::feeds::count_feeds(&state.pool, None).await?;
    let total_pages = (total as usize)
        .div_ceil(limit)
        .min(state.settings.pagination.max_pages);

    // Fetch stats
    let total_recipes = db::recipes::count_all_recipes(&state.pool).await?;
    let total_tags = db::tags::count_tags(&state.pool).await?;
    let active_feeds = db::feeds::count_feeds(&state.pool, Some("active")).await?;

    let stats = StatsData {
        total_feeds: total,
        active_feeds,
        total_recipes,
        total_tags,
    };

    // Convert feeds to cards
    let mut feed_cards = vec![];
    for feed in feeds {
        let recipe_count = db::recipes::count_recipes_by_feed(&state.pool, feed.id).await?;

        feed_cards.push(FeedCardData {
            id: feed.id,
            url: feed.url,
            title: feed.title.unwrap_or_else(|| "Untitled Feed".to_string()),
            author: feed.author.unwrap_or_default(),
            status: feed.status,
            recipe_count,
            last_fetched_at: feed
                .last_fetched_at
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default(),
        });
    }

    let template = FeedsTemplate {
        feeds: feed_cards,
        page: params.page,
        total_pages,
        stats,
    };

    Ok(Html(template.render().map_err(|e| {
        Error::Internal(format!("Template render failed: {e}"))
    })?))
}

/// Feed recipes page template
#[derive(Template)]
#[template(path = "feed_recipes.html")]
struct FeedRecipesTemplate {
    recipes: Vec<RecipeCardData>,
    page: usize,
    total_pages: usize,
    total: i64,
    feed_id: i64,
    feed_title: String,
}

#[derive(Deserialize)]
pub struct FeedRecipesParams {
    #[serde(default = "default_page")]
    page: usize,
}

/// GET /feeds/:id/recipes - Browse recipes from a specific feed
pub async fn feed_recipes_page(
    State(state): State<AppState>,
    Path(feed_id): Path<i64>,
    Query(params): Query<FeedRecipesParams>,
) -> Result<impl IntoResponse> {
    let limit = 24;
    let offset = (params.page.saturating_sub(1)) * limit;

    // Fetch recipes from this feed
    let recipes =
        db::recipes::list_recipes_by_feed(&state.pool, feed_id, limit as i64, offset as i64)
            .await?;
    let total = db::recipes::count_recipes_by_feed(&state.pool, feed_id).await?;
    let total_pages = (total as usize)
        .div_ceil(limit)
        .min(state.settings.pagination.max_pages);

    // Get feed title
    let feed = db::feeds::get_feed(&state.pool, feed_id).await?;
    let feed_title = feed.title.unwrap_or_else(|| "Unknown Feed".to_string());

    // Batch fetch tags for all recipes (avoid N+1 query problem)
    let recipe_ids: Vec<i64> = recipes.iter().map(|r| r.id).collect();
    let tags_map = db::tags::get_tags_for_recipes(&state.pool, &recipe_ids).await?;

    // Convert to card data
    let mut recipe_cards = vec![];
    for recipe in recipes {
        let tags = tags_map.get(&recipe.id).cloned().unwrap_or_default();

        recipe_cards.push(RecipeCardData {
            id: recipe.id,
            title: recipe.title,
            summary: recipe.summary.unwrap_or_default(),
            tags,
            servings: recipe.servings.map(|s| s.to_string()).unwrap_or_default(),
            total_time_minutes: recipe
                .total_time_minutes
                .map(|t| t.to_string())
                .unwrap_or_default(),
            difficulty: recipe.difficulty.unwrap_or_default(),
            image_url: recipe.image_url.unwrap_or_default(),
            source_url: recipe.source_url.unwrap_or_default(),
        });
    }

    let template = FeedRecipesTemplate {
        recipes: recipe_cards,
        page: params.page,
        total_pages,
        total,
        feed_id,
        feed_title,
    };

    Ok(Html(template.render().map_err(|e| {
        Error::Internal(format!("Template render failed: {e}"))
    })?))
}

/// About page template
#[derive(Template)]
#[template(path = "about.html")]
struct AboutTemplate {}

/// GET /about - About page
pub async fn about_page() -> Result<impl IntoResponse> {
    let template = AboutTemplate {};
    Ok(Html(template.render().map_err(|e| {
        Error::Internal(format!("Template render failed: {e}"))
    })?))
}

/// Browse page template
#[derive(Template)]
#[template(path = "browse.html")]
struct BrowseTemplate {
    recipes: Vec<RecipeCardData>,
    page: usize,
    total_pages: usize,
    total: i64,
}

#[derive(Deserialize)]
pub struct BrowseParams {
    #[serde(default = "default_page")]
    page: usize,
}

/// GET /browse - Browse all recipes page
pub async fn browse_page(
    State(state): State<AppState>,
    Query(params): Query<BrowseParams>,
) -> Result<impl IntoResponse> {
    let limit = 24;
    let offset = (params.page.saturating_sub(1)) * limit;

    // Fetch all recipes
    let recipes = db::recipes::list_all_recipes(&state.pool, limit as i64, offset as i64).await?;
    let total = db::recipes::count_all_recipes(&state.pool).await?;
    let total_pages = (total as usize)
        .div_ceil(limit)
        .min(state.settings.pagination.max_pages);

    // Batch fetch tags for all recipes (avoid N+1 query problem)
    let recipe_ids: Vec<i64> = recipes.iter().map(|r| r.id).collect();
    let tags_map = db::tags::get_tags_for_recipes(&state.pool, &recipe_ids).await?;

    // Convert to card data
    let recipe_cards: Vec<RecipeCardData> = recipes
        .into_iter()
        .map(|recipe| {
            let tags = tags_map.get(&recipe.id).cloned().unwrap_or_default();
            RecipeCardData {
                id: recipe.id,
                title: recipe.title,
                summary: recipe.summary.unwrap_or_default(),
                tags,
                servings: recipe.servings.map(|s| s.to_string()).unwrap_or_default(),
                total_time_minutes: recipe
                    .total_time_minutes
                    .map(|t| t.to_string())
                    .unwrap_or_default(),
                difficulty: recipe.difficulty.unwrap_or_default(),
                image_url: recipe.image_url.unwrap_or_default(),
                source_url: recipe.source_url.unwrap_or_default(),
            }
        })
        .collect();

    let template = BrowseTemplate {
        recipes: recipe_cards,
        page: params.page,
        total_pages,
        total,
    };

    Ok(Html(template.render().map_err(|e| {
        Error::Internal(format!("Template render failed: {e}"))
    })?))
}

/// GET /recipes - Redirect to /browse
pub async fn recipes_redirect() -> impl IntoResponse {
    axum::response::Redirect::permanent("/browse")
}
