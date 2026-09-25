use crate::db::models::Recipe;
use crate::error::{Error, Result};
use crate::indexer::extras::IndexExtras;
use crate::indexer::filters::normalize_difficulty;
use crate::indexer::locale::normalize_code;
use crate::indexer::plain_text::instructions_text;
use crate::indexer::schema::RecipeSchema;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tantivy::collector::{Count, TopDocs};
use tantivy::query::{BooleanQuery, Occur, Query, QueryParser, TermQuery};
use tantivy::schema::IndexRecordOption;
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
        let tantivy_query = if query.q.is_empty() {
            Box::new(tantivy::query::AllQuery) as Box<dyn Query>
        } else {
            query_parser
                .parse_query(&query.q)
                .map_err(|e| Error::Search(format!("Invalid query: {e}")))?
        };

        // AND an exact locale term onto the parsed query when filtering. The locale
        // field is untokenized (exact match), so normalize the incoming filter to the
        // canonical stored form first — otherwise `?locale=EN` or `?locale=en-us`
        // would silently match nothing.
        let tantivy_query = match query.locale.as_deref().filter(|l| !l.is_empty()) {
            Some(locale) => {
                let normalized = normalize_code(locale);
                let term = Term::from_field_text(self.schema.locale, &normalized);
                let locale_query: Box<dyn Query> =
                    Box::new(TermQuery::new(term, IndexRecordOption::Basic));

                Box::new(BooleanQuery::new(vec![
                    (Occur::Must, tantivy_query),
                    (Occur::Must, locale_query),
                ])) as Box<dyn Query>
            }
            None => tantivy_query,
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

        // Extract results with pagination
        let results: Vec<SearchResult> = top_docs
            .into_iter()
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

                let locale = doc.get_first(self.schema.locale).and_then(|v| match v {
                    tantivy::schema::OwnedValue::Str(s) => Some(s.to_string()),
                    _ => None,
                });

                Some(SearchResult {
                    recipe_id,
                    title,
                    summary,
                    score,
                    locale,
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
}
