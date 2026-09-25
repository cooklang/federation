//! Structured search filters, applied on top of the free-text query.

use serde::{Deserialize, Serialize};

/// Structured filters for a search. Each populated filter narrows the result
/// set; they combine with the free-text query and with each other (AND).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchFilters {
    /// The recipe has every one of these tags.
    pub tags: Vec<String>,
    /// The recipe uses every one of these ingredients.
    pub include_ingredients: Vec<String>,
    /// The recipe uses none of these ingredients.
    pub exclude_ingredients: Vec<String>,
    /// Total time at most this many minutes.
    pub max_time: Option<i64>,
    /// Servings at least this many (inclusive).
    pub min_servings: Option<i64>,
    /// Servings at most this many (inclusive).
    pub max_servings: Option<i64>,
    /// Exact difficulty; compared after [`normalize_difficulty`].
    pub difficulty: Option<String>,
    /// Only recipes from this feed.
    pub feed_id: Option<i64>,
}

impl SearchFilters {
    /// True when no filter is set.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Result ordering.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    /// Best match first (BM25 with field boosts).
    #[default]
    Relevance,
    /// Most recently added to the federation first.
    Newest,
}

/// Canonical form of a difficulty value, used both when indexing and when
/// filtering: trimmed and lowercased, so "Easy " matches `difficulty=easy`.
pub fn normalize_difficulty(value: &str) -> String {
    value.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn difficulty_is_trimmed_and_lowercased() {
        assert_eq!(normalize_difficulty("  Easy "), "easy");
        assert_eq!(normalize_difficulty("HARD"), "hard");
    }

    #[test]
    fn default_filters_are_empty() {
        assert!(SearchFilters::default().is_empty());
        let filters = SearchFilters {
            max_time: Some(30),
            ..SearchFilters::default()
        };
        assert!(!filters.is_empty());
    }
}
