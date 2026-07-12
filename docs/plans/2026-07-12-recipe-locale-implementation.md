# Recipe Locale Detection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Tag every recipe with a locale during indexing — from a declared Cooklang `locale:` key when present, otherwise by language detection — and make it storable, filterable in search, and visible in the API and web UI.

**Architecture:** A new `src/indexer/locale.rs` resolves a locale from the already-parsed `ParsedRecipeData` (declared metadata wins; otherwise `whatlang` detection over plain recipe text with markup stripped). Both ingestion paths (feed crawler, GitHub indexer) call it and persist `locale` + `locale_source` on the `recipes` row. The Tantivy schema gains an untokenized `locale` field so search can filter by an explicit `?locale=` parameter. A `backfill-locales` CLI command fills in existing rows from content already stored in the database.

**Tech Stack:** Rust, axum, sqlx (SQLite), Tantivy 0.22, cooklang 0.17, askama templates. New crates: `whatlang` 0.18 (detection), `isolang` 2.4 (ISO 639-3 → 639-1 mapping and English language names).

**Spec:** `docs/superpowers/specs/2026-07-12-recipe-locale-design.md`

**Conventions in this codebase:**
- Run all tests with `cargo test`. Run a single test with `cargo test <name> -- --nocapture`.
- Format with `cargo fmt` and lint with `cargo clippy --all-targets -- -D warnings` before each commit.
- The database pool is `SqlitePool` (`DbPool`). Integration tests use `SqlitePool::connect("sqlite::memory:")` + `sqlx::migrate!("./migrations")`.
- Recipe locale codes are stored as BCP-47-style: lowercase language, optional uppercase region, e.g. `en`, `de`, `en-US`. Cooklang declares them with an underscore (`en_US`); we normalize to a hyphen.

---

## File Structure

**Created:**
- `src/indexer/locale.rs` — locale resolution: declared-vs-detected, detection-text assembly, code normalization, display names. Single responsibility, no I/O.
- `migrations/008_recipe_locale.sql` — `locale` + `locale_source` columns and an index.
- `tests/locale_test.rs` — integration tests for storage, search filtering, and backfill.

**Modified:**
- `src/indexer/cooklang_parser.rs` — expose the declared `locale:` key on `RecipeMetadata`.
- `src/indexer/mod.rs` — register and re-export the new module.
- `src/indexer/schema.rs` — add the Tantivy `locale` field.
- `src/indexer/search.rs` — index locale, filter on it, return it in results.
- `src/db/models.rs` — `locale` / `locale_source` on `Recipe` and `NewRecipe`.
- `src/db/recipes.rs` — persist locale on insert/update; add `update_recipe_locale` and `list_locales`.
- `src/crawler/mod.rs` — resolve locale on recipe create and content update.
- `src/github/indexer.rs` — resolve locale on recipe create.
- `src/api/models.rs`, `src/api/handlers.rs` — `?locale=` search param, locale in responses.
- `src/web/handlers.rs`, `src/web/schema.rs`, `src/web/templates/search.html`, `src/web/templates/recipe.html` — language dropdown, card chip, detail pill, `inLanguage` in JSON-LD.
- `src/cli/mod.rs`, `src/cli/commands.rs`, `src/main.rs` — `backfill-locales` command.
- `Cargo.toml` — the two new dependencies.

---

### Task 1: Expose the declared `locale:` metadata key

Cooklang has a canonical `locale:` metadata key. `cooklang::Metadata::locale()` returns `Option<(&str, Option<&str>)>` — e.g. `("en", Some("US"))` for a frontmatter value of `en_US`. Cooklang validates it: language and region are each exactly two ASCII letters, or `locale()` returns `None`.

Today `src/indexer/cooklang_parser.rs` never reads it, and because `"locale"` is missing from the skip-list in the `custom` loop, a declared locale silently lands in `RecipeMetadata::custom`. We surface it as a real field and stop duplicating it into `custom`.

**Files:**
- Modify: `src/indexer/cooklang_parser.rs` (`RecipeMetadata` struct ~line 18; the `custom` skip-list ~line 150; the `RecipeMetadata { ... }` construction ~line 200)

- [ ] **Step 1: Write the failing tests**

Add to the `mod tests` block at the bottom of `src/indexer/cooklang_parser.rs`:

```rust
    #[test]
    fn test_declared_locale_with_region() {
        let content = r#"---
locale: en_US
---

Mix @flour{2%cups} with @water{1%cup}.
"#;

        let parsed = parse_recipe(content).unwrap();
        let metadata = parsed.metadata.expect("metadata should be present");

        assert_eq!(metadata.locale, Some("en-US".to_string()));
        // The locale must not also leak into `custom`.
        assert!(!metadata.custom.iter().any(|(k, _)| k == "locale"));
    }

    #[test]
    fn test_declared_locale_without_region() {
        let content = r#"---
locale: de
---

@Mehl{200%g} mit @Wasser{100%ml} verrühren.
"#;

        let parsed = parse_recipe(content).unwrap();
        let metadata = parsed.metadata.expect("metadata should be present");

        assert_eq!(metadata.locale, Some("de".to_string()));
    }

    #[test]
    fn test_no_declared_locale() {
        let content = "Mix @flour{2%cups} with @water{1%cup}.\n";

        let parsed = parse_recipe(content).unwrap();

        // No metadata at all, or metadata with no locale — both mean "not declared".
        let locale = parsed.metadata.and_then(|m| m.locale);
        assert_eq!(locale, None);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib cooklang_parser::tests::test_declared_locale -- --nocapture`
Expected: FAIL to compile — `no field 'locale' on type 'RecipeMetadata'`.

- [ ] **Step 3: Add the field to `RecipeMetadata`**

In the `RecipeMetadata` struct, add `locale` immediately after `title`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipeMetadata {
    pub tags: Vec<String>,
    pub title: Option<String>,
    pub locale: Option<String>,
    pub description: Option<String>,
    pub servings: Option<String>,
    pub time: Option<String>,
    pub difficulty: Option<String>,
    pub course: Option<String>,
    pub prep_time: Option<String>,
    pub cook_time: Option<String>,
    pub cuisine: Option<String>,
    pub diet: Option<String>,
    pub author: Option<String>,
    pub source: Option<String>,
    pub image: Option<String>,
    pub custom: Vec<(String, String)>,
}
```

- [ ] **Step 4: Stop leaking `locale` into `custom`**

In `parse_recipe`, the loop over `&meta.map` skips standard keys with a `matches!`. Add `Some("locale")` to that list — it currently reads `Some("tags") | Some("title") | ...`:

```rust
        if !matches!(
            key_str,
            Some("tags")
                | Some("title")
                | Some("locale")
                | Some("description")
                | Some("servings")
                | Some("time")
                | Some("difficulty")
                | Some("course")
                | Some("prep time")
                | Some("cook time")
                | Some("cuisine")
                | Some("diet")
                | Some("author")
                | Some("source")
                | Some("image")
        ) {
```

- [ ] **Step 5: Populate the field**

In the `Some(RecipeMetadata { ... })` construction, add the `locale` field after `title`. `meta.locale()` yields the validated `(language, Option<region>)` pair; normalize to lowercase language and uppercase region joined by a hyphen:

```rust
            locale: meta.locale().map(|(lang, region)| match region {
                Some(region) => format!("{}-{}", lang.to_lowercase(), region.to_uppercase()),
                None => lang.to_lowercase(),
            }),
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test --lib cooklang_parser`
Expected: PASS — including the three new tests and the pre-existing `test_parse_simple_recipe`.

- [ ] **Step 7: Commit**

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
git add src/indexer/cooklang_parser.rs
git commit -m "feat: expose declared cooklang locale metadata"
```

---

### Task 2: The locale resolution module

The core unit. It takes a `ParsedRecipeData` and returns an optional `RecipeLocale`. No database, no network, no Tantivy — which is what makes it cheap to test.

Detection runs over text assembled from the *parsed* recipe, never the raw `.cook` source, so Cooklang markup (`@flour{200%g}`, `#oven{}`, `~{20%minutes}`) can't skew the trigram statistics.

**Files:**
- Modify: `Cargo.toml`
- Create: `src/indexer/locale.rs`
- Modify: `src/indexer/mod.rs`

- [ ] **Step 1: Add the dependencies**

In `Cargo.toml`, under `[dependencies]`, after the `# Cooklang parsing` block:

```toml
# Language detection
whatlang = "0.18"
isolang = "2.4"
```

`isolang`'s default features include `english_names`, which is what gives us `Language::to_name()`.

- [ ] **Step 2: Register the module**

In `src/indexer/mod.rs`, add the module and re-exports:

```rust
// Phase 3: Recipe indexing and search module
// This module handles Cooklang parsing and Tantivy search indexing

pub mod cooklang_parser;
pub mod locale;
pub mod recipe;
pub mod schema;
pub mod search;

// Re-exports
pub use cooklang_parser::{parse_recipe as parse_cooklang_full, ParsedRecipeData};
pub use locale::{resolve_locale, LocaleSource, RecipeLocale};
pub use recipe::{parse_cooklang, ParsedRecipe};
pub use schema::RecipeSchema;
pub use search::{SearchIndex, SearchQuery, SearchResult, SearchResults};
```

- [ ] **Step 3: Write the failing tests**

Create `src/indexer/locale.rs` containing *only* the test module for now, so the tests fail against missing functions rather than a missing file:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::indexer::cooklang_parser::parse_recipe;

    const GERMAN: &str = "Den @Mehl{200%g} und das @Wasser{100%ml} in einer Schüssel \
verrühren, bis ein glatter Teig entsteht. Den Teig ruhen lassen und anschließend \
im #Ofen{} goldbraun backen.";

    const ENGLISH: &str = "Mix the @flour{200%g} and the @water{100%ml} in a bowl until \
a smooth dough forms. Let the dough rest, then bake it in the #oven{} until golden brown.";

    #[test]
    fn test_detects_german() {
        let parsed = parse_recipe(GERMAN).unwrap();
        let locale = resolve_locale(&parsed).expect("should detect a locale");

        assert_eq!(locale.code, "de");
        assert_eq!(locale.source, LocaleSource::Detected);
    }

    #[test]
    fn test_detects_english() {
        let parsed = parse_recipe(ENGLISH).unwrap();
        let locale = resolve_locale(&parsed).expect("should detect a locale");

        assert_eq!(locale.code, "en");
        assert_eq!(locale.source, LocaleSource::Detected);
    }

    #[test]
    fn test_declared_locale_beats_detection() {
        // The body is unmistakably German, but the author declared French.
        let content = format!("---\nlocale: fr\n---\n\n{GERMAN}");
        let parsed = parse_recipe(&content).unwrap();
        let locale = resolve_locale(&parsed).expect("should resolve a locale");

        assert_eq!(locale.code, "fr");
        assert_eq!(locale.source, LocaleSource::Declared);
    }

    #[test]
    fn test_declared_region_is_preserved() {
        let content = format!("---\nlocale: en_US\n---\n\n{ENGLISH}");
        let parsed = parse_recipe(&content).unwrap();
        let locale = resolve_locale(&parsed).unwrap();

        assert_eq!(locale.code, "en-US");
        assert_eq!(locale.source, LocaleSource::Declared);
    }

    #[test]
    fn test_detection_never_invents_a_region() {
        let parsed = parse_recipe(ENGLISH).unwrap();
        let locale = resolve_locale(&parsed).unwrap();

        assert!(!locale.code.contains('-'), "detected code should be bare: {}", locale.code);
    }

    #[test]
    fn test_too_short_content_is_not_detected() {
        let parsed = parse_recipe("Mix @salt{}.").unwrap();
        assert!(resolve_locale(&parsed).is_none());
    }

    #[test]
    fn test_non_linguistic_content_is_not_detected() {
        let parsed = parse_recipe("12345 67890 12345 67890 12345 67890 12345").unwrap();
        assert!(resolve_locale(&parsed).is_none());
    }

    #[test]
    fn test_detection_text_excludes_markup() {
        let parsed = parse_recipe(ENGLISH).unwrap();
        let text = detection_text(&parsed);

        assert!(text.contains("flour"), "ingredient names carry language signal");
        assert!(text.contains("smooth dough"), "step text must be present");
        assert!(!text.contains('@'), "cooklang markup must be stripped: {text}");
        assert!(!text.contains('{'), "cooklang markup must be stripped: {text}");
        assert!(!text.contains("200"), "quantities must not reach the detector: {text}");
    }

    #[test]
    fn test_locale_source_as_str() {
        assert_eq!(LocaleSource::Declared.as_str(), "declared");
        assert_eq!(LocaleSource::Detected.as_str(), "detected");
    }

    #[test]
    fn test_display_name() {
        assert_eq!(display_name("de").as_deref(), Some("German"));
        assert_eq!(display_name("en-US").as_deref(), Some("English"));
        assert_eq!(display_name("zzz"), None);
    }
}
```

- [ ] **Step 4: Run tests to verify they fail**

Run: `cargo test --lib locale`
Expected: FAIL to compile — `cannot find function 'resolve_locale' in this scope`.

- [ ] **Step 5: Write the implementation**

Prepend this above the test module in `src/indexer/locale.rs`:

```rust
//! Recipe locale resolution.
//!
//! A declared Cooklang `locale:` key always wins. Otherwise the language is
//! detected from the recipe's plain text — assembled from the *parsed* recipe so
//! that Cooklang markup and quantities never reach the detector. When detection
//! is unreliable we return `None` rather than storing a guess.

use serde::{Deserialize, Serialize};

use crate::indexer::cooklang_parser::{ParsedRecipeData, StepItem};

/// Below this many characters of plain text there is not enough signal to detect.
const MIN_DETECTION_CHARS: usize = 25;

/// Where a recipe's locale came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LocaleSource {
    /// The recipe declared a `locale:` metadata key.
    Declared,
    /// We detected it from the recipe text.
    Detected,
}

impl LocaleSource {
    /// The value stored in the `recipes.locale_source` column.
    pub fn as_str(&self) -> &'static str {
        match self {
            LocaleSource::Declared => "declared",
            LocaleSource::Detected => "detected",
        }
    }
}

/// A resolved locale: a BCP-47-style code (`en`, `de`, `en-US`) and its provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipeLocale {
    pub code: String,
    pub source: LocaleSource,
}

/// Resolve a recipe's locale: declared metadata first, then detection.
pub fn resolve_locale(parsed: &ParsedRecipeData) -> Option<RecipeLocale> {
    if let Some(code) = parsed.metadata.as_ref().and_then(|m| m.locale.clone()) {
        return Some(RecipeLocale {
            code,
            source: LocaleSource::Declared,
        });
    }

    detect(&detection_text(parsed)).map(|code| RecipeLocale {
        code,
        source: LocaleSource::Detected,
    })
}

/// The plain text a recipe is detected from: title, description, section names,
/// step text, notes, and ingredient names. Quantities, units and cookware are
/// excluded — they are noise, not language signal.
pub(crate) fn detection_text(parsed: &ParsedRecipeData) -> String {
    let mut parts: Vec<&str> = Vec::new();

    if let Some(meta) = &parsed.metadata {
        if let Some(title) = &meta.title {
            parts.push(title);
        }
        if let Some(description) = &meta.description {
            parts.push(description);
        }
    }

    for section in &parsed.sections {
        if let Some(name) = &section.name {
            parts.push(name);
        }
        for step in &section.steps {
            for item in &step.items {
                if let StepItem::Text { value } = item {
                    parts.push(value);
                }
            }
        }
        for note in &section.notes {
            parts.push(note);
        }
    }

    for ingredient in &parsed.ingredients {
        parts.push(&ingredient.name);
    }

    parts.join(" ")
}

/// Detect a language code from plain text, or `None` if we can't trust the result.
fn detect(text: &str) -> Option<String> {
    if text.chars().count() < MIN_DETECTION_CHARS {
        return None;
    }

    let info = whatlang::detect(text)?;
    if !info.is_reliable() {
        return None;
    }

    Some(to_bcp47(info.lang().code()))
}

/// Map whatlang's ISO 639-3 code to a two-letter code where one exists.
/// Languages without a 639-1 code (e.g. Cebuano) keep their 639-3 code.
fn to_bcp47(code_639_3: &str) -> String {
    isolang::Language::from_639_3(code_639_3)
        .and_then(|lang| lang.to_639_1())
        .map(str::to_string)
        .unwrap_or_else(|| code_639_3.to_string())
}

/// English display name for a stored code: `"de"` → `"German"`, `"en-US"` → `"English"`.
pub fn display_name(code: &str) -> Option<String> {
    let language = code.split('-').next()?;
    isolang::Language::from_639_1(language).map(|lang| lang.to_name().to_string())
}
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test --lib locale`
Expected: PASS — all ten tests.

If `test_non_linguistic_content_is_not_detected` fails, it means `whatlang` returned a reliable verdict for digits. Do not weaken the assertion: confirm the behavior by printing `whatlang::detect(...)` for that input, and if it genuinely is reported reliable, that is a real finding to raise rather than paper over.

- [ ] **Step 7: Commit**

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
git add Cargo.toml Cargo.lock src/indexer/locale.rs src/indexer/mod.rs
git commit -m "feat: add recipe locale resolution module"
```

---

### Task 3: Database column, model, and queries

**Files:**
- Create: `migrations/008_recipe_locale.sql`
- Modify: `src/db/models.rs` (`Recipe` ~line 38, `NewRecipe` ~line 67)
- Modify: `src/db/recipes.rs` (`create_recipe` ~line 127, `update_recipe_with_content` ~line 416; new `update_recipe_locale` and `list_locales`)

- [ ] **Step 1: Write the migration**

Create `migrations/008_recipe_locale.sql`:

```sql
-- Recipe locale: BCP-47-style code (e.g. 'en', 'de', 'en-US').
-- locale_source records whether the author declared it or we detected it.
ALTER TABLE recipes ADD COLUMN locale TEXT;
ALTER TABLE recipes ADD COLUMN locale_source TEXT;

CREATE INDEX IF NOT EXISTS idx_recipes_locale ON recipes(locale);
```

- [ ] **Step 2: Write the failing test**

Add to the `mod tests` block at the bottom of `src/db/recipes.rs`. Look at the existing test there for the setup pattern and the full `NewRecipe` literal; this test follows it.

```rust
    #[tokio::test]
    async fn test_create_and_update_recipe_locale() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();

        let feed = crate::db::feeds::create_feed(
            &pool,
            &crate::db::models::NewFeed {
                url: "https://example.com/feed.xml".to_string(),
                title: Some("Test Feed".to_string()),
            },
        )
        .await
        .unwrap();

        let new_recipe = NewRecipe {
            feed_id: feed.id,
            external_id: "recipe-1".to_string(),
            title: "Pfannkuchen".to_string(),
            source_url: None,
            enclosure_url: "https://example.com/recipe.cook".to_string(),
            content: Some("Mehl und Wasser verrühren.".to_string()),
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
            locale: Some("de".to_string()),
            locale_source: Some("detected".to_string()),
        };

        let recipe = create_recipe(&pool, &new_recipe).await.unwrap();
        assert_eq!(recipe.locale.as_deref(), Some("de"));
        assert_eq!(recipe.locale_source.as_deref(), Some("detected"));

        // An author-declared locale overwrites the detected one.
        update_recipe_locale(&pool, recipe.id, Some("fr"), Some("declared"))
            .await
            .unwrap();

        let updated = get_recipe(&pool, recipe.id).await.unwrap();
        assert_eq!(updated.locale.as_deref(), Some("fr"));
        assert_eq!(updated.locale_source.as_deref(), Some("declared"));

        // list_locales reports what is in the database.
        let locales = list_locales(&pool).await.unwrap();
        assert_eq!(locales, vec![("fr".to_string(), 1)]);
    }
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test --lib test_create_and_update_recipe_locale`
Expected: FAIL to compile — `struct 'NewRecipe' has no field named 'locale'`.

- [ ] **Step 4: Add the model fields**

In `src/db/models.rs`, add to the end of the `Recipe` struct (after `feed_entry_updated`):

```rust
    /// BCP-47-style locale code, e.g. "en", "de", "en-US". NULL when unknown.
    pub locale: Option<String>,
    /// How the locale was obtained: "declared" or "detected".
    pub locale_source: Option<String>,
```

Add the same two fields to the end of the `NewRecipe` struct (without the doc comments duplicated — a single `/// Locale code and its provenance ("declared" | "detected")` above the pair is enough):

```rust
    pub locale: Option<String>,
    pub locale_source: Option<String>,
```

- [ ] **Step 5: Persist locale on insert**

In `src/db/recipes.rs`, `create_recipe`: add the two columns to the INSERT column list, two more `?` placeholders, and two `.bind(...)` calls in matching order (immediately after `.bind(new_recipe.feed_entry_updated)`):

```rust
    let recipe = sqlx::query_as::<_, Recipe>(
        r#"
        INSERT INTO recipes (
            feed_id, external_id, title, source_url, enclosure_url,
            content, summary, servings, total_time_minutes, active_time_minutes,
            difficulty, image_url, published_at, updated_at, created_at, content_hash,
            content_etag, content_last_modified, feed_entry_updated, locale, locale_source
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        RETURNING *
        "#,
    )
```

and at the end of the bind chain:

```rust
    .bind(new_recipe.feed_entry_updated)
    .bind(&new_recipe.locale)
    .bind(&new_recipe.locale_source)
    .fetch_one(pool)
    .await?;
```

- [ ] **Step 6: Carry locale through content updates**

Still in `src/db/recipes.rs`, extend `update_recipe_with_content` with two more parameters and set the columns. Locale is derived from content, so it is recomputed whenever content changes:

```rust
#[allow(clippy::too_many_arguments)]
pub async fn update_recipe_with_content(
    pool: &DbPool,
    recipe_id: i64,
    content: &str,
    content_hash: Option<&str>,
    content_etag: Option<&str>,
    content_last_modified: Option<&chrono::DateTime<chrono::Utc>>,
    feed_entry_updated: Option<&chrono::DateTime<chrono::Utc>>,
    locale: Option<&str>,
    locale_source: Option<&str>,
) -> Result<()> {
    let now = Utc::now();

    sqlx::query(
        r#"
        UPDATE recipes
        SET content = ?, content_hash = ?, content_etag = ?,
            content_last_modified = ?, feed_entry_updated = ?, updated_at = ?,
            locale = ?, locale_source = ?
        WHERE id = ?
        "#,
    )
    .bind(content)
    .bind(content_hash)
    .bind(content_etag)
    .bind(content_last_modified)
    .bind(feed_entry_updated)
    .bind(now)
    .bind(locale)
    .bind(locale_source)
    .bind(recipe_id)
    .execute(pool)
    .await?;

    Ok(())
}
```

- [ ] **Step 7: Add `update_recipe_locale` and `list_locales`**

Append to `src/db/recipes.rs`, before the `#[cfg(test)]` module:

```rust
/// Update only a recipe's locale columns.
pub async fn update_recipe_locale(
    pool: &DbPool,
    recipe_id: i64,
    locale: Option<&str>,
    locale_source: Option<&str>,
) -> Result<()> {
    sqlx::query("UPDATE recipes SET locale = ?, locale_source = ? WHERE id = ?")
        .bind(locale)
        .bind(locale_source)
        .bind(recipe_id)
        .execute(pool)
        .await?;

    Ok(())
}

/// Every distinct locale present in the database with its recipe count,
/// most common first. Used to populate the language filter.
pub async fn list_locales(pool: &DbPool) -> Result<Vec<(String, i64)>> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        r#"
        SELECT locale, COUNT(*) as count
        FROM recipes
        WHERE locale IS NOT NULL
        GROUP BY locale
        ORDER BY count DESC, locale ASC
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows)
}
```

- [ ] **Step 8: Fix the other `NewRecipe` literals**

Adding fields to `NewRecipe` breaks every struct literal. Add `locale: None, locale_source: None,` to each of:
- `src/db/ingredients.rs:212` (test)
- `src/db/tags.rs:208` (test)
- `src/crawler/mod.rs:394` — leave as `None` for now; Task 4 fills it in.
- `src/github/indexer.rs:373` — leave as `None` for now; Task 5 fills it in.
- `tests/reindex_test.rs` (test literal)
- Any other site `cargo build` points at.

Also add `None, None` as the last two arguments to the `update_recipe_with_content` call at `src/crawler/mod.rs:359`; Task 4 replaces them with real values.

- [ ] **Step 9: Run the full test suite**

Run: `cargo test`
Expected: PASS, including the new `test_create_and_update_recipe_locale`.

- [ ] **Step 10: Commit**

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
git add migrations/008_recipe_locale.sql src/db tests/reindex_test.rs src/crawler/mod.rs src/github/indexer.rs
git commit -m "feat: add locale and locale_source columns to recipes"
```

---

### Task 4: Resolve locale in the feed crawler

`process_entry` in `src/crawler/mod.rs` currently parses the Cooklang content only on the *create* path, and only to pull out a fallback image (`src/crawler/mod.rs:381`). We hoist that parse so it happens once for both paths and feeds both the image lookup and locale resolution.

**Files:**
- Modify: `src/crawler/mod.rs` (`process_entry`, ~lines 350-425)

- [ ] **Step 1: Parse once, before the create/update branch**

Immediately after the `content_hash` is computed (~line 360) and *before* `let result = match existing_recipe {`, insert:

```rust
        // Parse the Cooklang content once: it feeds both the image fallback and
        // locale resolution, on the create and the update path alike.
        let parsed_content = content
            .as_ref()
            .and_then(|c| parse_cooklang_full(c).ok());

        let locale = parsed_content
            .as_ref()
            .and_then(crate::indexer::resolve_locale);
        let (locale_code, locale_source) = match &locale {
            Some(l) => (Some(l.code.as_str()), Some(l.source.as_str())),
            None => (None, None),
        };
```

- [ ] **Step 2: Pass locale into the update path**

In the `Some(recipe) => { ... }` arm, the `update_recipe_with_content` call gained two `None, None` arguments in Task 3. Replace them:

```rust
                if let Some(ref content_str) = content {
                    db::recipes::update_recipe_with_content(
                        pool,
                        recipe.id,
                        content_str,
                        content_hash.as_deref(),
                        content_etag.as_deref(),
                        content_last_modified_dt.as_ref(),
                        entry.updated.as_ref(),
                        locale_code,
                        locale_source,
                    )
                    .await?;
                }
```

- [ ] **Step 3: Use the hoisted parse for the image, and set locale on create**

In the `None => { ... }` arm, replace the local `metadata_image` re-parse with the hoisted `parsed_content`, and add the two locale fields to the `NewRecipe` literal (replacing the `None`s from Task 3):

```rust
                // Determine image URL: prefer feed entry image, fallback to Cooklang metadata
                let metadata_image = parsed_content
                    .as_ref()
                    .and_then(|parsed| parsed.metadata.as_ref())
                    .and_then(|m| m.image.clone());
                let image_url = entry
                    .image_url
                    .clone()
                    .or(metadata_image)
                    .and_then(|img| resolve_image_url(&img, enclosure_url));

                // Create new recipe
                let new_recipe = NewRecipe {
                    feed_id,
                    external_id: entry.id.clone(),
                    title: entry.title.clone(),
                    source_url: entry.source_url.clone(),
                    enclosure_url: enclosure_url.clone(),
                    content,
                    summary: entry.summary.clone(),
                    servings: entry.metadata.servings,
                    total_time_minutes: entry.metadata.total_time,
                    active_time_minutes: entry.metadata.active_time,
                    difficulty: entry.metadata.difficulty.clone(),
                    image_url,
                    published_at: entry.published,
                    content_hash,
                    content_etag,
                    content_last_modified: content_last_modified_dt,
                    feed_entry_updated: entry.updated,
                    locale: locale_code.map(str::to_string),
                    locale_source: locale_source.map(str::to_string),
                };
```

Note the `content` field moves the `Option<String>` into `new_recipe`, which is why `parsed_content` must be computed *before* this literal.

- [ ] **Step 4: Build**

Run: `cargo build`
Expected: success. If `parse_cooklang_full` is not in scope, check the existing `use` at the top of `src/crawler/mod.rs` — it is already imported for the image lookup.

- [ ] **Step 5: Run the crawler tests**

Run: `cargo test --test crawler_tests`
Expected: PASS (no behavior change for feeds without parseable content).

- [ ] **Step 6: Commit**

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
git add src/crawler/mod.rs
git commit -m "feat: resolve recipe locale in the feed crawler"
```

---

### Task 5: Resolve locale in the GitHub indexer

**Files:**
- Modify: `src/github/indexer.rs` (recipe create, ~lines 356-395)

- [ ] **Step 1: Resolve the locale**

`index_recipe` already parses the content into `let parsed = crate::indexer::parse_cooklang_full(&content);` (a `Result<ParsedRecipeData>`, ~line 324). Reuse it — do not parse a second time. Directly after the `let (summary, servings, total_time, metadata_image) = ...` block that follows it, add:

```rust
        // Locale: declared `locale:` metadata wins, otherwise detected from text.
        let locale = parsed.as_ref().ok().and_then(crate::indexer::resolve_locale);
        let (locale_code, locale_source) = match &locale {
            Some(l) => (Some(l.code.clone()), Some(l.source.as_str().to_string())),
            None => (None, None),
        };
```

`parsed.as_ref().ok()` borrows, so `parsed` is still available to the ingredient/tag extraction further down the function.

- [ ] **Step 2: Set it on the new recipe**

In the `NewRecipe` literal (~line 373), replace the `locale: None, locale_source: None,` placeholders from Task 3:

```rust
                locale: locale_code.clone(),
                locale_source: locale_source.clone(),
```

- [ ] **Step 3: Refresh locale on the update path**

The `if let Some(existing)` arm currently only updates the GitHub SHA. Add a locale refresh right after `update_github_recipe_sha`, so a file whose content changed gets a re-resolved locale:

```rust
            db::github::update_github_recipe_sha(&self.pool, existing.id, file_sha).await?;

            db::recipes::update_recipe_locale(
                &self.pool,
                existing.recipe_id,
                locale_code.as_deref(),
                locale_source.as_deref(),
            )
            .await?;

            recipe.id
```

- [ ] **Step 4: Build and test**

Run: `cargo build && cargo test`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
git add src/github/indexer.rs
git commit -m "feat: resolve recipe locale in the github indexer"
```

---

### Task 6: Index and filter locale in Tantivy

Two things to get right:

1. The `locale` field is `STRING` (untokenized), so `de` matches only the exact term `de`.
2. A recipe declared `en_US` is stored as `en-US`, but a user filtering by `en` must still find it. So `index_recipe` writes **both** the full code and its base language when they differ. `locale=en` then matches both `en` and `en-US` recipes; `locale=en-US` matches only the regional ones.

The field is deliberately **not** added to the `QueryParser`'s default field list — a free-text search for "de" must not match every German recipe. Filtering is only via the explicit parameter.

**Files:**
- Modify: `src/indexer/schema.rs`
- Modify: `src/indexer/search.rs` (`SearchQuery` ~line 18, `SearchResult` ~line 25, `index_recipe` ~line 81, `search` ~line 182)

- [ ] **Step 1: Write the failing test**

Add to the `mod tests` block at the bottom of `src/indexer/search.rs`. Copy the `Recipe` literal shape from the existing `test_index_recipe_deletes_before_adding` test — it constructs a full `Recipe` struct; add `locale` / `locale_source` to it.

```rust
    fn test_recipe(id: i64, title: &str, locale: Option<&str>) -> Recipe {
        Recipe {
            id,
            feed_id: 1,
            external_id: format!("ext-{id}"),
            title: title.to_string(),
            source_url: None,
            enclosure_url: format!("https://example.com/{id}.cook"),
            content: Some("Mix the flour and the water.".to_string()),
            summary: None,
            servings: None,
            total_time_minutes: None,
            active_time_minutes: None,
            difficulty: None,
            image_url: None,
            published_at: None,
            updated_at: None,
            indexed_at: None,
            created_at: chrono::Utc::now(),
            content_hash: None,
            content_etag: None,
            content_last_modified: None,
            feed_entry_updated: None,
            locale: locale.map(str::to_string),
            locale_source: locale.map(|_| "detected".to_string()),
        }
    }

    #[test]
    fn test_search_filters_by_locale() {
        let dir = tempdir().unwrap();
        let index = SearchIndex::new(dir.path()).unwrap();
        let mut writer = index.writer().unwrap();

        index
            .index_recipe(&mut writer, &test_recipe(1, "Pancakes", Some("en")), None, &[], &[])
            .unwrap();
        index
            .index_recipe(&mut writer, &test_recipe(2, "Pfannkuchen", Some("de")), None, &[], &[])
            .unwrap();
        index
            .index_recipe(&mut writer, &test_recipe(3, "Crepes", None), None, &[], &[])
            .unwrap();
        index.commit(&mut writer).unwrap();

        // Filtering by locale returns only that language.
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
        assert_eq!(results.results[0].recipe_id, 2);
        assert_eq!(results.results[0].locale.as_deref(), Some("de"));

        // No filter returns everything, including the recipe with no locale.
        let all = index
            .search(
                &SearchQuery {
                    q: String::new(),
                    page: 1,
                    limit: 10,
                    locale: None,
                },
                10,
            )
            .unwrap();
        assert_eq!(all.results.len(), 3);
    }

    #[test]
    fn test_locale_filter_combines_with_query() {
        let dir = tempdir().unwrap();
        let index = SearchIndex::new(dir.path()).unwrap();
        let mut writer = index.writer().unwrap();

        index
            .index_recipe(&mut writer, &test_recipe(1, "Pancakes", Some("en")), None, &[], &[])
            .unwrap();
        index
            .index_recipe(&mut writer, &test_recipe(2, "Pancakes", Some("de")), None, &[], &[])
            .unwrap();
        index.commit(&mut writer).unwrap();

        let results = index
            .search(
                &SearchQuery {
                    q: "pancakes".to_string(),
                    page: 1,
                    limit: 10,
                    locale: Some("en".to_string()),
                },
                10,
            )
            .unwrap();

        assert_eq!(results.results.len(), 1);
        assert_eq!(results.results[0].recipe_id, 1);
    }

    #[test]
    fn test_regional_locale_matches_base_language_filter() {
        let dir = tempdir().unwrap();
        let index = SearchIndex::new(dir.path()).unwrap();
        let mut writer = index.writer().unwrap();

        index
            .index_recipe(&mut writer, &test_recipe(1, "Biscuits", Some("en-US")), None, &[], &[])
            .unwrap();
        index.commit(&mut writer).unwrap();

        // Filtering by the base language finds the regional recipe...
        let base = index
            .search(
                &SearchQuery { q: String::new(), page: 1, limit: 10, locale: Some("en".to_string()) },
                10,
            )
            .unwrap();
        assert_eq!(base.results.len(), 1);

        // ...and the stored code keeps its region.
        assert_eq!(base.results[0].locale.as_deref(), Some("en-US"));

        // A different language does not match.
        let other = index
            .search(
                &SearchQuery { q: String::new(), page: 1, limit: 10, locale: Some("de".to_string()) },
                10,
            )
            .unwrap();
        assert_eq!(other.results.len(), 0);
    }
```

The existing tests in this module construct `SearchQuery` without a `locale` field — add `locale: None` to each of them.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib search`
Expected: FAIL to compile — `struct 'SearchQuery' has no field named 'locale'`.

- [ ] **Step 3: Add the schema field**

In `src/indexer/schema.rs`, add `pub locale: Field,` to the `RecipeSchema` struct after `file_path`, build it in `new()`, and add it to the returned struct:

```rust
        // File path (searchable, stored) - for GitHub recipes
        let file_path = schema_builder.add_text_field("file_path", TEXT | STORED);

        // Locale (exact-match filter, not tokenized, deliberately excluded from
        // the default query-parser fields so free text can't match it)
        let locale = schema_builder.add_text_field("locale", STRING | STORED);
```

- [ ] **Step 4: Write locale into the document**

In `src/indexer/search.rs`, `index_recipe`, after the `file_path` block:

```rust
        // Add locale, plus its base language when the code carries a region, so a
        // filter on "en" also matches an "en-US" recipe.
        if let Some(locale) = &recipe.locale {
            doc.add_text(self.schema.locale, locale);

            if let Some((language, _region)) = locale.split_once('-') {
                doc.add_text(self.schema.locale, language);
            }
        }
```

- [ ] **Step 5: Extend `SearchQuery` and `SearchResult`**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    pub q: String, // Unified query string
    pub page: usize,
    pub limit: usize,
    /// Optional exact-match language filter, e.g. "de" or "en-US".
    pub locale: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub recipe_id: i64,
    pub title: String,
    pub summary: Option<String>,
    pub score: f32,
    pub locale: Option<String>,
}
```

- [ ] **Step 6: Apply the filter in `search`**

Extend the imports at the top of `src/indexer/search.rs`:

```rust
use tantivy::query::{BooleanQuery, Occur, TermQuery};
use tantivy::schema::IndexRecordOption;
```

(keep the existing `Query`, `QueryParser`, `TopDocs`, `Term` imports; merge rather than duplicate.)

Then, right after the existing `let tantivy_query = if query.q.is_empty() { ... };` block:

```rust
        // AND an exact locale term onto the parsed query when filtering.
        let tantivy_query = match query.locale.as_deref().filter(|l| !l.is_empty()) {
            Some(locale) => {
                let term = Term::from_field_text(self.schema.locale, locale);
                let locale_query: Box<dyn Query> =
                    Box::new(TermQuery::new(term, IndexRecordOption::Basic));

                Box::new(BooleanQuery::new(vec![
                    (Occur::Must, tantivy_query),
                    (Occur::Must, locale_query),
                ])) as Box<dyn Query>
            }
            None => tantivy_query,
        };
```

- [ ] **Step 7: Return locale in results**

In the `filter_map` that builds each `SearchResult`, after `summary`:

```rust
                let locale = doc.get_first(self.schema.locale).and_then(|v| match v {
                    tantivy::schema::OwnedValue::Str(s) => Some(s.to_string()),
                    _ => None,
                });

                Some(SearchResult {
                    recipe_id,
                    title,
                    summary,
                    score,
                    locale,
                })
```

`get_first` returns the first value written for the field, which is the full code (with region) because Step 4 writes it before the base language.

- [ ] **Step 8: Fix the other `SearchQuery` literals**

`cargo build` will point at `src/api/handlers.rs:27` and `src/web/handlers.rs:74`. Add `locale: None` to both for now — Tasks 7 and 8 wire the real values.

- [ ] **Step 9: Run tests to verify they pass**

Run: `cargo test --lib search`
Expected: PASS, including the three new tests.

- [ ] **Step 10: Commit**

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
git add src/indexer/schema.rs src/indexer/search.rs src/api/handlers.rs src/web/handlers.rs
git commit -m "feat: index and filter recipes by locale in search"
```

---

### Task 7: API — `?locale=` filter and locale in responses

**Files:**
- Modify: `src/api/models.rs` (`SearchParams` ~line 5, `RecipeCard` ~line 31, `RecipeDetail` ~line 48)
- Modify: `src/api/handlers.rs` (`search_recipes` ~line 20, `get_recipe` ~line 67)

- [ ] **Step 1: Extend the API models**

In `src/api/models.rs`:

```rust
pub struct SearchParams {
    #[serde(default)]
    pub q: String, // Unified query string
    /// Optional language filter, e.g. "de".
    #[serde(default)]
    pub locale: Option<String>,
    #[serde(default = "default_page")]
    pub page: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
}
```

Add `pub locale: Option<String>,` to `RecipeCard`, and both fields to `RecipeDetail`:

```rust
pub struct RecipeCard {
    pub id: i64,
    pub title: String,
    pub summary: Option<String>,
    pub tags: Vec<String>,
    pub locale: Option<String>,
}
```

```rust
    pub enclosure_url: String,
    pub locale: Option<String>,
    pub locale_source: Option<String>,
    pub feed: FeedInfo,
```

- [ ] **Step 2: Thread the filter through the search handler**

In `src/api/handlers.rs`, `search_recipes`:

```rust
    let query = SearchQuery {
        q: params.q,
        page: params.page,
        limit: params.limit.min(state.settings.pagination.api_max_limit),
        locale: params.locale,
    };
```

and when building each card:

```rust
        recipe_cards.push(RecipeCard {
            id: result.recipe_id,
            title: result.title,
            summary: result.summary,
            tags,
            locale: result.locale,
        });
```

- [ ] **Step 3: Return locale from the detail endpoint**

In `get_recipe`, add to the `RecipeDetail` construction, after `enclosure_url`:

```rust
        enclosure_url: recipe.enclosure_url,
        locale: recipe.locale,
        locale_source: recipe.locale_source,
        feed: FeedInfo {
```

- [ ] **Step 4: Build and test**

Run: `cargo build && cargo test`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
git add src/api
git commit -m "feat: expose recipe locale in the API and filter search by it"
```

---

### Task 8: Web UI — language dropdown, card chip, detail pill

**Files:**
- Modify: `src/web/handlers.rs` (`SearchParams` ~line 51, `SearchTemplate` ~line 27, `RecipeCardData` ~line 37, `index` ~line 63, `RecipeData` ~line 174, `recipe_detail` ~line 207)
- Modify: `src/web/schema.rs` (`recipe_to_schema_json`)
- Modify: `src/web/templates/search.html`
- Modify: `src/web/templates/recipe.html`

- [ ] **Step 1: Extend the web view models**

In `src/web/handlers.rs`:

```rust
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
}

/// One entry in the language filter dropdown.
#[derive(Clone)]
#[allow(dead_code)] // Fields are used by Askama templates
struct LocaleOption {
    code: String,
    name: String,
    count: i64,
}
```

Add `locale_name: String` to `RecipeCardData` (empty string = unknown, matching how the other optional fields on that struct are handled), and to `SearchParams`:

```rust
#[derive(Deserialize)]
pub struct SearchParams {
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    q: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    locale: Option<String>,
    #[serde(default = "default_page")]
    page: usize,
}
```

- [ ] **Step 2: Build the dropdown options and thread the filter**

In `index`, at the top:

```rust
    let query = params.q.clone().unwrap_or_default();
    let locale = params.locale.clone().unwrap_or_default();

    // Language filter options: distinct locales in the database, most common first.
    // Regional codes ("en-US") are folded into their base language ("en") so the
    // dropdown lists one entry per language.
    let locales = {
        let mut counts: Vec<(String, i64)> = Vec::new();
        for (code, count) in db::recipes::list_locales(&state.pool).await? {
            let base = code.split('-').next().unwrap_or(&code).to_string();
            match counts.iter_mut().find(|(c, _)| *c == base) {
                Some((_, existing)) => *existing += count,
                None => counts.push((base, count)),
            }
        }
        counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

        counts
            .into_iter()
            .map(|(code, count)| LocaleOption {
                name: crate::indexer::locale::display_name(&code).unwrap_or_else(|| code.clone()),
                code,
                count,
            })
            .collect::<Vec<_>>()
    };
```

The results branch currently runs only `if query.is_empty()` → no results. A language filter with no query should still return results, so change the guard to cover both, and pass the locale into the search:

```rust
    let (results, total, total_pages) = if query.is_empty() && locale.is_empty() {
        (vec![], 0, 0)
    } else {
        // Build search query
        let search_query = SearchQuery {
            q: query.clone(),
            page: params.page,
            limit: state.settings.pagination.web_default_limit,
            locale: params.locale.clone(),
        };
```

In the same branch, where each `RecipeCardData` is built from the database row `r`, add:

```rust
                    locale_name: r
                        .locale
                        .as_deref()
                        .and_then(crate::indexer::locale::display_name)
                        .unwrap_or_default(),
```

Add the same line to the `recent_recipes` mapping further down (it builds `RecipeCardData` too), and update the `SearchTemplate` construction:

```rust
    let template = SearchTemplate {
        query,
        locale,
        locales,
        results,
        total,
        page: params.page,
        total_pages,
        recent_recipes,
    };
```

Note the `recent_recipes` block is gated on `query.is_empty()`; leave that gate alone — it should also require an empty locale so a filtered search doesn't show both. Change it to `if query.is_empty() && locale.is_empty()`.

- [ ] **Step 3: Add the dropdown to `search.html`**

In `src/web/templates/search.html`, inside the `<form method="GET" action="/">`, put the select between the text input and the submit button:

```html
            <div class="flex gap-2">
                <input
                    id="search-input"
                    type="text"
                    name="q"
                    value="{{ query }}"
                    placeholder="Search query"
                    class="flex-1 px-4 py-3 rounded-lg border border-gray-300 focus:ring-2 focus:ring-orange-500 focus:border-transparent"
                >
                {% if !locales.is_empty() %}
                <select
                    id="locale-select"
                    name="locale"
                    class="px-4 py-3 rounded-lg border border-gray-300 bg-white focus:ring-2 focus:ring-orange-500 focus:border-transparent"
                    aria-label="Filter by language"
                >
                    <option value="">All languages</option>
                    {% for option in locales %}
                    <option value="{{ option.code }}"{% if option.code == locale %} selected{% endif %}>{{ option.name }} ({{ option.count }})</option>
                    {% endfor %}
                </select>
                {% endif %}
                <button
                    type="submit"
                    class="px-6 py-3 bg-orange-600 text-white rounded-lg hover:bg-orange-700 focus:ring-2 focus:ring-orange-500 focus:ring-offset-2"
                >
                    Search
                </button>
            </div>
```

- [ ] **Step 4: Add the language chip to result cards**

Still in `search.html`, the card body ends with a row of facts:

```html
                <div class="flex items-center text-sm text-gray-500 space-x-4">
                    {% if !recipe.total_time_minutes.is_empty() %}
                    <span>⏱️ {{ recipe.total_time_minutes }}m</span>
                    {% endif %}
                    {% if !recipe.servings.is_empty() %}
                    <span>👥 {{ recipe.servings }}</span>
                    {% endif %}
                    {% if !recipe.difficulty.is_empty() %}
                    <span>📊 {{ recipe.difficulty }}</span>
                    {% endif %}
                </div>
```

Add the language as one more fact, after the difficulty block and inside that same `<div>`:

```html
                    {% if !recipe.locale_name.is_empty() %}
                    <span>🗣️ {{ recipe.locale_name }}</span>
                    {% endif %}
```

This card markup is repeated for the `recent_recipes` grid further down the same template — add the chip there too, so the homepage cards match.

- [ ] **Step 5: Add locale to the recipe detail page**

In `src/web/handlers.rs`, add to `RecipeData`:

```rust
    pub locale: String,
    pub locale_name: String,
    pub locale_detected: bool,
```

and in `recipe_detail`, where `RecipeData` is constructed:

```rust
        locale: recipe.locale.clone().unwrap_or_default(),
        locale_name: recipe
            .locale
            .as_deref()
            .and_then(crate::indexer::locale::display_name)
            .unwrap_or_default(),
        locale_detected: recipe.locale_source.as_deref() == Some("detected"),
```

`recipe.locale` is moved by `recipe.title` earlier in the literal only if you reorder — keep these lines after the existing fields and use `.clone()` as shown.

- [ ] **Step 6: Render the pill**

In `src/web/templates/recipe.html`, in the second metadata-pill block (the one reading `recipe.*`, after the `recipe.difficulty` pill around line 144):

```html
                {% if !recipe.locale_name.is_empty() %}
                <div class="metadata-pill">
                    <span class="emoji">🗣️</span>
                    <span><strong>{{ recipe.locale_name }}</strong>{% if recipe.locale_detected %} <span class="text-gray-500 text-sm">(detected)</span>{% endif %}</span>
                </div>
                {% endif %}
```

- [ ] **Step 7: Add `inLanguage` to the JSON-LD**

`recipe_to_schema_json` builds a `serde_json::Value` and assigns optional properties by index (`schema["image"] = json!(recipe.image_url);`). Follow that style — add this after the `// URL (source)` block:

```rust
    // Language
    if !recipe.locale.is_empty() {
        schema["inLanguage"] = json!(recipe.locale);
    }
```

- [ ] **Step 8: Build and test**

Run: `cargo build && cargo test`
Expected: PASS. Askama compiles templates at build time, so a template typo shows up here as a compile error.

- [ ] **Step 9: Verify in the browser**

Run: `cargo run -- serve`
Open `http://localhost:3000`, confirm the language dropdown lists languages with counts, that selecting one filters results, and that a recipe detail page shows the language pill.

- [ ] **Step 10: Commit**

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
git add src/web
git commit -m "feat: show and filter recipe language in the web ui"
```

---

### Task 9: `backfill-locales` CLI command

Recipe content is already in the `recipes` table, so backfill needs no network access. It walks recipes in batches keyed on `id` (never `OFFSET`, which would skip rows as we update them), resolves a locale, writes it, and re-indexes the recipe in Tantivy so the filter works over existing data.

**Note — deliberate side effect:** re-indexing touches every recipe with content, and the feed crawler does not currently write to the Tantivy index at all (only the GitHub indexer does). Backfill will therefore *add* feed-crawled recipes to the search index for the first time. This is intended: it closes a pre-existing gap. Expect search result counts to rise after the first run.

**Files:**
- Modify: `src/cli/mod.rs` (the `Commands` enum)
- Modify: `src/cli/commands.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Add the subcommand**

In `src/cli/mod.rs`, add to the end of the `Commands` enum, after `Reindex`:

```rust
    /// Detect and store the locale of recipes that don't have one
    BackfillLocales {
        /// Recompute the locale of every recipe, not just those without one
        #[arg(long)]
        force: bool,
    },
```

- [ ] **Step 2: Write the failing test**

Create `tests/locale_test.rs`:

```rust
use federation::cli::commands::backfill_locales;
use federation::db::models::{NewFeed, NewRecipe};
use federation::db::{feeds, recipes};
use federation::indexer::search::{SearchIndex, SearchQuery};
use sqlx::SqlitePool;
use tempfile::tempdir;

const GERMAN_RECIPE: &str = "Den Mehl und das Wasser in einer Schüssel verrühren, bis ein \
glatter Teig entsteht. Den Teig ruhen lassen und anschließend goldbraun backen.";

const ENGLISH_RECIPE: &str = "Mix the flour and the water in a bowl until a smooth dough \
forms. Let the dough rest, then bake it until golden brown.";

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

    let german = recipes::create_recipe(&pool, &new_recipe(feed_id, "de-1", "Pfannkuchen", GERMAN_RECIPE))
        .await
        .unwrap();
    let english = recipes::create_recipe(&pool, &new_recipe(feed_id, "en-1", "Pancakes", ENGLISH_RECIPE))
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

    let recipe = recipes::create_recipe(&pool, &new_recipe(feed_id, "de-1", "Pfannkuchen", GERMAN_RECIPE))
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
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --test locale_test`
Expected: FAIL to compile — `cannot find function 'backfill_locales'`.

- [ ] **Step 4: Implement the backfill**

Append to `src/cli/commands.rs` (check the file's existing `use` block; add whatever is missing):

```rust
/// What a backfill pass did.
#[derive(Debug, Default, Clone, Copy)]
pub struct BackfillStats {
    /// Recipes considered (had content, and matched the locale predicate).
    pub scanned: usize,
    /// Recipes whose locale we resolved and stored.
    pub updated: usize,
    /// Recipes we could not resolve a locale for.
    pub skipped: usize,
}

/// Detect and store locales for recipes that don't have one.
///
/// Recipe content is already in the database, so this makes no network calls.
/// Each touched recipe is re-indexed in Tantivy so the locale filter works over
/// existing data. With `force`, every recipe with content is recomputed.
pub async fn backfill_locales(
    pool: &crate::db::DbPool,
    search_index: &crate::indexer::search::SearchIndex,
    force: bool,
) -> Result<BackfillStats> {
    use crate::db::models::Recipe;

    /// Rows per batch. Keeps memory flat on large databases.
    const BATCH_SIZE: i64 = 500;

    let mut stats = BackfillStats::default();
    let mut writer = search_index.writer()?;
    let mut last_id: i64 = 0;

    loop {
        // Keyset pagination on id: rows we update drop out of the unfiltered
        // predicate, so an OFFSET would silently skip recipes.
        let sql = if force {
            "SELECT * FROM recipes WHERE content IS NOT NULL AND id > ? ORDER BY id LIMIT ?"
        } else {
            "SELECT * FROM recipes \
             WHERE content IS NOT NULL AND locale IS NULL AND id > ? ORDER BY id LIMIT ?"
        };

        let batch: Vec<Recipe> = sqlx::query_as::<_, Recipe>(sql)
            .bind(last_id)
            .bind(BATCH_SIZE)
            .fetch_all(pool)
            .await?;

        if batch.is_empty() {
            break;
        }

        for mut recipe in batch {
            last_id = recipe.id;
            stats.scanned += 1;

            let Some(content) = recipe.content.clone() else {
                stats.skipped += 1;
                continue;
            };

            let locale = match crate::indexer::parse_cooklang_full(&content) {
                Ok(parsed) => crate::indexer::resolve_locale(&parsed),
                Err(e) => {
                    warn!("Recipe {}: failed to parse content: {}", recipe.id, e);
                    None
                }
            };

            let Some(locale) = locale else {
                stats.skipped += 1;
                continue;
            };

            crate::db::recipes::update_recipe_locale(
                pool,
                recipe.id,
                Some(&locale.code),
                Some(locale.source.as_str()),
            )
            .await?;

            // Re-index with the locale so the search filter sees it.
            recipe.locale = Some(locale.code.clone());
            recipe.locale_source = Some(locale.source.as_str().to_string());

            let file_path = crate::db::github::get_github_recipe_by_recipe_id(pool, recipe.id)
                .await?
                .map(|gh| gh.file_path);
            let tags = crate::db::tags::get_tags_for_recipe(pool, recipe.id).await?;
            let ingredients = crate::db::ingredients::get_ingredients_for_recipe(pool, recipe.id)
                .await?
                .iter()
                .map(|i| i.name.clone())
                .collect::<Vec<_>>();

            search_index.index_recipe(
                &mut writer,
                &recipe,
                file_path.as_deref(),
                &tags,
                &ingredients,
            )?;

            stats.updated += 1;
        }
    }

    search_index.commit(&mut writer)?;

    Ok(stats)
}
```

If `warn!` is not already imported in this file, add `use tracing::warn;`.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --test locale_test`
Expected: PASS — both tests.

- [ ] **Step 6: Wire the command into `main.rs`**

In the `match cli.command` block, after the `Commands::Reindex` arm:

```rust
        Commands::BackfillLocales { force } => {
            backfill_locales(settings, force).await?;
        }
```

and add the function alongside `reindex_feed` (near line 276), following its shape:

```rust
async fn backfill_locales(settings: Settings, force: bool) -> Result<()> {
    info!("Backfilling recipe locales (force: {})", force);

    let pool = db::init_pool(&settings.database.url).await?;
    db::run_migrations(&pool).await?;

    let index_path = std::path::PathBuf::from(&settings.search.index_path);
    let search_index = SearchIndex::new(&index_path)?;

    let stats = federation::cli::commands::backfill_locales(&pool, &search_index, force).await?;

    println!(
        "\x1b[32m\u{2713}\x1b[0m Backfill complete: {} scanned, {} tagged, {} left without a locale",
        stats.scanned, stats.updated, stats.skipped
    );

    Ok(())
}
```

- [ ] **Step 7: Run it against the real database**

Run: `cargo run -- backfill-locales`
Expected: a summary line, e.g. `✓ Backfill complete: 412 scanned, 397 tagged, 15 left without a locale`.

Then re-run it: `cargo run -- backfill-locales`
Expected: `0 scanned` — already-tagged recipes are not rescanned, which proves the run was persisted.

- [ ] **Step 8: Commit**

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
git add src/cli src/main.rs tests/locale_test.rs
git commit -m "feat: add backfill-locales command"
```

---

### Task 10: End-to-end verification

- [ ] **Step 1: Full test suite**

Run: `cargo test`
Expected: PASS, no warnings from `cargo clippy --all-targets -- -D warnings`.

- [ ] **Step 2: Exercise the API**

Run: `cargo run -- serve` in one terminal, then:

```bash
curl -s 'http://localhost:3000/api/search?q=&locale=de&limit=3' | head -40
curl -s 'http://localhost:3000/api/recipes/1' | grep -o '"locale[^,]*'
```

Expected: the search response contains only recipes whose `locale` is `de`; the detail response carries `locale` and `locale_source`.

- [ ] **Step 3: Update the README**

`README.md` documents the CLI commands and search syntax. Add `backfill-locales` to the command list and mention the `locale` search filter in whatever section covers the API's query parameters. Match the existing formatting.

- [ ] **Step 4: Commit**

```bash
git add README.md
git commit -m "docs: document locale filter and backfill-locales command"
```
