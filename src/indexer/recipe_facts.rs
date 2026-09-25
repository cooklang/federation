//! Servings, total time and difficulty taken from a recipe's Cooklang metadata.
//!
//! The `cooklang` crate's `Metadata` helpers own the key names (`time`,
//! `prep time` + `cook time`, ...) and the time-unit parsing. Nothing here
//! re-derives them.

use cooklang::metadata::{CooklangValueExt, Metadata, Servings, StdKey};
use cooklang::Converter;
use serde::{Deserialize, Serialize};

use crate::db::models::Recipe;
use crate::indexer::filters::normalize_difficulty;

/// The difficulty values the `recipes.difficulty` column accepts: its `CHECK`
/// constraint in `migrations/001_init.sql:32`. Keep the two in step.
const DIFFICULTIES: [&str; 3] = ["easy", "medium", "hard"];

/// The servings, total time and difficulty of a recipe, as stored in the
/// `recipes` columns of the same names and used by the search filters.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecipeFacts {
    pub servings: Option<i64>,
    pub total_time_minutes: Option<i64>,
    pub difficulty: Option<String>,
}

impl RecipeFacts {
    /// Read the facts from parsed Cooklang metadata. Unusable values (no
    /// number in `servings`, an unparseable `time`, a `difficulty` other than
    /// easy/medium/hard) and zero counts are unknown (`None`).
    pub fn from_metadata(metadata: &Metadata, converter: &Converter) -> Self {
        Self {
            servings: servings(metadata),
            total_time_minutes: metadata
                .time(converter)
                .map(|time| i64::from(time.total()))
                .filter(|&minutes| minutes > 0),
            difficulty: difficulty(metadata),
        }
    }

    /// The facts currently stored on a recipe row.
    pub fn from_recipe(recipe: &Recipe) -> Self {
        Self {
            servings: recipe.servings,
            total_time_minutes: recipe.total_time_minutes,
            difficulty: recipe.difficulty.clone(),
        }
    }

    /// Keep every known value of `self`; take the rest from `fallback`.
    pub fn or(self, fallback: RecipeFacts) -> RecipeFacts {
        RecipeFacts {
            servings: self.servings.or(fallback.servings),
            total_time_minutes: self.total_time_minutes.or(fallback.total_time_minutes),
            difficulty: self.difficulty.or(fallback.difficulty),
        }
    }
}

/// `servings` as a count: a number, the first number in a text value such as
/// `2-4` or `4 people`, or the first element of a list such as `[4, 6]`.
fn servings(metadata: &Metadata) -> Option<i64> {
    let servings = metadata.servings().or_else(|| {
        metadata
            .get(StdKey::Servings)?
            .as_sequence()?
            .first()?
            .as_servings()
    })?;
    let count = match servings {
        Servings::Number(count) => count,
        Servings::Text(text) => first_number(&text)?,
    };
    (count > 0).then_some(i64::from(count))
}

/// `difficulty` from the metadata, see [`allowed_difficulty`].
fn difficulty(metadata: &Metadata) -> Option<String> {
    allowed_difficulty(&metadata.get(StdKey::Difficulty)?.as_str_like()?)
}

/// A difficulty normalized (`" Medium "` is `medium`), if it is one of
/// [`DIFFICULTIES`]; `None` otherwise, since anything else would violate the
/// `recipes.difficulty` column's constraint.
pub fn allowed_difficulty(value: &str) -> Option<String> {
    let value = normalize_difficulty(value);
    DIFFICULTIES.contains(&value.as_str()).then_some(value)
}

/// The first run of ASCII digits in `text`, e.g. `2` in `"2-4"`.
fn first_number(text: &str) -> Option<u32> {
    let digits: String = text
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indexer::parse_cooklang_full;

    fn facts(content: &str) -> RecipeFacts {
        parse_cooklang_full(content).unwrap().facts
    }

    #[test]
    fn servings_time_and_difficulty_come_from_the_metadata() {
        assert_eq!(
            facts("---\nservings: 4\ntime: 1h 30min\ndifficulty: \" Medium \"\n---\nSimmer @beef{500%g}.\n"),
            RecipeFacts {
                servings: Some(4),
                total_time_minutes: Some(90),
                difficulty: Some("medium".to_string()),
            }
        );
    }

    #[test]
    fn a_difficulty_outside_easy_medium_hard_is_unknown() {
        assert_eq!(
            facts("---\ndifficulty: HARD\n---\nStir.\n")
                .difficulty
                .as_deref(),
            Some("hard")
        );
        assert_eq!(
            facts("---\ndifficulty: moderate\n---\nStir.\n").difficulty,
            None
        );
    }

    #[test]
    fn a_servings_range_or_list_counts_as_its_first_number() {
        assert_eq!(facts("---\nservings: 2-4\n---\nStir.\n").servings, Some(2));
        assert_eq!(
            facts("---\nservings: 4 people\n---\nStir.\n").servings,
            Some(4)
        );
        assert_eq!(
            facts("---\nservings: [6, 8]\n---\nStir.\n").servings,
            Some(6)
        );
    }

    #[test]
    fn prep_and_cook_time_add_up_when_there_is_no_total_time() {
        assert_eq!(
            facts("---\nprep time: 15 min\ncook time: 30\n---\nStir.\n").total_time_minutes,
            Some(45)
        );
    }

    #[test]
    fn unusable_or_missing_values_are_unknown() {
        assert_eq!(
            facts("---\nservings: a few\ntime: soon\ndifficulty: \"  \"\n---\nStir.\n"),
            RecipeFacts::default()
        );
        assert_eq!(facts("---\nservings: 0\n---\nStir.\n").servings, None);
        assert_eq!(facts("Just @salt{}.\n"), RecipeFacts::default());
    }

    #[test]
    fn or_keeps_known_values_and_fills_the_gaps() {
        let known = RecipeFacts {
            servings: Some(2),
            total_time_minutes: None,
            difficulty: Some("hard".to_string()),
        };
        let fallback = RecipeFacts {
            servings: Some(4),
            total_time_minutes: Some(30),
            difficulty: Some("easy".to_string()),
        };
        assert_eq!(
            known.or(fallback),
            RecipeFacts {
                servings: Some(2),
                total_time_minutes: Some(30),
                difficulty: Some("hard".to_string()),
            }
        );
    }
}
