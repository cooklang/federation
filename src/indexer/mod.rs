// Phase 3: Recipe indexing and search module
// This module handles Cooklang parsing and Tantivy search indexing

pub mod cooklang_parser;
pub mod extras;
pub mod filters;
pub mod locale;
mod plain_text;
pub mod recipe;
pub mod recipe_facts;
pub mod schema;
pub mod search;

// Re-exports
pub use cooklang_parser::{parse_recipe as parse_cooklang_full, ParsedRecipeData};
pub use locale::{resolve_locale, LocaleSource, RecipeLocale};
pub use plain_text::instructions_text;
pub use recipe::{parse_cooklang, ParsedRecipe};
pub use schema::RecipeSchema;
pub use search::{SearchIndex, SearchQuery, SearchResult, SearchResults};
