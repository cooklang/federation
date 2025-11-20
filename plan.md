# Deduplication Implementation Plan

## Overview

This plan implements a two-phase approach to eliminate duplicate search results:

1. **Phase 0 (Critical):** Delete-before-add logic to fix same recipe_id duplicates
2. **Phase 2:** Content hash-based deduplication to prevent cross-feed duplicates

## Architecture Overview

```
┌─────────────────┐
│ Recipe Ingestion│
│  (RSS/GitHub)   │
└────────┬────────┘
         │
         ▼
┌─────────────────────────────────────┐
│ Content Hash Calculation            │
│ • Normalize title + content         │
│ • Generate SHA-256 hash             │
└────────┬────────────────────────────┘
         │
         ▼
┌─────────────────────────────────────┐
│ Database: Check for Duplicates      │
│ • Query by content_hash             │
│ • Return existing OR create new     │
└────────┬────────────────────────────┘
         │
         ▼
┌─────────────────────────────────────┐
│ Search Index: Delete-Before-Add     │
│ • DELETE old docs with recipe_id    │
│ • ADD new document                  │
│ • COMMIT to Tantivy                 │
└─────────────────────────────────────┘
```

---

## Phase 0: Delete-Before-Add Logic (CRITICAL)

### Problem
When recipes are updated, the current code adds a new Tantivy document without deleting the old one. This causes the same recipe_id to appear multiple times in search results.

### Solution
Modify `index_recipe()` to delete existing documents before adding new ones.

### Implementation

#### File: `src/indexer/search.rs`

**Location:** Lines 81-139 (in `index_recipe` function)

**Current Code:**
```rust
pub fn index_recipe(
    &self,
    writer: &mut IndexWriter,
    recipe: &Recipe,
    file_path: Option<&str>,
    tags: &[String],
    ingredients: &[String],
) -> Result<()> {
    debug!("Indexing recipe: {}", recipe.id);

    // Build document
    let mut doc = TantivyDocument::new();
    // ... add fields ...

    writer.add_document(doc)?;  // ❌ BUG: Adds without deleting!

    Ok(())
}
```

**New Code:**
```rust
pub fn index_recipe(
    &self,
    writer: &mut IndexWriter,
    recipe: &Recipe,
    file_path: Option<&str>,
    tags: &[String],
    ingredients: &[String],
) -> Result<()> {
    debug!("Indexing recipe: {}", recipe.id);

    // ✅ DELETE existing documents with this recipe_id FIRST
    let term = Term::from_field_i64(self.schema.id, recipe.id);
    writer.delete_term(term);
    debug!("Deleted existing search documents for recipe_id: {}", recipe.id);

    // Build document
    let mut doc = TantivyDocument::new();
    doc.add_i64(self.schema.id, recipe.id);
    doc.add_text(self.schema.title, &recipe.title);

    if let Some(summary) = &recipe.summary {
        doc.add_text(self.schema.summary, summary);
    }

    if let Some(content) = &recipe.content {
        let parsed = cooklang::parse(content);
        let instructions_text = parsed.sections
            .iter()
            .flat_map(|s| &s.items)
            .filter_map(|item| {
                if let cooklang::Item::Text(text) = item {
                    Some(text.text.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join(" ");

        if !instructions_text.is_empty() {
            doc.add_text(self.schema.instructions, &instructions_text);
        }
    }

    for ingredient in ingredients {
        doc.add_text(self.schema.ingredients, ingredient);
    }

    for tag in tags {
        doc.add_text(self.schema.tags, tag);
    }

    if let Some(difficulty) = &recipe.difficulty {
        doc.add_text(self.schema.difficulty, difficulty);
    }

    if let Some(path) = file_path {
        doc.add_text(self.schema.file_path, path);
    }

    // ✅ Now add the new/updated document
    writer.add_document(doc)?;
    debug!("Indexed recipe: {} - {}", recipe.id, recipe.title);

    Ok(())
}
```

**Changes:**
1. Add 2 lines before document creation:
   - `let term = Term::from_field_i64(self.schema.id, recipe.id);`
   - `writer.delete_term(term);`
2. Add debug logging for deletion

**Import Required:**
- `use tantivy::Term;` (should already be imported)

### Testing Phase 0

#### 1. Manual Testing

```bash
# Before fix: Identify a recipe with duplicates
curl "http://localhost:3000/api/search?q=Lasagna" | jq '.results[] | .id'

# After fix: Rebuild search index
rm -rf data/search_index/
cargo run --bin indexer

# Search again: Should see no duplicates
curl "http://localhost:3000/api/search?q=Lasagna" | jq '.results[] | .id' | sort | uniq -c

# Should show count of 1 for each recipe_id
```

#### 2. Update Testing

```bash
# Make a change to a recipe file in a GitHub repo
# Run indexer again
cargo run --bin indexer

# Search for that recipe
# Should appear only ONCE (not twice)
```

#### 3. Unit Test

**File:** `src/indexer/search.rs` (add to tests module)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_index_recipe_deletes_before_adding() {
        // Create test index
        let schema = SearchSchema::new();
        let index = Index::create_in_ram(schema.schema.clone());
        let search_index = SearchIndex {
            index: index.clone(),
            schema,
            reader: index.reader().unwrap(),
        };

        let mut writer = search_index.writer().unwrap();

        // Create test recipe
        let recipe = Recipe {
            id: 123,
            title: "Test Recipe".to_string(),
            summary: Some("Test summary".to_string()),
            content: None,
            // ... other fields
        };

        // Index recipe first time
        search_index.index_recipe(
            &mut writer,
            &recipe,
            None,
            &[],
            &[],
        ).unwrap();
        writer.commit().unwrap();

        // Verify one document exists
        search_index.reader.reload().unwrap();
        let searcher = search_index.reader.searcher();
        let query = TermQuery::new(
            Term::from_field_i64(search_index.schema.id, 123),
            Default::default(),
        );
        let count = searcher.search(&query, &Count).unwrap();
        assert_eq!(count, 1, "Should have exactly 1 document after first index");

        // Update recipe (same ID)
        let updated_recipe = Recipe {
            id: 123,
            title: "Updated Test Recipe".to_string(),
            summary: Some("Updated summary".to_string()),
            content: None,
            // ... other fields
        };

        // Index again (simulating an update)
        let mut writer = search_index.writer().unwrap();
        search_index.index_recipe(
            &mut writer,
            &updated_recipe,
            None,
            &[],
            &[],
        ).unwrap();
        writer.commit().unwrap();

        // Verify STILL only one document (not two!)
        search_index.reader.reload().unwrap();
        let searcher = search_index.reader.searcher();
        let count = searcher.search(&query, &Count).unwrap();
        assert_eq!(count, 1, "Should STILL have exactly 1 document after update (delete-before-add)");

        // Verify the title was updated
        let top_docs = searcher.search(&query, &TopDocs::with_limit(1)).unwrap();
        assert_eq!(top_docs.len(), 1);
        let doc = searcher.doc::<TantivyDocument>(top_docs[0].1).unwrap();
        let title = doc.get_first(search_index.schema.title)
            .unwrap()
            .as_str()
            .unwrap();
        assert_eq!(title, "Updated Test Recipe");
    }
}
```

### Deployment Steps

1. **Code Review:** Ensure changes are correct
2. **Test Locally:** Run unit tests and manual tests
3. **Deploy Code:** Push to production
4. **Rebuild Index:**
   ```bash
   # Stop application
   systemctl stop federation

   # Backup existing index (optional)
   cp -r data/search_index data/search_index.backup

   # Delete index to force clean rebuild
   rm -rf data/search_index/

   # Run indexer to rebuild
   cargo run --release --bin indexer

   # Start application
   systemctl start federation
   ```
5. **Verify:** Check search results for known duplicates
6. **Monitor:** Watch logs for any errors

### Estimated Effort
- **Coding:** 15 minutes
- **Testing:** 30 minutes
- **Deployment:** 15 minutes
- **Index Rebuild:** Depends on data size (estimate 10-60 minutes)
- **Total:** ~1-2 hours

---

## Phase 2: Content Hash Based Deduplication

### Problem
Different recipe_ids pointing to identical content from multiple feeds/sources create duplicate search results.

### Solution
Add content hash to recipes table and use it to detect duplicates during ingestion.

### Implementation

#### Step 1: Database Migration

**File:** `migrations/002_add_content_hash.sql` (new file)

```sql
-- Add content hash column for deduplication
ALTER TABLE recipes ADD COLUMN content_hash TEXT;

-- Index for fast duplicate lookups
CREATE INDEX idx_recipes_content_hash ON recipes(content_hash);

-- Note: We intentionally don't add UNIQUE constraint because:
-- 1. We want to track which feeds published the same recipe
-- 2. We'll deduplicate in search index instead
-- 3. Allows flexibility for future canonical recipe system
```

**Migration Test:**
```bash
# Apply migration
sqlite3 data/federation.db < migrations/002_add_content_hash.sql

# Verify
sqlite3 data/federation.db "PRAGMA table_info(recipes);" | grep content_hash
sqlite3 data/federation.db ".indexes recipes" | grep content_hash
```

#### Step 2: Content Hash Calculation

**File:** `src/db/recipes.rs` (add to beginning of file)

```rust
use sha2::{Sha256, Digest};

/// Calculate content hash for deduplication
///
/// Hash is based on:
/// - Normalized title (lowercase, trimmed, whitespace collapsed)
/// - Normalized content (cooklang content without comments/formatting)
///
/// This allows us to detect identical recipes even if they come from
/// different feeds or have minor formatting differences.
pub fn calculate_content_hash(title: &str, content: Option<&str>) -> String {
    let mut hasher = Sha256::new();

    // Normalize title
    let normalized_title = normalize_title(title);
    hasher.update(normalized_title.as_bytes());

    // Normalize and hash content if available
    if let Some(content) = content {
        let normalized_content = normalize_cooklang_content(content);
        hasher.update(normalized_content.as_bytes());
    }

    // Return hex string
    format!("{:x}", hasher.finalize())
}

/// Normalize title for consistent hashing
fn normalize_title(title: &str) -> String {
    title
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

/// Normalize cooklang content for consistent hashing
///
/// Removes:
/// - Comments (-- lines and [- ... -] blocks)
/// - Extra whitespace
/// - Empty lines
///
/// Preserves:
/// - Ingredient syntax (@ingredient{})
/// - Cookware syntax (#cookware{})
/// - Timer syntax (~timer{})
/// - Step order and content
fn normalize_cooklang_content(content: &str) -> String {
    let lines: Vec<String> = content
        .lines()
        .filter_map(|line| {
            // Remove inline comments
            let line = line.split("--").next().unwrap_or(line);

            // Trim whitespace
            let line = line.trim();

            // Skip empty lines
            if line.is_empty() {
                return None;
            }

            Some(line.to_string())
        })
        .collect();

    let mut result = lines.join("\n");

    // Remove block comments [- ... -]
    while let Some(start) = result.find("[-") {
        if let Some(end) = result[start..].find("-]") {
            result.replace_range(start..start + end + 2, "");
        } else {
            break;
        }
    }

    // Collapse multiple newlines into one
    while result.contains("\n\n\n") {
        result = result.replace("\n\n\n", "\n\n");
    }

    result.trim().to_string()
}

#[cfg(test)]
mod hash_tests {
    use super::*;

    #[test]
    fn test_normalize_title() {
        assert_eq!(
            normalize_title("  Chocolate   Cake  "),
            "chocolate cake"
        );
        assert_eq!(
            normalize_title("CHOCOLATE CAKE"),
            "chocolate cake"
        );
    }

    #[test]
    fn test_same_content_produces_same_hash() {
        let content1 = ">> ingredients\n@flour{500%g}\n@sugar{200%g}\n\n>> steps\nMix ingredients.";
        let content2 = ">> ingredients\n@flour{500%g}\n@sugar{200%g}\n\n>> steps\nMix ingredients.";

        let hash1 = calculate_content_hash("Chocolate Cake", Some(content1));
        let hash2 = calculate_content_hash("Chocolate Cake", Some(content2));

        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_whitespace_differences_produce_same_hash() {
        let content1 = "@flour{500%g}\n@sugar{200%g}";
        let content2 = "@flour{500%g}  \n  @sugar{200%g}";

        let hash1 = calculate_content_hash("Cake", Some(content1));
        let hash2 = calculate_content_hash("Cake", Some(content2));

        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_comments_dont_affect_hash() {
        let content1 = "@flour{500%g}\n-- This is a comment\n@sugar{200%g}";
        let content2 = "@flour{500%g}\n@sugar{200%g}";

        let hash1 = calculate_content_hash("Cake", Some(content1));
        let hash2 = calculate_content_hash("Cake", Some(content2));

        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_different_content_produces_different_hash() {
        let content1 = "@flour{500%g}";
        let content2 = "@flour{600%g}";

        let hash1 = calculate_content_hash("Cake", Some(content1));
        let hash2 = calculate_content_hash("Cake", Some(content2));

        assert_ne!(hash1, hash2);
    }
}
```

**Dependencies:** Add to `Cargo.toml` if not already present:
```toml
[dependencies]
sha2 = "0.10"
```

#### Step 3: Update Recipe Creation

**File:** `src/db/recipes.rs`

**Current `NewRecipe` struct:** (around line 20)
```rust
pub struct NewRecipe {
    pub feed_id: i64,
    pub external_id: String,
    pub title: String,
    pub source_url: Option<String>,
    pub enclosure_url: String,
    pub content: Option<String>,
    pub summary: Option<String>,
    pub servings: Option<i32>,
    pub total_time_minutes: Option<i32>,
    pub active_time_minutes: Option<i32>,
    pub difficulty: Option<String>,
    pub image_url: Option<String>,
    pub published_at: Option<String>,
    pub updated_at: Option<String>,
}
```

**Add field:**
```rust
pub struct NewRecipe {
    pub feed_id: i64,
    pub external_id: String,
    pub title: String,
    pub source_url: Option<String>,
    pub enclosure_url: String,
    pub content: Option<String>,
    pub summary: Option<String>,
    pub servings: Option<i32>,
    pub total_time_minutes: Option<i32>,
    pub active_time_minutes: Option<i32>,
    pub difficulty: Option<String>,
    pub image_url: Option<String>,
    pub published_at: Option<String>,
    pub updated_at: Option<String>,
    pub content_hash: Option<String>,  // NEW
}
```

**Update `Recipe` struct:** (around line 50)
```rust
pub struct Recipe {
    pub id: i64,
    pub feed_id: i64,
    pub external_id: String,
    pub title: String,
    pub source_url: Option<String>,
    pub enclosure_url: String,
    pub content: Option<String>,
    pub summary: Option<String>,
    pub servings: Option<i32>,
    pub total_time_minutes: Option<i32>,
    pub active_time_minutes: Option<i32>,
    pub difficulty: Option<String>,
    pub image_url: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub indexed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub content_hash: Option<String>,  // NEW
}
```

**Update `create_recipe` function:** (around line 100)

```rust
pub async fn create_recipe(pool: &DbPool, new_recipe: &NewRecipe) -> Result<Recipe> {
    let recipe = sqlx::query_as::<_, Recipe>(
        r#"
        INSERT INTO recipes (
            feed_id, external_id, title, source_url, enclosure_url,
            content, summary, servings, total_time_minutes, active_time_minutes,
            difficulty, image_url, published_at, updated_at, content_hash
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        RETURNING *
        "#,
    )
    .bind(new_recipe.feed_id)
    .bind(&new_recipe.external_id)
    .bind(&new_recipe.title)
    .bind(&new_recipe.source_url)
    .bind(&new_recipe.enclosure_url)
    .bind(&new_recipe.content)
    .bind(&new_recipe.summary)
    .bind(new_recipe.servings)
    .bind(new_recipe.total_time_minutes)
    .bind(new_recipe.active_time_minutes)
    .bind(&new_recipe.difficulty)
    .bind(&new_recipe.image_url)
    .bind(&new_recipe.published_at)
    .bind(&new_recipe.updated_at)
    .bind(&new_recipe.content_hash)  // NEW
    .fetch_one(pool)
    .await
    .context("Failed to create recipe")?;

    debug!("Created recipe: {} (hash: {:?})", recipe.id, recipe.content_hash);
    Ok(recipe)
}
```

**Update `get_or_create_recipe` function:** (around line 242)

```rust
pub async fn get_or_create_recipe(
    pool: &DbPool,
    new_recipe: &NewRecipe,
) -> Result<(Recipe, bool)> {
    // Try to find existing recipe by feed_id and external_id
    let existing = sqlx::query_as::<_, Recipe>(
        "SELECT * FROM recipes WHERE feed_id = ? AND external_id = ?"
    )
    .bind(new_recipe.feed_id)
    .bind(&new_recipe.external_id)
    .fetch_optional(pool)
    .await?;

    if let Some(recipe) = existing {
        debug!(
            "Recipe already exists: {} (feed: {}, external_id: {})",
            recipe.id, recipe.feed_id, recipe.external_id
        );
        Ok((recipe, false))
    } else {
        let recipe = create_recipe(pool, new_recipe).await?;
        debug!(
            "Created new recipe: {} (feed: {}, external_id: {}, hash: {:?})",
            recipe.id, recipe.feed_id, recipe.external_id, recipe.content_hash
        );
        Ok((recipe, true))
    }
}
```

**Add helper function to check for duplicates by hash:**

```rust
/// Check if a recipe with the same content hash already exists
/// Returns the existing recipe if found
pub async fn find_recipe_by_content_hash(
    pool: &DbPool,
    content_hash: &str,
) -> Result<Option<Recipe>> {
    let recipe = sqlx::query_as::<_, Recipe>(
        "SELECT * FROM recipes WHERE content_hash = ? LIMIT 1"
    )
    .bind(content_hash)
    .fetch_optional(pool)
    .await
    .context("Failed to query recipe by content hash")?;

    Ok(recipe)
}

/// Get all recipes with the same content hash (duplicates)
pub async fn find_duplicate_recipes(
    pool: &DbPool,
    content_hash: &str,
) -> Result<Vec<Recipe>> {
    let recipes = sqlx::query_as::<_, Recipe>(
        "SELECT * FROM recipes WHERE content_hash = ? ORDER BY created_at ASC"
    )
    .bind(content_hash)
    .fetch_all(pool)
    .await
    .context("Failed to query duplicate recipes")?;

    Ok(recipes)
}
```

#### Step 4: Update GitHub Indexer

**File:** `src/github/indexer.rs`

**Update `index_recipe` function:** (around line 287)

```rust
async fn index_recipe(
    &self,
    github_feed_id: i64,
    file: &CookFile,
    owner: &str,
    repo_name: &str,
) -> Result<Recipe> {
    let cook_url = format!(
        "https://raw.githubusercontent.com/{}/{}/{}/{}",
        owner, repo_name, "main", file.path
    );

    // Fetch .cook file content
    let content = reqwest::get(&cook_url)
        .await
        .context("Failed to fetch .cook file")?
        .text()
        .await
        .context("Failed to read .cook file content")?;

    // Parse recipe
    let parsed = cooklang::parse(&content);

    // Extract metadata
    let title = parsed.metadata.get("title")
        .map(|v| v.as_str())
        .unwrap_or(&file.name)
        .to_string();

    let summary = parsed.metadata.get("description")
        .or_else(|| parsed.metadata.get("summary"))
        .map(|v| v.as_str().to_string());

    let servings = parsed.metadata.get("servings")
        .and_then(|v| v.as_str().parse::<i32>().ok());

    let total_time = parsed.metadata.get("time")
        .or_else(|| parsed.metadata.get("total time"))
        .and_then(|v| parse_time_to_minutes(v.as_str()));

    let active_time = parsed.metadata.get("active time")
        .or_else(|| parsed.metadata.get("prep time"))
        .and_then(|v| parse_time_to_minutes(v.as_str()));

    let difficulty = parsed.metadata.get("difficulty")
        .map(|v| v.as_str().to_string());

    let image_url = parsed.metadata.get("image")
        .or_else(|| parsed.metadata.get("image url"))
        .map(|v| v.as_str().to_string());

    // ✅ Calculate content hash
    let content_hash = db::recipes::calculate_content_hash(&title, Some(&content));
    debug!("Calculated content hash for '{}': {}", title, content_hash);

    let new_recipe = db::recipes::NewRecipe {
        feed_id: github_feed_id,
        external_id: file.path.clone(),
        title,
        source_url: Some(format!(
            "https://github.com/{}/{}/blob/main/{}",
            owner, repo_name, file.path
        )),
        enclosure_url: cook_url,
        content: Some(content),
        summary,
        servings,
        total_time_minutes: total_time,
        active_time_minutes: active_time,
        difficulty,
        image_url,
        published_at: None,
        updated_at: None,
        content_hash: Some(content_hash),  // ✅ Set content hash
    };

    let (recipe, is_new) = db::recipes::get_or_create_recipe(&self.pool, &new_recipe).await?;

    if is_new {
        info!(
            "Indexed new recipe from GitHub: {} ({})",
            recipe.title, recipe.id
        );
    } else {
        info!(
            "Updated existing recipe from GitHub: {} ({})",
            recipe.title, recipe.id
        );
    }

    Ok(recipe)
}
```

#### Step 5: Update Feed Crawler

**File:** `src/crawler/mod.rs`

**Update recipe creation:** (around line 178)

```rust
// Inside the entry processing loop
for entry in entries {
    let external_id = entry.id.clone();
    let title = entry.title.as_ref()
        .map(|t| t.as_str())
        .unwrap_or("Untitled Recipe")
        .to_string();

    // ... fetch cook file content ...

    let content = if let Some(url) = &enclosure_url {
        match reqwest::get(url).await {
            Ok(response) => {
                match response.text().await {
                    Ok(text) => Some(text),
                    Err(e) => {
                        warn!("Failed to read .cook file from {}: {}", url, e);
                        None
                    }
                }
            }
            Err(e) => {
                warn!("Failed to fetch .cook file from {}: {}", url, e);
                None
            }
        }
    } else {
        None
    };

    // ✅ Calculate content hash
    let content_hash = if let Some(ref content) = content {
        Some(db::recipes::calculate_content_hash(&title, Some(content)))
    } else {
        Some(db::recipes::calculate_content_hash(&title, None))
    };

    let new_recipe = db::recipes::NewRecipe {
        feed_id: feed.id,
        external_id,
        title,
        source_url: entry.links.get(0).map(|l| l.href.clone()),
        enclosure_url: enclosure_url.unwrap_or_default(),
        content,
        summary: entry.summary.as_ref().map(|s| s.as_str().to_string()),
        servings: None,
        total_time_minutes: None,
        active_time_minutes: None,
        difficulty: None,
        image_url: None,
        published_at: entry.published.map(|dt| dt.to_rfc3339()),
        updated_at: entry.updated.map(|dt| dt.to_rfc3339()),
        content_hash,  // ✅ Set content hash
    };

    let (recipe, is_new) = db::recipes::get_or_create_recipe(&self.pool, &new_recipe).await?;

    if is_new {
        new_count += 1;
        // ... index ingredients, tags, etc ...
    }
}
```

#### Step 6: Add Content Hash to Search Index

**File:** `src/indexer/schema.rs`

**Update SearchSchema:**

```rust
pub struct SearchSchema {
    pub id: Field,
    pub content_hash: Field,  // NEW
    pub title: Field,
    pub summary: Field,
    pub instructions: Field,
    pub ingredients: Field,
    pub tags: Field,
    pub difficulty: Field,
    pub file_path: Field,
    pub schema: Schema,
}

impl SearchSchema {
    pub fn new() -> Self {
        let mut schema_builder = Schema::builder();

        let id = schema_builder.add_i64_field("id", STORED);

        // ✅ Add content_hash field
        let content_hash = schema_builder.add_text_field("content_hash", STRING | STORED);

        let title = schema_builder.add_text_field("title", TEXT | STORED);
        let summary = schema_builder.add_text_field("summary", TEXT | STORED);
        let instructions = schema_builder.add_text_field("instructions", TEXT);
        let ingredients = schema_builder.add_text_field("ingredients", TEXT | STORED);
        let tags = schema_builder.add_text_field("tags", TEXT | STORED);
        let difficulty = schema_builder.add_text_field("difficulty", STRING | STORED);
        let file_path = schema_builder.add_text_field("file_path", TEXT | STORED);

        let schema = schema_builder.build();

        Self {
            id,
            content_hash,  // NEW
            title,
            summary,
            instructions,
            ingredients,
            tags,
            difficulty,
            file_path,
            schema,
        }
    }
}
```

**File:** `src/indexer/search.rs`

**Update `index_recipe` to include content_hash:**

```rust
pub fn index_recipe(
    &self,
    writer: &mut IndexWriter,
    recipe: &Recipe,
    file_path: Option<&str>,
    tags: &[String],
    ingredients: &[String],
) -> Result<()> {
    debug!("Indexing recipe: {}", recipe.id);

    // Delete existing documents with this recipe_id
    let term = Term::from_field_i64(self.schema.id, recipe.id);
    writer.delete_term(term);

    // Build document
    let mut doc = TantivyDocument::new();
    doc.add_i64(self.schema.id, recipe.id);

    // ✅ Add content hash
    if let Some(ref content_hash) = recipe.content_hash {
        doc.add_text(self.schema.content_hash, content_hash);
    }

    doc.add_text(self.schema.title, &recipe.title);

    // ... rest of fields ...

    writer.add_document(doc)?;
    Ok(())
}
```

**Update search to deduplicate by content_hash:**

```rust
pub fn search(&self, query: &SearchQuery, max_limit: usize) -> Result<SearchResults> {
    let reader = self.reader.clone();
    reader.reload()?;
    let searcher = reader.searcher();

    // Parse query
    let query_parser = QueryParser::for_index(
        &self.index,
        vec![
            self.schema.title,
            self.schema.summary,
            self.schema.instructions,
            self.schema.ingredients,
            self.schema.tags,
            self.schema.file_path,
        ],
    );

    let tantivy_query = query_parser
        .parse_query(&query.q)
        .context("Failed to parse search query")?;

    // Calculate pagination
    let page = query.page.max(1);
    let limit = query.limit.min(max_limit);
    let offset = (page - 1) * limit;

    // ✅ Fetch extra results to account for deduplication
    let fetch_limit = (limit + offset) * 3;

    // Execute search
    let top_docs = searcher
        .search(&*tantivy_query, &TopDocs::with_limit(fetch_limit))
        .context("Search query failed")?;

    let total = searcher
        .search(&*tantivy_query, &Count)
        .context("Count query failed")?;

    // ✅ Deduplicate by content_hash
    let mut seen_hashes = std::collections::HashSet::new();
    let results: Vec<SearchResult> = top_docs
        .into_iter()
        .filter_map(|(score, doc_address)| {
            let doc: TantivyDocument = searcher.doc(doc_address).ok()?;

            // Extract content hash
            let content_hash = doc
                .get_first(self.schema.content_hash)
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            // Skip if we've seen this content hash
            if let Some(ref hash) = content_hash {
                if !seen_hashes.insert(hash.clone()) {
                    debug!("Skipping duplicate content_hash: {}", hash);
                    return None;
                }
            }

            // Extract other fields
            let recipe_id = doc.get_first(self.schema.id)?.as_i64()?;
            let title = doc
                .get_first(self.schema.title)?
                .as_str()?
                .to_string();
            let summary = doc
                .get_first(self.schema.summary)
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            Some(SearchResult {
                recipe_id,
                title,
                summary,
                score,
            })
        })
        .skip(offset)
        .take(limit)
        .collect();

    let total_pages = total.div_ceil(limit);

    Ok(SearchResults {
        results,
        total,
        page,
        total_pages,
    })
}
```

#### Step 7: Backfill Content Hashes

**File:** `src/bin/backfill_hashes.rs` (new file)

```rust
//! Backfill content hashes for existing recipes

use anyhow::{Context, Result};
use sqlx::sqlite::SqlitePool;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    info!("Starting content hash backfill");

    // Connect to database
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "sqlite:data/federation.db".to_string());

    let pool = SqlitePool::connect(&database_url)
        .await
        .context("Failed to connect to database")?;

    // Get all recipes without content_hash
    let recipes = sqlx::query!(
        "SELECT id, title, content FROM recipes WHERE content_hash IS NULL"
    )
    .fetch_all(&pool)
    .await
    .context("Failed to fetch recipes")?;

    info!("Found {} recipes to backfill", recipes.len());

    let mut updated = 0;
    let mut failed = 0;

    for recipe in recipes {
        let content_hash = federation::db::recipes::calculate_content_hash(
            &recipe.title,
            recipe.content.as_deref(),
        );

        match sqlx::query!(
            "UPDATE recipes SET content_hash = ? WHERE id = ?",
            content_hash,
            recipe.id
        )
        .execute(&pool)
        .await
        {
            Ok(_) => {
                updated += 1;
                if updated % 100 == 0 {
                    info!("Backfilled {} recipes...", updated);
                }
            }
            Err(e) => {
                warn!("Failed to update recipe {}: {}", recipe.id, e);
                failed += 1;
            }
        }
    }

    info!(
        "Backfill complete: {} updated, {} failed",
        updated, failed
    );

    // Find and report duplicates
    info!("Checking for duplicate content...");

    let duplicates = sqlx::query!(
        r#"
        SELECT content_hash, COUNT(*) as count
        FROM recipes
        WHERE content_hash IS NOT NULL
        GROUP BY content_hash
        HAVING count > 1
        ORDER BY count DESC
        LIMIT 20
        "#
    )
    .fetch_all(&pool)
    .await
    .context("Failed to query duplicates")?;

    info!("Found {} unique content hashes with duplicates:", duplicates.len());
    for dup in duplicates {
        info!(
            "  Hash {} has {} duplicates",
            dup.content_hash.unwrap_or_default(),
            dup.count
        );
    }

    Ok(())
}
```

**Update `Cargo.toml` to add backfill binary:**

```toml
[[bin]]
name = "backfill_hashes"
path = "src/bin/backfill_hashes.rs"
```

**Run backfill:**

```bash
cargo run --release --bin backfill_hashes
```

### Testing Phase 2

#### 1. Unit Tests

Already included in Step 2 (hash calculation tests)

#### 2. Integration Test

**File:** `tests/deduplication_test.rs` (new)

```rust
use federation::db;
use sqlx::sqlite::SqlitePool;
use anyhow::Result;

#[tokio::test]
async fn test_duplicate_detection_by_hash() -> Result<()> {
    // Create in-memory database
    let pool = SqlitePool::connect("sqlite::memory:").await?;

    // Run migrations
    sqlx::migrate!("./migrations").run(&pool).await?;

    // Create a test feed
    let feed = db::feeds::create_feed(&pool, &db::feeds::NewFeed {
        title: "Test Feed 1".to_string(),
        url: "https://example.com/feed1.xml".to_string(),
        feed_type: "rss".to_string(),
    }).await?;

    let feed2 = db::feeds::create_feed(&pool, &db::feeds::NewFeed {
        title: "Test Feed 2".to_string(),
        url: "https://example.com/feed2.xml".to_string(),
        feed_type: "rss".to_string(),
    }).await?;

    // Create identical recipe from two different feeds
    let content = "@flour{500%g}\n@sugar{200%g}\n\nMix ingredients.";
    let hash = db::recipes::calculate_content_hash("Chocolate Cake", Some(content));

    let recipe1 = db::recipes::NewRecipe {
        feed_id: feed.id,
        external_id: "recipe1".to_string(),
        title: "Chocolate Cake".to_string(),
        source_url: None,
        enclosure_url: "https://example.com/recipe1.cook".to_string(),
        content: Some(content.to_string()),
        summary: None,
        servings: None,
        total_time_minutes: None,
        active_time_minutes: None,
        difficulty: None,
        image_url: None,
        published_at: None,
        updated_at: None,
        content_hash: Some(hash.clone()),
    };

    let recipe2 = db::recipes::NewRecipe {
        feed_id: feed2.id,
        external_id: "recipe2".to_string(),
        title: "Chocolate Cake".to_string(),  // Same title
        source_url: None,
        enclosure_url: "https://example.com/recipe2.cook".to_string(),
        content: Some(content.to_string()),  // Same content
        summary: None,
        servings: None,
        total_time_minutes: None,
        active_time_minutes: None,
        difficulty: None,
        image_url: None,
        published_at: None,
        updated_at: None,
        content_hash: Some(hash.clone()),  // Same hash
    };

    // Create both recipes
    let (r1, _) = db::recipes::get_or_create_recipe(&pool, &recipe1).await?;
    let (r2, _) = db::recipes::get_or_create_recipe(&pool, &recipe2).await?;

    // They should have different IDs (different feeds)
    assert_ne!(r1.id, r2.id);

    // But the same content hash
    assert_eq!(r1.content_hash, r2.content_hash);

    // Find duplicates by hash
    let duplicates = db::recipes::find_duplicate_recipes(&pool, &hash).await?;

    // Should find both recipes
    assert_eq!(duplicates.len(), 2);
    assert!(duplicates.iter().any(|r| r.id == r1.id));
    assert!(duplicates.iter().any(|r| r.id == r2.id));

    Ok(())
}
```

#### 3. Manual Testing

```bash
# 1. Apply migration
sqlite3 data/federation.db < migrations/002_add_content_hash.sql

# 2. Rebuild with new code
cargo build --release

# 3. Backfill existing recipes
cargo run --release --bin backfill_hashes

# 4. Check for duplicates
sqlite3 data/federation.db <<EOF
SELECT content_hash, COUNT(*) as count, GROUP_CONCAT(title, ' | ') as titles
FROM recipes
WHERE content_hash IS NOT NULL
GROUP BY content_hash
HAVING count > 1
LIMIT 10;
EOF

# 5. Rebuild search index
rm -rf data/search_index/
cargo run --release --bin indexer

# 6. Test search
curl "http://localhost:3000/api/search?q=Lasagna" | jq '.results | length'

# Should see fewer results than before (duplicates removed)
```

### Deployment Steps

1. **Backup Database:**
   ```bash
   cp data/federation.db data/federation.db.backup
   ```

2. **Apply Migration:**
   ```bash
   sqlite3 data/federation.db < migrations/002_add_content_hash.sql
   ```

3. **Deploy New Code:**
   ```bash
   git pull
   cargo build --release
   ```

4. **Backfill Hashes:**
   ```bash
   cargo run --release --bin backfill_hashes
   ```

5. **Rebuild Search Index:**
   ```bash
   rm -rf data/search_index/
   cargo run --release --bin indexer
   ```

6. **Restart Application:**
   ```bash
   systemctl restart federation
   ```

7. **Verify:**
   ```bash
   # Check search results
   curl "http://localhost:3000/api/search?q=Lasagna"

   # Check logs for deduplication
   journalctl -u federation -f | grep "duplicate"
   ```

### Estimated Effort

- **Migration:** 30 minutes
- **Hash Calculation:** 2 hours
- **Recipe Creation Updates:** 2 hours
- **Indexer Updates:** 2 hours
- **Search Index Updates:** 2 hours
- **Backfill Script:** 1 hour
- **Testing:** 2 hours
- **Deployment:** 1 hour
- **Total:** ~12-15 hours (~2 days)

---

## Monitoring and Validation

### Metrics to Track

1. **Duplicate Detection Rate**
   ```sql
   -- How many recipes share content hashes
   SELECT
     COUNT(DISTINCT content_hash) as unique_recipes,
     COUNT(*) as total_recipes,
     COUNT(*) - COUNT(DISTINCT content_hash) as duplicates
   FROM recipes
   WHERE content_hash IS NOT NULL;
   ```

2. **Search Result Quality**
   ```bash
   # Before vs after comparison
   curl "http://localhost:3000/api/search?q=cake" | jq '.pagination.total'
   ```

3. **Performance**
   ```bash
   # Search latency
   time curl "http://localhost:3000/api/search?q=cake" > /dev/null
   ```

### Logging

Add to both GitHub indexer and feed crawler:

```rust
if let Some(ref hash) = recipe.content_hash {
    // Check if duplicate exists
    if let Ok(Some(existing)) = db::recipes::find_recipe_by_content_hash(&pool, hash).await {
        if existing.id != recipe.id {
            info!(
                "Duplicate content detected: '{}' (id: {}) matches existing '{}' (id: {})",
                recipe.title, recipe.id, existing.title, existing.id
            );
        }
    }
}
```

### Health Checks

1. **Content Hash Coverage:**
   ```sql
   SELECT
     COUNT(*) as total,
     COUNT(content_hash) as with_hash,
     ROUND(COUNT(content_hash) * 100.0 / COUNT(*), 2) as coverage_percent
   FROM recipes;
   ```

   Should be close to 100% after backfill.

2. **Duplicate Rate:**
   ```sql
   SELECT
     COUNT(*) as duplicate_groups,
     AVG(dup_count) as avg_duplicates_per_group
   FROM (
     SELECT content_hash, COUNT(*) as dup_count
     FROM recipes
     WHERE content_hash IS NOT NULL
     GROUP BY content_hash
     HAVING dup_count > 1
   );
   ```

3. **Search Index Integrity:**
   ```bash
   # Total recipes in database
   sqlite3 data/federation.db "SELECT COUNT(*) FROM recipes;"

   # Compare with search index document count
   # (should be similar, accounting for deduplication)
   ```

---

## Rollback Plan

If issues arise:

### Phase 0 Rollback

1. Revert code changes to `src/indexer/search.rs`
2. Rebuild and deploy
3. Rebuild search index

### Phase 2 Rollback

1. **Code Rollback:**
   ```bash
   git revert <commit-hash>
   cargo build --release
   ```

2. **Database Rollback:**
   ```sql
   -- Remove content_hash column
   ALTER TABLE recipes DROP COLUMN content_hash;
   ```

3. **Search Index Rollback:**
   ```bash
   rm -rf data/search_index/
   cargo run --release --bin indexer
   ```

4. **Restore Backup:**
   ```bash
   cp data/federation.db.backup data/federation.db
   ```

---

## Success Criteria

### Phase 0
- ✅ No recipe_id appears more than once in search results
- ✅ Recipe updates don't create duplicate search entries
- ✅ All unit tests pass
- ✅ Manual testing confirms fix

### Phase 2
- ✅ Content hash calculated for 100% of recipes
- ✅ Duplicate recipes detected and logged
- ✅ Search results deduplicated by content hash
- ✅ Search performance acceptable (<500ms for typical queries)
- ✅ Pagination accurate
- ✅ No false positives (different recipes incorrectly merged)

---

## Future Enhancements

After completing both phases, consider:

1. **Admin Dashboard**
   - View duplicate groups
   - Manually merge/unmerge recipes
   - Choose canonical version

2. **Content Similarity Score**
   - Beyond exact hash matching
   - Use fuzzy matching for near-duplicates
   - ML-based similarity detection

3. **Canonical Recipe System** (Phase 3 from research)
   - Full implementation with recipe sources tracking
   - User preference for preferred sources
   - "Also available from" UI feature

4. **Automated Duplicate Reports**
   - Daily/weekly digest of new duplicates
   - Notification when high-value duplicates detected

---

## Notes

- This plan uses **delete-before-add** logic to ensure atomicity
- Content hashing is SHA-256 based, extremely low collision probability
- Normalization ensures minor formatting differences don't affect hash
- Search index deduplication happens at query time for flexibility
- Database still tracks all recipe instances (important for attribution)
- Future canonical system can build on content_hash foundation
