//! Inputs to a search document that live outside the `recipes` row.

/// Everything indexed alongside a recipe row that comes from other tables:
/// its GitHub file path, tag and ingredient names, and its feed's title.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IndexExtras {
    pub file_path: Option<String>,
    pub tags: Vec<String>,
    pub ingredients: Vec<String>,
    pub feed_title: Option<String>,
}
