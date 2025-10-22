use crate::db::{models::*, DbPool};
use crate::error::Result;

/// Normalize ingredient name (lowercase, trim, handle plurals)
pub fn normalize_ingredient(name: &str) -> String {
    let normalized = name.trim().to_lowercase();

    // Simple plural handling - can be expanded
    // For now, just normalize to lowercase
    normalized
}

/// Get or create an ingredient by name
pub async fn get_or_create_ingredient(pool: &DbPool, name: &str) -> Result<Ingredient> {
    let normalized = normalize_ingredient(name);

    // Try to find existing ingredient
    let existing = sqlx::query_as::<_, Ingredient>("SELECT * FROM ingredients WHERE name = ?")
        .bind(&normalized)
        .fetch_optional(pool)
        .await?;

    if let Some(ingredient) = existing {
        Ok(ingredient)
    } else {
        // Create new ingredient
        let ingredient = sqlx::query_as::<_, Ingredient>(
            "INSERT INTO ingredients (name) VALUES (?) RETURNING *",
        )
        .bind(&normalized)
        .fetch_one(pool)
        .await?;

        Ok(ingredient)
    }
}

/// Add ingredient to recipe
pub async fn add_recipe_ingredient(
    pool: &DbPool,
    recipe_id: i64,
    ingredient_id: i64,
    quantity: Option<f64>,
    unit: Option<String>,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT OR REPLACE INTO recipe_ingredients (recipe_id, ingredient_id, quantity, unit)
        VALUES (?, ?, ?, ?)
        "#,
    )
    .bind(recipe_id)
    .bind(ingredient_id)
    .bind(quantity)
    .bind(unit)
    .execute(pool)
    .await?;

    Ok(())
}

/// Add multiple ingredients to recipe
pub async fn add_recipe_ingredients(
    pool: &DbPool,
    recipe_id: i64,
    ingredients: &[IngredientWithQuantity],
) -> Result<()> {
    for ing in ingredients {
        let ingredient = get_or_create_ingredient(pool, &ing.name).await?;
        add_recipe_ingredient(
            pool,
            recipe_id,
            ingredient.id,
            ing.quantity,
            ing.unit.clone(),
        )
        .await?;
    }

    Ok(())
}

/// Get ingredients for a recipe
pub async fn get_recipe_ingredients(
    pool: &DbPool,
    recipe_id: i64,
) -> Result<Vec<IngredientWithQuantity>> {
    let ingredients = sqlx::query_as::<_, IngredientWithQuantity>(
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

    Ok(ingredients)
}

/// Get ingredients for a recipe (alias for convenience)
pub async fn get_ingredients_for_recipe(
    pool: &DbPool,
    recipe_id: i64,
) -> Result<Vec<IngredientWithQuantity>> {
    get_recipe_ingredients(pool, recipe_id).await
}

/// Count total ingredients
pub async fn count_ingredients(pool: &DbPool) -> Result<i64> {
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM ingredients")
        .fetch_one(pool)
        .await?;
    Ok(count.0)
}

/// Remove all ingredients from a recipe
pub async fn clear_recipe_ingredients(pool: &DbPool, recipe_id: i64) -> Result<()> {
    sqlx::query("DELETE FROM recipe_ingredients WHERE recipe_id = ?")
        .bind(recipe_id)
        .execute(pool)
        .await?;

    Ok(())
}

/// Get all ingredients with usage count
pub async fn get_ingredients_with_count(pool: &DbPool) -> Result<Vec<(String, i64)>> {
    let ingredients: Vec<(String, i64)> = sqlx::query_as(
        r#"
        SELECT i.name, COUNT(ri.recipe_id) as count
        FROM ingredients i
        LEFT JOIN recipe_ingredients ri ON ri.ingredient_id = i.id
        GROUP BY i.id, i.name
        ORDER BY count DESC, i.name
        LIMIT 1000
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(ingredients)
}

/// Set recipe ingredients (replaces existing ingredients)
pub async fn set_recipe_ingredients(
    pool: &DbPool,
    recipe_id: i64,
    ingredients: &[crate::db::models::RecipeIngredient],
) -> Result<()> {
    // Clear existing ingredients
    clear_recipe_ingredients(pool, recipe_id).await?;

    // Add new ingredients
    for ing in ingredients {
        // Get or create ingredient
        let ingredient = get_or_create_ingredient(pool, &ing.name).await?;
        add_recipe_ingredient(
            pool,
            recipe_id,
            ingredient.id,
            ing.quantity,
            ing.unit.clone(),
        )
        .await?;
    }

    Ok(())
}

/// Delete ingredients that aren't associated with any recipes
pub async fn delete_unused_ingredients(pool: &DbPool) -> Result<i64> {
    let result = sqlx::query(
        r#"
        DELETE FROM ingredients
        WHERE id NOT IN (SELECT DISTINCT ingredient_id FROM recipe_ingredients)
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
    async fn test_ingredients() {
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
            },
        )
        .await
        .unwrap();

        // Add ingredients
        let ingredients = vec![
            IngredientWithQuantity {
                name: "Flour".to_string(),
                quantity: Some(300.0),
                unit: Some("g".to_string()),
            },
            IngredientWithQuantity {
                name: "Butter".to_string(),
                quantity: Some(200.0),
                unit: Some("g".to_string()),
            },
        ];

        add_recipe_ingredients(&pool, recipe.id, &ingredients)
            .await
            .unwrap();

        // Get ingredients
        let retrieved = get_recipe_ingredients(&pool, recipe.id).await.unwrap();
        assert_eq!(retrieved.len(), 2);

        // Ingredients should be normalized
        assert!(retrieved.iter().any(|i| i.name == "flour"));
        assert!(retrieved.iter().any(|i| i.name == "butter"));
    }
}
