use axum::http::{header, HeaderValue, Method};
use axum::{routing::get, Router};
use std::time::Duration;
use tower_http::{
    compression::CompressionLayer, cors::CorsLayer, limit::RequestBodyLimitLayer,
    services::ServeDir, set_header::SetResponseHeaderLayer, trace::TraceLayer,
};

#[cfg(not(test))]
use {
    std::net::IpAddr,
    std::sync::Arc,
    tower_governor::{governor::GovernorConfigBuilder, key_extractor::KeyExtractor, GovernorLayer},
};

use crate::api::handlers::{self as api_handlers, AppState};
use crate::config::Settings;
use crate::web::handlers as web_handlers;

/// Create the router with all endpoints (API + Web UI)
#[cfg_attr(test, allow(unused_variables))]
pub fn create_router(state: AppState, settings: &Settings) -> Router {
    // Public API routes - read-only, no authentication required
    #[cfg_attr(test, allow(unused_mut))]
    let mut api_routes = Router::new()
        // Search
        .route("/search", get(api_handlers::search_recipes))
        // Recipes
        .route("/recipes/:id", get(api_handlers::get_recipe))
        .route("/recipes/:id/download", get(api_handlers::download_recipe))
        // Feeds (read-only)
        .route("/feeds", get(api_handlers::list_feeds))
        .route("/feeds/:id", get(api_handlers::get_feed))
        // Stats
        .route("/stats", get(api_handlers::get_stats))
        .with_state(state.clone());

    // Apply rate limiting only in non-test builds
    // NOTE: Rate limiting uses a custom key extractor that:
    // 1. Tries to extract peer IP from connection
    // 2. Falls back to 127.0.0.1 for local testing when peer IP is unavailable
    // For production behind a reverse proxy, configure the proxy to set X-Real-IP or
    // X-Forwarded-For headers, and use PeerIpKeyExtractor instead.
    #[cfg(not(test))]
    {
        // Custom key extractor that provides fallback
        #[derive(Clone, Copy, Debug)]
        struct FallbackIpKeyExtractor;

        impl KeyExtractor for FallbackIpKeyExtractor {
            type Key = IpAddr;

            fn extract<B>(
                &self,
                req: &axum::http::Request<B>,
            ) -> Result<Self::Key, tower_governor::GovernorError> {
                // Try to get peer IP from extensions (set by axum)
                if let Some(addr) = req.extensions().get::<std::net::SocketAddr>() {
                    return Ok(addr.ip());
                }

                // Fall back to localhost for local development/testing
                Ok(IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1)))
            }
        }

        let governor_conf = Arc::new(
            GovernorConfigBuilder::default()
                .key_extractor(FallbackIpKeyExtractor)
                .per_second(settings.server.api_rate_limit)
                .burst_size(settings.server.api_rate_limit as u32 * 2)
                .finish()
                .unwrap(),
        );
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
                    "default-src 'self'; script-src 'self' 'unsafe-inline' https://plausible.io; style-src 'self' 'unsafe-inline'; img-src 'self' data: https:; font-src 'self' data:; connect-src 'self' https://plausible.io; object-src 'none'; base-uri 'self'"
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
    use tower::ServiceExt;

    // Helper to create test app state
    async fn create_test_state() -> AppState {
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

        AppState {
            pool,
            search_index: Arc::new(search_index),
            github_indexer: None,
            settings,
        }
    }

    #[tokio::test]
    async fn test_health_routes_exist() {
        let state = create_test_state().await;
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
}
