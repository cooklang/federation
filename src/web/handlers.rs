use askama::Template;
use axum::{
    extract::{Path, Query, RawQuery, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use serde::{Deserialize, Deserializer};

use crate::{
    api::filters::FilterParams,
    api::handlers::AppState,
    api::query::ValidatedQuery,
    db,
    error::Error,
    indexer::filters::SortOrder,
    indexer::search::{SearchQuery, SearchResults},
    Result,
};

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
    locale: String,
    locales: Vec<LocaleOption>,
    results: Vec<RecipeCardData>,
    total: usize,
    page: usize,
    total_pages: usize,
    recent_recipes: Vec<RecipeCardData>,
    /// True when the page shows search results rather than the landing view.
    searching: bool,
    form: FilterForm,
    filters_active: bool,
    /// URL-encoded `key=value&...` of the current search, without `page`.
    pagination_query: String,
    /// Link to the same search with every structured filter removed.
    clear_filters_href: String,
    /// Why the search could not run (bad filter or malformed query), shown
    /// inline next to the search box. Empty when there is no error.
    error_message: String,
}

/// Current structured-filter values, echoed back into the search form.
#[derive(Clone)]
#[allow(dead_code)] // Fields are used by Askama templates
struct FilterForm {
    tags: String,
    include_ingredients: String,
    exclude_ingredients: String,
    max_time: String,
    min_servings: String,
    max_servings: String,
    difficulty: String,
    feed_id: String,
    sort: String,
}

impl FilterForm {
    fn from_params(params: &FilterParams) -> Self {
        let value =
            |field: &Option<String>| field.as_deref().unwrap_or_default().trim().to_string();
        Self {
            tags: value(&params.tags),
            include_ingredients: value(&params.include_ingredients),
            exclude_ingredients: value(&params.exclude_ingredients),
            max_time: value(&params.max_time),
            min_servings: value(&params.min_servings),
            max_servings: value(&params.max_servings),
            difficulty: value(&params.difficulty),
            feed_id: value(&params.feed_id),
            sort: value(&params.sort),
        }
    }
}

/// `key=value&...` with URL-encoded values.
fn encode_query(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(key, value)| format!("{key}={}", urlencoding::encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

/// One entry in the language filter dropdown.
#[derive(Clone)]
#[allow(dead_code)] // Fields are used by Askama templates
struct LocaleOption {
    code: String,
    name: String,
    count: i64,
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
    locale_name: String,
}

#[derive(Deserialize)]
pub struct SearchParams {
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    q: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    locale: Option<String>,
    #[serde(default = "default_page")]
    page: usize,
    /// The same structured filters as `GET /api/search`.
    #[serde(flatten)]
    filters: FilterParams,
}

fn default_page() -> usize {
    1
}

impl SearchParams {
    /// Only the search box and language of a query string that did not
    /// deserialize (first value of each), so the error page keeps them.
    fn search_box_only(raw_query: &str) -> Self {
        let mut params = Self {
            q: None,
            locale: None,
            page: default_page(),
            filters: FilterParams::default(),
        };
        for (key, value) in url::form_urlencoded::parse(raw_query.as_bytes()) {
            let slot = match key.as_ref() {
                "q" => &mut params.q,
                "locale" => &mut params.locale,
                _ => continue,
            };
            if slot.is_none() && !value.is_empty() {
                *slot = Some(value.into_owned());
            }
        }
        params
    }
}

/// GET / - Search page
pub async fn index(
    State(state): State<AppState>,
    params: std::result::Result<ValidatedQuery<SearchParams>, Error>,
    RawQuery(raw_query): RawQuery,
) -> Result<Response> {
    // A bad filter, a query string that does not deserialize (`page=abc`, a
    // repeated key) or a malformed query is the user's typo, not a failure:
    // the page is re-rendered with the message next to the search box and the
    // input kept, instead of the API's JSON error body.
    let mut error_message = String::new();
    let params = match params {
        Ok(ValidatedQuery(params)) => params,
        Err(Error::Validation(message)) => {
            error_message = message;
            SearchParams::search_box_only(raw_query.as_deref().unwrap_or_default())
        }
        Err(other) => return Err(other),
    };
    let query = params.q.clone().unwrap_or_default();
    let locale = params.locale.clone().unwrap_or_default();

    let parsed = if !error_message.is_empty() {
        None
    } else {
        match params.filters.parse() {
            Ok(parsed) => Some(parsed),
            Err(Error::Validation(message)) => {
                error_message = message;
                None
            }
            Err(other) => return Err(other),
        }
    };
    let filters_active = match &parsed {
        Some((filters, sort)) => !filters.is_empty() || *sort != SortOrder::Relevance,
        None => !params.filters.query_pairs().is_empty(),
    };
    let searching = !query.is_empty() || !locale.is_empty() || filters_active;

    // Language filter options: one entry per language, most common first.
    // Regional codes ("en-US") are folded into their base language ("en") so the
    // dropdown lists one entry per language; shared with `GET /api/facets`.
    let locales = crate::api::facets::language_facets(&state.pool)
        .await?
        .into_iter()
        .map(|facet| LocaleOption {
            code: facet.code,
            name: facet.name,
            count: facet.count,
        })
        .collect::<Vec<_>>();

    let search_results = match &parsed {
        Some((filters, sort)) if searching => {
            let search_query = SearchQuery {
                q: query.clone(),
                page: params.page,
                limit: state.settings.pagination.web_default_limit,
                locale: params.locale.clone(),
            };
            match state.search_index.search_with(
                &search_query,
                filters,
                *sort,
                state.settings.pagination.max_search_results,
            ) {
                Ok(results) => Some(results),
                Err(Error::Validation(message)) => {
                    error_message = message;
                    None
                }
                Err(other) => return Err(other),
            }
        }
        _ => None,
    };

    let (results, total, total_pages) = match search_results {
        None => (vec![], 0, 0),
        Some(SearchResults {
            results: hits,
            total,
            total_pages,
            ..
        }) => {
            // Batch fetch tags for all recipes (avoid N+1 query problem)
            let recipe_ids: Vec<i64> = hits.iter().map(|r| r.recipe_id).collect();
            let tags_map = db::tags::get_tags_for_recipes(&state.pool, &recipe_ids).await?;

            let mut results = vec![];

            // Fetch details for each result
            for result in hits {
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
                        locale_name: r
                            .locale
                            .as_deref()
                            .and_then(crate::indexer::locale::display_name)
                            .unwrap_or_default(),
                    });
                }
            }

            (results, total, total_pages)
        }
    };

    // Fetch recently indexed recipes for the homepage
    let recent_recipes = if !searching {
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
                    locale_name: r
                        .locale
                        .as_deref()
                        .and_then(crate::indexer::locale::display_name)
                        .unwrap_or_default(),
                }
            })
            .collect()
    } else {
        vec![]
    };

    // Links: pagination keeps every parameter; "Clear filters" keeps q and locale.
    let mut base_pairs: Vec<(&str, &str)> = Vec::new();
    if !query.is_empty() {
        base_pairs.push(("q", query.as_str()));
    }
    if !locale.is_empty() {
        base_pairs.push(("locale", locale.as_str()));
    }
    let clear_filters_href = format!("/?{}", encode_query(&base_pairs));
    let mut all_pairs = base_pairs.clone();
    all_pairs.extend(params.filters.query_pairs());
    let pagination_query = encode_query(&all_pairs);

    let status = if error_message.is_empty() {
        StatusCode::OK
    } else {
        StatusCode::BAD_REQUEST
    };

    let template = SearchTemplate {
        query,
        locale,
        locales,
        results,
        total,
        page: params.page,
        total_pages,
        recent_recipes,
        searching,
        form: FilterForm::from_params(&params.filters),
        filters_active,
        pagination_query,
        clear_filters_href,
        error_message,
    };

    let html = template
        .render()
        .map_err(|e| Error::Internal(format!("Template render failed: {e}")))?;
    Ok((status, Html(html)).into_response())
}

/// Recipe detail page template
#[derive(Template)]
#[template(path = "recipe.html")]
struct RecipeTemplate {
    recipe: RecipeData,
    schema_json: String,
    meta_description: String,
    canonical_url: String,
}

/// Meta-description text: the recipe summary when there is one, truncated to
/// search-snippet length on a character boundary; a generic line otherwise.
fn recipe_meta_description(title: &str, summary: &str) -> String {
    const MAX_LEN: usize = 155;
    let summary = summary.trim();
    if summary.is_empty() {
        return format!("{title} — a community Cooklang recipe: ingredients, steps, and the plain-text .cook source.");
    }
    if summary.chars().count() <= MAX_LEN {
        summary.to_string()
    } else {
        let truncated: String = summary.chars().take(MAX_LEN - 1).collect();
        format!("{}…", truncated.trim_end())
    }
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
    pub locale: String,
    pub locale_name: String,
    pub locale_detected: bool,
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
        locale: recipe.locale.clone().unwrap_or_default(),
        locale_name: recipe
            .locale
            .as_deref()
            .and_then(crate::indexer::locale::display_name)
            .unwrap_or_default(),
        locale_detected: recipe.locale_source.as_deref() == Some("detected"),
    };

    // Generate Schema.org JSON-LD
    let schema = super::schema::recipe_to_schema_json(&recipe_data);
    let schema_json = serde_json::to_string_pretty(&schema).unwrap_or_else(|_| "{}".to_string());

    let meta_description = recipe_meta_description(&recipe_data.title, &recipe_data.summary);
    let canonical_url = format!(
        "{}/recipes/{}",
        super::seo::base_url(&state),
        recipe_data.id
    );

    let template = RecipeTemplate {
        recipe: recipe_data,
        schema_json,
        meta_description,
        canonical_url,
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
            locale_name: recipe
                .locale
                .as_deref()
                .and_then(crate::indexer::locale::display_name)
                .unwrap_or_default(),
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
                locale_name: recipe
                    .locale
                    .as_deref()
                    .and_then(crate::indexer::locale::display_name)
                    .unwrap_or_default(),
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

/// Validate page template
#[derive(Template)]
#[template(path = "validate.html")]
struct ValidateTemplate {
    url: String,
    result: Option<ValidateResult>,
}

#[derive(Clone)]
struct ValidateResult {
    valid: bool,
    title: String,
    feed_type: String,
    entry_count: usize,
    sample_entries: Vec<String>,
    error: String,
}

#[derive(Deserialize)]
pub struct ValidateParams {
    #[serde(default)]
    url: String,
}

/// GET /validate - Validate feed page
pub async fn validate_page(Query(params): Query<ValidateParams>) -> Result<impl IntoResponse> {
    let result = if params.url.is_empty() {
        None
    } else {
        Some(
            match crate::utils::feed_validation::validate_feed_url(&params.url).await {
                Ok(info) => ValidateResult {
                    valid: true,
                    title: info.title,
                    feed_type: info.feed_type,
                    entry_count: info.entry_count,
                    sample_entries: info.sample_entries,
                    error: String::new(),
                },
                Err(e) => ValidateResult {
                    valid: false,
                    title: String::new(),
                    feed_type: String::new(),
                    entry_count: 0,
                    sample_entries: vec![],
                    error: e.to_string(),
                },
            },
        )
    };

    let template = ValidateTemplate {
        url: params.url,
        result,
    };

    Ok(Html(template.render().map_err(|e| {
        Error::Internal(format!("Template render failed: {e}"))
    })?))
}
