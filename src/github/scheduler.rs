use crate::github::GitHubIndexer;
use std::time::Duration;
use tokio::time::interval;
use tracing::{error, info};

/// GitHub repository synchronization scheduler
pub struct GitHubScheduler {
    indexer: GitHubIndexer,
    interval_secs: u64,
}

impl GitHubScheduler {
    /// Create a new GitHub scheduler
    pub fn new(indexer: GitHubIndexer, interval_secs: u64) -> Self {
        Self {
            indexer,
            interval_secs,
        }
    }

    /// Start the background scheduler
    /// Returns a handle that can be used to stop the scheduler
    pub fn start(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            info!(
                "GitHub scheduler started (interval: {}s)",
                self.interval_secs
            );

            let mut ticker = interval(Duration::from_secs(self.interval_secs));

            loop {
                ticker.tick().await;
                self.sync_all_repositories().await;
            }
        })
    }

    /// Synchronize all GitHub repositories
    async fn sync_all_repositories(&self) {
        info!("Starting scheduled GitHub repository synchronization");

        match self.indexer.list_repositories().await {
            Ok(repos) => {
                info!("Found {} GitHub repositories to sync", repos.len());

                let mut success_count = 0;
                let mut error_count = 0;

                for repo in repos {
                    info!("Syncing repository: {}/{}", repo.owner, repo.repo_name);

                    match self.indexer.index_repository(repo.id).await {
                        Ok(recipe_count) => {
                            if recipe_count > 0 {
                                info!(
                                    "Successfully synced {}/{}: {} recipes updated",
                                    repo.owner, repo.repo_name, recipe_count
                                );
                            } else {
                                info!("Repository {}/{} is up to date", repo.owner, repo.repo_name);
                            }
                            success_count += 1;
                        }
                        Err(e) => {
                            error!(
                                "Failed to sync repository {}/{}: {}",
                                repo.owner, repo.repo_name, e
                            );
                            error_count += 1;
                        }
                    }
                }

                info!(
                    "GitHub synchronization complete: {} successful, {} errors",
                    success_count, error_count
                );
            }
            Err(e) => {
                error!("Failed to list GitHub repositories: {}", e);
            }
        }
    }
}

/// GitHub scheduler statistics
#[derive(Debug, Clone, Default)]
pub struct SchedulerStats {
    pub last_run: Option<chrono::DateTime<chrono::Utc>>,
    pub next_run: Option<chrono::DateTime<chrono::Utc>>,
    pub total_runs: u64,
    pub last_success_count: u64,
    pub last_error_count: u64,
}
