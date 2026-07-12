use federation::cli::commands::backfill_locales;
use federation::db::models::{NewFeed, NewRecipe};
use federation::db::{feeds, recipes};
use federation::indexer::search::{SearchIndex, SearchQuery};
use sqlx::SqlitePool;
use tempfile::tempdir;

const GERMAN_RECIPE: &str = "Den Mehl und das Wasser in einer Schüssel verrühren, bis ein \
glatter Teig entsteht. Den Teig ruhen lassen und anschließend goldbraun backen.";

const ENGLISH_RECIPE: &str = "Mix the flour and the water together in a large bowl until a \
smooth dough forms. Knead the dough for about ten minutes, then cover it and let it rest in a \
warm place. Roll it out thinly, then bake it in a hot oven until the edges are golden brown.";

async fn setup() -> (SqlitePool, i64) {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    let feed = feeds::create_feed(
        &pool,
        &NewFeed {
            url: "https://example.com/feed.xml".to_string(),
            title: Some("Test Feed".to_string()),
        },
    )
    .await
    .unwrap();

    (pool, feed.id)
}

fn new_recipe(feed_id: i64, external_id: &str, title: &str, content: &str) -> NewRecipe {
    NewRecipe {
        feed_id,
        external_id: external_id.to_string(),
        title: title.to_string(),
        source_url: None,
        enclosure_url: format!("https://example.com/{external_id}.cook"),
        content: Some(content.to_string()),
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
        locale: None,
        locale_source: None,
    }
}

#[tokio::test]
async fn test_backfill_detects_locales_and_makes_them_searchable() {
    let (pool, feed_id) = setup().await;
    let dir = tempdir().unwrap();
    let index = SearchIndex::new(dir.path()).unwrap();

    let german = recipes::create_recipe(
        &pool,
        &new_recipe(feed_id, "de-1", "Pfannkuchen", GERMAN_RECIPE),
    )
    .await
    .unwrap();
    let english = recipes::create_recipe(
        &pool,
        &new_recipe(feed_id, "en-1", "Pancakes", ENGLISH_RECIPE),
    )
    .await
    .unwrap();

    assert_eq!(german.locale, None, "precondition: no locale yet");

    let stats = backfill_locales(&pool, &index, false).await.unwrap();
    assert_eq!(stats.scanned, 2);
    assert_eq!(stats.updated, 2);

    let german = recipes::get_recipe(&pool, german.id).await.unwrap();
    assert_eq!(german.locale.as_deref(), Some("de"));
    assert_eq!(german.locale_source.as_deref(), Some("detected"));

    let english = recipes::get_recipe(&pool, english.id).await.unwrap();
    assert_eq!(english.locale.as_deref(), Some("en"));

    // The backfilled recipes are searchable by locale.
    let results = index
        .search(
            &SearchQuery {
                q: String::new(),
                page: 1,
                limit: 10,
                locale: Some("de".to_string()),
            },
            10,
        )
        .unwrap();

    assert_eq!(results.results.len(), 1);
    assert_eq!(results.results[0].recipe_id, german.id);
}

#[tokio::test]
async fn test_backfill_skips_recipes_that_already_have_a_locale() {
    let (pool, feed_id) = setup().await;
    let dir = tempdir().unwrap();
    let index = SearchIndex::new(dir.path()).unwrap();

    let recipe = recipes::create_recipe(
        &pool,
        &new_recipe(feed_id, "de-1", "Pfannkuchen", GERMAN_RECIPE),
    )
    .await
    .unwrap();

    // Pretend the author declared French.
    recipes::update_recipe_locale(&pool, recipe.id, Some("fr"), Some("declared"))
        .await
        .unwrap();

    let stats = backfill_locales(&pool, &index, false).await.unwrap();
    assert_eq!(stats.scanned, 0, "already-tagged recipes are not scanned");
    assert_eq!(stats.updated, 0);

    let unchanged = recipes::get_recipe(&pool, recipe.id).await.unwrap();
    assert_eq!(unchanged.locale.as_deref(), Some("fr"));
    assert_eq!(unchanged.locale_source.as_deref(), Some("declared"));

    // --force recomputes it, and detection overrides the stale value.
    let stats = backfill_locales(&pool, &index, true).await.unwrap();
    assert_eq!(stats.scanned, 1);
    assert_eq!(stats.updated, 1);

    let forced = recipes::get_recipe(&pool, recipe.id).await.unwrap();
    assert_eq!(forced.locale.as_deref(), Some("de"));
    assert_eq!(forced.locale_source.as_deref(), Some("detected"));
}
