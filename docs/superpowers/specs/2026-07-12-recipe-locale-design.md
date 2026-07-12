# Recipe Locale Detection — Design

Date: 2026-07-12

## Goal

Tag every recipe with a locale during indexing, so recipes can be filtered by
language in search and their language shown in the web UI and API.

## Sources of locale

Locale is **derived from recipe content**. It is recomputed whenever a recipe's
content is parsed (create or content update), so an author who later adds a
`locale:` key gets picked up on the next crawl.

Resolution order:

1. **Declared** — Cooklang's canonical `locale:` metadata key. `cooklang`'s
   `Metadata::locale()` returns `(language, Option<region>)`, e.g.
   `("en", Some("US"))`. Normalized to `en` / `en-US` (lowercase language,
   uppercase region). A declared locale always wins.
2. **Detected** — `whatlang::detect()` run over a plain-text rendering of the
   recipe. Accepted only when `Info::is_reliable()` is true and the text clears
   a minimum length.
3. **Neither** — `locale` stays NULL. We do not store a low-confidence guess.

### Detection input

The detector is fed text built from `ParsedRecipeData` (the output of
`src/indexer/cooklang_parser.rs`), never the raw `.cook` source. This keeps
Cooklang markup (`@flour{200%g}`, `#oven{}`, `~{20%minutes}`) out of the
trigram statistics. The text is the concatenation of:

- metadata title and description
- every `StepItem::Text` value across all sections
- section notes
- ingredient names (strong language signal: "flour" vs "farine")

Quantities, units and cookware are excluded.

Guard: if the assembled text is shorter than 25 characters, return `None`
rather than detecting — there is not enough signal.

### Code normalization

`whatlang` returns ISO 639-3 (`eng`, `deu`). Cooklang's declared locale is
BCP-47-ish (`en`, `en-US`). Everything is normalized to a 2-letter BCP-47
language code via the `isolang` crate; if a language has no 639-1 code, the
639-3 code is stored as-is. Region is only ever kept when it was *declared* —
detection never invents one.

## Components

### `src/indexer/locale.rs` (new)

```rust
pub enum LocaleSource { Declared, Detected }

pub struct RecipeLocale {
    pub code: String,          // "en", "en-US", "de"
    pub source: LocaleSource,  // declared | detected
}

pub fn resolve_locale(parsed: &ParsedRecipeData) -> Option<RecipeLocale>;
```

`LocaleSource` serializes to the strings `"declared"` / `"detected"` for
storage.

New dependencies: `whatlang` (detection) and `isolang` (639-3 → 639-1 mapping
and English display names for the UI).

### Database — migration `008_recipe_locale.sql`

```sql
ALTER TABLE recipes ADD COLUMN locale TEXT;
ALTER TABLE recipes ADD COLUMN locale_source TEXT;  -- 'declared' | 'detected'
CREATE INDEX IF NOT EXISTS idx_recipes_locale ON recipes(locale);
```

Portable across the SQLite and Postgres backends both used by sqlx here.

`Recipe` and `NewRecipe` in `src/db/models.rs` gain
`locale: Option<String>` and `locale_source: Option<String>`. The INSERT in
`src/db/recipes.rs` and `update_recipe_with_content` carry both fields.

Keeping `locale_source` separate from `locale` means the UI can distinguish a
stated language from our guess, and a future re-detection pass can refresh
detected values without clobbering declared ones.

### Ingestion

Both ingestion paths already parse Cooklang content:

- `src/crawler/mod.rs` (feed crawler) — currently parses content only to fish
  out a fallback image URL, and only on the create path.
- `src/github/indexer.rs` (GitHub indexer).

Both are changed to parse content **once** per recipe and reuse the resulting
`ParsedRecipeData` for image extraction *and* locale resolution, on both the
create and the content-update path.

### Search

`RecipeSchema` (`src/indexer/schema.rs`) gains:

```rust
let locale = schema_builder.add_text_field("locale", STRING | STORED);
```

`STRING` (untokenized) gives exact term matching on the code.

The field is deliberately **excluded** from the QueryParser's default field
list, so a free-text search for "de" does not match every German recipe.
Filtering is explicit instead:

- `SearchQuery` gains `locale: Option<String>`.
- `SearchIndex::search()` ANDs a `TermQuery` on the locale field onto the
  parsed free-text query (a `BooleanQuery` with both as MUST clauses).
- `SearchResult` gains `locale: Option<String>`, read from the stored field, so
  result cards can show the language without a database round-trip.

`index_recipe()` writes the locale into the document.

### API

- `GET /api/search?q=…&locale=de` — new optional `locale` param on
  `SearchParams`, threaded into `SearchQuery`.
- Recipe responses in `src/api/models.rs` gain `locale` and `locale_source`.

### Web UI

- `recipe.html` — language displayed alongside servings/time, using the English
  language name from `isolang` (e.g. "German"), with a quiet "detected"
  qualifier when `locale_source = detected`.
- `search.html` — a language dropdown that sets `?locale=`, threaded through the
  existing web search handler.
- Search and browse cards — a small language chip.

### Backfill — `federation backfill-locales [--force]`

A new CLI subcommand alongside the existing `reindex`. Recipe content is already
stored in the `recipes` table, so backfill needs no network fetches.

- Pages through `recipes WHERE locale IS NULL AND content IS NOT NULL`
  (bounded batches, so memory stays flat on large databases).
- Resolves locale for each, updates the row.
- Re-indexes the recipe's Tantivy document (with its tags and ingredients from
  the database) so the `locale` filter works over pre-existing data.
- `--force` recomputes every recipe with content, not just NULL-locale ones.

New and updated recipes get their locale automatically via the crawler and
GitHub indexer, so this command is a one-off for existing data.

## Error handling

- Cooklang parse failure: recipe is stored as before with `locale = NULL`.
  Locale is a nice-to-have; it never blocks ingestion.
- Unreliable or too-short detection: `locale = NULL`, no guess stored.
- Invalid declared locale (Cooklang's `locale()` returns `None` for malformed
  values): falls through to detection.
- Backfill: a failure on one recipe is logged and skipped; the pass continues.

## Testing

Unit tests in `src/indexer/locale.rs`:

- a declared `locale:` wins over what detection would have said
- English, German, Russian and French recipe fixtures each detect correctly
- region is preserved from a declared `en-US`, and never invented by detection
- too-short content and non-linguistic content (e.g. digits/symbols) → `None`
- ISO 639-3 → 639-1 mapping (`deu` → `de`)

Integration tests:

- a recipe created by the crawler carries a locale and a `locale_source`
- `locale=` search filter returns only recipes in that language, and combines
  with a free-text query
- `backfill-locales` fills NULL-locale rows and leaves already-set rows
  untouched (unless `--force`)

## Out of scope

- Per-language Tantivy tokenizers/stemmers. The locale is stored and filterable,
  but text analysis stays language-agnostic for now.
- Translating UI strings, or serving different content per user locale.
- Storing a raw detection confidence value.
