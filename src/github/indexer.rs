use crate::github::{client::GitHubClient, config::GitHubConfig};
use crate::utils::resolve_image_url;
use crate::{
    db::{
        self,
        models::{NewFeed, NewGitHubFeed, NewGitHubRecipe, NewRecipe, UpdateRecipe},
        DbPool,
    },
    indexer::search::SearchIndex,
    Error, Result,
};
use futures::stream::{self, StreamExt};
use std::collections::HashSet;
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
    /// Serialises the "is this content already known?" check with the insert
    /// that follows it. Files are processed concurrently, and without this two
    /// copies of one recipe both pass the check before either row exists.
    create_lock: Arc<tokio::sync::Mutex<()>>,
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
            create_lock: Arc::new(tokio::sync::Mutex::new(())),
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
        let cook_paths: Vec<String> = cook_files.iter().map(|(path, _)| path.clone()).collect();
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
                Ok(Some(recipe_id)) => {
                    indexed_count += 1;
                    successful_recipe_ids.push(recipe_id);
                }
                Ok(None) => {}
                Err(e) => {
                    warn!("Failed to index recipe: {}", e);
                }
            }
        }

        // Files that left the repository take their recipes with them.
        let current_paths: HashSet<&str> = cook_paths.iter().map(String::as_str).collect();
        let stale: Vec<_> = db::github::list_github_recipes_by_feed(&self.pool, github_feed_id)
            .await?
            .into_iter()
            .filter(|r| !current_paths.contains(r.file_path.as_str()))
            .collect();

        // Batch commit to search index
        if !successful_recipe_ids.is_empty() || !stale.is_empty() {
            let mut search_writer = self.search_index.writer()?;

            for github_recipe in &stale {
                info!(
                    "Removing {}/{}:{}: no longer in the repository",
                    github_feed.owner, github_feed.repo_name, github_recipe.file_path
                );
                self.search_index
                    .delete_recipe(&mut search_writer, github_recipe.recipe_id)?;
                db::recipes::delete_recipe(&self.pool, github_recipe.recipe_id).await?;
            }

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
            self.search_index.commit(&mut search_writer)?;
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
    ) -> Result<Option<i64>> {
        debug!("Indexing recipe: {}", file_path);

        // Check if recipe already exists with same SHA
        if let Some(existing) =
            db::github::get_github_recipe_by_path(&self.pool, github_feed.id, file_path).await?
        {
            if existing.file_sha == file_sha {
                debug!("Recipe {} hasn't changed, skipping", file_path);
                return Ok(Some(existing.recipe_id));
            }
        }

        // Download raw content
        let raw_url = self.config.raw_url(
            &github_feed.owner,
            &github_feed.repo_name,
            &github_feed.default_branch,
            file_path,
        );

        let content = self.client.download_raw_content(&raw_url).await?;

        // Parse Cooklang content to extract metadata
        let parsed = crate::indexer::parse_cooklang_full(&content);

        let title = recipe_title(parsed.as_ref().ok(), file_path);
        let (summary, servings, total_time, metadata_image) = if let Ok(ref parsed_data) = parsed {
            // Extract metadata from parsed content
            let summary = None; // Can be enhanced to extract from recipe notes
            let servings = None; // Can be extracted from metadata
            let total_time = None; // Can be extracted from timer sum
            let metadata_image = parsed_data.metadata.as_ref().and_then(|m| m.image.clone());
            (summary, servings, total_time, metadata_image)
        } else {
            (None, None, None, None)
        };

        // Locale: declared `locale:` metadata wins, otherwise detected from text.
        let locale = parsed
            .as_ref()
            .ok()
            .and_then(crate::indexer::resolve_locale);
        let (locale_code, locale_source) = match &locale {
            Some(l) => (Some(l.code.clone()), Some(l.source.as_str().to_string())),
            None => (None, None),
        };

        // Look for image with the same name (sibling file) as fallback
        let sibling_image_url = Self::find_recipe_image(
            file_path,
            tree_entries,
            &github_feed.owner,
            &github_feed.repo_name,
            &github_feed.default_branch,
        );

        // Prioritize metadata image, fallback to sibling image file
        // Resolve relative metadata image URLs against the raw content URL
        let image_url = metadata_image
            .and_then(|img| resolve_image_url(&img, &raw_url))
            .or(sibling_image_url);

        let html_url = format!(
            "https://github.com/{}/{}/blob/{}/{}",
            github_feed.owner, github_feed.repo_name, github_feed.default_branch, file_path
        );

        // Content hash identifies the same recipe wherever it lives: a second
        // path in this repository, a fork, or a mirror feed.
        let content_hash = db::recipes::calculate_content_hash(&title, Some(&content));

        // Create or update recipe
        let recipe_id = if let Some(existing) =
            db::github::get_github_recipe_by_path(&self.pool, github_feed.id, file_path).await?
        {
            // The file changed upstream: refresh everything we derive from it.
            let recipe = db::recipes::get_recipe(&self.pool, existing.recipe_id).await?;
            let update = UpdateRecipe {
                title: Some(title.clone()),
                source_url: Some(html_url.clone()),
                content: Some(content.clone()),
                summary,
                servings,
                total_time_minutes: total_time,
                active_time_minutes: recipe.active_time_minutes,
                difficulty: recipe.difficulty.clone(),
                image_url,
                updated_at: None,
            };
            db::recipes::update_recipe(&self.pool, recipe.id, &update).await?;
            db::recipes::set_content_hash(&self.pool, recipe.id, &content_hash).await?;
            db::github::update_github_recipe_sha(&self.pool, existing.id, file_sha).await?;

            db::recipes::update_recipe_locale(
                &self.pool,
                recipe.id,
                locale_code.as_deref(),
                locale_source.as_deref(),
            )
            .await?;

            recipe.id
        } else {
            let _guard = self.create_lock.lock().await;

            if let Some(duplicate) =
                db::recipes::find_recipe_by_content_hash(&self.pool, &content_hash).await?
            {
                info!(
                    "Skipping {}/{}:{}: identical to recipe {} ({})",
                    github_feed.owner,
                    github_feed.repo_name,
                    file_path,
                    duplicate.id,
                    duplicate.title
                );
                return Ok(None);
            }

            let content_hash = Some(content_hash);

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
                locale: locale_code.clone(),
                locale_source: locale_source.clone(),
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
        Ok(Some(recipe_id))
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

/// The title of a GitHub recipe: a declared `title:` wins; otherwise something
/// readable is made out of the file name.
pub fn recipe_title(parsed: Option<&crate::indexer::ParsedRecipeData>, file_path: &str) -> String {
    parsed
        .and_then(|p| p.metadata.as_ref())
        .and_then(|m| m.title.as_deref())
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| title_from_path(file_path))
}

/// A readable title from a recipe's path: the file stem with any leading
/// `YYYY-MM-DD-` date dropped, separators turned into spaces and each word
/// capitalised. `recipes/2025-12-01-tiramisu-brownies.cook` becomes
/// `Tiramisu Brownies`.
pub fn title_from_path(file_path: &str) -> String {
    let stem = file_path
        .rsplit('/')
        .next()
        .unwrap_or(file_path)
        .strip_suffix(".cook")
        .unwrap_or(file_path);

    let stem = strip_leading_date(stem);

    // A name with spaces was written for people already; its hyphens are
    // deliberate ("Sweet-and-Spicy Ketchup").
    if stem.contains(char::is_whitespace) {
        return stem.trim().to_string();
    }

    let words: Vec<String> = stem
        .split(['-', '_'])
        .filter(|w| !w.is_empty())
        .map(capitalize)
        .collect();

    if words.is_empty() {
        stem.to_string()
    } else {
        words.join(" ")
    }
}

fn strip_leading_date(stem: &str) -> &str {
    let bytes = stem.as_bytes();
    let is_date = bytes.len() > 11
        && bytes[..10].iter().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                *b == b'-'
            } else {
                b.is_ascii_digit()
            }
        })
        && bytes[10] == b'-';
    if is_date {
        &stem[11..]
    } else {
        stem
    }
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) if word.chars().any(char::is_lowercase) || word.chars().count() == 1 => {
            first.to_uppercase().collect::<String>() + chars.as_str()
        }
        // Leave acronyms and mixed-case words like "BBQ" or "McMuffin" alone.
        Some(_) => word.to_string(),
        None => String::new(),
    }
}

#[cfg(test)]
mod title_tests {
    use super::title_from_path;

    #[test]
    fn humanizes_dated_slugs() {
        assert_eq!(
            title_from_path("recipes/2025-12-01-tiramisu-brownies.cook"),
            "Tiramisu Brownies"
        );
        assert_eq!(
            title_from_path("2025-12-04-cottage_cheese-gnocchi.cook"),
            "Cottage Cheese Gnocchi"
        );
    }

    #[test]
    fn keeps_already_readable_names() {
        assert_eq!(
            title_from_path("cook/sides/Vegan Caesar Salad.cook"),
            "Vegan Caesar Salad"
        );
        assert_eq!(title_from_path("BBQ Ribs.cook"), "BBQ Ribs");
        assert_eq!(title_from_path("Pizza.cook"), "Pizza");
    }

    #[test]
    fn keeps_hyphens_in_names_that_already_have_spaces() {
        assert_eq!(
            title_from_path("Sweet-and-Spicy Ketchup.cook"),
            "Sweet-and-Spicy Ketchup"
        );
        assert_eq!(
            title_from_path("Gnudi with Tomato-Butter Sauce.cook"),
            "Gnudi with Tomato-Butter Sauce"
        );
        assert_eq!(
            title_from_path("2025-12-01-Sun-Dried Tomato Pesto.cook"),
            "Sun-Dried Tomato Pesto"
        );
    }

    #[test]
    fn does_not_mistake_numbers_for_dates() {
        assert_eq!(title_from_path("2-minute-noodles.cook"), "2 Minute Noodles");
        assert_eq!(
            title_from_path("2025-holiday-cookies.cook"),
            "2025 Holiday Cookies"
        );
    }
}
