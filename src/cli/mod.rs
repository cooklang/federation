// Phase 6: CLI module
// This module provides command-line interface

pub mod commands;

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "federation")]
#[command(about = "Cooklang Federation - Federated recipe search", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Start the federation server
    Serve {
        /// Port to listen on
        #[arg(short, long, env = "PORT")]
        port: Option<u16>,

        /// Host to bind to
        #[arg(long, env = "HOST")]
        host: Option<String>,
    },

    /// Search for recipes (to be implemented in Phase 6)
    Search {
        /// Search query
        query: String,

        /// Filter by tags
        #[arg(long)]
        tags: Option<String>,

        /// Maximum cooking time in minutes
        #[arg(long)]
        max_time: Option<i64>,
    },

    /// Download a recipe (to be implemented in Phase 6)
    Download {
        /// Recipe ID
        recipe_id: i64,

        /// Output directory
        #[arg(short, long)]
        output: Option<String>,
    },

    /// Publish recipes as a feed (to be implemented in Phase 6)
    Publish {
        /// Input directory containing .cook files
        #[arg(short, long)]
        input: String,

        /// Output feed file
        #[arg(short, long)]
        output: String,
    },

    /// Run database migrations
    Migrate,

    /// Validate a feed URL
    Validate {
        /// Feed URL to validate
        url: String,
    },

    /// Reindex a feed (delete all recipes and re-crawl)
    Reindex {
        /// Feed URL to reindex
        url: String,
    },
}
