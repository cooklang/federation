//! Schema.org JSON-LD generation for recipes
//! Ported from CookCLI's cooklang_to_schema.rs

use serde_json::{json, Value};

/// Extract first number from a string like "30 minutes" -> 30
fn extract_number(s: &str) -> Option<i32> {
    // Find the first sequence of digits
    let mut num_str = String::new();
    for c in s.chars() {
        if c.is_numeric() {
            num_str.push(c);
        } else if !num_str.is_empty() {
            // We've found a complete number, stop here
            break;
        }
    }
    num_str.parse::<i32>().ok()
}

/// Convert time string to ISO 8601 duration format
/// "30 minutes" -> "PT30M", "1 hour" -> "PT1H"
fn format_iso_duration(time_str: &str) -> Option<String> {
    let lower = time_str.to_lowercase();

    // Check for hours
    if lower.contains("hour") {
        if let Some(hours) = extract_number(&lower) {
            // Check if also contains minutes
            if lower.contains("min") {
                // Split and get minutes part
                if let Some(min_part) = lower.split("hour").nth(1) {
                    if let Some(minutes) = extract_number(min_part) {
                        return Some(format!("PT{hours}H{minutes}M"));
                    }
                }
            }
            return Some(format!("PT{hours}H"));
        }
    }

    // Check for minutes
    if lower.contains("min") {
        if let Some(minutes) = extract_number(&lower) {
            return Some(format!("PT{minutes}M"));
        }
    }

    // Fallback: assume minutes if just a number
    if let Some(minutes) = extract_number(&lower) {
        return Some(format!("PT{minutes}M"));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_iso_duration() {
        assert_eq!(format_iso_duration("30 minutes"), Some("PT30M".to_string()));
        assert_eq!(format_iso_duration("1 hour"), Some("PT1H".to_string()));
        assert_eq!(format_iso_duration("1 hour 30 minutes"), Some("PT1H30M".to_string()));
        assert_eq!(format_iso_duration("45 min"), Some("PT45M".to_string()));
        assert_eq!(format_iso_duration("15"), Some("PT15M".to_string()));
    }

    #[test]
    fn test_extract_number() {
        assert_eq!(extract_number("30 minutes"), Some(30));
        assert_eq!(extract_number("1 hour"), Some(1));
        assert_eq!(extract_number("no number"), None);
    }
}
