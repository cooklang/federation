use tantivy::schema::{
    Field, IndexRecordOption, Schema, TextFieldIndexing, TextOptions, FAST, INDEXED, STORED,
    STRING, TEXT,
};
use tantivy::tokenizer::{LowerCaser, RemoveLongFilter, SimpleTokenizer, Stemmer, TextAnalyzer};
use tantivy::Index;

/// Name of the analyzer used for every free-text recipe field. It lowercases and
/// stems, so `cake` finds "Cakes" and `tags:dessert` finds "desserts".
pub const RECIPE_TOKENIZER: &str = "recipe_text";

/// Schema for recipe search index
#[derive(Clone)]
pub struct RecipeSchema {
    pub schema: Schema,
    pub id: Field,
    pub title: Field,
    pub summary: Field,
    pub instructions: Field,
    pub ingredients: Field,
    pub tags: Field,
    pub difficulty: Field,
    pub servings: Field,
    pub total_time: Field,
    pub file_path: Field,
    pub locale: Field,
}

impl RecipeSchema {
    pub fn new() -> Self {
        let mut schema_builder = Schema::builder();

        // Recipe ID (stored, indexed for deletion, fast for filtering)
        let id = schema_builder.add_i64_field("id", STORED | FAST | INDEXED);

        let stemmed = TextOptions::default().set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer(RECIPE_TOKENIZER)
                .set_index_option(IndexRecordOption::WithFreqsAndPositions),
        );

        // Title (searchable, stored, boosted at query time)
        let title = schema_builder.add_text_field("title", stemmed.clone() | STORED);

        // Summary (searchable, stored)
        let summary = schema_builder.add_text_field("summary", stemmed.clone() | STORED);

        // Instructions (searchable): rendered prose, not raw Cooklang markup
        let instructions = schema_builder.add_text_field("instructions", stemmed.clone());

        // Ingredients (searchable as text, faceted)
        let ingredients = schema_builder.add_text_field("ingredients", stemmed.clone() | STORED);

        // Tags (searchable, faceted)
        let tags = schema_builder.add_text_field("tags", stemmed | STORED);

        // Difficulty (faceted, filterable)
        let difficulty = schema_builder.add_text_field("difficulty", STRING | STORED);

        // Servings (filterable)
        let servings = schema_builder.add_i64_field("servings", FAST | STORED);

        // Total time in minutes (filterable)
        let total_time = schema_builder.add_i64_field("total_time", FAST | STORED);

        // File path (searchable, stored) - for GitHub recipes
        let file_path = schema_builder.add_text_field("file_path", TEXT | STORED);

        // Locale (exact-match filter, not tokenized, deliberately excluded from
        // the default query-parser fields so free text can't match it)
        let locale = schema_builder.add_text_field("locale", STRING | STORED);

        let schema = schema_builder.build();

        Self {
            schema,
            id,
            title,
            summary,
            instructions,
            ingredients,
            tags,
            difficulty,
            servings,
            total_time,
            file_path,
            locale,
        }
    }
}

impl RecipeSchema {
    /// Register the recipe text analyzer on an index. Tantivy stores tokenizer
    /// *names* in the schema, so this must run every time an index is opened,
    /// before any document is written or any query parsed.
    pub fn register_tokenizers(index: &Index) {
        let analyzer = TextAnalyzer::builder(SimpleTokenizer::default())
            .filter(RemoveLongFilter::limit(40))
            .filter(LowerCaser)
            .filter(Stemmer::new(tantivy::tokenizer::Language::English))
            .build();
        index.tokenizers().register(RECIPE_TOKENIZER, analyzer);
    }
}

impl Default for RecipeSchema {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_creation() {
        let schema = RecipeSchema::new();
        assert!(schema.schema.get_field("title").is_ok());
        assert!(schema.schema.get_field("ingredients").is_ok());
        assert!(schema.schema.get_field("tags").is_ok());
    }
}
