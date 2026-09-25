//! Structured-filter query parameters shared by `GET /api/search` and the
//! website search form.

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::indexer::filters::{normalize_difficulty, SearchFilters, SortOrder};

/// Structured-filter query parameters. Values arrive as strings, so an empty
/// form field means "no filter" and a malformed number becomes a 400 with a
/// JSON message instead of axum's plain-text query rejection.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct FilterParams {
    /// Comma-separated; the recipe has all of them.
    #[serde(default)]
    pub tags: Option<String>,
    /// Comma-separated; the recipe uses all of them.
    #[serde(default)]
    pub include_ingredients: Option<String>,
    /// Comma-separated; the recipe uses none of them.
    #[serde(default)]
    pub exclude_ingredients: Option<String>,
    /// Minutes.
    #[serde(default)]
    pub max_time: Option<String>,
    #[serde(default)]
    pub min_servings: Option<String>,
    #[serde(default)]
    pub max_servings: Option<String>,
    #[serde(default)]
    pub difficulty: Option<String>,
    #[serde(default)]
    pub feed_id: Option<String>,
    /// `relevance` (default) or `newest`.
    #[serde(default)]
    pub sort: Option<String>,
}

impl FilterParams {
    /// Validate and convert to search filters and an order.
    pub fn parse(&self) -> Result<(SearchFilters, SortOrder)> {
        let filters = SearchFilters {
            tags: split_list(self.tags.as_deref()),
            include_ingredients: split_list(self.include_ingredients.as_deref()),
            exclude_ingredients: split_list(self.exclude_ingredients.as_deref()),
            max_time: parse_count("max_time", self.max_time.as_deref())?,
            min_servings: parse_count("min_servings", self.min_servings.as_deref())?,
            max_servings: parse_count("max_servings", self.max_servings.as_deref())?,
            difficulty: non_empty(self.difficulty.as_deref()).map(normalize_difficulty),
            feed_id: parse_count("feed_id", self.feed_id.as_deref())?,
        };

        if let (Some(min), Some(max)) = (filters.min_servings, filters.max_servings) {
            if min > max {
                return Err(Error::Validation(format!(
                    "min_servings ({min}) must not be greater than max_servings ({max})"
                )));
            }
        }

        let sort = match non_empty(self.sort.as_deref())
            .map(str::to_lowercase)
            .as_deref()
        {
            None | Some("relevance") => SortOrder::Relevance,
            Some("newest") => SortOrder::Newest,
            Some(other) => {
                return Err(Error::Validation(format!(
                    "sort must be \"relevance\" or \"newest\", got \"{other}\""
                )))
            }
        };

        Ok((filters, sort))
    }

    /// The populated parameters as `(name, value)` pairs in a fixed order, for
    /// building links that keep the current filters (pagination).
    pub fn query_pairs(&self) -> Vec<(&'static str, &str)> {
        [
            ("tags", &self.tags),
            ("include_ingredients", &self.include_ingredients),
            ("exclude_ingredients", &self.exclude_ingredients),
            ("max_time", &self.max_time),
            ("min_servings", &self.min_servings),
            ("max_servings", &self.max_servings),
            ("difficulty", &self.difficulty),
            ("feed_id", &self.feed_id),
            ("sort", &self.sort),
        ]
        .into_iter()
        .filter_map(|(name, value)| non_empty(value.as_deref()).map(|value| (name, value)))
        .collect()
    }
}

/// `Some(trimmed)` unless the value is missing or blank.
fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

/// Comma-separated list, items trimmed, empty items dropped.
fn split_list(value: Option<&str>) -> Vec<String> {
    value
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

/// A non-negative whole number, or `None` when blank.
fn parse_count(name: &str, value: Option<&str>) -> Result<Option<i64>> {
    let Some(raw) = non_empty(value) else {
        return Ok(None);
    };
    match raw.parse::<i64>() {
        Ok(number) if number >= 0 => Ok(Some(number)),
        _ => Err(Error::Validation(format!(
            "{name} must be a non-negative whole number, got \"{raw}\""
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;
    use crate::indexer::filters::SortOrder;

    #[test]
    fn lists_are_trimmed_and_empty_items_dropped() {
        let (filters, _) = FilterParams {
            tags: Some(" vegan, ,Dessert ,".into()),
            include_ingredients: Some("garlic,lemon".into()),
            exclude_ingredients: Some(",".into()),
            ..FilterParams::default()
        }
        .parse()
        .unwrap();
        assert_eq!(filters.tags, vec!["vegan", "Dessert"]);
        assert_eq!(filters.include_ingredients, vec!["garlic", "lemon"]);
        assert!(filters.exclude_ingredients.is_empty());
    }

    #[test]
    fn empty_values_mean_no_filter() {
        let (filters, sort) = FilterParams {
            tags: Some(String::new()),
            max_time: Some(String::new()),
            min_servings: Some("  ".into()),
            difficulty: Some(String::new()),
            sort: Some(String::new()),
            ..FilterParams::default()
        }
        .parse()
        .unwrap();
        assert!(filters.is_empty());
        assert_eq!(sort, SortOrder::Relevance);
    }

    #[test]
    fn numbers_are_parsed() {
        let (filters, _) = FilterParams {
            max_time: Some("30".into()),
            min_servings: Some("2".into()),
            max_servings: Some(" 6 ".into()),
            feed_id: Some("12".into()),
            ..FilterParams::default()
        }
        .parse()
        .unwrap();
        assert_eq!(filters.max_time, Some(30));
        assert_eq!(filters.min_servings, Some(2));
        assert_eq!(filters.max_servings, Some(6));
        assert_eq!(filters.feed_id, Some(12));
    }

    #[test]
    fn invalid_numbers_are_rejected_naming_the_parameter() {
        let cases = [
            (
                FilterParams {
                    max_time: Some("soon".into()),
                    ..FilterParams::default()
                },
                "max_time",
            ),
            (
                FilterParams {
                    min_servings: Some("-1".into()),
                    ..FilterParams::default()
                },
                "min_servings",
            ),
            (
                FilterParams {
                    max_servings: Some("2.5".into()),
                    ..FilterParams::default()
                },
                "max_servings",
            ),
            (
                FilterParams {
                    feed_id: Some("abc".into()),
                    ..FilterParams::default()
                },
                "feed_id",
            ),
        ];
        for (params, name) in cases {
            match params.parse() {
                Err(Error::Validation(message)) => {
                    assert!(message.contains(name), "{name}: {message}")
                }
                other => panic!("{name}: expected a validation error, got {other:?}"),
            }
        }
    }

    #[test]
    fn min_servings_above_max_is_rejected() {
        let result = FilterParams {
            min_servings: Some("6".into()),
            max_servings: Some("2".into()),
            ..FilterParams::default()
        }
        .parse();
        assert!(matches!(result, Err(Error::Validation(_))));
    }

    #[test]
    fn sort_and_difficulty_are_normalised() {
        let (filters, sort) = FilterParams {
            difficulty: Some(" Easy ".into()),
            sort: Some("Newest".into()),
            ..FilterParams::default()
        }
        .parse()
        .unwrap();
        assert_eq!(filters.difficulty.as_deref(), Some("easy"));
        assert_eq!(sort, SortOrder::Newest);

        let result = FilterParams {
            sort: Some("rating".into()),
            ..FilterParams::default()
        }
        .parse();
        assert!(
            matches!(result, Err(Error::Validation(ref m)) if m.contains("sort")),
            "{result:?}"
        );
    }

    #[test]
    fn query_pairs_keep_order_and_skip_empty_values() {
        let params = FilterParams {
            tags: Some("vegan".into()),
            max_time: Some(String::new()),
            sort: Some("newest".into()),
            ..FilterParams::default()
        };
        assert_eq!(
            params.query_pairs(),
            vec![("tags", "vegan"), ("sort", "newest")]
        );
    }
}
