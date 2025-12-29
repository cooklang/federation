//! Schema.org JSON-LD generation for recipes
//! Ported from CookCLI's cooklang_to_schema.rs

use serde_json::{json, Value};
use super::handlers::{RecipeData, IngredientData};
use crate::indexer::cooklang_parser::{RecipeSection, StepItem};

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

/// Convert RecipeData to Schema.org Recipe JSON-LD
pub fn recipe_to_schema_json(recipe: &RecipeData) -> Value {
    let mut schema = json!({
        "@context": "https://schema.org",
        "@type": "Recipe"
    });

    // Name: prefer metadata title, fallback to recipe title
    let name = recipe.metadata.as_ref()
        .and_then(|m| m.title.as_ref())
        .unwrap_or(&recipe.title);
    schema["name"] = json!(name);

    // Description: prefer metadata description, fallback to summary
    let description = recipe.metadata.as_ref()
        .and_then(|m| m.description.as_ref())
        .or(if recipe.summary.is_empty() { None } else { Some(&recipe.summary) });
    if let Some(desc) = description {
        schema["description"] = json!(desc);
    }

    // Image
    if !recipe.image_url.is_empty() {
        schema["image"] = json!(recipe.image_url);
    }

    // Author: prefer metadata author, fallback to feed author
    let author = recipe.metadata.as_ref()
        .and_then(|m| m.author.as_ref())
        .or(if recipe.feed.author.is_empty() { None } else { Some(&recipe.feed.author) });
    if let Some(author_name) = author {
        schema["author"] = json!({
            "@type": "Person",
            "name": author_name
        });
    }

    // URL (source)
    if !recipe.source_url.is_empty() {
        schema["url"] = json!(recipe.source_url);
    }

    // Keywords from tags
    if let Some(metadata) = &recipe.metadata {
        if !metadata.tags.is_empty() {
            schema["keywords"] = json!(metadata.tags.join(", "));
        }
    } else if !recipe.tags.is_empty() {
        schema["keywords"] = json!(recipe.tags.join(", "));
    }

    // Servings
    let servings = recipe.metadata.as_ref()
        .and_then(|m| m.servings.as_ref())
        .or(if recipe.servings.is_empty() { None } else { Some(&recipe.servings) });
    if let Some(s) = servings {
        schema["recipeYield"] = json!(format!("{} servings", s));
    }

    // Time fields
    add_time_fields(&mut schema, recipe);

    // Recipe category (course)
    if let Some(metadata) = &recipe.metadata {
        if let Some(course) = &metadata.course {
            schema["recipeCategory"] = json!(course);
        }
        if let Some(cuisine) = &metadata.cuisine {
            schema["recipeCuisine"] = json!(cuisine);
        }
    }

    // Ingredients
    let ingredients = create_ingredients_list(&recipe.ingredients);
    if !ingredients.is_empty() {
        schema["recipeIngredient"] = json!(ingredients);
    }

    // Instructions
    if let Some(sections) = &recipe.parsed_sections {
        let instructions = create_instructions_list(sections);
        if !instructions.is_empty() {
            schema["recipeInstructions"] = json!(instructions);
        }
    }

    schema
}

fn add_time_fields(schema: &mut Value, recipe: &RecipeData) {
    if let Some(metadata) = &recipe.metadata {
        // Prep time
        if let Some(prep) = &metadata.prep_time {
            if let Some(duration) = format_iso_duration(prep) {
                schema["prepTime"] = json!(duration);
            }
        }

        // Cook time
        if let Some(cook) = &metadata.cook_time {
            if let Some(duration) = format_iso_duration(cook) {
                schema["cookTime"] = json!(duration);
            }
        }

        // Total time from metadata
        if let Some(time) = &metadata.time {
            if let Some(duration) = format_iso_duration(time) {
                schema["totalTime"] = json!(duration);
            }
        }
    }

    // Fallback to database total_time_minutes
    if schema.get("totalTime").is_none() && !recipe.total_time_minutes.is_empty() {
        if let Ok(minutes) = recipe.total_time_minutes.parse::<i32>() {
            schema["totalTime"] = json!(format!("PT{}M", minutes));
        }
    }
}

fn create_ingredients_list(ingredients: &[IngredientData]) -> Vec<String> {
    ingredients
        .iter()
        .map(|i| {
            let mut text = String::new();
            if !i.quantity.is_empty() {
                text.push_str(&i.quantity);
                text.push(' ');
            }
            if !i.unit.is_empty() {
                text.push_str(&i.unit);
                text.push(' ');
            }
            text.push_str(&i.name);
            text.trim().to_string()
        })
        .collect()
}

fn create_instructions_list(sections: &[RecipeSection]) -> Vec<Value> {
    let mut instructions = Vec::new();
    let has_named_sections = sections.iter().any(|s| s.name.is_some());

    if has_named_sections {
        // Use HowToSection grouping
        for section in sections {
            let steps: Vec<Value> = section.steps.iter().enumerate().map(|(_i, step)| {
                let text = step_to_text(step);
                json!({
                    "@type": "HowToStep",
                    "text": text
                })
            }).collect();

            if !steps.is_empty() {
                let section_json = if let Some(name) = &section.name {
                    json!({
                        "@type": "HowToSection",
                        "name": name,
                        "itemListElement": steps
                    })
                } else {
                    // Anonymous section - just add steps directly
                    json!({
                        "@type": "HowToSection",
                        "itemListElement": steps
                    })
                };
                instructions.push(section_json);
            }
        }
    } else {
        // Flat HowToStep list (no sections or single unnamed section)
        let mut step_num = 0;
        for section in sections {
            for step in &section.steps {
                step_num += 1;
                let text = step_to_text(step);
                instructions.push(json!({
                    "@type": "HowToStep",
                    "name": format!("Step {}", step_num),
                    "text": text
                }));
            }
        }
    }

    instructions
}

fn step_to_text(step: &crate::indexer::cooklang_parser::StepData) -> String {
    let mut text = String::new();
    for item in &step.items {
        match item {
            StepItem::Text { value } => text.push_str(value),
            StepItem::Ingredient { name, .. } => text.push_str(name),
            StepItem::Cookware { name, .. } => text.push_str(name),
            StepItem::Timer { text: timer_text, .. } => text.push_str(timer_text),
            StepItem::Quantity { value } => text.push_str(value),
        }
    }
    text.trim().to_string()
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
