use crate::db::{models::*, DbPool};
use crate::error::{Error, Result};
use chrono::Utc;
use sha2::{Digest, Sha256};

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
        if let Some(end_pos) = result[start..].find("-]") {
            let end = start + end_pos + 2; // +2 for the "-]" itself
                                           // Also remove trailing newline if the block comment is on its own line
            let actual_end = if result.len() > end && result.chars().nth(end) == Some('\n') {
                end + 1
            } else {
                end
            };
            result.replace_range(start..actual_end, "");
            // If there's a newline before the comment and we're at the start, trim it
            result = result.trim().to_string();
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

/// Check if a recipe with the same content hash already exists
/// Returns the existing recipe if found
pub async fn find_recipe_by_content_hash(
    pool: &DbPool,
    content_hash: &str,
) -> Result<Option<Recipe>> {
    let recipe =
        sqlx::query_as::<_, Recipe>("SELECT * FROM recipes WHERE content_hash = ? LIMIT 1")
            .bind(content_hash)
            .fetch_optional(pool)
            .await?;

    Ok(recipe)
}

/// Get all recipes with the same content hash (duplicates)
pub async fn find_duplicate_recipes(pool: &DbPool, content_hash: &str) -> Result<Vec<Recipe>> {
    let recipes = sqlx::query_as::<_, Recipe>(
        "SELECT * FROM recipes WHERE content_hash = ? ORDER BY created_at ASC",
    )
    .bind(content_hash)
    .fetch_all(pool)
    .await?;

    Ok(recipes)
}

/// Create a new recipe
pub async fn create_recipe(pool: &DbPool, new_recipe: &NewRecipe) -> Result<Recipe> {
    let now = Utc::now();

    let recipe = sqlx::query_as::<_, Recipe>(
        r#"
        INSERT INTO recipes (
            feed_id, external_id, title, source_url, enclosure_url,
            content, summary, servings, total_time_minutes, active_time_minutes,
            difficulty, image_url, published_at, updated_at, created_at, content_hash
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
    .bind(new_recipe.published_at)
    .bind(now)
    .bind(now)
    .bind(&new_recipe.content_hash)
    .fetch_one(pool)
    .await?;

    Ok(recipe)
}

/// Get recipe by ID
pub async fn get_recipe(pool: &DbPool, recipe_id: i64) -> Result<Recipe> {
    let recipe = sqlx::query_as::<_, Recipe>("SELECT * FROM recipes WHERE id = ?")
        .bind(recipe_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| Error::NotFound(format!("Recipe {recipe_id} not found")))?;

    Ok(recipe)
}

/// Get recipe with all details (tags, ingredients, feed info)
pub async fn get_recipe_with_details(pool: &DbPool, recipe_id: i64) -> Result<RecipeWithDetails> {
    let recipe = get_recipe(pool, recipe_id).await?;

    // Get tags
    let tags: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT t.name
        FROM tags t
        JOIN recipe_tags rt ON rt.tag_id = t.id
        WHERE rt.recipe_id = ?
        ORDER BY t.name
        "#,
    )
    .bind(recipe_id)
    .fetch_all(pool)
    .await?;

    // Get ingredients
    let ingredients: Vec<IngredientWithQuantity> = sqlx::query_as(
        r#"
        SELECT i.name, ri.quantity, ri.unit
        FROM ingredients i
        JOIN recipe_ingredients ri ON ri.ingredient_id = i.id
        WHERE ri.recipe_id = ?
        "#,
    )
    .bind(recipe_id)
    .fetch_all(pool)
    .await?;

    // Get feed info
    let feed: FeedInfo = sqlx::query_as("SELECT id, title, author FROM feeds WHERE id = ?")
        .bind(recipe.feed_id)
        .fetch_one(pool)
        .await?;

    Ok(RecipeWithDetails {
        recipe,
        tags,
        ingredients,
        feed,
    })
}

/// Update recipe content
pub async fn update_recipe_content(pool: &DbPool, recipe_id: i64, content: &str) -> Result<()> {
    let now = Utc::now();

    sqlx::query(
        r#"
        UPDATE recipes
        SET content = ?, updated_at = ?
        WHERE id = ?
        "#,
    )
    .bind(content)
    .bind(now)
    .bind(recipe_id)
    .execute(pool)
    .await?;

    Ok(())
}

/// Mark recipe as indexed
pub async fn mark_recipe_indexed(pool: &DbPool, recipe_id: i64) -> Result<()> {
    let now = Utc::now();

    sqlx::query("UPDATE recipes SET indexed_at = ? WHERE id = ?")
        .bind(now)
        .bind(recipe_id)
        .execute(pool)
        .await?;

    Ok(())
}

/// List recipes by feed
pub async fn list_recipes_by_feed(
    pool: &DbPool,
    feed_id: i64,
    limit: i64,
    offset: i64,
) -> Result<Vec<Recipe>> {
    let recipes = sqlx::query_as::<_, Recipe>(
        "SELECT * FROM recipes WHERE feed_id = ? ORDER BY published_at DESC LIMIT ? OFFSET ?",
    )
    .bind(feed_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok(recipes)
}

/// Count recipes by feed
pub async fn count_recipes_by_feed(pool: &DbPool, feed_id: i64) -> Result<i64> {
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM recipes WHERE feed_id = ?")
        .bind(feed_id)
        .fetch_one(pool)
        .await?;
    Ok(count.0)
}

/// Count all recipes
pub async fn count_all_recipes(pool: &DbPool) -> Result<i64> {
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM recipes")
        .fetch_one(pool)
        .await?;
    Ok(count.0)
}

/// List all recipes with pagination
pub async fn list_all_recipes(pool: &DbPool, limit: i64, offset: i64) -> Result<Vec<Recipe>> {
    let recipes = sqlx::query_as::<_, Recipe>(
        "SELECT * FROM recipes ORDER BY published_at DESC, id DESC LIMIT ? OFFSET ?",
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok(recipes)
}

/// Delete recipe
pub async fn delete_recipe(pool: &DbPool, recipe_id: i64) -> Result<()> {
    sqlx::query("DELETE FROM recipes WHERE id = ?")
        .bind(recipe_id)
        .execute(pool)
        .await?;

    Ok(())
}

/// Get recipe by feed ID and external ID
pub async fn get_recipe_by_external_id(
    pool: &DbPool,
    feed_id: i64,
    external_id: &str,
) -> Result<Recipe> {
    let recipe =
        sqlx::query_as::<_, Recipe>("SELECT * FROM recipes WHERE feed_id = ? AND external_id = ?")
            .bind(feed_id)
            .bind(external_id)
            .fetch_optional(pool)
            .await?
            .ok_or_else(|| {
                Error::NotFound(format!(
                    "Recipe with external_id '{external_id}' not found in feed {feed_id}"
                ))
            })?;

    Ok(recipe)
}

/// Update recipe
pub async fn update_recipe(pool: &DbPool, recipe_id: i64, update: &UpdateRecipe) -> Result<Recipe> {
    let now = Utc::now();

    let recipe = sqlx::query_as::<_, Recipe>(
        r#"
        UPDATE recipes
        SET title = ?, source_url = ?, content = ?, summary = ?,
            servings = ?, total_time_minutes = ?, active_time_minutes = ?,
            difficulty = ?, image_url = ?, updated_at = ?
        WHERE id = ?
        RETURNING *
        "#,
    )
    .bind(&update.title)
    .bind(&update.source_url)
    .bind(&update.content)
    .bind(&update.summary)
    .bind(update.servings)
    .bind(update.total_time_minutes)
    .bind(update.active_time_minutes)
    .bind(&update.difficulty)
    .bind(&update.image_url)
    .bind(update.updated_at.unwrap_or(now))
    .bind(recipe_id)
    .fetch_one(pool)
    .await?;

    Ok(recipe)
}

/// Get or create recipe by feed and external ID
pub async fn get_or_create_recipe(pool: &DbPool, new_recipe: &NewRecipe) -> Result<(Recipe, bool)> {
    // Try to find existing recipe
    let existing =
        sqlx::query_as::<_, Recipe>("SELECT * FROM recipes WHERE feed_id = ? AND external_id = ?")
            .bind(new_recipe.feed_id)
            .bind(&new_recipe.external_id)
            .fetch_optional(pool)
            .await?;

    if let Some(recipe) = existing {
        Ok((recipe, false))
    } else {
        let recipe = create_recipe(pool, new_recipe).await?;
        Ok((recipe, true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{feeds, init_pool, run_migrations};

    #[tokio::test]
    async fn test_recipe_crud() {
        let pool = init_pool("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();

        // Create a feed first
        let feed = feeds::create_feed(
            &pool,
            &NewFeed {
                url: "https://example.com/feed.xml".to_string(),
                title: Some("Test Feed".to_string()),
            },
        )
        .await
        .unwrap();

        // Create recipe
        let new_recipe = NewRecipe {
            feed_id: feed.id,
            external_id: "recipe-1".to_string(),
            title: "Test Recipe".to_string(),
            source_url: Some("https://example.com/recipe-1".to_string()),
            enclosure_url: "https://example.com/recipe-1.cook".to_string(),
            content: Some(">> servings: 4\nTest content".to_string()),
            summary: Some("A test recipe".to_string()),
            servings: Some(4),
            total_time_minutes: Some(30),
            active_time_minutes: Some(15),
            difficulty: Some("easy".to_string()),
            image_url: None,
            published_at: Some(Utc::now()),
            content_hash: None,
        };

        let recipe = create_recipe(&pool, &new_recipe).await.unwrap();
        assert_eq!(recipe.title, new_recipe.title);

        // Get recipe
        let retrieved = get_recipe(&pool, recipe.id).await.unwrap();
        assert_eq!(retrieved.id, recipe.id);

        // Update content
        update_recipe_content(&pool, recipe.id, "Updated content")
            .await
            .unwrap();

        // Delete
        delete_recipe(&pool, recipe.id).await.unwrap();
    }

    #[test]
    fn test_normalize_title() {
        assert_eq!(normalize_title("  Chocolate   Cake  "), "chocolate cake");
        assert_eq!(normalize_title("CHOCOLATE CAKE"), "chocolate cake");
        assert_eq!(normalize_title("Chocolate\tCake"), "chocolate cake");
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

    #[test]
    fn test_title_case_differences_produce_same_hash() {
        let content = "@flour{500%g}";

        let hash1 = calculate_content_hash("Chocolate Cake", Some(content));
        let hash2 = calculate_content_hash("CHOCOLATE CAKE", Some(content));
        let hash3 = calculate_content_hash("chocolate cake", Some(content));

        assert_eq!(hash1, hash2);
        assert_eq!(hash2, hash3);
    }

    #[test]
    fn test_block_comments_dont_affect_hash() {
        let content1 = "@flour{500%g}\n[- This is a block comment -]\n@sugar{200%g}";
        let content2 = "@flour{500%g}\n@sugar{200%g}";

        let hash1 = calculate_content_hash("Cake", Some(content1));
        let hash2 = calculate_content_hash("Cake", Some(content2));

        assert_eq!(hash1, hash2);
    }
}
