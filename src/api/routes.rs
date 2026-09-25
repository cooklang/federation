use axum::http::{header, HeaderValue, Method};
use axum::{routing::get, Router};
use std::time::Duration;
use tower_http::{
    compression::CompressionLayer, cors::CorsLayer, limit::RequestBodyLimitLayer,
    services::ServeDir, set_header::SetResponseHeaderLayer, trace::TraceLayer,
};

#[cfg(not(test))]
use {
    crate::api::rate_limit::{governor_burst, governor_period, ClientIpKeyExtractor},
    std::sync::Arc,
    tower_governor::{governor::GovernorConfigBuilder, GovernorLayer},
};

use crate::api::handlers::{self as api_handlers, AppState};
use crate::config::Settings;
use crate::web::handlers as web_handlers;

/// Every minute, forget rate-limit clients whose buckets have refilled
/// (`forget_idle` returns how many remain), so the per-IP map does not grow with
/// every address ever seen. The task holds only a weak reference and ends once
/// the router, and with it the limiter, is dropped. Nothing is started outside
/// a Tokio runtime.
#[cfg(not(test))]
fn spawn_idle_client_cleanup<T, F>(limiter: std::sync::Weak<T>, forget_idle: F)
where
    T: Send + Sync + 'static,
    F: Fn(&T) -> usize + Send + 'static,
{
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return;
    };
    runtime.spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.tick().await; // the first tick completes immediately
        loop {
            interval.tick().await;
            let Some(limiter) = limiter.upgrade() else {
                break;
            };
            let clients = forget_idle(&limiter);
            tracing::debug!(clients, "rate limiter forgot idle clients");
        }
    });
}

/// Create the router with all endpoints (API + Web UI)
#[cfg_attr(test, allow(unused_variables))]
pub fn create_router(state: AppState, settings: &Settings) -> Router {
    // Public API routes - read-only, no authentication required
    #[cfg_attr(test, allow(unused_mut))]
    let mut api_routes = Router::new()
        // Search
        .route("/search", get(api_handlers::search_recipes))
        .route("/facets", get(api_handlers::get_facets))
        // Recipes
        .route("/recipes/:id", get(api_handlers::get_recipe))
        .route("/recipes/:id/download", get(api_handlers::download_recipe))
        // Feeds (read-only)
        .route("/feeds", get(api_handlers::list_feeds))
        .route("/feeds/:id", get(api_handlers::get_feed))
        // Stats
        .route("/stats", get(api_handlers::get_stats))
        .with_state(state.clone());

    // Rate limit per client (see `api::rate_limit::client_ip` for how the client
    // is identified behind a proxy): API_RATE_LIMIT requests per second
    // sustained, bursts of twice that. Compiled out of the library's unit tests;
    // `tests/rate_limit_test.rs` exercises it through this router.
    #[cfg(not(test))]
    {
        let governor_conf = Arc::new(
            GovernorConfigBuilder::default()
                .key_extractor(ClientIpKeyExtractor)
                .period(governor_period(settings.server.api_rate_limit))
                .burst_size(governor_burst(settings.server.api_rate_limit))
                .finish()
                .expect("governor period and burst size are non-zero"),
        );
        spawn_idle_client_cleanup(Arc::downgrade(governor_conf.limiter()), |limiter| {
            limiter.retain_recent();
            limiter.len()
        });
        let governor_layer = GovernorLayer {
            config: governor_conf,
        };
        api_routes = api_routes.layer(governor_layer);
    }

    let api_routes = api_routes;

    // Web UI routes
    let web_routes = Router::new()
        .route("/", get(web_handlers::index))
        .route("/browse", get(web_handlers::browse_page))
        .route("/recipes", get(web_handlers::recipes_redirect))
        .route("/recipes/:id", get(web_handlers::recipe_detail))
        .route("/feeds", get(web_handlers::feeds_page))
        .route("/feeds/:id/recipes", get(web_handlers::feed_recipes_page))
        .route("/about", get(web_handlers::about_page))
        .route("/validate", get(web_handlers::validate_page))
        .route("/sitemap.xml", get(crate::web::seo::sitemap_xml))
        .route("/robots.txt", get(crate::web::seo::robots_txt))
        .with_state(state.clone());

    // Health check routes (no state needed for health, state needed for ready)
    let health_routes = Router::new()
        .route("/health", get(api_handlers::health_check))
        .route("/ready", get(api_handlers::readiness_check))
        .with_state(state.clone());

    // Static file serving
    let static_routes = Router::new().nest_service("/static", ServeDir::new("src/web/static"));

    // Main router with middleware
    Router::new()
        .merge(web_routes)
        .merge(health_routes)
        .merge(static_routes)
        .nest("/api", api_routes)
        .layer(
            // Request body size limit - prevent memory exhaustion from large payloads
            RequestBodyLimitLayer::new(settings.pagination.max_request_body_size),
        )
        .layer(
            // CORS - allow all origins for read-only public API
            CorsLayer::new()
                .allow_methods([Method::GET, Method::OPTIONS])
                .allow_headers([
                    header::CONTENT_TYPE,
                    header::ACCEPT,
                ])
                .allow_origin(tower_http::cors::Any)
                .max_age(Duration::from_secs(3600)),
        )
        .layer(
            // Security headers
            SetResponseHeaderLayer::if_not_present(
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            ),
        )
        .layer(
            SetResponseHeaderLayer::if_not_present(
                header::X_FRAME_OPTIONS,
                HeaderValue::from_static("DENY"),
            ),
        )
        .layer(
            SetResponseHeaderLayer::if_not_present(
                header::HeaderName::from_static("x-xss-protection"),
                HeaderValue::from_static("1; mode=block"),
            ),
        )
        .layer(
            SetResponseHeaderLayer::if_not_present(
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static(
                    "default-src 'self'; script-src 'self' 'unsafe-inline' https://plau.cook.md; style-src 'self' 'unsafe-inline'; img-src 'self' data: https:; font-src 'self' data:; connect-src 'self' https://plau.cook.md; object-src 'none'; base-uri 'self'"
                ),
            ),
        )
        .layer(
            // HSTS - enforce HTTPS (only if served over HTTPS)
            SetResponseHeaderLayer::if_not_present(
                header::STRICT_TRANSPORT_SECURITY,
                HeaderValue::from_static("max-age=31536000; includeSubDomains"),
            ),
        )
        .layer(
            // Compression
            CompressionLayer::new(),
        )
        .layer(
            // Tracing
            TraceLayer::new_for_http(),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tempfile::TempDir;
    use tower::ServiceExt;

    // Helper to create test app state.
    //
    // Returns the search index's `TempDir` guard alongside the state: the caller
    // must keep it alive for the whole test, because dropping it deletes the
    // index directory out from under Tantivy.
    async fn create_test_state() -> (AppState, TempDir) {
        use std::sync::Arc;

        // Create in-memory database
        let pool = sqlx::SqlitePool::connect(":memory:").await.unwrap();

        // Run migrations
        crate::db::run_migrations(&pool).await.unwrap();

        // Create temporary directory for search index
        let temp_dir = tempfile::tempdir().unwrap();
        let search_index = crate::indexer::search::SearchIndex::new(temp_dir.path()).unwrap();

        let settings = crate::config::Settings {
            database: crate::config::DatabaseConfig {
                url: ":memory:".to_string(),
                max_connections: 5,
                min_connections: 2,
                connection_timeout_seconds: 30,
                idle_timeout_seconds: 600,
            },
            server: crate::config::ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 3000,
                external_url: None,
                api_rate_limit: 100,
            },
            crawler: crate::config::CrawlerConfig {
                interval_seconds: 3600,
                max_feed_size: 5242880,
                max_recipe_size: 1048576,
                rate_limit: 1,
                user_agent: "test".to_string(),
            },
            search: crate::config::SearchConfig {
                index_path: "/tmp/test".into(),
            },
            pagination: crate::config::PaginationConfig {
                api_max_limit: 100,
                web_default_limit: 50,
                feed_page_size: 20,
                max_search_results: 1000,
                max_request_body_size: 10485760,
                max_pages: 10000,
            },
        };

        let state = AppState {
            pool,
            search_index: Arc::new(search_index),
            github_indexer: None,
            settings,
            facets_cache: Arc::new(crate::api::facets::FacetsCache::default()),
        };

        (state, temp_dir)
    }

    #[tokio::test]
    async fn test_health_routes_exist() {
        // The TempDir guard must stay alive for the whole test - dropping it deletes
        // the search index directory out from under Tantivy.
        let (state, _index_dir) = create_test_state().await;
        let app = create_router(state.clone(), &state.settings);

        // Test that API routes exist
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/stats")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    async fn response_json(response: axum::response::Response) -> serde_json::Value {
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn test_search_locale_filter_and_recipe_detail_expose_locale() {
        use crate::db::models::{NewFeed, NewRecipe};
        use crate::db::{feeds, recipes};

        // The TempDir guard must stay alive for the whole test - dropping it deletes
        // the search index directory out from under Tantivy.
        let (state, _index_dir) = create_test_state().await;

        // Seed a feed and two recipes in different locales.
        let feed = feeds::create_feed(
            &state.pool,
            &NewFeed {
                url: "https://example.com/feed.xml".to_string(),
                title: Some("Test Feed".to_string()),
            },
        )
        .await
        .unwrap();

        let new_recipe = |external_id: &str, title: &str, locale: &str| NewRecipe {
            feed_id: feed.id,
            external_id: external_id.to_string(),
            title: title.to_string(),
            source_url: None,
            enclosure_url: format!("https://example.com/{external_id}.cook"),
            content: None,
            summary: None,
            servings: None,
            total_time_minutes: None,
            active_time_minutes: None,
            difficulty: None,
            image_url: None,
            published_at: None,
            content_hash: None,
            content_etag: None,
            content_last_modified: None,
            feed_entry_updated: None,
            locale: Some(locale.to_string()),
            locale_source: Some("declared".to_string()),
        };

        let en_recipe =
            recipes::create_recipe(&state.pool, &new_recipe("recipe-en", "Pancakes", "en"))
                .await
                .unwrap();

        let de_recipe =
            recipes::create_recipe(&state.pool, &new_recipe("recipe-de", "Pfannkuchen", "de"))
                .await
                .unwrap();

        // Index both recipes into the search index and commit so they're
        // immediately visible (SearchIndex::commit reloads the reader).
        let mut writer = state.search_index.writer().unwrap();
        state
            .search_index
            .index_recipe(&mut writer, &en_recipe, None, &[], &[])
            .unwrap();
        state
            .search_index
            .index_recipe(&mut writer, &de_recipe, None, &[], &[])
            .unwrap();
        state.search_index.commit(&mut writer).unwrap();

        // GET /api/search?locale=de returns only the German recipe.
        let app = create_router(state.clone(), &state.settings);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/search?locale=de")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let json = response_json(response).await;
        let results = json["results"].as_array().unwrap();
        assert_eq!(
            results.len(),
            1,
            "expected only the German recipe: {json:?}"
        );
        assert_eq!(results[0]["id"].as_i64().unwrap(), de_recipe.id);
        assert_eq!(results[0]["locale"].as_str().unwrap(), "de");

        // GET /api/recipes/:id includes locale and locale_source.
        let app = create_router(state.clone(), &state.settings);
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/recipes/{}", de_recipe.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let json = response_json(response).await;
        assert_eq!(json["locale"].as_str().unwrap(), "de");
        assert_eq!(json["locale_source"].as_str().unwrap(), "declared");
    }

    /// Two indexed English recipes in one feed:
    /// "Quick Garlic Pasta" (20 min, serves 2, tags dinner+quick) and
    /// "Slow Garlic Stew" (180 min, serves 6, tag dinner). Returns their ids.
    async fn seed_search_fixture(state: &AppState) -> (i64, i64) {
        use crate::db::models::{NewFeed, NewRecipe};
        use crate::db::{feeds, recipes, tags};
        use crate::indexer::extras::IndexExtras;

        let feed = feeds::create_feed(
            &state.pool,
            &NewFeed {
                url: "https://example.com/filters.xml".to_string(),
                title: Some("Filter Feed".to_string()),
            },
        )
        .await
        .unwrap();

        let make = |external_id: &str, title: &str, total_time: i64, servings: i64| NewRecipe {
            feed_id: feed.id,
            external_id: external_id.to_string(),
            title: title.to_string(),
            source_url: None,
            enclosure_url: format!("https://example.com/{external_id}.cook"),
            // Non-NULL so this recipe counts toward the SQL-backed facets
            // (`content IS NOT NULL`, matching what a rebuild would index).
            content: Some("Recipe body text.".to_string()),
            summary: Some(format!("{title} summary")),
            servings: Some(servings),
            total_time_minutes: Some(total_time),
            active_time_minutes: None,
            difficulty: Some("easy".to_string()),
            image_url: Some(format!("https://example.com/{external_id}.jpg")),
            published_at: None,
            content_hash: None,
            content_etag: None,
            content_last_modified: None,
            feed_entry_updated: None,
            locale: Some("en".to_string()),
            locale_source: Some("declared".to_string()),
        };

        let quick =
            recipes::create_recipe(&state.pool, &make("quick", "Quick Garlic Pasta", 20, 2))
                .await
                .unwrap();
        let slow = recipes::create_recipe(&state.pool, &make("slow", "Slow Garlic Stew", 180, 6))
            .await
            .unwrap();
        tags::set_recipe_tags(&state.pool, quick.id, &["dinner".into(), "quick".into()])
            .await
            .unwrap();
        tags::set_recipe_tags(&state.pool, slow.id, &["dinner".into()])
            .await
            .unwrap();

        let mut writer = state.search_index.writer().unwrap();
        for recipe in [&quick, &slow] {
            let extras = IndexExtras {
                file_path: None,
                tags: tags::get_tags_for_recipe(&state.pool, recipe.id)
                    .await
                    .unwrap(),
                ingredients: vec!["garlic".to_string()],
                feed_title: Some("Filter Feed".to_string()),
            };
            state
                .search_index
                .index_recipe_full(&mut writer, recipe, &extras)
                .unwrap();
        }
        state.search_index.commit(&mut writer).unwrap();

        (quick.id, slow.id)
    }

    async fn get_json(state: &AppState, uri: &str) -> (StatusCode, serde_json::Value) {
        let app = create_router(state.clone(), &state.settings);
        let response = app
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        (status, response_json(response).await)
    }

    async fn get_text(state: &AppState, uri: &str) -> (StatusCode, String) {
        let app = create_router(state.clone(), &state.settings);
        let response = app
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8(body.to_vec()).unwrap())
    }

    fn result_ids(json: &serde_json::Value) -> Vec<i64> {
        let mut ids: Vec<i64> = json["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_i64().unwrap())
            .collect();
        ids.sort();
        ids
    }

    #[tokio::test]
    async fn search_structured_filters_narrow_results() {
        let (state, _index_dir) = create_test_state().await;
        let (quick, slow) = seed_search_fixture(&state).await;

        let (status, json) = get_json(&state, "/api/search?q=garlic&max_time=30").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(result_ids(&json), vec![quick]);

        let (_, json) = get_json(&state, "/api/search?tags=dinner,%20quick").await;
        assert_eq!(result_ids(&json), vec![quick]);

        // Filters and ordinary params (page) deserialize side by side.
        let (_, json) = get_json(&state, "/api/search?min_servings=4&max_servings=8&page=1").await;
        assert_eq!(result_ids(&json), vec![slow]);

        let (_, json) = get_json(
            &state,
            "/api/search?q=garlic&locale=en&exclude_ingredients=garlic",
        )
        .await;
        assert!(result_ids(&json).is_empty());

        let (_, json) = get_json(&state, "/api/search?difficulty=EASY&sort=newest").await;
        assert_eq!(result_ids(&json), vec![quick, slow]);
    }

    #[tokio::test]
    async fn search_cards_carry_rich_fields() {
        let (state, _index_dir) = create_test_state().await;
        let (quick, _) = seed_search_fixture(&state).await;

        let (status, json) = get_json(&state, "/api/search?q=quick").await;
        assert_eq!(status, StatusCode::OK);
        let card = &json["results"][0];
        assert_eq!(card["id"].as_i64().unwrap(), quick);
        assert_eq!(card["total_time_minutes"], 20);
        assert_eq!(card["servings"], 2);
        assert_eq!(card["difficulty"], "easy");
        assert_eq!(card["image_url"], "https://example.com/quick.jpg");
        assert_eq!(card["feed"]["title"], "Filter Feed");
        assert!(card["feed"]["id"].as_i64().is_some());
        assert_eq!(card["tags"], serde_json::json!(["dinner", "quick"]));
        assert_eq!(card["locale"], "en");
    }

    #[tokio::test]
    async fn search_rejects_bad_parameters_with_400() {
        let (state, _index_dir) = create_test_state().await;
        seed_search_fixture(&state).await;

        for (uri, needle) in [
            ("/api/search?max_time=soon", "max_time"),
            ("/api/search?min_servings=-2", "min_servings"),
            ("/api/search?sort=rating", "sort"),
            ("/api/search?q=nosuchfield:pasta", "Invalid query"),
        ] {
            let (status, json) = get_json(&state, uri).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
            assert!(
                json["error"].as_str().unwrap().contains(needle),
                "{uri}: {json}"
            );
        }
    }

    #[tokio::test]
    async fn search_with_zero_limit_does_not_fail() {
        let (state, _index_dir) = create_test_state().await;
        seed_search_fixture(&state).await;

        let (status, json) = get_json(&state, "/api/search?limit=0").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["results"].as_array().unwrap().len(), 1);
        assert_eq!(json["pagination"]["limit"], 1);
    }

    #[tokio::test]
    async fn facets_endpoint_reports_counts_and_honours_tag_limit() {
        let (state, _index_dir) = create_test_state().await;
        seed_search_fixture(&state).await;

        let (status, json) = get_json(&state, "/api/facets").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            json["tags"][0],
            serde_json::json!({ "name": "dinner", "count": 2 })
        );
        assert_eq!(
            json["locales"][0],
            serde_json::json!({ "code": "en", "name": "English", "count": 2 })
        );
        assert_eq!(
            json["difficulties"][0],
            serde_json::json!({ "name": "easy", "count": 2 })
        );

        // Served from the cache (loaded with every tag), then trimmed.
        let (_, limited) = get_json(&state, "/api/facets?tag_limit=1").await;
        assert_eq!(limited["tags"].as_array().unwrap().len(), 1);

        let (status, json) = get_json(&state, "/api/facets?tag_limit=lots").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(json["error"].as_str().unwrap().contains("tag_limit"));
    }

    /// A repeated list parameter is not merged: axum's `Query` rejects the
    /// duplicate key before the handler runs. Lists are comma-separated.
    #[tokio::test]
    async fn search_rejects_a_repeated_list_parameter() {
        let (state, _index_dir) = create_test_state().await;
        let app = create_router(state.clone(), &state.settings);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/search?tags=a&tags=b")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    /// The website's language dropdown is built from `api::facets::language_facets`
    /// (the same function `GET /api/facets` uses) instead of its own inline
    /// query + fold. This pins its rendered output - codes, names, order and
    /// counts - so that refactor stays behaviour-preserving: regional codes
    /// ("en-US") fold into their base language, most common language first.
    #[tokio::test]
    async fn search_page_language_dropdown_folds_regional_codes() {
        use crate::db::models::{NewFeed, NewRecipe};
        use crate::db::{feeds, recipes};

        let (state, _index_dir) = create_test_state().await;

        let feed = feeds::create_feed(
            &state.pool,
            &NewFeed {
                url: "https://example.com/locales.xml".to_string(),
                title: Some("Locale Feed".to_string()),
            },
        )
        .await
        .unwrap();

        let new_recipe = |external_id: &str, locale: &str| NewRecipe {
            feed_id: feed.id,
            external_id: external_id.to_string(),
            title: format!("Recipe {external_id}"),
            source_url: None,
            enclosure_url: format!("https://example.com/{external_id}.cook"),
            // Non-NULL so this recipe counts toward `language_facets`
            // (`content IS NOT NULL`, matching what a rebuild would index).
            content: Some("Recipe body text.".to_string()),
            summary: None,
            servings: None,
            total_time_minutes: None,
            active_time_minutes: None,
            difficulty: None,
            image_url: None,
            published_at: None,
            content_hash: None,
            content_etag: None,
            content_last_modified: None,
            feed_entry_updated: None,
            locale: Some(locale.to_string()),
            locale_source: Some("declared".to_string()),
        };

        // en, en-US (folds into en) and de: "en" ends up with count 2, "de" with 1.
        for (external_id, locale) in [("a", "en"), ("b", "en-US"), ("c", "de")] {
            recipes::create_recipe(&state.pool, &new_recipe(external_id, locale))
                .await
                .unwrap();
        }

        let (status, body) = get_text(&state, "/").await;
        assert_eq!(status, StatusCode::OK);

        let english = r#"<option value="en">English (2)</option>"#;
        let german = r#"<option value="de">German (1)</option>"#;
        assert!(body.contains(english), "{body}");
        assert!(body.contains(german), "{body}");
        assert!(
            body.find(english).unwrap() < body.find(german).unwrap(),
            "English (more recipes) must be listed before German"
        );
        // Only one option per language: the regional "en-US" code never
        // appears in the dropdown on its own.
        assert!(!body.contains(r#"value="en-US""#));
    }

    #[tokio::test]
    async fn website_search_applies_filters_and_echoes_them_in_the_form() {
        let (state, _index_dir) = create_test_state().await;
        seed_search_fixture(&state).await;

        let (status, html) = get_text(&state, "/?tags=dinner&max_time=30").await;
        assert_eq!(status, StatusCode::OK);
        assert!(html.contains("Quick Garlic Pasta"));
        assert!(!html.contains("Slow Garlic Stew"));
        assert!(html.contains(r#"name="tags" value="dinner""#));
        assert!(html.contains(r#"<option value="30" selected>"#));
        assert!(html.contains("Clear filters"));
    }

    #[tokio::test]
    async fn website_language_dropdown_lists_languages() {
        let (state, _index_dir) = create_test_state().await;
        seed_search_fixture(&state).await;

        let (status, html) = get_text(&state, "/").await;
        assert_eq!(status, StatusCode::OK);
        assert!(html.contains("English (2)"));
    }

    #[tokio::test]
    async fn website_pagination_links_keep_url_encoded_filters() {
        let (mut state, _index_dir) = create_test_state().await;
        state.settings.pagination.web_default_limit = 1;
        seed_search_fixture(&state).await;

        let (status, html) = get_text(&state, "/?q=garlic&tags=dinner&sort=newest").await;
        assert_eq!(status, StatusCode::OK);
        // Askama HTML-escapes the `&` separators of the interpolated query.
        assert!(
            html.contains("?q=garlic&amp;tags=dinner&amp;sort=newest&page=2"),
            "{html}"
        );

        // The second page itself loads with the filters applied.
        let (status, html) = get_text(&state, "/?q=garlic&tags=dinner&sort=newest&page=2").await;
        assert_eq!(status, StatusCode::OK);
        assert!(html.contains("page 2 of 2"), "{html}");

        // Values are URL-encoded in links and HTML-escaped in the form.
        let (_, html) = get_text(&state, "/?q=garlic&exclude_ingredients=a%26b%20c").await;
        assert!(
            html.contains(r#"name="exclude_ingredients" value="a&amp;b c""#),
            "{html}"
        );
        assert!(
            html.contains("?q=garlic&amp;exclude_ingredients=a%26b%20c&page=2"),
            "{html}"
        );
    }

    #[tokio::test]
    async fn website_invalid_filter_renders_the_form_with_an_inline_error() {
        let (state, _index_dir) = create_test_state().await;
        seed_search_fixture(&state).await;

        let (status, html) =
            get_text(&state, "/?q=pasta&tags=%3Cb%3Edinner&min_servings=lots").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(html.contains(r#"id="search-error""#), "{html}");
        assert!(html.contains("min_servings must be a non-negative whole number"));
        // The user's input is kept, escaped.
        assert!(html.contains(r#"name="q""#));
        assert!(html.contains(r#"value="pasta""#));
        assert!(html.contains(r#"name="tags" value="&lt;b&gt;dinner""#));
        assert!(!html.contains("<b>dinner"));
        // An HTML page, not the API's JSON error body.
        assert!(!html.trim_start().starts_with('{'));
    }

    #[tokio::test]
    async fn website_malformed_query_renders_the_form_with_an_inline_error() {
        let (state, _index_dir) = create_test_state().await;
        seed_search_fixture(&state).await;

        let (status, html) = get_text(&state, "/?q=nosuchfield:pasta").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(html.contains(r#"id="search-error""#), "{html}");
        assert!(html.contains("Invalid query"));
        assert!(html.contains(r#"value="nosuchfield:pasta""#));
        assert!(!html.contains("No recipes found"));
        assert!(html.contains(r#"aria-label="Search recipes""#));
        assert!(html.contains(r#"aria-invalid="true" aria-describedby="search-error""#));

        // Without an error the search box is neither invalid nor described by it.
        let (status, html) = get_text(&state, "/?q=garlic").await;
        assert_eq!(status, StatusCode::OK);
        assert!(html.contains(r#"aria-label="Search recipes""#));
        assert!(!html.contains("aria-invalid"));
        assert!(!html.contains("aria-describedby"));
    }

    #[tokio::test]
    async fn about_page_documents_the_real_api() {
        let (state, _index_dir) = create_test_state().await;
        let (status, html) = get_text(&state, "/about").await;
        assert_eq!(status, StatusCode::OK);

        for documented in [
            "/api/facets",
            "include_ingredients",
            "exclude_ingredients",
            "min_servings",
            "feed_id",
            "sort",
            "tag_limit",
            "total_time_minutes",
            // Literal template text is not HTML-escaped by Askama.
            r#""pagination": {"#,
        ] {
            assert!(
                html.contains(documented),
                "About page should document {documented}"
            );
        }
        // Fields from the old, invented response examples that the API never had.
        for invented in ["recipe_url", "feed_name", "\"parsed\""] {
            assert!(
                !html.contains(invented),
                "{invented} is not a real API field"
            );
        }
    }
}
