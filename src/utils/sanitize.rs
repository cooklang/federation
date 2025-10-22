// Sanitization utilities
use ammonia;

/// Sanitize HTML content using ammonia library for comprehensive XSS protection
pub fn sanitize_html(text: &str) -> String {
    ammonia::clean(text)
}

/// Sanitize plain text content (escape HTML entities)
/// Use this for text that should not contain any HTML
pub fn sanitize_text(text: &str) -> String {
    // For plain text, escape all HTML entities
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
        .replace('/', "&#x2F;")
        .trim()
        .to_string()
}

/// Truncate text to a maximum length
pub fn truncate(text: &str, max_len: usize) -> String {
    if text.len() <= max_len {
        text.to_string()
    } else {
        format!("{}...", &text[..max_len.saturating_sub(3)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_html() {
        // ammonia should remove script tags entirely
        assert!(!sanitize_html("<script>alert('xss')</script>").contains("script"));

        // ammonia should allow safe HTML
        let safe_html = "<p>Hello <strong>world</strong></p>";
        let sanitized = sanitize_html(safe_html);
        assert!(sanitized.contains("<p>"));
        assert!(sanitized.contains("<strong>"));
    }

    #[test]
    fn test_sanitize_text() {
        // Should escape all HTML entities
        assert_eq!(
            sanitize_text("<script>alert('xss')</script>"),
            "&lt;script&gt;alert(&#x27;xss&#x27;)&lt;&#x2F;script&gt;"
        );

        // Should handle various special characters
        assert_eq!(
            sanitize_text("A & B < C > D \"quoted\" 'single'"),
            "A &amp; B &lt; C &gt; D &quot;quoted&quot; &#x27;single&#x27;"
        );
    }

    #[test]
    fn test_truncate() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 8), "hello...");
    }
}
