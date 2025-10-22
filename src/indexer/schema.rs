use tantivy::schema::{Field, Schema, FAST, STORED, STRING, TEXT};

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
}

impl RecipeSchema {
    pub fn new() -> Self {
        let mut schema_builder = Schema::builder();

        // Recipe ID (stored, not searchable)
        let id = schema_builder.add_i64_field("id", STORED | FAST);

        // Title (searchable, stored, boosted)
        let title = schema_builder.add_text_field("title", TEXT | STORED);

        // Summary (searchable, stored)
        let summary = schema_builder.add_text_field("summary", TEXT | STORED);

        // Instructions (searchable)
        let instructions = schema_builder.add_text_field("instructions", TEXT);

        // Ingredients (searchable as text, faceted)
        let ingredients = schema_builder.add_text_field("ingredients", TEXT | STORED);

        // Tags (searchable, faceted)
        let tags = schema_builder.add_text_field("tags", TEXT | STORED);

        // Difficulty (faceted, filterable)
        let difficulty = schema_builder.add_text_field("difficulty", STRING | STORED);

        // Servings (filterable)
        let servings = schema_builder.add_i64_field("servings", FAST | STORED);

        // Total time in minutes (filterable)
        let total_time = schema_builder.add_i64_field("total_time", FAST | STORED);

        // File path (searchable, stored) - for GitHub recipes
        let file_path = schema_builder.add_text_field("file_path", TEXT | STORED);

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
        }
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
