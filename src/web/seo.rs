// SEO endpoints: sitemap.xml and robots.txt.
//
// Without a sitemap, crawlers only discover recipes through the /browse
// pagination chain, which leaves most of the corpus unvisited (observed
// 2026-08: a handful of indexed pages out of ~6k recipes).

use std::fmt::Write as _;

use axum::{extract::State, http::header, response::IntoResponse};

use crate::{api::handlers::AppState, db, Result};

/// Public origin used in sitemap/canonical URLs when EXTERNAL_URL is unset.
pub const DEFAULT_BASE_URL: &str = "https://recipes.cooklang.org";

/// The public base URL without a trailing slash.
pub fn base_url(state: &AppState) -> String {
    state
        .settings
        .server
        .external_url
        .clone()
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
        .trim_end_matches('/')
        .to_string()
}

/// GET /sitemap.xml — static pages plus every recipe URL.
pub async fn sitemap_xml(State(state): State<AppState>) -> Result<impl IntoResponse> {
    let base = base_url(&state);
    let ids = db::recipes::list_recipe_ids(&state.pool).await?;

    let mut xml = String::with_capacity(200 + (ids.len() + 4) * 60);
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str("<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n");
    for path in ["/", "/browse", "/feeds", "/about"] {
        let _ = writeln!(xml, "  <url><loc>{base}{path}</loc></url>");
    }
    for id in ids {
        let _ = writeln!(xml, "  <url><loc>{base}/recipes/{id}</loc></url>");
    }
    xml.push_str("</urlset>\n");

    Ok(([(header::CONTENT_TYPE, "application/xml")], xml))
}

/// GET /robots.txt — allow everything, point at the sitemap.
pub async fn robots_txt(State(state): State<AppState>) -> impl IntoResponse {
    let base = base_url(&state);
    let body = format!("User-agent: *\nAllow: /\n\nSitemap: {base}/sitemap.xml\n");
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], body)
}
