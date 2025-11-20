use crate::db::{models::Tag, DbPool};
use crate::error::Result;

/// Get or create a tag by name
pub async fn get_or_create_tag(pool: &DbPool, name: &str) -> Result<Tag> {
    // Normalize tag name (lowercase, trim)
    let normalized = name.trim().to_lowercase();

    // Try to find existing tag
    let existing = sqlx::query_as::<_, Tag>("SELECT * FROM tags WHERE name = ?")
        .bind(&normalized)
        .fetch_optional(pool)
        .await?;

    if let Some(tag) = existing {
        Ok(tag)
    } else {
        // Create new tag
        let tag = sqlx::query_as::<_, Tag>("INSERT INTO tags (name) VALUES (?) RETURNING *")
            .bind(&normalized)
            .fetch_one(pool)
            .await?;

        Ok(tag)
    }
}

/// Add tag to recipe
pub async fn add_recipe_tag(pool: &DbPool, recipe_id: i64, tag_id: i64) -> Result<()> {
    sqlx::query("INSERT OR IGNORE INTO recipe_tags (recipe_id, tag_id) VALUES (?, ?)")
        .bind(recipe_id)
        .bind(tag_id)
        .execute(pool)
        .await?;

    Ok(())
}

/// Add multiple tags to recipe
pub async fn add_recipe_tags(pool: &DbPool, recipe_id: i64, tag_names: &[String]) -> Result<()> {
    for tag_name in tag_names {
        let tag = get_or_create_tag(pool, tag_name).await?;
        add_recipe_tag(pool, recipe_id, tag.id).await?;
    }

    Ok(())
}

/// Get tags for a recipe
pub async fn get_recipe_tags(pool: &DbPool, recipe_id: i64) -> Result<Vec<Tag>> {
    let tags = sqlx::query_as::<_, Tag>(
        r#"
        SELECT t.*
        FROM tags t
        JOIN recipe_tags rt ON rt.tag_id = t.id
        WHERE rt.recipe_id = ?
        ORDER BY t.name
        "#,
    )
    .bind(recipe_id)
    .fetch_all(pool)
    .await?;

    Ok(tags)
}

/// Get tag names for a recipe (convenience function for API)
pub async fn get_tags_for_recipe(pool: &DbPool, recipe_id: i64) -> Result<Vec<String>> {
    let tags = get_recipe_tags(pool, recipe_id).await?;
    Ok(tags.into_iter().map(|t| t.name).collect())
}

/// Get tags for multiple recipes in a single query (batch loading to avoid N+1)
pub async fn get_tags_for_recipes(
    pool: &DbPool,
    recipe_ids: &[i64],
) -> Result<std::collections::HashMap<i64, Vec<String>>> {
    use std::collections::HashMap;

    if recipe_ids.is_empty() {
        return Ok(HashMap::new());
    }

    // Build query with IN clause
    let placeholders = recipe_ids
        .iter()
        .enumerate()
        .map(|(i, _)| format!("?{}", i + 1))
        .collect::<Vec<_>>()
        .join(",");

    let query_str = format!(
        r#"
        SELECT rt.recipe_id, t.name
        FROM recipe_tags rt
        JOIN tags t ON rt.tag_id = t.id
        WHERE rt.recipe_id IN ({placeholders})
        ORDER BY rt.recipe_id, t.name
        "#
    );

    let mut query = sqlx::query_as::<_, (i64, String)>(&query_str);
    for id in recipe_ids {
        query = query.bind(id);
    }

    let results: Vec<(i64, String)> = query.fetch_all(pool).await?;

    // Group tags by recipe_id
    let mut tags_map: HashMap<i64, Vec<String>> = HashMap::new();
    for (recipe_id, tag_name) in results {
        tags_map.entry(recipe_id).or_default().push(tag_name);
    }

    // Ensure all recipe_ids have an entry (even if empty)
    for &recipe_id in recipe_ids {
        tags_map.entry(recipe_id).or_default();
    }

    Ok(tags_map)
}

/// Count total tags
pub async fn count_tags(pool: &DbPool) -> Result<i64> {
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM tags")
        .fetch_one(pool)
        .await?;
    Ok(count.0)
}

/// Remove all tags from a recipe
pub async fn clear_recipe_tags(pool: &DbPool, recipe_id: i64) -> Result<()> {
    sqlx::query("DELETE FROM recipe_tags WHERE recipe_id = ?")
        .bind(recipe_id)
        .execute(pool)
        .await?;

    Ok(())
}

/// Get all tags with usage count
pub async fn get_tags_with_count(pool: &DbPool) -> Result<Vec<(String, i64)>> {
    let tags: Vec<(String, i64)> = sqlx::query_as(
        r#"
        SELECT t.name, COUNT(rt.recipe_id) as count
        FROM tags t
        LEFT JOIN recipe_tags rt ON rt.tag_id = t.id
        GROUP BY t.id, t.name
        ORDER BY count DESC, t.name
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(tags)
}

/// Set recipe tags (replaces existing tags)
pub async fn set_recipe_tags(pool: &DbPool, recipe_id: i64, tag_names: &[String]) -> Result<()> {
    // Clear existing tags
    clear_recipe_tags(pool, recipe_id).await?;

    // Add new tags
    add_recipe_tags(pool, recipe_id, tag_names).await?;

    Ok(())
}

/// Delete tags that aren't associated with any recipes
pub async fn delete_unused_tags(pool: &DbPool) -> Result<i64> {
    let result = sqlx::query(
        r#"
        DELETE FROM tags
        WHERE id NOT IN (SELECT DISTINCT tag_id FROM recipe_tags)
        "#,
    )
    .execute(pool)
    .await?;

    Ok(result.rows_affected() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::{NewFeed, NewRecipe};
    use crate::db::{feeds, init_pool, recipes, run_migrations};
    use chrono::Utc;

    #[tokio::test]
    async fn test_tags() {
        let pool = init_pool("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();

        // Create feed and recipe
        let feed = feeds::create_feed(
            &pool,
            &NewFeed {
                url: "https://example.com/feed.xml".to_string(),
                title: Some("Test Feed".to_string()),
            },
        )
        .await
        .unwrap();

        let recipe = recipes::create_recipe(
            &pool,
            &NewRecipe {
                feed_id: feed.id,
                external_id: "recipe-1".to_string(),
                title: "Test Recipe".to_string(),
                source_url: None,
                enclosure_url: "https://example.com/recipe-1.cook".to_string(),
                content: None,
                summary: None,
                servings: None,
                total_time_minutes: None,
                active_time_minutes: None,
                difficulty: None,
                image_url: None,
                published_at: Some(Utc::now()),
                content_hash: None,
            },
        )
        .await
        .unwrap();

        // Add tags
        add_recipe_tags(
            &pool,
            recipe.id,
            &["dessert".to_string(), "cookies".to_string()],
        )
        .await
        .unwrap();

        // Get tags
        let tags = get_recipe_tags(&pool, recipe.id).await.unwrap();
        assert_eq!(tags.len(), 2);

        // Tags should be normalized
        assert!(tags.iter().any(|t| t.name == "dessert"));
        assert!(tags.iter().any(|t| t.name == "cookies"));
    }
}
