//! Structured search filters, applied on top of the free-text query.

/// Canonical form of a difficulty value, used both when indexing and when
/// filtering: trimmed and lowercased, so "Easy " matches `difficulty=easy`.
pub fn normalize_difficulty(value: &str) -> String {
    value.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn difficulty_is_trimmed_and_lowercased() {
        assert_eq!(normalize_difficulty("  Easy "), "easy");
        assert_eq!(normalize_difficulty("HARD"), "hard");
    }
}
