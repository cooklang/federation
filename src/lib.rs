pub mod config;
pub mod db;
pub mod error;

// Phase 2 modules
pub mod crawler;

// Phase 3 modules
pub mod indexer;

// GitHub integration
pub mod github;

// Phase 4 modules
pub mod api;

// Phase 5 modules
pub mod web;

// Phase 6 modules
pub mod cli;

// Utilities
pub mod utils;

// Re-exports
pub use config::Settings;
pub use error::{Error, Result};
