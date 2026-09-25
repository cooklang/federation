//! The API rate limit, exercised through the real router.
//!
//! The governor layer is compiled out of the library's own unit tests
//! (`#[cfg(not(test))]`), but integration tests link the normal library build,
//! so here it is active. Requests carry axum's `ConnectInfo<SocketAddr>`, as
//! `into_make_service_with_connect_info` sets it in `main.rs`.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use axum::Router;
use tempfile::TempDir;
use tower::ServiceExt;

use federation::api::facets::FacetsCache;
use federation::api::handlers::AppState;
use federation::api::routes::create_router;
use federation::config::{
    CrawlerConfig, DatabaseConfig, PaginationConfig, SearchConfig, ServerConfig, Settings,
};
use federation::db;
use federation::indexer::SearchIndex;

/// A router whose API allows `api_rate_limit` requests per second per client.
/// The `TempDir` holds the search index and must outlive the router.
async fn router(api_rate_limit: u64) -> (Router, TempDir) {
    let pool = db::init_pool("sqlite::memory:").await.unwrap();
    db::run_migrations(&pool).await.unwrap();
    let index_dir = tempfile::tempdir().unwrap();
    let settings = Settings {
        database: DatabaseConfig {
            url: "sqlite::memory:".to_string(),
            max_connections: 5,
            min_connections: 1,
            connection_timeout_seconds: 30,
            idle_timeout_seconds: 600,
        },
        server: ServerConfig {
            host: "127.0.0.1".to_string(),
            port: 3000,
            external_url: None,
            api_rate_limit,
        },
        crawler: CrawlerConfig {
            interval_seconds: 3600,
            max_feed_size: 5_242_880,
            max_recipe_size: 1_048_576,
            rate_limit: 1,
            user_agent: "test".to_string(),
        },
        search: SearchConfig {
            index_path: index_dir.path().to_path_buf(),
        },
        pagination: PaginationConfig {
            api_max_limit: 100,
            web_default_limit: 50,
            feed_page_size: 20,
            max_search_results: 1000,
            max_request_body_size: 10_485_760,
            max_pages: 10_000,
        },
    };
    let state = AppState {
        pool,
        search_index: Arc::new(SearchIndex::new(index_dir.path()).unwrap()),
        github_indexer: None,
        settings: settings.clone(),
        facets_cache: Arc::new(FacetsCache::default()),
    };
    (create_router(state, &settings), index_dir)
}

/// `GET /api/stats` from `peer`, optionally with an `X-Forwarded-For` header.
async fn get_stats(app: &Router, peer: &str, forwarded_for: Option<&str>) -> StatusCode {
    let mut builder = Request::builder().uri("/api/stats");
    if let Some(value) = forwarded_for {
        builder = builder.header("x-forwarded-for", value);
    }
    let mut request = builder.body(Body::empty()).unwrap();
    request
        .extensions_mut()
        .insert(ConnectInfo::<SocketAddr>(peer.parse().unwrap()));
    app.clone().oneshot(request).await.unwrap().status()
}

/// Sends `count` requests and returns their statuses.
async fn burst(
    app: &Router,
    peer: &str,
    forwarded_for: Option<&str>,
    count: usize,
) -> Vec<StatusCode> {
    let mut statuses = Vec::with_capacity(count);
    for _ in 0..count {
        statuses.push(get_stats(app, peer, forwarded_for).await);
    }
    statuses
}

const OK: StatusCode = StatusCode::OK;
const LIMITED: StatusCode = StatusCode::TOO_MANY_REQUESTS;

#[tokio::test]
async fn each_client_address_has_its_own_bucket() {
    // 1 request/s sustained, burst of 2.
    let (app, _index) = router(1).await;

    assert_eq!(
        burst(&app, "203.0.113.7:5000", None, 3).await,
        [OK, OK, LIMITED]
    );
    // A different client is unaffected by the first one's exhausted bucket.
    assert_eq!(
        burst(&app, "198.51.100.20:6000", None, 3).await,
        [OK, OK, LIMITED]
    );
}

#[tokio::test]
async fn a_public_peer_cannot_escape_its_bucket_with_x_forwarded_for() {
    let (app, _index) = router(1).await;

    assert_eq!(
        burst(&app, "203.0.113.7:5000", Some("198.51.100.1"), 2).await,
        [OK, OK]
    );
    // A new spoofed address does not buy a fresh bucket.
    assert_eq!(
        get_stats(&app, "203.0.113.7:5000", Some("198.51.100.2")).await,
        LIMITED
    );
}

#[tokio::test]
async fn behind_a_local_proxy_each_forwarded_client_has_its_own_bucket() {
    let (app, _index) = router(1).await;

    let proxies = [
        ("127.0.0.1:40000", "198.51.100.1"),
        ("10.0.0.2:40000", "198.51.100.2, 10.0.0.2"),
        ("[::1]:40000", "2001:db8::1"),
        ("[fd00::2]:40000", "198.51.100.3"),
    ];
    for (proxy, client) in proxies {
        assert_eq!(
            burst(&app, proxy, Some(client), 3).await,
            [OK, OK, LIMITED],
            "{proxy} forwarding {client}"
        );
    }
    // The bucket belongs to the forwarded client, not the proxy: the same
    // client through another local proxy is still limited.
    assert_eq!(
        get_stats(&app, "172.16.0.9:40000", Some("198.51.100.1")).await,
        LIMITED
    );
    // A local proxy that forwards nothing usable is keyed by itself.
    assert_eq!(
        burst(&app, "10.0.0.7:40000", Some("garbage"), 3).await,
        [OK, OK, LIMITED]
    );
}

#[tokio::test]
async fn the_configured_rate_is_requests_per_second() {
    // 20/s sustained with a burst of 40: a normal user (debounced search,
    // paging) never comes close, and the bucket refills in well under a second.
    let (app, _index) = router(20).await;
    let peer = "203.0.113.9:5000";

    // The burst is available at once. (Requests are sent until the first 429
    // rather than exactly 41, so a slow machine that earns a token or two
    // mid-burst does not fail the test.)
    let mut allowed = 0;
    while allowed < 200 && get_stats(&app, peer, None).await == OK {
        allowed += 1;
    }
    assert!((40..200).contains(&allowed), "allowed {allowed}");

    // One token comes back every 50 ms (the old config gave one per 20 s).
    tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    assert_eq!(get_stats(&app, peer, None).await, OK);
}
