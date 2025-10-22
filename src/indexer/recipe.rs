use crate::db::models::IngredientWithQuantity;
use crate::error::Result;
use serde::{Deserialize, Serialize};
use tracing::debug;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedRecipe {
    pub metadata: RecipeMetadata,
    pub ingredients: Vec<IngredientWithQuantity>,
    pub instructions: String,
    pub sections: Vec<RecipeSection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipeMetadata {
    pub servings: Option<i64>,
    pub total_time: Option<String>,
    pub active_time: Option<String>,
    pub tags: Vec<String>,
    pub difficulty: Option<String>,
    pub other: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipeSection {
    pub name: Option<String>,
    pub instructions: String,
}

/// Parse a Cooklang recipe file
/// Note: Simplified implementation - full cooklang parsing to be enhanced
pub fn parse_cooklang(content: &str) -> Result<ParsedRecipe> {
    debug!("Parsing Cooklang recipe (simplified)");

    // Parse metadata from >> lines
    let mut metadata = RecipeMetadata {
        servings: None,
        total_time: None,
        active_time: None,
        tags: Vec::new(),
        difficulty: None,
        other: std::collections::HashMap::new(),
    };

    let mut instructions = String::new();
    let mut ingredients = Vec::new();

    for line in content.lines() {
        if let Some(meta_line) = line.strip_prefix(">>") {
            // Parse metadata
            if let Some((key, value)) = meta_line.split_once(':') {
                let key = key.trim().to_lowercase();
                let value = value.trim();

                match key.as_str() {
                    "servings" => metadata.servings = value.parse::<i64>().ok(),
                    "total time" => metadata.total_time = Some(value.to_string()),
                    "active time" => metadata.active_time = Some(value.to_string()),
                    "difficulty" => metadata.difficulty = Some(value.to_string()),
                    "tags" => {
                        metadata.tags = value
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect();
                    }
                    _ => {
                        metadata.other.insert(key, value.to_string());
                    }
                }
            }
        } else if !line.trim().is_empty() {
            // Add to instructions
            instructions.push_str(line);
            instructions.push('\n');

            // Extract ingredients (@name{quantity%unit})
            let mut chars = line.chars().peekable();
            while let Some(c) = chars.next() {
                if c == '@' {
                    let mut name = String::new();
                    while let Some(&ch) = chars.peek() {
                        if ch == '{' || ch.is_whitespace() {
                            break;
                        }
                        name.push(chars.next().unwrap());
                    }

                    if !name.is_empty()
                        && !ingredients
                            .iter()
                            .any(|i: &IngredientWithQuantity| i.name == name)
                    {
                        ingredients.push(IngredientWithQuantity {
                            name,
                            quantity: None,
                            unit: None,
                        });
                    }
                }
            }
        }
    }

    Ok(ParsedRecipe {
        metadata,
        ingredients,
        instructions: instructions.trim().to_string(),
        sections: vec![RecipeSection {
            name: None,
            instructions: instructions.trim().to_string(),
        }],
    })
}

/// Extract time in minutes from a time string (e.g., "45 minutes", "1 hour")
pub fn parse_time_to_minutes(time_str: &str) -> Option<i64> {
    let time_lower = time_str.to_lowercase();

    // Try to extract number
    let num: i64 = time_lower
        .chars()
        .filter(|c| c.is_numeric())
        .collect::<String>()
        .parse()
        .ok()?;

    // Check unit
    if time_lower.contains("hour") {
        Some(num * 60)
    } else if time_lower.contains("min") {
        Some(num)
    } else {
        // Assume minutes if no unit specified
        Some(num)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cooklang() {
        let content = r#">> servings: 4
>> total time: 30 minutes
>> tags: dinner, italian

Mix @flour{200%g} with @water{100%ml}.

Bake for ~{15%minutes}."#;

        let result = parse_cooklang(content);
        assert!(result.is_ok());

        let recipe = result.unwrap();
        assert_eq!(recipe.metadata.servings, Some(4));
        assert_eq!(recipe.metadata.total_time, Some("30 minutes".to_string()));
        assert_eq!(recipe.metadata.tags.len(), 2);
        assert!(recipe.metadata.tags.contains(&"dinner".to_string()));
        assert!(recipe.ingredients.len() >= 2);
    }

    #[test]
    fn test_parse_time_to_minutes() {
        assert_eq!(parse_time_to_minutes("45 minutes"), Some(45));
        assert_eq!(parse_time_to_minutes("1 hour"), Some(60));
        assert_eq!(parse_time_to_minutes("2 hours"), Some(120));
        assert_eq!(parse_time_to_minutes("30"), Some(30));
    }

    #[test]
    fn test_parse_sample_recipe() {
        let content = include_str!("../../tests/fixtures/sample_recipe.cook");
        let result = parse_cooklang(content);

        assert!(result.is_ok());

        let recipe = result.unwrap();
        assert_eq!(recipe.metadata.servings, Some(24));
        assert!(!recipe.ingredients.is_empty());
        assert!(recipe.instructions.contains("Preheat"));
    }
}
