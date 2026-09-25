use crate::db::models::Recipe;
use crate::error::{Error, Result};
use crate::indexer::extras::IndexExtras;
use crate::indexer::filters::{normalize_difficulty, SearchFilters};
use crate::indexer::locale::normalize_code;
use crate::indexer::plain_text::instructions_text;
use crate::indexer::schema::RecipeSchema;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tantivy::collector::{Count, TopDocs};
use tantivy::query::{
    BooleanQuery, ConstScoreQuery, Occur, PhraseQuery, Query, QueryParser, TermQuery,
};
use tantivy::schema::IndexRecordOption;
use tantivy::tokenizer::TokenStream;
use tantivy::{doc, Index, IndexReader, IndexWriter, ReloadPolicy, Term};
use tracing::{debug, info};

pub struct SearchIndex {
    index: Index,
    reader: IndexReader,
    schema: RecipeSchema,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    pub q: String, // Unified query string
    pub page: usize,
    pub limit: usize,
    /// Optional exact-match language filter, e.g. "de" or "en-US".
    pub locale: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub recipe_id: i64,
    pub title: String,
    pub summary: Option<String>,
    pub score: f32,
    pub locale: Option<String>,
    pub total_time_minutes: Option<i64>,
    pub servings: Option<i64>,
    pub difficulty: Option<String>,
    pub image_url: Option<String>,
    pub feed_id: Option<i64>,
    pub feed_title: Option<String>,
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
            let index = Index::open_in_dir(path)
                .map_err(|e| Error::Search(format!("Failed to open index: {e}")))?;

            // Tantivy pins field ids to the schema stored on disk. If our schema has
            // changed since the index was written, every field id we hold is wrong and
            // writing a document corrupts or panics. Refuse to open it.
            if index.schema() != schema.schema {
                return Err(Error::Search(format!(
                    "Search index at {} was built with a different schema and cannot be used. \
                     Delete it and rebuild: rm -rf {} && federation backfill-locales --force",
                    path.display(),
                    path.display(),
                )));
            }

            index
        } else {
            Index::create_in_dir(path, schema.schema.clone())
                .map_err(|e| Error::Search(format!("Failed to create index: {e}")))?
        };

        RecipeSchema::register_tokenizers(&index);

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

    /// Index a recipe with everything a search document carries: the row's own
    /// fields plus the tags, ingredients, file path and feed title in `extras`.
    /// Deletes any existing document for the recipe first, so re-indexing is
    /// idempotent.
    pub fn index_recipe_full(
        &self,
        writer: &mut IndexWriter,
        recipe: &Recipe,
        extras: &IndexExtras,
    ) -> Result<()> {
        debug!("Indexing recipe: {}", recipe.id);

        // Delete existing documents with this recipe_id FIRST
        let term = Term::from_field_i64(self.schema.id, recipe.id);
        writer.delete_term(term);

        // `sort=newest` means "when the recipe entered the federation", which is
        // `created_at`. Deliberately ignore the DB `indexed_at` column here: it
        // changes on re-index, and preferring it would bubble old recipes to the
        // top every time they get re-touched.
        let indexed_at = recipe.created_at.timestamp();

        let mut doc = doc!(
            self.schema.id => recipe.id,
            self.schema.title => recipe.title.clone(),
            self.schema.feed_id => recipe.feed_id,
            self.schema.indexed_at => indexed_at,
        );

        if let Some(summary) = &recipe.summary {
            doc.add_text(self.schema.summary, summary);
        }

        // Add instructions as rendered prose, not raw Cooklang markup
        if let Some(content) = &recipe.content {
            doc.add_text(self.schema.instructions, instructions_text(content));
        }

        if let Some(servings) = recipe.servings {
            doc.add_i64(self.schema.servings, servings);
        }

        if let Some(time) = recipe.total_time_minutes {
            doc.add_i64(self.schema.total_time, time);
        }

        // Difficulty is an exact-match field: store it canonically so the
        // `difficulty=` filter and facet values agree.
        if let Some(difficulty) = &recipe.difficulty {
            let difficulty = normalize_difficulty(difficulty);
            if !difficulty.is_empty() {
                doc.add_text(self.schema.difficulty, difficulty);
            }
        }

        if let Some(image_url) = &recipe.image_url {
            doc.add_text(self.schema.image_url, image_url);
        }

        if let Some(feed_title) = &extras.feed_title {
            doc.add_text(self.schema.feed_title, feed_title);
        }

        // Add file path (for GitHub recipes)
        if let Some(path) = &extras.file_path {
            doc.add_text(self.schema.file_path, path);
        }

        // Add locale, plus its base language when the code carries a region, so a
        // filter on "en" also matches an "en-US" recipe.
        if let Some(locale) = &recipe.locale {
            doc.add_text(self.schema.locale, locale);

            if let Some((language, _region)) = locale.split_once('-') {
                doc.add_text(self.schema.locale, language);
            }
        }

        for tag in &extras.tags {
            doc.add_text(self.schema.tags, tag);
        }

        for ingredient in &extras.ingredients {
            doc.add_text(self.schema.ingredients, ingredient);
        }

        writer.add_document(doc)?;

        Ok(())
    }

    /// Index a recipe without a feed title. Kept for tests and simple callers;
    /// production paths load [`IndexExtras`] and call [`Self::index_recipe_full`].
    pub fn index_recipe(
        &self,
        writer: &mut IndexWriter,
        recipe: &Recipe,
        file_path: Option<&str>,
        tags: &[String],
        ingredients: &[String],
    ) -> Result<()> {
        self.index_recipe_full(
            writer,
            recipe,
            &IndexExtras {
                file_path: file_path.map(str::to_string),
                tags: tags.to_vec(),
                ingredients: ingredients.to_vec(),
                feed_title: None,
            },
        )
    }

    /// Delete a recipe from the index
    pub fn delete_recipe(&self, writer: &mut IndexWriter, recipe_id: i64) -> Result<()> {
        let term = Term::from_field_i64(self.schema.id, recipe_id);
        writer.delete_term(term);
        Ok(())
    }

    /// Search recipes using unified query string
    pub fn search(&self, query: &SearchQuery, max_limit: usize) -> Result<SearchResults> {
        self.search_with(query, &SearchFilters::default(), max_limit)
    }

    /// Search with structured filters ANDed onto the parsed query string.
    pub fn search_with(
        &self,
        query: &SearchQuery,
        filters: &SearchFilters,
        max_limit: usize,
    ) -> Result<SearchResults> {
        let searcher = self.reader.searcher();

        // Build query parser over the fields free text should search. The file
        // path is excluded: directory names are not recipe content, but it can
        // still be targeted explicitly with `file_path:`.
        let mut query_parser = QueryParser::for_index(
            &self.index,
            vec![
                self.schema.title,
                self.schema.summary,
                self.schema.instructions,
                self.schema.ingredients,
                self.schema.tags,
                self.schema.difficulty,
            ],
        );

        // A word in the title or tags says more about a recipe than the same word
        // buried in a step.
        query_parser.set_field_boost(self.schema.title, 3.0);
        query_parser.set_field_boost(self.schema.tags, 2.0);
        query_parser.set_field_boost(self.schema.ingredients, 1.5);

        // Every term narrows the result set: `vegan tags:dessert` means vegan AND
        // dessert. Tantivy's default is OR, which turns field filters into suggestions.
        query_parser.set_conjunction_by_default();

        // Parse unified query string
        let parsed_query = if query.q.is_empty() {
            Box::new(tantivy::query::AllQuery) as Box<dyn Query>
        } else {
            query_parser
                .parse_query(&query.q)
                .map_err(|e| Error::Search(format!("Invalid query: {e}")))?
        };

        let mut clauses: Vec<(Occur, Box<dyn Query>)> = vec![(Occur::Must, parsed_query)];

        // AND an exact locale term onto the parsed query when filtering. The locale
        // field is untokenized (exact match), so normalize the incoming filter to the
        // canonical stored form first — otherwise `?locale=EN` or `?locale=en-us`
        // would silently match nothing.
        if let Some(locale) = query.locale.as_deref().filter(|l| !l.is_empty()) {
            let term = Term::from_field_text(self.schema.locale, &normalize_code(locale));
            clauses.push((
                Occur::Must,
                Box::new(TermQuery::new(term, IndexRecordOption::Basic)),
            ));
        }

        clauses.extend(self.filter_clauses(filters)?);

        let tantivy_query: Box<dyn Query> = if clauses.len() == 1 {
            clauses.remove(0).1
        } else {
            Box::new(BooleanQuery::new(clauses))
        };

        // Calculate offset
        let offset = (query.page.saturating_sub(1)) * query.limit;
        let limit = query.limit.min(max_limit);

        // Execute search: the page of hits plus a full count in one pass.
        let (top_docs, total) = searcher
            .search(
                &*tantivy_query,
                &(TopDocs::with_limit(limit).and_offset(offset), Count),
            )
            .map_err(|e| Error::Search(format!("Search failed: {e}")))?;

        let results: Vec<SearchResult> = top_docs
            .into_iter()
            .filter_map(|(score, doc_address)| {
                let doc = searcher.doc::<tantivy::TantivyDocument>(doc_address).ok()?;
                self.result_from_doc(&doc, score)
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

    /// Query clauses for the structured filters. Positive filters are wrapped
    /// in a zero constant score, so they narrow results without changing how
    /// the free-text query ranks them.
    fn filter_clauses(&self, filters: &SearchFilters) -> Result<Vec<(Occur, Box<dyn Query>)>> {
        let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();

        for tag in &filters.tags {
            if let Some(query) = self.text_match_query(self.schema.tags, tag)? {
                clauses.push((Occur::Must, unscored(query)));
            }
        }

        for ingredient in &filters.include_ingredients {
            if let Some(query) = self.text_match_query(self.schema.ingredients, ingredient)? {
                clauses.push((Occur::Must, unscored(query)));
            }
        }

        for ingredient in &filters.exclude_ingredients {
            if let Some(query) = self.text_match_query(self.schema.ingredients, ingredient)? {
                clauses.push((Occur::MustNot, query));
            }
        }

        Ok(clauses)
    }

    /// A query matching `text` in `field` after running it through that field's
    /// analyzer, so a filter value is lowercased and stemmed exactly like the
    /// indexed values. One token becomes a term query and several become a phrase.
    /// Text that yields no tokens returns `None`, which means no filter.
    fn text_match_query(
        &self,
        field: tantivy::schema::Field,
        text: &str,
    ) -> Result<Option<Box<dyn Query>>> {
        let mut analyzer = self
            .index
            .tokenizer_for_field(field)
            .map_err(|e| Error::Search(format!("No analyzer for field: {e}")))?;

        let mut terms = Vec::new();
        {
            let mut stream = analyzer.token_stream(text);
            stream.process(&mut |token| terms.push(Term::from_field_text(field, &token.text)));
        }

        Ok(match terms.len() {
            0 => None,
            1 => Some(
                Box::new(TermQuery::new(terms.remove(0), IndexRecordOption::Basic))
                    as Box<dyn Query>,
            ),
            _ => Some(Box::new(PhraseQuery::new(terms)) as Box<dyn Query>),
        })
    }

    /// Build a result card from a stored document. Returns `None` only for a
    /// document without an id or title, which the indexer never writes.
    fn result_from_doc(&self, doc: &tantivy::TantivyDocument, score: f32) -> Option<SearchResult> {
        Some(SearchResult {
            recipe_id: stored_i64(doc, self.schema.id)?,
            title: stored_str(doc, self.schema.title)?,
            summary: stored_str(doc, self.schema.summary),
            score,
            locale: stored_str(doc, self.schema.locale),
            total_time_minutes: stored_i64(doc, self.schema.total_time),
            servings: stored_i64(doc, self.schema.servings),
            difficulty: stored_str(doc, self.schema.difficulty),
            image_url: stored_str(doc, self.schema.image_url),
            feed_id: stored_i64(doc, self.schema.feed_id),
            feed_title: stored_str(doc, self.schema.feed_title),
        })
    }

    /// Commit changes to the index
    pub fn commit(&self, writer: &mut IndexWriter) -> Result<()> {
        writer
            .commit()
            .map_err(|e| Error::Search(format!("Failed to commit: {e}")))?;

        // `ReloadPolicy::OnCommitWithDelay` reloads the reader asynchronously via a
        // filesystem watcher, so a `search()` call immediately after `commit()` can
        // race ahead of that reload and observe a stale (empty) index. Reload
        // explicitly so callers of this helper see their own writes right away.
        self.reader
            .reload()
            .map_err(|e| Error::Search(format!("Failed to reload reader: {e}")))?;

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

/// First stored string value of `field`, if any.
fn stored_str(doc: &tantivy::TantivyDocument, field: tantivy::schema::Field) -> Option<String> {
    match doc.get_first(field)? {
        tantivy::schema::OwnedValue::Str(s) => Some(s.to_string()),
        _ => None,
    }
}

/// First stored i64 value of `field`, if any.
fn stored_i64(doc: &tantivy::TantivyDocument, field: tantivy::schema::Field) -> Option<i64> {
    match doc.get_first(field)? {
        tantivy::schema::OwnedValue::I64(value) => Some(*value),
        _ => None,
    }
}

/// Wrap a filter so it contributes nothing to the relevance score.
fn unscored(query: Box<dyn Query>) -> Box<dyn Query> {
    Box::new(ConstScoreQuery::new(query, 0.0))
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
    fn test_opening_an_index_with_a_stale_schema_is_refused() {
        use tantivy::schema::{Schema, TEXT};

        // An index written by an older build, whose schema no longer matches ours.
        // Tantivy pins field ids to the on-disk schema, so using our field ids against
        // it would panic or corrupt the index rather than fail cleanly.
        let dir = tempdir().unwrap();
        let mut builder = Schema::builder();
        builder.add_text_field("title", TEXT);
        Index::create_in_dir(dir.path(), builder.build()).unwrap();

        let Err(err) = SearchIndex::new(dir.path()) else {
            panic!("an index with a mismatched schema must not open");
        };
        let message = err.to_string();

        assert!(
            message.contains("different schema"),
            "error should name the cause: {message}"
        );
        assert!(
            message.contains("rm -rf"),
            "error should tell the operator how to rebuild: {message}"
        );
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
            locale: None,
        };

        let result = index.search(&query, 1000);
        assert!(result.is_ok());

        // Test field-specific query
        let query = SearchQuery {
            q: "tags:dessert".to_string(),
            page: 1,
            limit: 20,
            locale: None,
        };

        let result = index.search(&query, 1000);
        assert!(result.is_ok());

        // Test complex query
        let query = SearchQuery {
            q: "chocolate tags:dessert total_time:[0 TO 60]".to_string(),
            page: 1,
            limit: 20,
            locale: None,
        };

        let result = index.search(&query, 1000);
        assert!(result.is_ok());
    }

    #[test]
    fn test_index_recipe_deletes_before_adding() {
        use crate::db::models::Recipe;
        use chrono::Utc;
        use tantivy::collector::Count;
        use tantivy::query::AllQuery;
        use tantivy::schema::Value;

        let dir = tempdir().unwrap();
        let index = SearchIndex::new(dir.path()).unwrap();
        let mut writer = index.writer().unwrap();

        // Create test recipe
        let recipe = Recipe {
            id: 123,
            feed_id: 1,
            external_id: "test-recipe".to_string(),
            title: "Original Title".to_string(),
            summary: Some("Test summary".to_string()),
            source_url: None,
            enclosure_url: "https://example.com/test.cook".to_string(),
            content: Some("@flour{500%g}\n@sugar{200%g}".to_string()),
            servings: Some(4),
            total_time_minutes: Some(30),
            active_time_minutes: Some(15),
            difficulty: Some("easy".to_string()),
            image_url: None,
            published_at: Some(Utc::now()),
            updated_at: Some(Utc::now()),
            indexed_at: None,
            created_at: Utc::now(),
            content_hash: None,
            content_etag: None,
            content_last_modified: None,
            feed_entry_updated: None,
            locale: None,
            locale_source: None,
        };

        // Index recipe first time
        index
            .index_recipe(&mut writer, &recipe, None, &[], &[])
            .unwrap();
        writer.commit().unwrap();
        drop(writer); // Drop writer to release lock

        // Reload reader and verify one document exists
        index.reader.reload().unwrap();
        let searcher = index.reader.searcher();
        let all_query = AllQuery;
        let count = searcher.search(&all_query, &Count).unwrap();
        assert_eq!(count, 1, "Should have exactly 1 document after first index");

        // Update recipe (same ID, different title)
        let updated_recipe = Recipe {
            id: 123,
            title: "Updated Title".to_string(),
            summary: Some("Updated summary".to_string()),
            ..recipe
        };

        // Index again (simulating an update)
        let mut writer = index.writer().unwrap();
        index
            .index_recipe(&mut writer, &updated_recipe, None, &[], &[])
            .unwrap();
        writer.commit().unwrap();

        // Reload and verify still only one document total
        index.reader.reload().unwrap();
        let searcher = index.reader.searcher();
        let total = searcher.search(&all_query, &Count).unwrap();
        assert_eq!(
            total, 1,
            "Should STILL have exactly 1 document total after update (delete-before-add removed the old one)"
        );

        // Verify the document has the updated title
        let top_docs = searcher
            .search(&all_query, &TopDocs::with_limit(1))
            .unwrap();
        assert_eq!(top_docs.len(), 1, "Should have exactly 1 document");

        let doc = searcher
            .doc::<tantivy::TantivyDocument>(top_docs[0].1)
            .unwrap();
        let title = doc.get_first(index.schema.title).unwrap().as_str().unwrap();
        assert_eq!(
            title, "Updated Title",
            "Document should have the updated title, not the original"
        );

        // Verify it has the correct ID
        let id_value = doc.get_first(index.schema.id).unwrap();
        if let tantivy::schema::OwnedValue::I64(id) = id_value {
            assert_eq!(*id, 123, "Document should have ID 123");
        } else {
            panic!("ID field should be I64");
        }
    }

    fn test_recipe(id: i64, title: &str, locale: Option<&str>) -> Recipe {
        Recipe {
            id,
            feed_id: 1,
            external_id: format!("ext-{id}"),
            title: title.to_string(),
            source_url: None,
            enclosure_url: format!("https://example.com/{id}.cook"),
            content: Some("Mix the flour and the water.".to_string()),
            summary: None,
            servings: None,
            total_time_minutes: None,
            active_time_minutes: None,
            difficulty: None,
            image_url: None,
            published_at: None,
            updated_at: None,
            indexed_at: None,
            created_at: chrono::Utc::now(),
            content_hash: None,
            content_etag: None,
            content_last_modified: None,
            feed_entry_updated: None,
            locale: locale.map(str::to_string),
            locale_source: locale.map(|_| "detected".to_string()),
        }
    }

    #[test]
    fn test_search_filters_by_locale() {
        let dir = tempdir().unwrap();
        let index = SearchIndex::new(dir.path()).unwrap();
        let mut writer = index.writer().unwrap();

        index
            .index_recipe(
                &mut writer,
                &test_recipe(1, "Pancakes", Some("en")),
                None,
                &[],
                &[],
            )
            .unwrap();
        index
            .index_recipe(
                &mut writer,
                &test_recipe(2, "Pfannkuchen", Some("de")),
                None,
                &[],
                &[],
            )
            .unwrap();
        index
            .index_recipe(&mut writer, &test_recipe(3, "Crepes", None), None, &[], &[])
            .unwrap();
        index.commit(&mut writer).unwrap();

        // Filtering by locale returns only that language.
        let results = index
            .search(
                &SearchQuery {
                    q: String::new(),
                    page: 1,
                    limit: 10,
                    locale: Some("de".to_string()),
                },
                10,
            )
            .unwrap();
        assert_eq!(results.results.len(), 1);
        assert_eq!(results.results[0].recipe_id, 2);
        assert_eq!(results.results[0].locale.as_deref(), Some("de"));

        // No filter returns everything, including the recipe with no locale.
        let all = index
            .search(
                &SearchQuery {
                    q: String::new(),
                    page: 1,
                    limit: 10,
                    locale: None,
                },
                10,
            )
            .unwrap();
        assert_eq!(all.results.len(), 3);
    }

    #[test]
    fn test_locale_filter_combines_with_query() {
        let dir = tempdir().unwrap();
        let index = SearchIndex::new(dir.path()).unwrap();
        let mut writer = index.writer().unwrap();

        index
            .index_recipe(
                &mut writer,
                &test_recipe(1, "Pancakes", Some("en")),
                None,
                &[],
                &[],
            )
            .unwrap();
        index
            .index_recipe(
                &mut writer,
                &test_recipe(2, "Pancakes", Some("de")),
                None,
                &[],
                &[],
            )
            .unwrap();
        index.commit(&mut writer).unwrap();

        let results = index
            .search(
                &SearchQuery {
                    q: "pancakes".to_string(),
                    page: 1,
                    limit: 10,
                    locale: Some("en".to_string()),
                },
                10,
            )
            .unwrap();

        assert_eq!(results.results.len(), 1);
        assert_eq!(results.results[0].recipe_id, 1);
    }

    #[test]
    fn test_regional_locale_matches_base_language_filter() {
        let dir = tempdir().unwrap();
        let index = SearchIndex::new(dir.path()).unwrap();
        let mut writer = index.writer().unwrap();

        index
            .index_recipe(
                &mut writer,
                &test_recipe(1, "Biscuits", Some("en-US")),
                None,
                &[],
                &[],
            )
            .unwrap();
        index.commit(&mut writer).unwrap();

        // Filtering by the base language finds the regional recipe...
        let base = index
            .search(
                &SearchQuery {
                    q: String::new(),
                    page: 1,
                    limit: 10,
                    locale: Some("en".to_string()),
                },
                10,
            )
            .unwrap();
        assert_eq!(base.results.len(), 1);

        // ...and the stored code keeps its region.
        assert_eq!(base.results[0].locale.as_deref(), Some("en-US"));

        // A different language does not match.
        let other = index
            .search(
                &SearchQuery {
                    q: String::new(),
                    page: 1,
                    limit: 10,
                    locale: Some("de".to_string()),
                },
                10,
            )
            .unwrap();
        assert_eq!(other.results.len(), 0);
    }

    #[test]
    fn test_locale_filter_is_case_insensitive() {
        // Locale codes are stored canonically as `en-US`. The field is untokenized
        // (exact match), so a differently-cased filter must be normalized before
        // comparison, or the API silently returns zero results.
        let dir = tempdir().unwrap();
        let index = SearchIndex::new(dir.path()).unwrap();
        let mut writer = index.writer().unwrap();

        index
            .index_recipe(
                &mut writer,
                &test_recipe(1, "Biscuits", Some("en-US")),
                None,
                &[],
                &[],
            )
            .unwrap();
        index.commit(&mut writer).unwrap();

        for filter in ["EN", "en-us", "En-Us"] {
            let results = index
                .search(
                    &SearchQuery {
                        q: String::new(),
                        page: 1,
                        limit: 10,
                        locale: Some(filter.to_string()),
                    },
                    10,
                )
                .unwrap();
            assert_eq!(
                results.results.len(),
                1,
                "filter {filter:?} should match the recipe stored as en-US"
            );
        }
    }
}

#[cfg(test)]
mod quality_tests {
    use super::*;
    use crate::db::models::Recipe;
    use tempfile::tempdir;

    fn recipe(id: i64, title: &str, content: &str) -> Recipe {
        Recipe {
            id,
            feed_id: 1,
            external_id: format!("ext-{id}"),
            title: title.to_string(),
            source_url: None,
            enclosure_url: format!("https://example.com/{id}.cook"),
            content: Some(content.to_string()),
            summary: None,
            servings: None,
            total_time_minutes: None,
            active_time_minutes: None,
            difficulty: None,
            image_url: None,
            published_at: None,
            updated_at: None,
            indexed_at: None,
            created_at: chrono::Utc::now(),
            content_hash: None,
            content_etag: None,
            content_last_modified: None,
            feed_entry_updated: None,
            locale: Some("en".to_string()),
            locale_source: None,
        }
    }

    fn q(text: &str, page: usize, limit: usize) -> SearchQuery {
        SearchQuery {
            q: text.to_string(),
            page,
            limit,
            locale: None,
        }
    }

    fn ids(results: &SearchResults) -> Vec<i64> {
        results.results.iter().map(|r| r.recipe_id).collect()
    }

    #[test]
    fn all_query_terms_must_match() {
        let dir = tempdir().unwrap();
        let index = SearchIndex::new(dir.path()).unwrap();
        let mut w = index.writer().unwrap();
        index
            .index_recipe(
                &mut w,
                &recipe(1, "Vegan Lemon Cake", "Bake it."),
                None,
                &["dessert".into(), "vegan".into()],
                &[],
            )
            .unwrap();
        index
            .index_recipe(
                &mut w,
                &recipe(2, "Vegan Noodle Soup", "Simmer it."),
                None,
                &["soup".into(), "vegan".into()],
                &[],
            )
            .unwrap();
        index
            .index_recipe(
                &mut w,
                &recipe(3, "Chocolate Tart", "Chill it."),
                None,
                &["dessert".into()],
                &[],
            )
            .unwrap();
        index.commit(&mut w).unwrap();

        let results = index.search(&q("vegan tags:dessert", 1, 10), 100).unwrap();
        assert_eq!(
            ids(&results),
            vec![1],
            "only the vegan dessert should match"
        );

        let results = index
            .search(&q("tags:dessert -chocolate", 1, 10), 100)
            .unwrap();
        assert_eq!(ids(&results), vec![1], "exclusion should still work");
    }

    #[test]
    fn total_counts_every_hit_and_pages_are_reachable() {
        let dir = tempdir().unwrap();
        let index = SearchIndex::new(dir.path()).unwrap();
        let mut w = index.writer().unwrap();
        for id in 1..=5 {
            index
                .index_recipe(
                    &mut w,
                    &recipe(id, &format!("Pancakes {id}"), "Flip them."),
                    None,
                    &[],
                    &[],
                )
                .unwrap();
        }
        index.commit(&mut w).unwrap();

        let page1 = index.search(&q("pancakes", 1, 2), 100).unwrap();
        assert_eq!(page1.total, 5);
        assert_eq!(page1.total_pages, 3);
        assert_eq!(page1.results.len(), 2);

        let page3 = index.search(&q("pancakes", 3, 2), 100).unwrap();
        assert_eq!(page3.total, 5);
        assert_eq!(page3.results.len(), 1);

        let all: std::collections::BTreeSet<i64> = (1..=3)
            .flat_map(|p| ids(&index.search(&q("pancakes", p, 2), 100).unwrap()))
            .collect();
        assert_eq!(all.len(), 5, "paging must visit every recipe exactly once");
    }

    #[test]
    fn title_match_outranks_instructions_match() {
        let dir = tempdir().unwrap();
        let index = SearchIndex::new(dir.path()).unwrap();
        let mut w = index.writer().unwrap();
        // The body mentions brownies many times, the other recipe only in the title.
        index
            .index_recipe(
                &mut w,
                &recipe(
                    1,
                    "Vanilla Ice Cream",
                    "Serve with brownies. Brownies love ice cream. Brownies again.",
                ),
                None,
                &[],
                &[],
            )
            .unwrap();
        index
            .index_recipe(
                &mut w,
                &recipe(2, "Fudgy Brownies", "Bake until set."),
                None,
                &[],
                &[],
            )
            .unwrap();
        index.commit(&mut w).unwrap();

        let results = index.search(&q("brownies", 1, 10), 100).unwrap();
        assert_eq!(ids(&results), vec![2, 1]);
    }

    #[test]
    fn free_text_does_not_match_file_path() {
        let dir = tempdir().unwrap();
        let index = SearchIndex::new(dir.path()).unwrap();
        let mut w = index.writer().unwrap();
        index
            .index_recipe(
                &mut w,
                &recipe(1, "Tomato Soup", "Simmer."),
                Some("archive/breakfast/soup.cook"),
                &[],
                &[],
            )
            .unwrap();
        index.commit(&mut w).unwrap();

        let results = index.search(&q("breakfast", 1, 10), 100).unwrap();
        assert!(
            ids(&results).is_empty(),
            "a directory name is not recipe content"
        );

        let results = index.search(&q("file_path:breakfast", 1, 10), 100).unwrap();
        assert_eq!(ids(&results), vec![1], "explicit field queries still work");
    }

    #[test]
    fn singular_query_matches_plural_title() {
        let dir = tempdir().unwrap();
        let index = SearchIndex::new(dir.path()).unwrap();
        let mut w = index.writer().unwrap();
        index
            .index_recipe(
                &mut w,
                &recipe(1, "Chocolate Cakes", "Bake."),
                None,
                &["desserts".into()],
                &["eggs".into()],
            )
            .unwrap();
        index.commit(&mut w).unwrap();

        assert_eq!(ids(&index.search(&q("cake", 1, 10), 100).unwrap()), vec![1]);
        assert_eq!(
            ids(&index.search(&q("tags:dessert", 1, 10), 100).unwrap()),
            vec![1]
        );
        assert_eq!(
            ids(&index.search(&q("ingredients:egg", 1, 10), 100).unwrap()),
            vec![1]
        );
    }

    #[test]
    fn instructions_index_plain_text_not_cooklang_markup() {
        let dir = tempdir().unwrap();
        let index = SearchIndex::new(dir.path()).unwrap();
        let mut w = index.writer().unwrap();
        let content = "---\ntitle: Dressing\n---\nWhisk @olive oil{2%tbsp} with #bowl{} for ~{1%minute}. -- secret note\n";
        index
            .index_recipe(&mut w, &recipe(1, "Dressing", content), None, &[], &[])
            .unwrap();
        index.commit(&mut w).unwrap();

        assert_eq!(
            ids(&index
                .search(&q("instructions:\"olive oil\"", 1, 10), 100)
                .unwrap()),
            vec![1]
        );
        assert_eq!(
            ids(&index.search(&q("whisk bowl", 1, 10), 100).unwrap()),
            vec![1]
        );
        assert!(
            ids(&index.search(&q("instructions:tbsp", 1, 10), 100).unwrap()).is_empty(),
            "units are noise"
        );
        assert!(
            ids(&index.search(&q("instructions:secret", 1, 10), 100).unwrap()).is_empty(),
            "comments are not indexed"
        );
        assert!(
            ids(&index.search(&q("instructions:title", 1, 10), 100).unwrap()).is_empty(),
            "frontmatter keys are not indexed"
        );
    }
}

#[cfg(test)]
mod card_tests {
    use super::*;
    use crate::db::models::Recipe;
    use crate::indexer::extras::IndexExtras;
    use chrono::TimeZone;
    use tantivy::schema::Value;
    use tempfile::tempdir;

    fn created_at() -> chrono::DateTime<chrono::Utc> {
        chrono::Utc.with_ymd_and_hms(2026, 9, 1, 12, 0, 0).unwrap()
    }

    fn card_recipe() -> Recipe {
        Recipe {
            id: 7,
            feed_id: 12,
            external_id: "ext-7".to_string(),
            title: "Lemon Tart".to_string(),
            source_url: Some("https://example.com/lemon-tart".to_string()),
            enclosure_url: "https://example.com/lemon-tart.cook".to_string(),
            content: Some("Bake the @pastry{}.".to_string()),
            summary: Some("Sharp and sweet.".to_string()),
            servings: Some(6),
            total_time_minutes: Some(45),
            active_time_minutes: Some(20),
            difficulty: Some(" Easy ".to_string()),
            image_url: Some("https://example.com/lemon-tart.jpg".to_string()),
            published_at: None,
            updated_at: None,
            indexed_at: None,
            created_at: created_at(),
            content_hash: None,
            content_etag: None,
            content_last_modified: None,
            feed_entry_updated: None,
            locale: Some("en".to_string()),
            locale_source: Some("declared".to_string()),
        }
    }

    fn card_extras() -> IndexExtras {
        IndexExtras {
            file_path: None,
            tags: vec!["dessert".to_string()],
            ingredients: vec!["pastry".to_string()],
            feed_title: Some("Jane's Kitchen".to_string()),
        }
    }

    #[test]
    fn index_recipe_full_stores_card_fields() {
        let dir = tempdir().unwrap();
        let index = SearchIndex::new(dir.path()).unwrap();
        let mut writer = index.writer().unwrap();
        index
            .index_recipe_full(&mut writer, &card_recipe(), &card_extras())
            .unwrap();
        index.commit(&mut writer).unwrap();

        let searcher = index.reader.searcher();
        let top = searcher
            .search(&tantivy::query::AllQuery, &TopDocs::with_limit(1))
            .unwrap();
        let doc = searcher.doc::<tantivy::TantivyDocument>(top[0].1).unwrap();

        assert_eq!(
            doc.get_first(index.schema.feed_id).and_then(|v| v.as_i64()),
            Some(12)
        );
        assert_eq!(
            doc.get_first(index.schema.indexed_at)
                .and_then(|v| v.as_i64()),
            Some(created_at().timestamp()),
            "indexed_at falls back to created_at"
        );
        assert_eq!(
            doc.get_first(index.schema.image_url)
                .and_then(|v| v.as_str()),
            Some("https://example.com/lemon-tart.jpg")
        );
        assert_eq!(
            doc.get_first(index.schema.feed_title)
                .and_then(|v| v.as_str()),
            Some("Jane's Kitchen")
        );
        assert_eq!(
            doc.get_first(index.schema.difficulty)
                .and_then(|v| v.as_str()),
            Some("easy"),
            "difficulty is normalised at index time"
        );
    }

    #[test]
    fn index_recipe_full_ignores_db_indexed_at_in_favor_of_created_at() {
        let dir = tempdir().unwrap();
        let index = SearchIndex::new(dir.path()).unwrap();
        let mut writer = index.writer().unwrap();

        let mut recipe = card_recipe();
        // A later DB `indexed_at` must not win: `sort=newest` means "when the
        // recipe entered the federation" (created_at), and re-indexing must not
        // bubble old recipes to the top just because they were re-touched.
        recipe.indexed_at = Some(created_at() + chrono::Duration::days(30));

        index
            .index_recipe_full(&mut writer, &recipe, &card_extras())
            .unwrap();
        index.commit(&mut writer).unwrap();

        let searcher = index.reader.searcher();
        let top = searcher
            .search(&tantivy::query::AllQuery, &TopDocs::with_limit(1))
            .unwrap();
        let doc = searcher.doc::<tantivy::TantivyDocument>(top[0].1).unwrap();

        assert_eq!(
            doc.get_first(index.schema.indexed_at)
                .and_then(|v| v.as_i64()),
            Some(created_at().timestamp()),
            "indexed_at must always be created_at, never the DB indexed_at"
        );
    }

    #[test]
    fn index_recipe_full_stores_no_difficulty_when_whitespace_only() {
        let dir = tempdir().unwrap();
        let index = SearchIndex::new(dir.path()).unwrap();
        let mut writer = index.writer().unwrap();

        let mut recipe = card_recipe();
        recipe.difficulty = Some("   ".to_string());

        index
            .index_recipe_full(&mut writer, &recipe, &card_extras())
            .unwrap();
        index.commit(&mut writer).unwrap();

        let searcher = index.reader.searcher();
        let top = searcher
            .search(&tantivy::query::AllQuery, &TopDocs::with_limit(1))
            .unwrap();
        let doc = searcher.doc::<tantivy::TantivyDocument>(top[0].1).unwrap();

        assert_eq!(
            doc.get_first(index.schema.difficulty)
                .and_then(|v| v.as_str()),
            None,
            "whitespace-only difficulty stores no difficulty field"
        );
    }

    fn query(q: &str) -> SearchQuery {
        SearchQuery {
            q: q.to_string(),
            page: 1,
            limit: 10,
            locale: None,
        }
    }

    #[test]
    fn search_results_carry_card_fields() {
        let dir = tempdir().unwrap();
        let index = SearchIndex::new(dir.path()).unwrap();
        let mut writer = index.writer().unwrap();
        index
            .index_recipe_full(&mut writer, &card_recipe(), &card_extras())
            .unwrap();
        index.commit(&mut writer).unwrap();

        let results = index.search(&query("tart"), 10).unwrap();
        let card = &results.results[0];

        assert_eq!(card.recipe_id, 7);
        assert_eq!(card.total_time_minutes, Some(45));
        assert_eq!(card.servings, Some(6));
        assert_eq!(card.difficulty.as_deref(), Some("easy"));
        assert_eq!(
            card.image_url.as_deref(),
            Some("https://example.com/lemon-tart.jpg")
        );
        assert_eq!(card.feed_id, Some(12));
        assert_eq!(card.feed_title.as_deref(), Some("Jane's Kitchen"));
    }

    #[test]
    fn search_results_leave_missing_card_fields_empty() {
        let dir = tempdir().unwrap();
        let index = SearchIndex::new(dir.path()).unwrap();
        let mut writer = index.writer().unwrap();
        let bare = Recipe {
            servings: None,
            total_time_minutes: None,
            difficulty: None,
            image_url: None,
            ..card_recipe()
        };
        index
            .index_recipe(&mut writer, &bare, None, &[], &[])
            .unwrap();
        index.commit(&mut writer).unwrap();

        let results = index.search(&query("tart"), 10).unwrap();
        let card = &results.results[0];

        assert_eq!(card.total_time_minutes, None);
        assert_eq!(card.servings, None);
        assert_eq!(card.difficulty, None);
        assert_eq!(card.image_url, None);
        assert_eq!(card.feed_title, None);
        assert_eq!(card.feed_id, Some(12), "feed_id is always indexed");
    }
}

#[cfg(test)]
mod filter_tests {
    use super::*;
    use crate::db::models::Recipe;
    use crate::indexer::extras::IndexExtras;
    use crate::indexer::filters::SearchFilters;
    use chrono::TimeZone;
    use tempfile::{tempdir, TempDir};

    struct Fixture {
        index: SearchIndex,
        _dir: TempDir,
    }

    struct Spec {
        id: i64,
        title: &'static str,
        feed_id: i64,
        tags: &'static [&'static str],
        ingredients: &'static [&'static str],
        total_time: Option<i64>,
        servings: Option<i64>,
        difficulty: Option<&'static str>,
        locale: Option<&'static str>,
        created_day: u32,
    }

    fn recipe(spec: &Spec) -> Recipe {
        Recipe {
            id: spec.id,
            feed_id: spec.feed_id,
            external_id: format!("ext-{}", spec.id),
            title: spec.title.to_string(),
            source_url: None,
            enclosure_url: format!("https://example.com/{}.cook", spec.id),
            content: Some("Cook it.".to_string()),
            summary: None,
            servings: spec.servings,
            total_time_minutes: spec.total_time,
            active_time_minutes: None,
            difficulty: spec.difficulty.map(str::to_string),
            image_url: None,
            published_at: None,
            updated_at: None,
            indexed_at: None,
            created_at: chrono::Utc
                .with_ymd_and_hms(2026, 1, spec.created_day, 0, 0, 0)
                .unwrap(),
            content_hash: None,
            content_etag: None,
            content_last_modified: None,
            feed_entry_updated: None,
            locale: spec.locale.map(str::to_string),
            locale_source: None,
        }
    }

    fn fixture(specs: &[Spec]) -> Fixture {
        let dir = tempdir().unwrap();
        let index = SearchIndex::new(dir.path()).unwrap();
        let mut writer = index.writer().unwrap();
        for spec in specs {
            let extras = IndexExtras {
                file_path: None,
                tags: strings(spec.tags),
                ingredients: strings(spec.ingredients),
                feed_title: None,
            };
            index
                .index_recipe_full(&mut writer, &recipe(spec), &extras)
                .unwrap();
        }
        index.commit(&mut writer).unwrap();
        Fixture { index, _dir: dir }
    }

    /// Four recipes over two feeds, two languages and four creation days.
    fn corpus() -> Fixture {
        fixture(&[
            Spec {
                id: 1,
                title: "Vegan Chocolate Cake",
                feed_id: 10,
                tags: &["vegan", "desserts"],
                ingredients: &["cocoa", "flour"],
                total_time: Some(60),
                servings: Some(8),
                difficulty: Some("Medium"),
                locale: Some("en"),
                created_day: 1,
            },
            Spec {
                id: 2,
                title: "Garlic Lemon Chicken",
                feed_id: 10,
                tags: &["dinner"],
                ingredients: &["garlic", "lemon", "chicken thigh"],
                total_time: Some(30),
                servings: Some(4),
                difficulty: Some("easy"),
                locale: Some("en"),
                created_day: 3,
            },
            Spec {
                id: 3,
                title: "Peanut Noodles",
                feed_id: 20,
                tags: &["dinner", "vegan"],
                ingredients: &["peanut butter", "noodles", "garlic"],
                total_time: Some(15),
                servings: Some(2),
                difficulty: Some("easy"),
                locale: Some("en"),
                created_day: 2,
            },
            Spec {
                id: 4,
                title: "Zitronenkuchen",
                feed_id: 20,
                tags: &["dessert"],
                ingredients: &["lemon", "flour"],
                total_time: None,
                servings: None,
                difficulty: None,
                locale: Some("de"),
                created_day: 4,
            },
        ])
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn query(q: &str, locale: Option<&str>) -> SearchQuery {
        SearchQuery {
            q: q.to_string(),
            page: 1,
            limit: 50,
            locale: locale.map(str::to_string),
        }
    }

    /// Matching ids, sorted, for relevance searches (order is not under test).
    fn ids(fixture: &Fixture, q: &str, locale: Option<&str>, filters: &SearchFilters) -> Vec<i64> {
        let mut ids: Vec<i64> = fixture
            .index
            .search_with(&query(q, locale), filters, 100)
            .unwrap()
            .results
            .iter()
            .map(|r| r.recipe_id)
            .collect();
        ids.sort();
        ids
    }

    fn tags(items: &[&str]) -> SearchFilters {
        SearchFilters {
            tags: strings(items),
            ..SearchFilters::default()
        }
    }

    #[test]
    fn tags_filter_requires_every_tag() {
        let f = corpus();
        assert_eq!(ids(&f, "", None, &tags(&["vegan"])), vec![1, 3]);
        assert_eq!(ids(&f, "", None, &tags(&["vegan", "dinner"])), vec![3]);
    }

    #[test]
    fn tags_filter_is_stemmed_and_case_insensitive() {
        let f = corpus();
        // "Desserts" and the stored "desserts"/"dessert" all stem to "dessert".
        assert_eq!(ids(&f, "", None, &tags(&["Desserts"])), vec![1, 4]);
    }

    #[test]
    fn blank_filter_values_are_ignored() {
        let f = corpus();
        assert_eq!(ids(&f, "", None, &tags(&[" "])), vec![1, 2, 3, 4]);
    }

    #[test]
    fn include_ingredients_requires_every_ingredient() {
        let f = corpus();
        let filters = SearchFilters {
            include_ingredients: strings(&["garlic", "lemon"]),
            ..SearchFilters::default()
        };
        assert_eq!(ids(&f, "", None, &filters), vec![2]);

        let filters = SearchFilters {
            include_ingredients: strings(&["garlic"]),
            ..SearchFilters::default()
        };
        assert_eq!(ids(&f, "", None, &filters), vec![2, 3]);
    }

    #[test]
    fn multi_word_ingredient_matches_as_a_phrase() {
        let f = corpus();
        let filters = SearchFilters {
            include_ingredients: strings(&["peanut butter"]),
            ..SearchFilters::default()
        };
        assert_eq!(ids(&f, "", None, &filters), vec![3]);

        let filters = SearchFilters {
            include_ingredients: strings(&["butter peanut"]),
            ..SearchFilters::default()
        };
        assert!(ids(&f, "", None, &filters).is_empty(), "word order matters");
    }

    #[test]
    fn exclude_ingredients_removes_any_match() {
        let f = corpus();
        let filters = SearchFilters {
            exclude_ingredients: strings(&["peanut"]),
            ..SearchFilters::default()
        };
        assert_eq!(ids(&f, "", None, &filters), vec![1, 2, 4]);

        let filters = SearchFilters {
            exclude_ingredients: strings(&["garlic", "flour"]),
            ..SearchFilters::default()
        };
        assert!(ids(&f, "", None, &filters).is_empty());
    }

    #[test]
    fn filters_combine_with_query_and_locale() {
        let f = corpus();
        assert_eq!(ids(&f, "lemon", Some("de"), &tags(&["dessert"])), vec![4]);

        let filters = SearchFilters {
            include_ingredients: strings(&["garlic"]),
            ..SearchFilters::default()
        };
        assert_eq!(ids(&f, "lemon", None, &filters), vec![2]);
    }
}
