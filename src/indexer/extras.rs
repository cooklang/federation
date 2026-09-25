//! Inputs to a search document that live outside the `recipes` row, and the
//! shared "load everything and (re)index these recipes" path.

use crate::db::{self, models::Recipe, DbPool};
use crate::error::{Error, Result};
use crate::indexer::search::SearchIndex;

/// Everything indexed alongside a recipe row that comes from other tables:
/// its GitHub file path, tag and ingredient names, and its feed's title.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IndexExtras {
    pub file_path: Option<String>,
    pub tags: Vec<String>,
    pub ingredients: Vec<String>,
    pub feed_title: Option<String>,
}

impl IndexExtras {
    /// Load a recipe's extras from the database.
    pub async fn load(pool: &DbPool, recipe: &Recipe) -> Result<Self> {
        let file_path = db::github::get_github_recipe_by_recipe_id(pool, recipe.id)
            .await?
            .map(|github_recipe| github_recipe.file_path);
        let tags = db::tags::get_tags_for_recipe(pool, recipe.id).await?;
        let ingredients = db::ingredients::get_ingredients_for_recipe(pool, recipe.id)
            .await?
            .into_iter()
            .map(|ingredient| ingredient.name)
            .collect();
        let feed_title = match db::feeds::get_feed(pool, recipe.feed_id).await {
            Ok(feed) => feed.title,
            Err(Error::NotFound(_)) => None,
            Err(e) => return Err(e),
        };

        Ok(Self {
            file_path,
            tags,
            ingredients,
            feed_title,
        })
    }
}

/// (Re)index the given recipes with their tags, ingredients, file path and
/// feed title, then commit once. All database reads happen before the writer
/// is taken, so the writer gate is held only for the index writes.
/// Returns the number of recipes indexed.
pub async fn reindex_recipes(
    pool: &DbPool,
    search_index: &SearchIndex,
    recipe_ids: &[i64],
) -> Result<usize> {
    if recipe_ids.is_empty() {
        return Ok(0);
    }

    let mut documents = Vec::with_capacity(recipe_ids.len());
    for &recipe_id in recipe_ids {
        let recipe = db::recipes::get_recipe(pool, recipe_id).await?;
        let extras = IndexExtras::load(pool, &recipe).await?;
        documents.push((recipe, extras));
    }

    let mut writer = search_index.locked_writer().await?;
    let written = documents
        .iter()
        .try_for_each(|(recipe, extras)| {
            search_index.index_recipe_full(&mut writer, recipe, extras)
        })
        .and_then(|()| search_index.commit(&mut writer));
    if let Err(e) = written {
        // Discard this batch's pending deletes/adds explicitly so a partial
        // batch is never committed by a later writer.
        let _ = writer.rollback();
        return Err(e);
    }

    Ok(documents.len())
}
