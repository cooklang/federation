use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),

    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Feed parsing error: {0}")]
    FeedParse(String),

    #[error("Recipe parsing error: {0}")]
    RecipeParse(String),

    #[error("Search error: {0}")]
    Search(String),

    #[error("Tantivy error: {0}")]
    Tantivy(#[from] tantivy::TantivyError),

    #[error("Invalid URL: {0}")]
    InvalidUrl(#[from] url::ParseError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// Get a sanitized error message safe for logging
    /// Filters out potentially sensitive information
    pub fn log_safe(&self) -> String {
        match self {
            // Database errors might contain sensitive schema information
            Error::Database(_) => "Database operation failed".to_string(),
            Error::Migration(_) => "Database migration failed".to_string(),

            // HTTP errors might contain internal URLs or authentication info
            Error::Http(_) => "External HTTP request failed".to_string(),

            // Internal errors might contain sensitive details
            Error::Internal(msg) => {
                // Filter out common sensitive patterns
                if msg.to_lowercase().contains("password")
                    || msg.to_lowercase().contains("secret")
                    || msg.to_lowercase().contains("token")
                    || msg.to_lowercase().contains("key")
                {
                    "Internal error (details redacted)".to_string()
                } else {
                    format!("Internal error: {msg}")
                }
            }

            // These errors are generally safe to log as-is
            Error::FeedParse(msg) => format!("Feed parsing error: {msg}"),
            Error::RecipeParse(msg) => format!("Recipe parsing error: {msg}"),
            Error::Search(msg) => format!("Search error: {msg}"),
            Error::Tantivy(_) => "Search index error".to_string(),
            Error::InvalidUrl(_) => "Invalid URL provided".to_string(),
            Error::Io(_) => "File system operation failed".to_string(),
            Error::Config(msg) => format!("Configuration error: {msg}"),
            Error::NotFound(msg) => format!("Not found: {msg}"),
            Error::Validation(msg) => format!("Validation error: {msg}"),
        }
    }
}

// Implement IntoResponse for API error handling
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        // Log the full error internally using the safe logging method
        tracing::error!("Request error: {}", self.log_safe());

        let (status, error_message) = match &self {
            Error::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            Error::Validation(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            Error::Database(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database error".to_string(),
            ),
            Error::Search(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Search error".to_string(),
            ),
            Error::Http(_) => (
                StatusCode::BAD_GATEWAY,
                "External service error".to_string(),
            ),
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error".to_string(),
            ),
        };

        let body = Json(json!({
            "error": error_message,
        }));

        (status, body).into_response()
    }
}
