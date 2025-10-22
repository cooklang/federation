use crate::db::{models::*, DbPool};
use crate::error::{Error, Result};
use chrono::Utc;

/// Create a new GitHub feed
pub async fn create_github_feed(pool: &DbPool, new_feed: &NewGitHubFeed) -> Result<GitHubFeed> {
    let now = Utc::now();

    let feed = sqlx::query_as::<_, GitHubFeed>(
        r#"
        INSERT INTO github_feeds (feed_id, repository_url, owner, repo_name, default_branch, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?)
        RETURNING *
        "#,
    )
    .bind(new_feed.feed_id)
    .bind(&new_feed.repository_url)
    .bind(&new_feed.owner)
    .bind(&new_feed.repo_name)
    .bind(&new_feed.default_branch)
    .bind(now)
    .bind(now)
    .fetch_one(pool)
    .await?;

    Ok(feed)
}

/// Get GitHub feed by ID
pub async fn get_github_feed(pool: &DbPool, id: i64) -> Result<GitHubFeed> {
    let feed = sqlx::query_as::<_, GitHubFeed>("SELECT * FROM github_feeds WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| Error::NotFound(format!("GitHub feed {id} not found")))?;

    Ok(feed)
}

/// Get GitHub feed by repository URL
pub async fn get_github_feed_by_url(pool: &DbPool, url: &str) -> Result<Option<GitHubFeed>> {
    let feed =
        sqlx::query_as::<_, GitHubFeed>("SELECT * FROM github_feeds WHERE repository_url = ?")
            .bind(url)
            .fetch_optional(pool)
            .await?;

    Ok(feed)
}

/// Get GitHub feed by owner and repo
pub async fn get_github_feed_by_repo(
    pool: &DbPool,
    owner: &str,
    repo: &str,
) -> Result<Option<GitHubFeed>> {
    let feed = sqlx::query_as::<_, GitHubFeed>(
        "SELECT * FROM github_feeds WHERE owner = ? AND repo_name = ?",
    )
    .bind(owner)
    .bind(repo)
    .fetch_optional(pool)
    .await?;

    Ok(feed)
}

/// List all GitHub feeds
pub async fn list_github_feeds(pool: &DbPool) -> Result<Vec<GitHubFeed>> {
    let feeds =
        sqlx::query_as::<_, GitHubFeed>("SELECT * FROM github_feeds ORDER BY created_at DESC")
            .fetch_all(pool)
            .await?;

    Ok(feeds)
}

/// List GitHub feeds with statistics
pub async fn list_github_feeds_with_stats(pool: &DbPool) -> Result<Vec<GitHubFeedWithStats>> {
    let feeds = sqlx::query_as::<_, GitHubFeedWithStats>(
        r#"
        SELECT
            gf.*,
            COUNT(DISTINCT gr.id) as recipe_count,
            f.title as feed_title
        FROM github_feeds gf
        LEFT JOIN github_recipes gr ON gf.id = gr.github_feed_id
        LEFT JOIN feeds f ON gf.feed_id = f.id
        GROUP BY gf.id
        ORDER BY gf.created_at DESC
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(feeds)
}

/// Update GitHub feed commit SHA
pub async fn update_github_feed_commit(
    pool: &DbPool,
    id: i64,
    commit_sha: &str,
) -> Result<GitHubFeed> {
    let now = Utc::now();

    let feed = sqlx::query_as::<_, GitHubFeed>(
        r#"
        UPDATE github_feeds
        SET last_commit_sha = ?, updated_at = ?
        WHERE id = ?
        RETURNING *
        "#,
    )
    .bind(commit_sha)
    .bind(now)
    .bind(id)
    .fetch_one(pool)
    .await?;

    Ok(feed)
}

/// Update GitHub feed default branch
pub async fn update_github_feed_branch(
    pool: &DbPool,
    id: i64,
    default_branch: &str,
) -> Result<GitHubFeed> {
    let now = Utc::now();

    let feed = sqlx::query_as::<_, GitHubFeed>(
        r#"
        UPDATE github_feeds
        SET default_branch = ?, updated_at = ?
        WHERE id = ?
        RETURNING *
        "#,
    )
    .bind(default_branch)
    .bind(now)
    .bind(id)
    .fetch_one(pool)
    .await?;

    Ok(feed)
}

/// Delete GitHub feed
pub async fn delete_github_feed(pool: &DbPool, id: i64) -> Result<()> {
    sqlx::query("DELETE FROM github_feeds WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;

    Ok(())
}

/// Create a new GitHub recipe
pub async fn create_github_recipe(
    pool: &DbPool,
    new_recipe: &NewGitHubRecipe,
) -> Result<GitHubRecipe> {
    let now = Utc::now();

    let recipe = sqlx::query_as::<_, GitHubRecipe>(
        r#"
        INSERT INTO github_recipes (recipe_id, github_feed_id, file_path, file_sha, raw_url, html_url, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        RETURNING *
        "#,
    )
    .bind(new_recipe.recipe_id)
    .bind(new_recipe.github_feed_id)
    .bind(&new_recipe.file_path)
    .bind(&new_recipe.file_sha)
    .bind(&new_recipe.raw_url)
    .bind(&new_recipe.html_url)
    .bind(now)
    .bind(now)
    .fetch_one(pool)
    .await?;

    Ok(recipe)
}

/// Get GitHub recipe by recipe ID
pub async fn get_github_recipe_by_recipe_id(
    pool: &DbPool,
    recipe_id: i64,
) -> Result<Option<GitHubRecipe>> {
    let recipe =
        sqlx::query_as::<_, GitHubRecipe>("SELECT * FROM github_recipes WHERE recipe_id = ?")
            .bind(recipe_id)
            .fetch_optional(pool)
            .await?;

    Ok(recipe)
}

/// Get GitHub recipe by feed and file path
pub async fn get_github_recipe_by_path(
    pool: &DbPool,
    github_feed_id: i64,
    file_path: &str,
) -> Result<Option<GitHubRecipe>> {
    let recipe = sqlx::query_as::<_, GitHubRecipe>(
        "SELECT * FROM github_recipes WHERE github_feed_id = ? AND file_path = ?",
    )
    .bind(github_feed_id)
    .bind(file_path)
    .fetch_optional(pool)
    .await?;

    Ok(recipe)
}

/// List all GitHub recipes for a feed
pub async fn list_github_recipes_by_feed(
    pool: &DbPool,
    github_feed_id: i64,
) -> Result<Vec<GitHubRecipe>> {
    let recipes = sqlx::query_as::<_, GitHubRecipe>(
        "SELECT * FROM github_recipes WHERE github_feed_id = ? ORDER BY file_path",
    )
    .bind(github_feed_id)
    .fetch_all(pool)
    .await?;

    Ok(recipes)
}

/// Update GitHub recipe SHA
pub async fn update_github_recipe_sha(
    pool: &DbPool,
    id: i64,
    file_sha: &str,
) -> Result<GitHubRecipe> {
    let now = Utc::now();

    let recipe = sqlx::query_as::<_, GitHubRecipe>(
        r#"
        UPDATE github_recipes
        SET file_sha = ?, updated_at = ?
        WHERE id = ?
        RETURNING *
        "#,
    )
    .bind(file_sha)
    .bind(now)
    .bind(id)
    .fetch_one(pool)
    .await?;

    Ok(recipe)
}

/// Delete GitHub recipe
pub async fn delete_github_recipe(pool: &DbPool, id: i64) -> Result<()> {
    sqlx::query("DELETE FROM github_recipes WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;

    Ok(())
}

/// Delete all GitHub recipes for a feed
pub async fn delete_github_recipes_by_feed(pool: &DbPool, github_feed_id: i64) -> Result<()> {
    sqlx::query("DELETE FROM github_recipes WHERE github_feed_id = ?")
        .bind(github_feed_id)
        .execute(pool)
        .await?;

    Ok(())
}
