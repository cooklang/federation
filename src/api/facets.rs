//! Facet counts for search filter UIs: tags, languages and difficulties.

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;

use crate::api::models::{DifficultyFacet, FacetsResponse, LocaleFacet, TagFacet};
use crate::db::{self, DbPool};
use crate::error::{Error, Result};
use crate::indexer::locale::display_name;

/// Tags returned by `GET /api/facets` when `tag_limit` is not given.
pub const DEFAULT_TAG_LIMIT: usize = 200;

/// Largest `tag_limit` a client may ask for.
pub const MAX_TAG_LIMIT: usize = 1000;

/// How long computed facets are served before they are recomputed.
pub const FACETS_TTL: Duration = Duration::from_secs(300);

/// In-memory cache for the facet lists. One entry, holding every list with
/// tags capped at [`MAX_TAG_LIMIT`]; requests trim tags to their own limit.
pub struct FacetsCache {
    ttl: Duration,
    entry: RwLock<Option<(Instant, Arc<FacetsResponse>)>>,
}

impl FacetsCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            entry: RwLock::new(None),
        }
    }

    /// The cached facets while fresh, else the result of `load` (which is then
    /// cached). Concurrent callers on an expired entry load it once.
    pub async fn get_or_load<F, Fut>(&self, load: F) -> Result<Arc<FacetsResponse>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<FacetsResponse>>,
    {
        if let Some((loaded_at, facets)) = self.entry.read().await.as_ref() {
            if loaded_at.elapsed() < self.ttl {
                return Ok(facets.clone());
            }
        }

        let mut entry = self.entry.write().await;
        if let Some((loaded_at, facets)) = entry.as_ref() {
            if loaded_at.elapsed() < self.ttl {
                return Ok(facets.clone());
            }
        }

        let facets = Arc::new(load().await?);
        *entry = Some((Instant::now(), facets.clone()));
        Ok(facets)
    }
}

impl Default for FacetsCache {
    fn default() -> Self {
        Self::new(FACETS_TTL)
    }
}

/// `tag_limit`: blank means [`DEFAULT_TAG_LIMIT`], values above
/// [`MAX_TAG_LIMIT`] are capped, anything but a positive integer is a 400.
pub fn parse_tag_limit(value: Option<&str>) -> Result<usize> {
    let Some(raw) = value.map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok(DEFAULT_TAG_LIMIT);
    };
    match raw.parse::<usize>() {
        Ok(limit) if limit > 0 => Ok(limit.min(MAX_TAG_LIMIT)),
        _ => Err(Error::Validation(format!(
            "tag_limit must be a positive whole number, got \"{raw}\""
        ))),
    }
}

/// Recipe counts per language, most common first. Regional codes ("en-US")
/// are folded into their base language ("en"), so each language appears once.
/// This is the list behind the website's language dropdown.
pub async fn language_facets(pool: &DbPool) -> Result<Vec<LocaleFacet>> {
    let mut counts: Vec<(String, i64)> = Vec::new();
    for (code, count) in db::recipes::list_locales(pool).await? {
        let base = code.split('-').next().unwrap_or(&code).to_string();
        match counts.iter_mut().find(|(c, _)| *c == base) {
            Some((_, existing)) => *existing += count,
            None => counts.push((base, count)),
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    Ok(counts
        .into_iter()
        .map(|(code, count)| LocaleFacet {
            name: display_name(&code).unwrap_or_else(|| code.clone()),
            code,
            count,
        })
        .collect())
}

/// Every facet list, with tags capped at [`MAX_TAG_LIMIT`].
pub async fn load_facets(pool: &DbPool) -> Result<FacetsResponse> {
    let tags = db::tags::top_tags(pool, MAX_TAG_LIMIT as i64)
        .await?
        .into_iter()
        .map(|(name, count)| TagFacet { name, count })
        .collect();
    let locales = language_facets(pool).await?;
    let difficulties = db::recipes::list_difficulties(pool)
        .await?
        .into_iter()
        .map(|(name, count)| DifficultyFacet { name, count })
        .collect();

    Ok(FacetsResponse {
        tags,
        locales,
        difficulties,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::{NewFeed, NewRecipe};

    /// en, en-US, de and an unknown-locale recipe; three tagged dessert, two
    /// vegan, one soup; difficulties "easy", "easy", "hard" and none.
    ///
    /// `difficulty` is only ever "easy", "medium", "hard" or NULL in this
    /// table (`recipes.difficulty`'s `CHECK` constraint,
    /// `migrations/001_init.sql:32`, matching `recipe_facts::allowed_difficulty`),
    /// so unlike the plan's draft this does not exercise mixed case or
    /// padding at the DB layer — `create_recipe` does not normalize before
    /// insert, and a value the CHECK rejects would fail the seed itself.
    async fn seeded_pool() -> DbPool {
        let pool = db::init_pool("sqlite::memory:").await.unwrap();
        db::run_migrations(&pool).await.unwrap();
        let feed = db::feeds::create_feed(
            &pool,
            &NewFeed {
                url: "https://example.com/feed.xml".to_string(),
                title: Some("Feed".to_string()),
            },
        )
        .await
        .unwrap();

        type SeedRow<'a> = (&'a str, Option<&'a str>, Option<&'a str>, &'a [&'a str]);
        let rows: [SeedRow; 4] = [
            ("a", Some("en"), Some("easy"), &["dessert", "vegan"]),
            ("b", Some("en-US"), Some("easy"), &["dessert"]),
            ("c", Some("de"), Some("hard"), &["dessert", "vegan"]),
            ("d", None, None, &["soup"]),
        ];
        for (external_id, locale, difficulty, tag_names) in rows {
            let recipe = db::recipes::create_recipe(
                &pool,
                &NewRecipe {
                    feed_id: feed.id,
                    external_id: external_id.to_string(),
                    title: format!("Recipe {external_id}"),
                    source_url: None,
                    enclosure_url: format!("https://example.com/{external_id}.cook"),
                    content: None,
                    summary: None,
                    servings: None,
                    total_time_minutes: None,
                    active_time_minutes: None,
                    difficulty: difficulty.map(str::to_string),
                    image_url: None,
                    published_at: None,
                    content_hash: None,
                    content_etag: None,
                    content_last_modified: None,
                    feed_entry_updated: None,
                    locale: locale.map(str::to_string),
                    locale_source: locale.map(|_| "declared".to_string()),
                },
            )
            .await
            .unwrap();
            let tag_names: Vec<String> = tag_names.iter().map(|t| t.to_string()).collect();
            db::tags::set_recipe_tags(&pool, recipe.id, &tag_names)
                .await
                .unwrap();
        }
        pool
    }

    #[tokio::test]
    async fn facets_count_tags_languages_and_difficulties() {
        let pool = seeded_pool().await;
        let facets = load_facets(&pool).await.unwrap();

        let tags: Vec<(&str, i64)> = facets
            .tags
            .iter()
            .map(|t| (t.name.as_str(), t.count))
            .collect();
        assert_eq!(tags, vec![("dessert", 3), ("vegan", 2), ("soup", 1)]);

        let locales: Vec<(&str, &str, i64)> = facets
            .locales
            .iter()
            .map(|l| (l.code.as_str(), l.name.as_str(), l.count))
            .collect();
        assert_eq!(
            locales,
            vec![("en", "English", 2), ("de", "German", 1)],
            "regional codes fold into their language"
        );

        let difficulties: Vec<(&str, i64)> = facets
            .difficulties
            .iter()
            .map(|d| (d.name.as_str(), d.count))
            .collect();
        assert_eq!(difficulties, vec![("easy", 2), ("hard", 1)]);
    }

    #[tokio::test]
    async fn top_tags_honours_the_limit() {
        let pool = seeded_pool().await;
        let tags = db::tags::top_tags(&pool, 2).await.unwrap();
        assert_eq!(
            tags,
            vec![("dessert".to_string(), 3), ("vegan".to_string(), 2)]
        );
    }

    #[tokio::test]
    async fn cache_serves_the_stored_value_until_it_expires() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let loads = AtomicUsize::new(0);
        let load = || async {
            loads.fetch_add(1, Ordering::SeqCst);
            Ok::<_, crate::error::Error>(FacetsResponse::default())
        };

        let cache = FacetsCache::new(Duration::from_secs(300));
        cache.get_or_load(load).await.unwrap();
        cache.get_or_load(load).await.unwrap();
        assert_eq!(
            loads.load(Ordering::SeqCst),
            1,
            "second call is served from cache"
        );

        let expired = FacetsCache::new(Duration::ZERO);
        expired.get_or_load(load).await.unwrap();
        expired.get_or_load(load).await.unwrap();
        assert_eq!(
            loads.load(Ordering::SeqCst),
            3,
            "an expired entry is reloaded"
        );
    }

    #[test]
    fn tag_limit_defaults_caps_and_rejects_garbage() {
        assert_eq!(parse_tag_limit(None).unwrap(), DEFAULT_TAG_LIMIT);
        assert_eq!(parse_tag_limit(Some(" ")).unwrap(), DEFAULT_TAG_LIMIT);
        assert_eq!(parse_tag_limit(Some("5")).unwrap(), 5);
        assert_eq!(parse_tag_limit(Some("5000")).unwrap(), MAX_TAG_LIMIT);
        assert!(parse_tag_limit(Some("0")).is_err());
        assert!(parse_tag_limit(Some("many")).is_err());
    }
}
