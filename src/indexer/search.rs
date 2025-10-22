use crate::db::models::Recipe;
use crate::error::{Error, Result};
use crate::indexer::schema::RecipeSchema;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tantivy::collector::TopDocs;
use tantivy::query::{Query, QueryParser};
use tantivy::{doc, Index, IndexReader, IndexWriter, ReloadPolicy, Term};
use tracing::{debug, info};

pub struct SearchIndex {
    index: Index,
    reader: IndexReader,
    schema: RecipeSchema,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    pub q: String,        // Unified query string
    pub page: usize,
    pub limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub recipe_id: i64,
    pub title: String,
    pub summary: Option<String>,
    pub score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResults {
    pub results: Vec<SearchResult>,
    pub total: usize,
    pub page: usize,
    pub total_pages: usize,
}

impl SearchIndex {
    /// Create or open search index
    pub fn new(index_path: impl AsRef<Path>) -> Result<Self> {
        let path = index_path.as_ref();
        let schema = RecipeSchema::new();

        // Create directory if it doesn't exist
        std::fs::create_dir_all(path)?;

        // Open or create index
        let index = if path.join("meta.json").exists() {
            Index::open_in_dir(path)
                .map_err(|e| Error::Search(format!("Failed to open index: {e}")))?
        } else {
            Index::create_in_dir(path, schema.schema.clone())
                .map_err(|e| Error::Search(format!("Failed to create index: {e}")))?
        };

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .map_err(|e| Error::Search(format!("Failed to create reader: {e}")))?;

        info!("Search index initialized at {:?}", path);

        Ok(Self {
            index,
            reader,
            schema,
        })
    }

    /// Get index writer
    pub fn writer(&self) -> Result<IndexWriter> {
        self.index
            .writer(50_000_000) // 50MB buffer
            .map_err(|e| Error::Search(format!("Failed to create writer: {e}")))
    }

    /// Index a recipe
    pub fn index_recipe(
        &self,
        writer: &mut IndexWriter,
        recipe: &Recipe,
        file_path: Option<&str>,
        tags: &[String],
        ingredients: &[String],
    ) -> Result<()> {
        debug!("Indexing recipe: {}", recipe.id);

        let mut doc = doc!(
            self.schema.id => recipe.id,
            self.schema.title => recipe.title.clone(),
        );

        // Add summary
        if let Some(summary) = &recipe.summary {
            doc.add_text(self.schema.summary, summary);
        }

        // Add instructions (from content)
        if let Some(content) = &recipe.content {
            doc.add_text(self.schema.instructions, content);
        }

        // Add servings
        if let Some(servings) = recipe.servings {
            doc.add_i64(self.schema.servings, servings);
        }

        // Add total time
        if let Some(time) = recipe.total_time_minutes {
            doc.add_i64(self.schema.total_time, time);
        }

        // Add difficulty
        if let Some(difficulty) = &recipe.difficulty {
            doc.add_text(self.schema.difficulty, difficulty);
        }

        // Add file path (for GitHub recipes)
        if let Some(path) = file_path {
            doc.add_text(self.schema.file_path, path);
        }

        // Add tags
        for tag in tags {
            doc.add_text(self.schema.tags, tag);
        }

        // Add ingredients
        for ingredient in ingredients {
            doc.add_text(self.schema.ingredients, ingredient);
        }

        writer.add_document(doc)?;

        Ok(())
    }

    /// Add tags to a recipe in the index
    pub fn add_recipe_tags(
        &self,
        _writer: &mut IndexWriter,
        _recipe_id: i64,
        _tags: &[String],
    ) -> Result<()> {
        // Note: In a real implementation, we'd need to fetch the full recipe
        // and re-index it with tags. For now, this is a placeholder.
        // This would be improved in a production implementation.

        Ok(())
    }

    /// Add ingredients to a recipe in the index
    pub fn add_recipe_ingredients(
        &self,
        _writer: &mut IndexWriter,
        _recipe_id: i64,
        _ingredients: &[String],
    ) -> Result<()> {
        // Similar to tags - would need full re-indexing
        Ok(())
    }

    /// Delete a recipe from the index
    pub fn delete_recipe(&self, writer: &mut IndexWriter, recipe_id: i64) -> Result<()> {
        let term = Term::from_field_i64(self.schema.id, recipe_id);
        writer.delete_term(term);
        Ok(())
    }

    /// Search recipes using unified query string
    pub fn search(&self, query: &SearchQuery) -> Result<SearchResults> {
        let searcher = self.reader.searcher();

        // Build query parser with all searchable fields
        let query_parser = QueryParser::for_index(
            &self.index,
            vec![
                self.schema.title,
                self.schema.summary,
                self.schema.instructions,
                self.schema.ingredients,
                self.schema.tags,
                self.schema.difficulty,
                self.schema.file_path,
            ],
        );

        // Parse unified query string
        let tantivy_query = if query.q.is_empty() {
            Box::new(tantivy::query::AllQuery) as Box<dyn Query>
        } else {
            query_parser
                .parse_query(&query.q)
                .map_err(|e| Error::Search(format!("Invalid query: {e}")))?
        };

        // Calculate offset
        let offset = (query.page.saturating_sub(1)) * query.limit;
        let limit = query.limit.min(100); // Max 100 results per page

        // Execute search
        let top_docs = searcher
            .search(&*tantivy_query, &TopDocs::with_limit(limit + offset))
            .map_err(|e| Error::Search(format!("Search failed: {e}")))?;

        // Get total count
        let total = top_docs.len();

        // Extract results with pagination
        let results: Vec<SearchResult> = top_docs
            .into_iter()
            .skip(offset)
            .take(limit)
            .filter_map(|(score, doc_address)| {
                let doc = searcher.doc::<tantivy::TantivyDocument>(doc_address).ok()?;

                let recipe_id = match doc.get_first(self.schema.id)? {
                    tantivy::schema::OwnedValue::I64(id) => *id,
                    _ => return None,
                };

                let title = match doc.get_first(self.schema.title)? {
                    tantivy::schema::OwnedValue::Str(s) => s.to_string(),
                    _ => return None,
                };

                let summary = doc.get_first(self.schema.summary).and_then(|v| match v {
                    tantivy::schema::OwnedValue::Str(s) => Some(s.to_string()),
                    _ => None,
                });

                Some(SearchResult {
                    recipe_id,
                    title,
                    summary,
                    score,
                })
            })
            .collect();

        let total_pages = total.div_ceil(limit);

        Ok(SearchResults {
            results,
            total,
            page: query.page,
            total_pages,
        })
    }

    /// Commit changes to the index
    pub fn commit(&self, writer: &mut IndexWriter) -> Result<()> {
        writer
            .commit()
            .map_err(|e| Error::Search(format!("Failed to commit: {e}")))?;
        Ok(())
    }

    /// Optimize the search index (merge segments)
    pub async fn optimize(&self) -> Result<()> {
        use tantivy::TantivyDocument;

        let writer = self.index.writer::<TantivyDocument>(50_000_000)?;

        writer
            .wait_merging_threads()
            .map_err(|e| Error::Search(format!("Failed to optimize index: {e}")))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_create_index() {
        let dir = tempdir().unwrap();
        let index = SearchIndex::new(dir.path());
        assert!(index.is_ok());
    }

    #[test]
    fn test_search_unified() {
        let dir = tempdir().unwrap();
        let index = SearchIndex::new(dir.path()).unwrap();

        // Test simple query
        let query = SearchQuery {
            q: "chocolate".to_string(),
            page: 1,
            limit: 20,
        };

        let result = index.search(&query);
        assert!(result.is_ok());

        // Test field-specific query
        let query = SearchQuery {
            q: "tags:dessert".to_string(),
            page: 1,
            limit: 20,
        };

        let result = index.search(&query);
        assert!(result.is_ok());

        // Test complex query
        let query = SearchQuery {
            q: "chocolate tags:dessert total_time:[0 TO 60]".to_string(),
            page: 1,
            limit: 20,
        };

        let result = index.search(&query);
        assert!(result.is_ok());
    }
}
