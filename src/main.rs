use clap::Parser;
use federation::{
    api::{handlers::AppState, routes},
    cli::{Cli, Commands},
    config::Settings,
    db,
    indexer::search::SearchIndex,
    Error, Result,
};
use std::sync::Arc;
use tracing::{info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<()> {
    // Load environment variables from .env file if it exists
    // Silently ignore if file doesn't exist
    let _ = dotenvy::dotenv();

    // Initialize logging
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,federation=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Parse CLI arguments
    let cli = Cli::parse();

    // Load configuration
    let settings = Settings::from_env()?;
    settings.validate()?;

    // Handle commands
    match cli.command {
        Commands::Serve { port, host } => {
            serve(settings, port, host).await?;
        }
        Commands::Migrate => {
            migrate(settings).await?;
        }
        Commands::Search {
            query,
            tags,
            max_time,
        } => {
            search_recipes(settings, query, tags, max_time).await?;
        }
        Commands::Download { recipe_id, output } => {
            download_recipe(settings, recipe_id, output).await?;
        }
        Commands::Publish { input, output } => {
            publish_recipes(input, output).await?;
        }
        Commands::Validate { url } => {
            validate_feed(url).await?;
        }
        Commands::Reindex { url } => {
            reindex_feed(settings, url).await?;
        }
    }

    Ok(())
}

async fn serve(mut settings: Settings, port: Option<u16>, host: Option<String>) -> Result<()> {
    // Override settings with CLI arguments
    if let Some(port) = port {
        settings.server.port = port;
    }
    if let Some(host) = host {
        settings.server.host = host;
    }

    info!("Starting Cooklang Federation server");
    info!("Database: {}", settings.database.url);
    info!("Server: {}:{}", settings.server.host, settings.server.port);

    // Initialize database with connection pooling configuration
    let pool = db::init_pool_with_config(&settings.database).await?;
    info!(
        "Database connection established (max_connections: {}, min_connections: {})",
        settings.database.max_connections, settings.database.min_connections
    );

    // Run migrations
    db::run_migrations(&pool).await?;
    info!("Database migrations completed");

    // Load feed configuration and sync to database
    let feed_config_path =
        std::env::var("FEED_CONFIG_PATH").unwrap_or_else(|_| "config/feeds.yaml".to_string());

    match federation::config::feeds::FeedConfig::from_file(&feed_config_path) {
        Ok(feed_config) => {
            info!(
                "Loaded feed configuration: {} feeds ({} enabled)",
                feed_config.total_feeds(),
                feed_config.enabled_count()
            );

            match federation::config::sync::sync_feeds_from_config(&pool, &feed_config).await {
                Ok(report) => {
                    info!(
                        "Feed synchronization completed: {} added, {} updated, {} disabled, {} re-enabled, {} unchanged",
                        report.added, report.updated, report.disabled, report.re_enabled, report.unchanged
                    );
                    if !report.errors.is_empty() {
                        warn!(
                            "{} feed sync errors occurred - check logs for details",
                            report.errors.len()
                        );
                    }
                }
                Err(e) => {
                    warn!("Failed to sync feeds from config: {}", e);
                }
            }
        }
        Err(e) => {
            warn!(
                "Failed to load feed configuration from {}: {}",
                feed_config_path, e
            );
            warn!("Continuing without feed sync - feeds must be managed manually");
        }
    }

    // Initialize search index
    let index_path = std::path::PathBuf::from(&settings.search.index_path);
    let search_index = SearchIndex::new(&index_path)?;
    info!("Search index initialized at {:?}", index_path);

    // Initialize crawler
    let crawler = Arc::new(federation::crawler::Crawler::new(settings.crawler.clone())?);
    info!("Crawler initialized");

    // Start background scheduler
    let scheduler = Arc::new(federation::crawler::scheduler::Scheduler::new(
        pool.clone(),
        crawler,
        settings.crawler.interval_seconds,
    ));
    let _scheduler_handle = scheduler.start();
    info!(
        "Background scheduler started (interval: {}s)",
        settings.crawler.interval_seconds
    );

    // Wrap search index in Arc for sharing
    let search_index = Arc::new(search_index);

    // Initialize GitHub indexer if enabled
    let github_indexer = {
        let github_config = federation::github::GitHubConfig::from_env();
        if github_config.is_enabled() {
            let interval_secs = github_config.update_interval_secs;
            match federation::github::GitHubIndexer::new(
                github_config,
                pool.clone(),
                search_index.clone(),
            ) {
                Ok(indexer) => {
                    info!("GitHub integration enabled");

                    // Start GitHub scheduler
                    let scheduler =
                        federation::github::GitHubScheduler::new(indexer.clone(), interval_secs);
                    let _github_scheduler_handle = scheduler.start();
                    info!("GitHub scheduler started (interval: {}s)", interval_secs);

                    Some(indexer)
                }
                Err(e) => {
                    warn!("Failed to initialize GitHub integration: {}", e);
                    None
                }
            }
        } else {
            None
        }
    };

    // Create application state
    let state = AppState {
        pool,
        search_index,
        github_indexer,
        settings: settings.clone(),
    };

    // Create router with rate limiting
    let app = routes::create_router(state, &settings);

    // Start server
    let addr = format!("{}:{}", settings.server.host, settings.server.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| Error::Internal(format!("Failed to bind to {addr}: {e}")))?;

    println!("\n========================================");
    println!("Cooklang Federation Server");
    println!("========================================");
    println!("Status: Running");
    println!("Address: http://{addr}");
    println!("Database: Connected");
    println!("Search Index: Ready");
    println!(
        "Background Crawler: Active ({}s interval)",
        settings.crawler.interval_seconds
    );
    println!("\nAPI Endpoints:");
    println!("  GET  /api/search");
    println!("  GET  /api/recipes/:id");
    println!("  GET  /api/recipes/:id/download");
    println!("  GET  /api/feeds");
    println!("  GET  /api/feeds/:id");
    println!("  GET  /api/stats");
    println!("\nNote: Feeds are now managed via config/feeds.yaml (see config/README.md)");
    println!("\nPress Ctrl+C to stop");
    println!("========================================\n");

    info!("Server listening on {}", addr);

    axum::serve(listener, app)
        .await
        .map_err(|e| Error::Internal(format!("Server error: {e}")))?;

    info!("Shutting down...");
    Ok(())
}

async fn migrate(settings: Settings) -> Result<()> {
    info!("Running database migrations");

    let pool = db::init_pool(&settings.database.url).await?;
    db::run_migrations(&pool).await?;

    println!("✓ Database migrations completed successfully");
    Ok(())
}

async fn search_recipes(
    settings: Settings,
    query: String,
    tags: Option<String>,
    max_time: Option<i64>,
) -> Result<()> {
    let server_url = settings
        .server
        .external_url
        .unwrap_or_else(|| format!("http://{}:{}", settings.server.host, settings.server.port));

    federation::cli::commands::search(&server_url, &query, tags, max_time).await
}

async fn download_recipe(settings: Settings, recipe_id: i64, output: Option<String>) -> Result<()> {
    let server_url = settings
        .server
        .external_url
        .unwrap_or_else(|| format!("http://{}:{}", settings.server.host, settings.server.port));

    federation::cli::commands::download(&server_url, recipe_id, output).await
}

async fn publish_recipes(input: String, output: String) -> Result<()> {
    federation::cli::commands::publish(&input, &output, None, None).await
}

async fn validate_feed(url: String) -> Result<()> {
    federation::cli::commands::validate_feed(&url).await
}

async fn reindex_feed(settings: Settings, url: String) -> Result<()> {
    info!("Reindexing feed: {}", url);

    // Initialize database
    let pool = db::init_pool(&settings.database.url).await?;
    db::run_migrations(&pool).await?;

    // Delete recipes and reset feed caching headers
    let deleted_count = federation::cli::commands::reindex_feed(&pool, &url).await?;
    println!("  Deleted {} recipes", deleted_count);

    // Initialize crawler and re-crawl the feed
    println!("  Crawling feed...");
    let crawler = federation::crawler::Crawler::new(settings.crawler)?;
    let result = crawler.crawl_feed(&pool, &url).await?;

    println!(
        "\x1b[32m\u{2713}\x1b[0m Reindex complete: {} new recipes indexed",
        result.new_recipes
    );

    Ok(())
}
