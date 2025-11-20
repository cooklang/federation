# Research: Fixing Duplicate Search Results (Issue #5)

## Executive Summary

**Issue:** The search function returns duplicate results when the same recipe is published by multiple feeds/sources.

**Root Cause:** The system uses a per-feed deduplication strategy `(feed_id, external_id)` but does not detect when identical recipes come from different sources. Each recipe gets a unique ID in the database and search index, leading to duplicates in search results.

**Recommended Solution:** Implement a hybrid approach with both immediate post-search deduplication and long-term content-based canonical recipe system.

---

## Problem Description

### Issue Details
- **GitHub Issue:** #5 - "Search function returns duplicates"
- **Reporter:** tmlmt (Nov 20, 2025)
- **Platform:** recipes.cooklang.org
- **Symptom:** Searching for recipes (e.g., "Lasagna") returns multiple search result items pointing to identical or near-identical recipes

### User Impact
When people copy and republish recipes from other sources, the search results show redundant entries, creating a poor user experience with:
- Cluttered search results
- Difficulty identifying unique recipes
- Wasted time reviewing duplicate content
- Reduced perceived quality of the platform

---

## Root Cause Analysis

### Current Architecture Overview

The Cooklang Federation system indexes recipes from multiple sources:

1. **RSS/Atom Feeds** - Recipe feeds from various publishers
2. **GitHub Repositories** - .cook files from GitHub repos

#### Data Flow
```
┌─────────────────┐         ┌──────────────────┐
│  RSS/Atom Feed  │────────▶│   Feed Crawler   │
└─────────────────┘         │ (crawler/mod.rs) │
                            └─────────┬─────────┘
┌─────────────────┐                  │
│ GitHub Repos    │────────┐         │
└─────────────────┘         │         │
                            ▼         ▼
                    ┌────────────────────────┐
                    │   SQLite Database      │
                    │   (recipes table)      │
                    │   UNIQUE(feed_id,      │
                    │          external_id)  │
                    └───────────┬────────────┘
                                │
                                ▼
                    ┌────────────────────────┐
                    │  Tantivy Search Index  │
                    │  (indexer/search.rs)   │
                    └───────────┬────────────┘
                                │
                                ▼
                    ┌────────────────────────┐
                    │    Search API          │
                    │  (api/handlers.rs:20)  │
                    └────────────────────────┘
```

### The Deduplication Gap

#### Current Deduplication Strategy

**Database Level** (`migrations/001_init.sql:38`):
```sql
CREATE TABLE recipes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    feed_id INTEGER NOT NULL,
    external_id TEXT NOT NULL,
    title TEXT NOT NULL,
    ...
    UNIQUE(feed_id, external_id)  -- Only prevents duplicates within same feed
);
```

**What This Prevents:**
- ✅ Same feed publishing the same recipe twice (same `external_id`)
- ✅ GitHub repo having the same file path twice

**What This DOESN'T Prevent:**
- ❌ Feed A publishing "Chocolate Cake" and Feed B publishing the same "Chocolate Cake"
- ❌ Recipe appearing in both RSS feed and GitHub repo
- ❌ Multiple people copying and republishing the same recipe

#### Example Duplicate Scenario

```
Scenario: User searches for "Lasagna"

Database State:
┌────┬─────────┬─────────────┬──────────────────┐
│ ID │ Feed ID │ External ID │ Title            │
├────┼─────────┼─────────────┼──────────────────┤
│ 42 │    1    │ "recipe-x"  │ "Lasagna Recipe" │ ← Feed A
│ 89 │    2    │ "recipe-y"  │ "Lasagna Recipe" │ ← Feed B (copied from A)
│145 │    3    │ "lasagna.ck"│ "Lasagna Recipe" │ ← GitHub (copied from A)
└────┴─────────┴─────────────┴──────────────────┘

Search Index: Contains all 3 entries with IDs 42, 89, 145

Search Results: Returns all 3, showing the same recipe 3 times
```

---

## Technical Deep Dive

### Database Schema

**File:** `migrations/001_init.sql:20-39`

```sql
CREATE TABLE IF NOT EXISTS recipes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    feed_id INTEGER NOT NULL REFERENCES feeds(id) ON DELETE CASCADE,
    external_id TEXT NOT NULL,       -- Source-specific ID (RSS entry ID or file path)
    title TEXT NOT NULL,
    source_url TEXT,                 -- Original URL (if available)
    enclosure_url TEXT NOT NULL,     -- .cook file URL
    content TEXT,                    -- Full recipe content
    summary TEXT,
    servings INTEGER,
    total_time_minutes INTEGER,
    active_time_minutes INTEGER,
    difficulty TEXT CHECK(difficulty IN ('easy', 'medium', 'hard')),
    image_url TEXT,
    published_at TIMESTAMP,
    updated_at TIMESTAMP,
    indexed_at TIMESTAMP,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(feed_id, external_id)     -- ⚠️ Only per-feed uniqueness
);
```

**Key Fields for Deduplication:**
- `title` - Recipe name (useful but not unique)
- `content` - Full .cook file content (best for content-based matching)
- `enclosure_url` - Could indicate same source, but often different URLs
- `source_url` - Often NULL or different even for copied recipes

### Search Implementation

**File:** `src/indexer/search.rs:174-252`

```rust
pub fn search(&self, query: &SearchQuery, max_limit: usize) -> Result<SearchResults> {
    let searcher = self.reader.searcher();

    // Parse query and search
    let tantivy_query = query_parser.parse_query(&query.q)?;
    let top_docs = searcher.search(
        &*tantivy_query,
        &TopDocs::with_limit(limit + offset)
    )?;

    // Extract results - NO DEDUPLICATION HAPPENS HERE
    let results: Vec<SearchResult> = top_docs
        .into_iter()
        .skip(offset)
        .take(limit)
        .filter_map(|(score, doc_address)| {
            // ... extract recipe_id, title, summary from Tantivy document
            Some(SearchResult {
                recipe_id,  // Each duplicate has different ID
                title,
                summary,
                score,
            })
        })
        .collect();

    Ok(SearchResults { results, total, page, total_pages })
}
```

**API Handler:** `src/api/handlers.rs:20-64`

```rust
pub async fn search_recipes(
    State(state): State<AppState>,
    Query(params): Query<SearchParams>,
) -> Result<Json<SearchResponse>> {
    // Execute search
    let results = state.search_index.search(&query, max_results)?;

    // Fetch tags for all recipes
    let recipe_ids: Vec<i64> = results.results.iter().map(|r| r.recipe_id).collect();
    let tags_map = db::tags::get_tags_for_recipes(&state.pool, &recipe_ids).await?;

    // Build recipe cards - NO DEDUPLICATION HERE EITHER
    let mut recipe_cards = Vec::new();
    for result in results.results {
        recipe_cards.push(RecipeCard {
            id: result.recipe_id,
            title: result.title,
            summary: result.summary,
            tags: tags_map.get(&result.recipe_id).cloned().unwrap_or_default(),
        });
    }

    Ok(Json(SearchResponse { results: recipe_cards, pagination }))
}
```

**Observation:** Neither the search index nor the API handler performs any deduplication logic.

### Search Index Schema

**File:** `src/indexer/schema.rs:1-89`

Fields indexed by Tantivy:
- `id` (i64) - Recipe database ID (unique per recipe entry)
- `title` (TEXT) - Searchable, stored
- `summary` (TEXT) - Searchable, stored
- `instructions` (TEXT) - Searchable, NOT stored
- `ingredients` (TEXT) - Searchable, stored
- `tags` (TEXT) - Searchable, stored
- `difficulty` (STRING) - Searchable, stored
- `file_path` (TEXT) - Searchable, stored

**Note:** Each recipe entry gets indexed with its unique database ID. There's no canonical ID or content hash to group duplicates.

### Recipe Ingestion Flow

#### From GitHub

**File:** `src/github/indexer.rs:287-423`

```rust
// Process each .cook file
for file in cook_files {
    let recipe = self.index_recipe(
        github_feed_id,
        &file,
        &repo.owner,
        &repo.repo_name,
    ).await?;

    successful_recipe_ids.push(recipe.id);
}

// Batch add to search index
if !successful_recipe_ids.is_empty() {
    let mut search_writer = self.search_index.writer()?;

    for recipe_id in successful_recipe_ids {
        let recipe = db::recipes::get_recipe(&self.pool, recipe_id).await?;
        let tags = db::tags::get_tags_for_recipe(&self.pool, recipe_id).await?;
        let ingredients = db::ingredients::get_ingredients_for_recipe(...).await?;

        self.search_index.index_recipe(
            &mut search_writer,
            &recipe,
            file_path.as_deref(),
            &tags,
            &ingredients,
        )?;
    }

    search_writer.commit()?;
}
```

#### From RSS/Atom Feeds

**File:** `src/crawler/mod.rs:178-223`

```rust
for entry in entries {
    // Get or create recipe
    let (recipe, is_new) = db::recipes::get_or_create_recipe(
        &self.pool,
        &new_recipe,
    ).await?;

    if is_new {
        new_count += 1;
        // Parse and index ingredients, tags...
    }
}
```

**⚠️ IMPORTANT:** The feed crawler does NOT add recipes to the search index! This is a separate issue but worth noting.

### Get-or-Create Pattern

**File:** `src/db/recipes.rs:242-257`

```rust
pub async fn get_or_create_recipe(
    pool: &DbPool,
    new_recipe: &NewRecipe,
) -> Result<(Recipe, bool)> {
    // Try to find existing recipe BY FEED_ID AND EXTERNAL_ID ONLY
    let existing = sqlx::query_as::<_, Recipe>(
        "SELECT * FROM recipes WHERE feed_id = ? AND external_id = ?"
    )
    .bind(new_recipe.feed_id)
    .bind(&new_recipe.external_id)
    .fetch_optional(pool)
    .await?;

    if let Some(recipe) = existing {
        Ok((recipe, false))  // Already exists in this feed
    } else {
        let recipe = create_recipe(pool, new_recipe).await?;
        Ok((recipe, true))   // New for this feed (but might be duplicate of another feed's recipe!)
    }
}
```

**The Problem:** This function only checks if the recipe exists in the SAME feed. It doesn't check if an identical recipe already exists from a different feed.

---

## Solution Approaches

### Option 1: Post-Search Deduplication (Quick Fix)

**Implementation Location:** `src/api/handlers.rs:20-64` (search_recipes function)

**Strategy:** Deduplicate search results after they come back from Tantivy but before returning to user.

#### Approach 1A: Title-Based Deduplication (Simplest)

```rust
// After getting results from search index
let mut seen_titles = std::collections::HashSet::new();
let mut deduped_cards = Vec::new();

for result in results.results {
    let normalized_title = result.title.to_lowercase().trim();

    if seen_titles.insert(normalized_title) {
        // First time seeing this title
        let tags = tags_map.get(&result.recipe_id).cloned().unwrap_or_default();
        deduped_cards.push(RecipeCard {
            id: result.recipe_id,
            title: result.title,
            summary: result.summary,
            tags,
        });
    }
    // else: skip duplicate
}
```

**Pros:**
- ✅ Simple to implement (5-10 lines of code)
- ✅ No database changes required
- ✅ Works immediately
- ✅ No dependencies needed

**Cons:**
- ❌ Title-only matching is imperfect (e.g., "Lasagna" vs "My Mom's Lasagna")
- ❌ Might incorrectly deduplicate different recipes with similar names
- ❌ Pagination counts will be off (total count includes duplicates)
- ❌ Wastes search index capacity on duplicates

#### Approach 1B: Fuzzy Title Matching

```rust
use strsim::jaro_winkler;  // Add to Cargo.toml

let mut deduped_cards = Vec::new();
let threshold = 0.90;  // 90% similarity

for result in results.results {
    let is_duplicate = deduped_cards.iter().any(|existing: &RecipeCard| {
        let similarity = jaro_winkler(&existing.title, &result.title);
        similarity >= threshold
    });

    if !is_duplicate {
        // Add to results
    }
}
```

**Pros:**
- ✅ More accurate than exact title matching
- ✅ Catches variants like "Chocolate Cake" vs "Classic Chocolate Cake"
- ✅ Still relatively simple

**Cons:**
- ❌ Requires new dependency (`strsim` crate)
- ❌ O(n²) complexity for large result sets (but limited by page size)
- ❌ Still doesn't fix pagination counts
- ❌ Similarity threshold is arbitrary and needs tuning

**Recommended Library:** `strsim = "0.11"` - Pure Rust, no unsafe code, well-maintained

#### Approach 1C: Over-Fetch and Deduplicate

```rust
// Fetch more results than requested to account for duplicates
let expanded_limit = query.limit * 3;  // Fetch 3x more
let results = state.search_index.search(&query_with_expanded_limit, max)?;

// Deduplicate with fuzzy matching
let deduped = deduplicate_recipes(results.results, 0.90);

// Trim to actual requested limit
let final_results = deduped.into_iter().take(query.limit).collect();
```

**Pros:**
- ✅ Maintains accurate pagination (mostly)
- ✅ Ensures user gets full page of unique results
- ✅ Better user experience

**Cons:**
- ❌ Inefficient - searches more than needed
- ❌ Pagination metadata still inaccurate
- ❌ Complexity in determining over-fetch multiplier

---

### Option 2: Content Hash Based Deduplication (Medium-Term)

**Implementation:** Add content-based hashing to detect identical recipes.

#### Database Migration

**New file:** `migrations/00X_add_content_hash.sql`

```sql
-- Add content hash column for deduplication
ALTER TABLE recipes ADD COLUMN content_hash TEXT;

-- Index for fast lookup
CREATE INDEX idx_recipes_content_hash ON recipes(content_hash);

-- Trigger to auto-calculate hash on insert/update (optional)
CREATE TRIGGER calculate_content_hash_insert
AFTER INSERT ON recipes
BEGIN
    UPDATE recipes
    SET content_hash = LOWER(HEX(
        -- Hash of normalized title + content
        CAST(title || COALESCE(content, '') AS BLOB)
    ))
    WHERE id = NEW.id AND content_hash IS NULL;
END;
```

#### Recipe Processing Update

**File:** `src/db/recipes.rs` (update `get_or_create_recipe`)

```rust
use sha2::{Sha256, Digest};

pub async fn get_or_create_recipe(
    pool: &DbPool,
    new_recipe: &NewRecipe,
) -> Result<(Recipe, bool)> {
    // Calculate content hash
    let content_hash = calculate_content_hash(
        &new_recipe.title,
        new_recipe.content.as_deref(),
    );

    // First check if recipe exists by content hash
    let existing_by_hash = sqlx::query_as::<_, Recipe>(
        "SELECT * FROM recipes WHERE content_hash = ? LIMIT 1"
    )
    .bind(&content_hash)
    .fetch_optional(pool)
    .await?;

    if let Some(recipe) = existing_by_hash {
        // Same recipe already exists from another feed
        // Could: link them as duplicates, or just return the existing one
        return Ok((recipe, false));
    }

    // Check by feed_id + external_id (existing logic)
    // ... existing code ...

    // Create new recipe with content_hash
    create_recipe_with_hash(pool, new_recipe, content_hash).await
}

fn calculate_content_hash(title: &str, content: Option<&str>) -> String {
    let mut hasher = Sha256::new();

    // Normalize title (lowercase, trim, remove extra whitespace)
    let normalized_title = title
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    hasher.update(normalized_title.as_bytes());

    if let Some(content) = content {
        // Normalize content (remove whitespace variations, comments, etc.)
        let normalized_content = normalize_cooklang_content(content);
        hasher.update(normalized_content.as_bytes());
    }

    format!("{:x}", hasher.finalize())
}

fn normalize_cooklang_content(content: &str) -> String {
    // Remove comments, normalize whitespace, etc.
    content
        .lines()
        .map(|line| {
            // Remove comments
            let line = line.split("--").next().unwrap_or(line);
            // Trim and normalize whitespace
            line.trim()
        })
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}
```

#### Search Index Update

**File:** `src/indexer/schema.rs` - Add content_hash field

```rust
pub struct SearchSchema {
    pub id: Field,
    pub content_hash: Field,  // NEW
    pub title: Field,
    // ... other fields
}

impl SearchSchema {
    pub fn new() -> Self {
        let mut schema_builder = Schema::builder();

        let id = schema_builder.add_i64_field("id", STORED);
        let content_hash = schema_builder.add_text_field("content_hash", STRING | STORED);  // NEW
        let title = schema_builder.add_text_field("title", TEXT | STORED);
        // ...
    }
}
```

**File:** `src/indexer/search.rs` - Deduplicate by content_hash

```rust
pub fn search(&self, query: &SearchQuery, max_limit: usize) -> Result<SearchResults> {
    // ... existing search logic ...

    // NEW: Deduplicate by content_hash
    let mut seen_hashes = std::collections::HashSet::new();
    let results: Vec<SearchResult> = top_docs
        .into_iter()
        .skip(offset)
        .take(limit * 2)  // Fetch more to account for deduplication
        .filter_map(|(score, doc_address)| {
            let doc = searcher.doc::<tantivy::TantivyDocument>(doc_address).ok()?;

            let content_hash = doc.get_first(self.schema.content_hash)?
                .as_str()?
                .to_string();

            // Skip if we've seen this content hash
            if !seen_hashes.insert(content_hash) {
                return None;  // Duplicate
            }

            // Extract and return result
            let recipe_id = ...;
            Some(SearchResult { recipe_id, title, summary, score })
        })
        .take(limit)  // Take only requested amount after deduplication
        .collect();

    Ok(SearchResults { results, ... })
}
```

**Pros:**
- ✅ Accurate content-based deduplication
- ✅ Persistent - works across all search queries
- ✅ Can be used for other features (e.g., detecting updates)
- ✅ Relatively straightforward

**Cons:**
- ❌ Requires database migration
- ❌ Needs careful hash calculation (what to include/exclude)
- ❌ Need to backfill hashes for existing recipes
- ❌ Hash collisions possible (though unlikely with SHA256)

---

### Option 3: Canonical Recipe System (Long-Term, Robust)

**Implementation:** Create a separate canonical recipes table and link duplicates.

#### Database Schema

**New file:** `migrations/00X_canonical_recipes.sql`

```sql
-- Canonical recipes table (one entry per unique recipe)
CREATE TABLE canonical_recipes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    canonical_title TEXT NOT NULL,
    content_hash TEXT UNIQUE NOT NULL,
    first_seen_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_canonical_recipes_hash ON canonical_recipes(content_hash);
CREATE INDEX idx_canonical_recipes_title ON canonical_recipes(canonical_title);

-- Link recipes to their canonical version
ALTER TABLE recipes ADD COLUMN canonical_recipe_id INTEGER REFERENCES canonical_recipes(id);
CREATE INDEX idx_recipes_canonical_id ON recipes(canonical_recipe_id);

-- Recipe sources tracking (which feed published this recipe)
CREATE TABLE recipe_sources (
    canonical_recipe_id INTEGER NOT NULL REFERENCES canonical_recipes(id) ON DELETE CASCADE,
    recipe_id INTEGER NOT NULL REFERENCES recipes(id) ON DELETE CASCADE,
    is_primary BOOLEAN DEFAULT 0,  -- Which version to show by default
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (canonical_recipe_id, recipe_id)
);
```

#### Recipe Ingestion Update

```rust
pub async fn get_or_create_canonical_recipe(
    pool: &DbPool,
    new_recipe: &NewRecipe,
) -> Result<(Recipe, CanonicalRecipe, bool)> {
    let content_hash = calculate_content_hash(&new_recipe.title, new_recipe.content.as_deref());

    // Check if canonical recipe exists
    let canonical = match get_canonical_by_hash(pool, &content_hash).await? {
        Some(canon) => canon,
        None => {
            // Create new canonical recipe
            create_canonical_recipe(pool, &new_recipe.title, &content_hash).await?
        }
    };

    // Check if this specific feed entry exists
    let existing = get_recipe_by_feed_and_external_id(
        pool,
        new_recipe.feed_id,
        &new_recipe.external_id,
    ).await?;

    let (recipe, is_new) = match existing {
        Some(r) => (r, false),
        None => {
            let mut recipe = create_recipe(pool, new_recipe).await?;
            recipe.canonical_recipe_id = Some(canonical.id);
            update_recipe_canonical_id(pool, recipe.id, canonical.id).await?;
            (recipe, true)
        }
    };

    // Link recipe to canonical version
    link_recipe_to_canonical(pool, canonical.id, recipe.id).await?;

    Ok((recipe, canonical, is_new))
}
```

#### Search Index Update

Index by canonical_recipe_id instead of recipe_id:

```rust
pub struct SearchSchema {
    pub canonical_recipe_id: Field,  // Index the canonical ID
    pub recipe_id: Field,             // Keep for reference
    pub title: Field,
    // ...
}

pub fn index_recipe(&self, writer: &mut IndexWriter, recipe: &Recipe, ...) -> Result<()> {
    let mut doc = TantivyDocument::new();

    // Index with canonical ID (deduplicates at index time)
    if let Some(canonical_id) = recipe.canonical_recipe_id {
        doc.add_i64(self.schema.canonical_recipe_id, canonical_id);
    }

    doc.add_i64(self.schema.recipe_id, recipe.id);
    doc.add_text(self.schema.title, &recipe.title);
    // ...

    // When adding to index, remove old versions of same canonical recipe
    self.delete_by_canonical_id(writer, canonical_id)?;
    writer.add_document(doc)?;

    Ok(())
}
```

#### API Response Enhancement

```rust
#[derive(Debug, Clone, Serialize)]
pub struct RecipeCard {
    pub id: i64,                              // Canonical ID
    pub title: String,
    pub summary: Option<String>,
    pub tags: Vec<String>,
    pub source_count: usize,                   // NEW: How many sources have this recipe
    pub sources: Vec<RecipeSource>,            // NEW: List of sources
}

#[derive(Debug, Clone, Serialize)]
pub struct RecipeSource {
    pub feed_id: i64,
    pub feed_title: Option<String>,
    pub recipe_url: String,
}
```

**Pros:**
- ✅ Most robust and scalable solution
- ✅ True deduplication at the data model level
- ✅ Enables rich features (show all sources, choose preferred version)
- ✅ Accurate search results and pagination
- ✅ Clean separation of concerns

**Cons:**
- ❌ Complex implementation (significant refactoring)
- ❌ Migration complexity for existing data
- ❌ Requires backfilling canonical IDs for all existing recipes
- ❌ Changes API contracts (may need versioning)
- ❌ Needs careful handling of updates (which version wins?)

---

### Option 4: Smart Result Grouping (UX-Focused)

**Implementation:** Group duplicates in search results but show them as alternatives.

#### API Response Update

```rust
#[derive(Debug, Clone, Serialize)]
pub struct SearchResponse {
    pub results: Vec<RecipeGroup>,  // Changed from Vec<RecipeCard>
    pub pagination: Pagination,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecipeGroup {
    pub primary: RecipeCard,
    pub alternatives: Vec<RecipeCard>,  // Other sources for same recipe
    pub total_sources: usize,
}
```

#### Search Handler Update

```rust
pub async fn search_recipes(...) -> Result<Json<SearchResponse>> {
    let results = state.search_index.search(&query, max)?;

    // Group similar recipes
    let groups = group_similar_recipes(results.results, &state.pool).await?;

    Ok(Json(SearchResponse { results: groups, pagination }))
}

async fn group_similar_recipes(
    results: Vec<SearchResult>,
    pool: &DbPool,
) -> Result<Vec<RecipeGroup>> {
    let mut groups: Vec<RecipeGroup> = Vec::new();

    for result in results {
        // Check if similar to existing group
        let similar_group = groups.iter_mut().find(|g| {
            is_similar_recipe(&g.primary.title, &result.title)
        });

        match similar_group {
            Some(group) => {
                // Add as alternative
                group.alternatives.push(RecipeCard { ... });
                group.total_sources += 1;
            }
            None => {
                // Create new group
                groups.push(RecipeGroup {
                    primary: RecipeCard { ... },
                    alternatives: vec![],
                    total_sources: 1,
                });
            }
        }
    }

    Ok(groups)
}
```

#### Frontend Display

```
Search results for "Lasagna":

┌─────────────────────────────────────────────────┐
│ 🍝 Classic Lasagna                              │
│ A delicious Italian layered pasta dish...       │
│ Tags: Italian, Pasta, Main Course              │
│                                                  │
│ 📚 Also available from:                         │
│   • John's Recipe Blog                          │
│   • GitHub: recipes/italian                     │
│   [View all 3 sources]                          │
└─────────────────────────────────────────────────┘
```

**Pros:**
- ✅ Transparent to users (shows all sources)
- ✅ Users can choose preferred source
- ✅ No information loss
- ✅ Respects original content creators

**Cons:**
- ❌ Requires frontend changes
- ❌ More complex UI
- ❌ Still needs similarity detection algorithm
- ❌ Pagination becomes complicated

---

## Recommended Implementation Plan

### Phase 1: Quick Fix (Days 1-2)

**Goal:** Immediately improve user experience with minimal changes.

**Implementation:** Option 1B + 1C (Fuzzy matching with over-fetch)

**Steps:**
1. Add `strsim = "0.11"` to `Cargo.toml`
2. Implement `deduplicate_by_similarity()` function in `src/api/handlers.rs`
3. Update `search_recipes()` handler to:
   - Over-fetch results (3x multiplier)
   - Deduplicate using fuzzy title matching (90% threshold)
   - Trim to requested limit
4. Add tests for deduplication logic
5. Deploy and monitor

**Code Location:** `src/api/handlers.rs:20-64`

**Estimated Effort:** 2-4 hours

**Risks:**
- May incorrectly group slightly different recipes
- Pagination counts slightly inaccurate
- Not a permanent solution

### Phase 2: Content Hash System (Weeks 1-2)

**Goal:** Implement persistent, accurate deduplication.

**Implementation:** Option 2 (Content hash based)

**Steps:**
1. Create migration `00X_add_content_hash.sql`
2. Implement `calculate_content_hash()` function
3. Update `get_or_create_recipe()` to check content_hash first
4. Create migration script to backfill hashes for existing recipes
5. Add content_hash to search index schema
6. Update search logic to deduplicate by hash
7. Add monitoring for duplicate detection rate
8. Deploy migration and backfill

**Code Locations:**
- `migrations/00X_add_content_hash.sql` (new)
- `src/db/recipes.rs:242-257` (update)
- `src/indexer/schema.rs` (update)
- `src/indexer/search.rs:174-252` (update)

**Estimated Effort:** 1-2 weeks

**Risks:**
- Migration on large dataset may take time
- Hash calculation needs tuning
- Need to handle edge cases (missing content, etc.)

### Phase 3: Canonical Recipe System (Months 1-2)

**Goal:** Full-featured duplicate management with source tracking.

**Implementation:** Option 3 (Canonical recipes)

**Steps:**
1. Design canonical recipe schema
2. Create migrations for new tables
3. Implement canonical recipe management
4. Update all recipe ingestion paths
5. Migrate existing recipes to canonical system
6. Update search index to use canonical IDs
7. Update API to show source information
8. Update frontend to display multiple sources
9. Add admin tools for managing duplicates

**Code Locations:**
- `migrations/00X_canonical_recipes.sql` (new)
- `src/db/recipes.rs` (major refactor)
- `src/indexer/` (updates)
- `src/api/models.rs` (new fields)
- `src/api/handlers.rs` (updates)

**Estimated Effort:** 1-2 months

**Risks:**
- Large migration requiring careful planning
- API breaking changes may need versioning
- Complex data backfill

---

## Implementation Details: Phase 1 (Quick Fix)

### Code Changes

**File:** `Cargo.toml`
```toml
[dependencies]
# ... existing dependencies ...
strsim = "0.11"  # Add string similarity
```

**File:** `src/api/handlers.rs`

```rust
use strsim::jaro_winkler;

/// Deduplicate search results by title similarity
fn deduplicate_recipes(
    results: Vec<SearchResult>,
    threshold: f64,
) -> Vec<SearchResult> {
    let mut deduped = Vec::new();

    for result in results {
        // Check if similar to any existing result
        let is_duplicate = deduped.iter().any(|existing: &SearchResult| {
            let similarity = jaro_winkler(&existing.title, &result.title);
            similarity >= threshold
        });

        if !is_duplicate {
            deduped.push(result);
        } else {
            debug!(
                "Skipping duplicate: '{}' (similar to existing result)",
                result.title
            );
        }
    }

    deduped
}

/// GET /api/search - Search recipes
pub async fn search_recipes(
    State(state): State<AppState>,
    Query(params): Query<SearchParams>,
) -> Result<Json<SearchResponse>> {
    debug!("Search request: {:?}", params);

    // Build search query with over-fetching to account for deduplication
    let over_fetch_multiplier = 3;
    let expanded_limit = params.limit * over_fetch_multiplier;

    let query = SearchQuery {
        q: params.q,
        page: params.page,
        limit: expanded_limit.min(state.settings.pagination.api_max_limit),
    };

    // Execute search
    let results = state
        .search_index
        .search(&query, state.settings.pagination.max_search_results)?;

    // Deduplicate by title similarity (90% threshold)
    let deduped_results = deduplicate_recipes(results.results, 0.90);

    // Trim to actual requested limit
    let final_results: Vec<_> = deduped_results
        .into_iter()
        .take(params.limit)
        .collect();

    // Batch fetch tags for all recipes
    let recipe_ids: Vec<i64> = final_results.iter().map(|r| r.recipe_id).collect();
    let tags_map = db::tags::get_tags_for_recipes(&state.pool, &recipe_ids).await?;

    // Build recipe cards
    let mut recipe_cards = Vec::new();
    for result in final_results {
        let tags = tags_map.get(&result.recipe_id).cloned().unwrap_or_default();

        recipe_cards.push(RecipeCard {
            id: result.recipe_id,
            title: result.title,
            summary: result.summary,
            tags,
        });
    }

    // Note: pagination.total is not fully accurate due to deduplication
    // but gives a reasonable approximation
    let estimated_total = (results.total as f64 / over_fetch_multiplier as f64) as usize;

    Ok(Json(SearchResponse {
        results: recipe_cards,
        pagination: Pagination {
            page: params.page,
            limit: params.limit,
            total: estimated_total,
            total_pages: estimated_total.div_ceil(params.limit),
        },
    }))
}
```

### Testing

**File:** `tests/search_deduplication_test.rs` (new)

```rust
#[tokio::test]
async fn test_deduplication_exact_titles() {
    // Create test recipes with identical titles
    // Run search
    // Assert only one result returned
}

#[tokio::test]
async fn test_deduplication_similar_titles() {
    // Create recipes: "Chocolate Cake" and "Classic Chocolate Cake"
    // Run search with 90% threshold
    // Assert only one result returned
}

#[tokio::test]
async fn test_no_deduplication_different_recipes() {
    // Create recipes: "Chocolate Cake" and "Vanilla Cake"
    // Run search
    // Assert both results returned
}
```

---

## Monitoring and Metrics

### Metrics to Track

1. **Duplicate Detection Rate**
   - How many search results are being deduplicated
   - Track per query

2. **False Positive Rate**
   - Different recipes incorrectly merged
   - User feedback / manual review

3. **Search Result Quality**
   - Click-through rate on search results
   - User satisfaction surveys

4. **Performance Impact**
   - Search latency before/after deduplication
   - Database query performance

### Logging

```rust
debug!(
    "Search deduplication: {} results -> {} unique (removed {} duplicates)",
    original_count,
    deduped_count,
    original_count - deduped_count
);
```

---

## Alternative Considerations

### Why Not Use Tantivy's Built-in Deduplication?

Tantivy doesn't have built-in deduplication features. It's designed as a search library, not a data deduplication system. We need to implement this at the application level.

### Why Not Prevent Duplicates at Ingestion?

This would be ideal, but:
- Requires significant refactoring of ingestion pipeline
- Need to decide which source is "primary" for each recipe
- May lose valuable information (different feeds may have different metadata)
- Complex migration for existing data

Better to fix search results first (user-facing) then optimize backend later.

### Why Not Use Database Views?

SQLite views could help, but:
- Search index is in Tantivy, not SQLite
- Would need to rebuild entire search index architecture
- Doesn't solve the fundamental problem of multiple recipe IDs

---

## Open Questions and Future Considerations

### 1. Handling Recipe Variations

**Question:** Are "Chocolate Cake" and "Vegan Chocolate Cake" duplicates?

**Answer:** Probably not - they're variations. Need careful tuning of similarity threshold.

**Future Enhancement:** Use ingredient lists and instructions for similarity, not just titles.

### 2. User Preferences

**Question:** Should users be able to choose preferred sources?

**Answer:** Yes, in Phase 3 (canonical system).

**Implementation:** Allow users to select preferred feeds, hide certain sources, etc.

### 3. Recipe Updates

**Question:** If a recipe is updated in one feed, should all linked duplicates be updated?

**Answer:** No - each feed's version should be independent. But canonical version should track "most recently updated" or "most complete."

### 4. Content Licensing

**Question:** Legal implications of grouping recipes from different sources?

**Answer:** Consult legal team. May need to clearly attribute each source and maintain clear separation.

### 5. Backfill Strategy

**Question:** How to handle existing recipes when implementing content hash system?

**Answer:**
- Run backfill migration during low-traffic period
- Process in batches to avoid locking database
- Monitor progress and have rollback plan
- Accept that some hashes may need recalculation if algorithm changes

---

## Conclusion

The duplicate search results issue stems from the system's per-feed deduplication strategy, which allows identical recipes from different sources to have different database IDs and appear multiple times in search results.

**Recommended approach:**
1. **Immediate (Phase 1):** Implement fuzzy title-based deduplication in search handler
2. **Short-term (Phase 2):** Add content hash system for accurate deduplication
3. **Long-term (Phase 3):** Build canonical recipe system with full source tracking

This phased approach balances quick user experience improvements with long-term architectural robustness.

---

## Code References

Key files to modify:

| File | Lines | Purpose | Phase |
|------|-------|---------|-------|
| `src/api/handlers.rs` | 20-64 | Search API handler | 1 |
| `Cargo.toml` | - | Add strsim dependency | 1 |
| `migrations/00X_add_content_hash.sql` | - | Add hash column | 2 |
| `src/db/recipes.rs` | 242-257 | Recipe creation logic | 2 |
| `src/indexer/schema.rs` | 1-89 | Search index schema | 2 |
| `src/indexer/search.rs` | 174-252 | Search implementation | 2 |
| `migrations/00X_canonical_recipes.sql` | - | Canonical system | 3 |

---

**Research completed:** 2025-11-20
**Issue:** https://github.com/cooklang/federation/issues/5
**Status:** Ready for implementation
