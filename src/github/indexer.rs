use crate::github::{client::GitHubClient, config::GitHubConfig};
use crate::{
    db::{
        self,
        models::{NewFeed, NewGitHubFeed, NewGitHubRecipe, NewRecipe},
        DbPool,
    },
    indexer::search::SearchIndex,
    Error, Result,
};
use futures::stream::{self, StreamExt};
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info, warn};

/// GitHub repository indexer
#[derive(Clone)]
pub struct GitHubIndexer {
    client: GitHubClient,
    pool: DbPool,
    search_index: Arc<SearchIndex>,
    config: GitHubConfig,
}

impl GitHubIndexer {
    /// Create a new GitHub indexer
    pub fn new(config: GitHubConfig, pool: DbPool, search_index: Arc<SearchIndex>) -> Result<Self> {
        let client = GitHubClient::new(config.clone())?;

        Ok(Self {
            client,
            pool,
            search_index,
            config,
        })
    }

    /// Add a GitHub repository to the federation
    pub async fn add_repository(&self, repository_url: &str) -> Result<i64> {
        info!("Adding GitHub repository: {}", repository_url);

        // Parse repository URL
        let repo_info = crate::github::parse_repository_url(repository_url)?;

        // Check if repository already exists
        if let Some(existing) =
            db::github::get_github_feed_by_repo(&self.pool, &repo_info.owner, &repo_info.repo)
                .await?
        {
            return Err(Error::Validation(format!(
                "Repository {}/{} is already indexed (ID: {})",
                repo_info.owner, repo_info.repo, existing.id
            )));
        }

        // Fetch repository information from GitHub
        let github_repo = self
            .client
            .get_repository(&repo_info.owner, &repo_info.repo)
            .await?;

        // Check if repository is archived
        if github_repo.archived {
            return Err(Error::Validation(format!(
                "Repository {}/{} is archived and cannot be indexed",
                repo_info.owner, repo_info.repo
            )));
        }

        // Create feed for this repository
        let new_feed = NewFeed {
            url: repository_url.to_string(),
            title: Some(
                github_repo
                    .description
                    .unwrap_or(github_repo.full_name.clone()),
            ),
        };

        let feed = db::feeds::create_feed(&self.pool, &new_feed).await?;

        // Create GitHub feed entry
        let new_github_feed = NewGitHubFeed {
            feed_id: feed.id,
            repository_url: repository_url.to_string(),
            owner: repo_info.owner,
            repo_name: repo_info.repo,
            default_branch: github_repo.default_branch,
        };

        let github_feed = db::github::create_github_feed(&self.pool, &new_github_feed).await?;

        // Index the repository
        self.index_repository(github_feed.id).await?;

        Ok(github_feed.id)
    }

    /// Index or re-index a GitHub repository
    pub async fn index_repository(&self, github_feed_id: i64) -> Result<usize> {
        info!("Indexing GitHub repository: {}", github_feed_id);

        // Get GitHub feed info
        let mut github_feed = db::github::get_github_feed(&self.pool, github_feed_id).await?;

        // Fetch current repository info to ensure we have the latest default branch
        let github_repo = self
            .client
            .get_repository(&github_feed.owner, &github_feed.repo_name)
            .await?;

        // Update default branch if it has changed
        if github_repo.default_branch != github_feed.default_branch {
            info!(
                "Default branch changed for {}/{}: {} -> {}",
                github_feed.owner,
                github_feed.repo_name,
                github_feed.default_branch,
                github_repo.default_branch
            );
            github_feed = db::github::update_github_feed_branch(
                &self.pool,
                github_feed_id,
                &github_repo.default_branch,
            )
            .await?;
        }

        // Get latest commit SHA
        let latest_commit_sha = self
            .client
            .get_branch_commit(
                &github_feed.owner,
                &github_feed.repo_name,
                &github_feed.default_branch,
            )
            .await?;

        // Check if repository has changed
        if let Some(last_sha) = &github_feed.last_commit_sha {
            if last_sha == &latest_commit_sha {
                debug!("Repository hasn't changed, skipping indexing");
                return Ok(0);
            }
        }

        // Get commit to find tree SHA
        let commit = self
            .client
            .get_commit(
                &github_feed.owner,
                &github_feed.repo_name,
                &latest_commit_sha,
            )
            .await?;

        // Get repository tree
        let tree = self
            .client
            .get_tree(
                &github_feed.owner,
                &github_feed.repo_name,
                &commit.commit.tree.sha,
            )
            .await?;

        // Find all .cook files and collect them into owned data
        let cook_files: Vec<(String, String)> = tree
            .tree
            .iter()
            .filter(|entry| entry.entry_type == "blob" && entry.path.ends_with(".cook"))
            .map(|entry| (entry.path.clone(), entry.sha.clone()))
            .collect();

        let total_recipes = cook_files.len();
        info!(
            "Found {} .cook files in {}/{} - Processing with concurrency {}",
            total_recipes, github_feed.owner, github_feed.repo_name, self.config.recipe_concurrency
        );

        // Start timing
        let start = Instant::now();

        // Clone tree entries for parallel processing
        let tree_entries = tree.tree.clone();

        // Process recipes in parallel
        let concurrency = self.config.recipe_concurrency;
        let results: Vec<_> = stream::iter(cook_files)
            .map(|(path, sha)| {
                let indexer = self.clone();
                let github_feed = github_feed.clone();
                let tree_entries = tree_entries.clone();
                async move {
                    indexer
                        .index_recipe(&github_feed, &path, &sha, &tree_entries)
                        .await
                }
            })
            .buffer_unordered(concurrency)
            .collect()
            .await;

        // Count successes and collect successful recipe IDs for batch search indexing
        let mut indexed_count = 0;
        let mut successful_recipe_ids = Vec::new();

        for result in results {
            match result {
                Ok(recipe_id) => {
                    indexed_count += 1;
                    successful_recipe_ids.push(recipe_id);
                }
                Err(e) => {
                    warn!("Failed to index recipe: {}", e);
                }
            }
        }

        // Batch commit to search index
        if !successful_recipe_ids.is_empty() {
            let mut search_writer = self.search_index.writer()?;

            for recipe_id in successful_recipe_ids {
                let recipe = db::recipes::get_recipe(&self.pool, recipe_id).await?;

                // Get file path from github_recipes if this is a GitHub recipe
                let file_path = if let Some(github_recipe) =
                    db::github::get_github_recipe_by_recipe_id(&self.pool, recipe_id).await?
                {
                    Some(github_recipe.file_path)
                } else {
                    None
                };

                // Fetch tags for this recipe
                let tags = db::tags::get_tags_for_recipe(&self.pool, recipe_id).await?;

                // Fetch ingredients for this recipe
                let ingredients =
                    db::ingredients::get_ingredients_for_recipe(&self.pool, recipe_id)
                        .await?
                        .iter()
                        .map(|ing| ing.name.clone())
                        .collect::<Vec<_>>();

                self.search_index.index_recipe(
                    &mut search_writer,
                    &recipe,
                    file_path.as_deref(),
                    &tags,
                    &ingredients,
                )?;
            }

            // Single commit for all recipes
            search_writer.commit()?;
        }

        // Update GitHub feed with latest commit SHA
        db::github::update_github_feed_commit(&self.pool, github_feed_id, &latest_commit_sha)
            .await?;

        // Calculate metrics
        let duration = start.elapsed();
        let recipes_per_second = if duration.as_secs_f64() > 0.0 {
            indexed_count as f64 / duration.as_secs_f64()
        } else {
            0.0
        };

        info!(
            "Indexed repository {}/{} - {}/{} recipes in {:.2}s ({:.1} recipes/sec)",
            github_feed.owner,
            github_feed.repo_name,
            indexed_count,
            total_recipes,
            duration.as_secs_f64(),
            recipes_per_second
        );

        Ok(indexed_count)
    }

    /// Index a single recipe file from GitHub
    /// Returns the recipe ID on success
    async fn index_recipe(
        &self,
        github_feed: &crate::db::models::GitHubFeed,
        file_path: &str,
        file_sha: &str,
        tree_entries: &[crate::github::models::TreeEntry],
    ) -> Result<i64> {
        debug!("Indexing recipe: {}", file_path);

        // Check if recipe already exists with same SHA
        if let Some(existing) =
            db::github::get_github_recipe_by_path(&self.pool, github_feed.id, file_path).await?
        {
            if existing.file_sha == file_sha {
                debug!("Recipe {} hasn't changed, skipping", file_path);
                return Ok(existing.recipe_id);
            }
        }

        // Download raw content
        let raw_url = format!(
            "https://raw.githubusercontent.com/{}/{}/{}/{}",
            github_feed.owner, github_feed.repo_name, github_feed.default_branch, file_path
        );

        let content = self.client.download_raw_content(&raw_url).await?;

        // Use filename without extension as title
        let title = file_path
            .split('/')
            .next_back()
            .and_then(|f| f.strip_suffix(".cook"))
            .unwrap_or(file_path)
            .to_string();

        // Look for image with the same name
        let image_url = Self::find_recipe_image(
            file_path,
            tree_entries,
            &github_feed.owner,
            &github_feed.repo_name,
            &github_feed.default_branch,
        );

        // Parse Cooklang content to extract metadata
        let parsed = crate::indexer::parse_cooklang_full(&content);
        let (summary, servings, total_time) = if parsed.is_ok() {
            // Extract metadata from parsed content
            let summary = None; // Can be enhanced to extract from recipe notes
            let servings = None; // Can be extracted from metadata
            let total_time = None; // Can be extracted from timer sum
            (summary, servings, total_time)
        } else {
            (None, None, None)
        };

        let html_url = format!(
            "https://github.com/{}/{}/blob/{}/{}",
            github_feed.owner, github_feed.repo_name, github_feed.default_branch, file_path
        );

        // Create or update recipe
        let recipe_id = if let Some(existing) =
            db::github::get_github_recipe_by_path(&self.pool, github_feed.id, file_path).await?
        {
            // Update existing recipe
            let recipe = db::recipes::get_recipe(&self.pool, existing.recipe_id).await?;

            // Update recipe content (use existing update function if available)
            // For now, we'll keep the existing recipe and just update the github_recipe SHA
            db::github::update_github_recipe_sha(&self.pool, existing.id, file_sha).await?;

            recipe.id
        } else {
            // Create new recipe
            // Calculate content hash for deduplication
            let content_hash = Some(db::recipes::calculate_content_hash(&title, Some(&content)));

            let new_recipe = NewRecipe {
                feed_id: github_feed.feed_id,
                external_id: file_path.to_string(),
                title: title.clone(),
                source_url: Some(html_url.clone()),
                enclosure_url: raw_url.clone(),
                content: Some(content.clone()),
                summary,
                servings,
                total_time_minutes: total_time,
                active_time_minutes: None,
                difficulty: None,
                image_url,
                published_at: None,
                content_hash,
                content_etag: None,
                content_last_modified: None,
                feed_entry_updated: None,
            };

            let recipe = db::recipes::create_recipe(&self.pool, &new_recipe).await?;

            // Create GitHub recipe entry
            let new_github_recipe = NewGitHubRecipe {
                recipe_id: recipe.id,
                github_feed_id: github_feed.id,
                file_path: file_path.to_string(),
                file_sha: file_sha.to_string(),
                raw_url: raw_url.clone(),
                html_url: html_url.clone(),
            };

            db::github::create_github_recipe(&self.pool, &new_github_recipe).await?;

            recipe.id
        };

        // Extract and store ingredients, cookware, and tags from parsed content
        if let Ok(parsed_data) = parsed {
            // Store ingredients
            let ingredients: Vec<crate::db::models::RecipeIngredient> = parsed_data
                .ingredients
                .iter()
                .map(|ing| crate::db::models::RecipeIngredient {
                    name: ing.name.clone(),
                    quantity: ing.quantity_value,
                    unit: ing.unit.clone(),
                })
                .collect();

            if !ingredients.is_empty() {
                db::ingredients::set_recipe_ingredients(&self.pool, recipe_id, &ingredients)
                    .await?;
            }

            // Store metadata tags from recipe
            if let Some(metadata) = &parsed_data.metadata {
                if !metadata.tags.is_empty() {
                    db::tags::set_recipe_tags(&self.pool, recipe_id, &metadata.tags).await?;
                }
            }
        }

        // Return recipe ID for batch search indexing
        Ok(recipe_id)
    }

    /// Remove a GitHub repository from the federation
    pub async fn remove_repository(&self, github_feed_id: i64) -> Result<()> {
        info!("Removing GitHub repository: {}", github_feed_id);

        // Get GitHub feed
        let github_feed = db::github::get_github_feed(&self.pool, github_feed_id).await?;

        // Get all recipes for this feed
        let recipes = db::github::list_github_recipes_by_feed(&self.pool, github_feed_id).await?;

        // Remove from search index
        let mut writer = self.search_index.writer()?;
        for recipe in &recipes {
            if let Err(e) = self
                .search_index
                .delete_recipe(&mut writer, recipe.recipe_id)
            {
                warn!(
                    "Failed to remove recipe {} from search index: {}",
                    recipe.recipe_id, e
                );
            }
        }
        writer.commit()?;

        // Delete GitHub feed (cascades to recipes)
        db::github::delete_github_feed(&self.pool, github_feed_id).await?;

        // Delete the base feed
        db::feeds::delete_feed(&self.pool, github_feed.feed_id).await?;

        info!("Successfully removed GitHub repository: {}", github_feed_id);

        Ok(())
    }

    /// List all GitHub repositories
    pub async fn list_repositories(&self) -> Result<Vec<crate::db::models::GitHubFeedWithStats>> {
        db::github::list_github_feeds_with_stats(&self.pool).await
    }

    /// Get rate limit status
    pub async fn get_rate_limit_status(&self) -> (u32, u32, chrono::DateTime<chrono::Utc>) {
        self.client.get_rate_limit_status().await
    }

    /// Find an image file for a recipe by looking for files with the same name
    fn find_recipe_image(
        recipe_path: &str,
        tree_entries: &[crate::github::models::TreeEntry],
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> Option<String> {
        // Get the base path and filename without .cook extension
        let recipe_base = recipe_path.strip_suffix(".cook")?;

        // Common image extensions
        let image_exts = [".jpg", ".jpeg", ".png", ".webp", ".gif"];

        // Look for matching image files
        for ext in &image_exts {
            let image_path = format!("{recipe_base}{ext}");
            if tree_entries.iter().any(|entry| entry.path == image_path) {
                // Return raw.githubusercontent.com URL
                return Some(format!(
                    "https://raw.githubusercontent.com/{owner}/{repo}/{branch}/{image_path}"
                ));
            }
        }

        None
    }
}
