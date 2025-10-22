// Phase 3: Recipe indexing and search module
// This module handles Cooklang parsing and Tantivy search indexing

pub mod cooklang_parser;
pub mod recipe;
pub mod schema;
pub mod search;

// Re-exports
pub use cooklang_parser::{parse_recipe as parse_cooklang_full, ParsedRecipeData};
pub use recipe::{parse_cooklang, ParsedRecipe};
pub use schema::RecipeSchema;
pub use search::{SearchIndex, SearchQuery, SearchResult, SearchResults};
