// Utility functions
pub mod feed_validation;
pub mod sanitize;
pub mod validation;

/// Resolve an image URL against a base URL.
/// If the image URL is already absolute, return it as-is.
/// If relative, resolve it against the base URL's path.
pub fn resolve_image_url(image_url: &str, base_url: &str) -> Option<String> {
    // Try parsing as absolute URL first
    if url::Url::parse(image_url).is_ok() {
        return Some(image_url.to_string());
    }

    // Resolve relative URL against the base URL
    if let Ok(base) = url::Url::parse(base_url) {
        if let Ok(resolved) = base.join(image_url) {
            return Some(resolved.to_string());
        }
    }

    None
}
