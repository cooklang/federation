// Cooklang parser integration for recipe parsing and structured data extraction
use anyhow::{Context, Result};
use cooklang::{Content, Converter, CooklangParser, Extensions, Item};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

/// Parsed recipe structure for JSON storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedRecipeData {
    pub sections: Vec<RecipeSection>,
    pub ingredients: Vec<IngredientData>,
    pub cookware: Vec<CookwareData>,
    pub timers: Vec<TimerData>,
    pub metadata: Option<RecipeMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipeMetadata {
    pub tags: Vec<String>,
    pub description: Option<String>,
    pub servings: Option<String>,
    pub time: Option<String>,
    pub difficulty: Option<String>,
    pub course: Option<String>,
    pub prep_time: Option<String>,
    pub cook_time: Option<String>,
    pub cuisine: Option<String>,
    pub diet: Option<String>,
    pub author: Option<String>,
    pub custom: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipeSection {
    pub name: Option<String>,
    pub steps: Vec<StepData>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepData {
    pub items: Vec<StepItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StepItem {
    Text { value: String },
    Ingredient { index: usize, name: String },
    Cookware { index: usize, name: String },
    Timer { index: usize, text: String },
    Quantity { value: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngredientData {
    pub name: String,
    pub quantity: Option<String>,
    pub quantity_value: Option<f64>, // Raw numeric value for database
    pub unit: Option<String>,
    pub is_reference: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CookwareData {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimerData {
    pub quantity: Option<String>,
    pub unit: Option<String>,
}

/// Format a cooklang quantity value to string
fn format_quantity(value: &cooklang::Value) -> Option<String> {
    match value {
        cooklang::Value::Number(num) => {
            let val = num.value();
            // Handle common fractions
            let frac_val = (val * 4.0).round() / 4.0; // Round to nearest 1/4
            let diff = (val - frac_val).abs();

            if diff < 0.01 {
                let whole = frac_val.floor() as i32;
                let frac = frac_val - whole as f64;

                if frac < 0.01 {
                    return Some(whole.to_string());
                }

                let frac_str = if (frac - 0.25).abs() < 0.01 {
                    "¼"
                } else if (frac - 0.5).abs() < 0.01 {
                    "½"
                } else if (frac - 0.75).abs() < 0.01 {
                    "¾"
                } else {
                    return Some(format!("{val:.2}"));
                };

                if whole > 0 {
                    Some(format!("{whole} {frac_str}"))
                } else {
                    Some(frac_str.to_string())
                }
            } else {
                Some(
                    format!("{val:.2}")
                        .trim_end_matches('0')
                        .trim_end_matches('.')
                        .to_string(),
                )
            }
        }
        cooklang::Value::Range { start, end } => {
            let start_val = cooklang::Value::Number(*start);
            let end_val = cooklang::Value::Number(*end);
            let start_str = format_quantity(&start_val)?;
            let end_str = format_quantity(&end_val)?;
            Some(format!("{start_str}-{end_str}"))
        }
        cooklang::Value::Text(t) => Some(t.to_string()),
    }
}

/// Parse a Cooklang recipe string into structured data
pub fn parse_recipe(content: &str) -> Result<ParsedRecipeData> {
    debug!("Parsing Cooklang recipe with cooklang library");

    // Create parser with basic extensions
    let parser = CooklangParser::new(Extensions::empty(), Converter::default());

    // Parse the recipe
    let parsed = parser.parse(content);

    // Log warnings if any
    if parsed.report().has_warnings() {
        for warning in parsed.report().warnings() {
            warn!("Recipe parsing warning: {}", warning);
        }
    }

    // Extract result
    let (recipe, _warnings) = parsed
        .into_result()
        .context("Failed to parse recipe with cooklang parser")?;

    // Extract metadata
    let meta = &recipe.metadata;
    let tags: Vec<String> = meta
        .tags()
        .map(|tags_vec| tags_vec.iter().map(|t| t.to_string()).collect())
        .unwrap_or_default();

    let mut custom = Vec::new();
    for (key, value) in &meta.map {
        // Skip standard metadata fields
        let key_str = key.as_str();
        if !matches!(
            key_str,
            Some("tags")
                | Some("description")
                | Some("servings")
                | Some("time")
                | Some("difficulty")
                | Some("course")
                | Some("prep time")
                | Some("cook time")
                | Some("cuisine")
                | Some("diet")
                | Some("author")
        ) {
            if let (Some(k), Some(v)) = (key.as_str(), value.as_str()) {
                custom.push((k.to_string(), v.to_string()));
            }
        }
    }

    let metadata = if tags.is_empty() && meta.map.is_empty() {
        None
    } else {
        // Extract time information - handle both "time" field and composed prep/cook times
        let (time_str, prep_str, cook_str) =
            if let Some(recipe_time) = meta.time(&Converter::default()) {
                match recipe_time {
                    cooklang::metadata::RecipeTime::Total(minutes) => {
                        (Some(format!("{minutes} minutes")), None, None)
                    }
                    cooklang::metadata::RecipeTime::Composed {
                        prep_time,
                        cook_time,
                    } => {
                        let prep = prep_time.map(|m| format!("{m} minutes"));
                        let cook = cook_time.map(|m| format!("{m} minutes"));
                        (None, prep, cook)
                    }
                }
            } else {
                // Fallback to raw metadata fields
                let prep = meta
                    .map
                    .get("prep time")
                    .or_else(|| meta.map.get("prep_time"))
                    .and_then(|p| p.as_str())
                    .map(|p| p.to_string());
                let cook = meta
                    .map
                    .get("cook time")
                    .or_else(|| meta.map.get("cook_time"))
                    .and_then(|c| c.as_str())
                    .map(|c| c.to_string());
                (None, prep, cook)
            };

        Some(RecipeMetadata {
            tags,
            description: meta.description().map(|d| d.to_string()),
            servings: meta.servings().map(|s| format!("{s}")),
            time: time_str,
            difficulty: meta
                .map
                .get("difficulty")
                .and_then(|d| d.as_str())
                .map(|d| d.to_string()),
            course: meta
                .map
                .get("course")
                .and_then(|c| c.as_str())
                .map(|c| c.to_string()),
            prep_time: prep_str,
            cook_time: cook_str,
            cuisine: meta
                .map
                .get("cuisine")
                .and_then(|c| c.as_str())
                .map(|c| c.to_string()),
            diet: meta
                .map
                .get("diet")
                .and_then(|d| d.as_str())
                .map(|d| d.to_string()),
            author: meta.author().and_then(|a| a.name().map(|n| n.to_string())),
            custom,
        })
    };

    // Extract ingredients
    let mut ingredients = Vec::new();
    for ingredient in &recipe.ingredients {
        let quantity_value = ingredient.quantity.as_ref().and_then(|q| {
            match q.value() {
                cooklang::Value::Number(num) => Some(num.value()),
                cooklang::Value::Range { start, .. } => Some(start.value()), // Use start of range
                _ => None,
            }
        });

        ingredients.push(IngredientData {
            name: ingredient.name.to_string(),
            quantity: ingredient
                .quantity
                .as_ref()
                .and_then(|q| format_quantity(q.value())),
            quantity_value,
            unit: ingredient
                .quantity
                .as_ref()
                .and_then(|q| q.unit().map(|u| u.to_string())),
            is_reference: ingredient.reference.is_some(),
        });
    }

    // Extract cookware
    let mut cookware = Vec::new();
    for item in &recipe.cookware {
        cookware.push(CookwareData {
            name: item.name.to_string(),
        });
    }

    // Extract timers
    let mut timers = Vec::new();
    for timer in &recipe.timers {
        timers.push(TimerData {
            quantity: timer
                .quantity
                .as_ref()
                .and_then(|q| format_quantity(q.value())),
            unit: timer
                .quantity
                .as_ref()
                .and_then(|q| q.unit().map(|u| u.to_string())),
        });
    }

    // Extract sections and steps
    let mut sections = Vec::new();
    for section in &recipe.sections {
        let mut section_steps = Vec::new();
        let mut section_notes = Vec::new();

        for content in &section.content {
            match content {
                Content::Step(step) => {
                    let mut step_items = Vec::new();

                    for item in &step.items {
                        match item {
                            Item::Text { value } => {
                                step_items.push(StepItem::Text {
                                    value: value.to_string(),
                                });
                            }
                            Item::Ingredient { index } => {
                                if let Some(ing) = recipe.ingredients.get(*index) {
                                    step_items.push(StepItem::Ingredient {
                                        index: *index,
                                        name: ing.name.to_string(),
                                    });
                                }
                            }
                            Item::Cookware { index } => {
                                if let Some(cw) = recipe.cookware.get(*index) {
                                    step_items.push(StepItem::Cookware {
                                        index: *index,
                                        name: cw.name.to_string(),
                                    });
                                }
                            }
                            Item::Timer { index } => {
                                if let Some(timer) = recipe.timers.get(*index) {
                                    let mut timer_text = String::new();

                                    if let Some(quantity) = &timer.quantity {
                                        if let Some(formatted) = format_quantity(quantity.value()) {
                                            timer_text.push_str(&formatted);
                                        }
                                        if let Some(unit) = quantity.unit() {
                                            if !timer_text.is_empty() {
                                                timer_text.push(' ');
                                            }
                                            timer_text.push_str(unit);
                                        }
                                    }

                                    if timer_text.is_empty() {
                                        timer_text = "timer".to_string();
                                    }

                                    step_items.push(StepItem::Timer {
                                        index: *index,
                                        text: timer_text,
                                    });
                                }
                            }
                            Item::InlineQuantity { index } => {
                                if let Some(q) = recipe.inline_quantities.get(*index) {
                                    let mut qty = format_quantity(q.value()).unwrap_or_default();
                                    if let Some(unit) = q.unit() {
                                        if !qty.is_empty() {
                                            qty.push_str(&format!(" {unit}"));
                                        } else {
                                            qty = unit.to_string();
                                        }
                                    }
                                    step_items.push(StepItem::Quantity { value: qty });
                                }
                            }
                        }
                    }

                    section_steps.push(StepData { items: step_items });
                }
                Content::Text(text) => {
                    // These are notes (lines starting with --)
                    let text = text.trim();
                    if text != "-" && !text.is_empty() {
                        section_notes.push(text.to_string());
                    }
                }
            }
        }

        // Only add section if it has steps or notes
        if !section_steps.is_empty() || !section_notes.is_empty() {
            sections.push(RecipeSection {
                name: section.name.clone(),
                steps: section_steps,
                notes: section_notes,
            });
        }
    }

    Ok(ParsedRecipeData {
        sections,
        ingredients,
        cookware,
        timers,
        metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_recipe() {
        let content = r#"
>> servings: 2

Mix @flour{2%cups} with @water{1%cup}.
Heat in #oven{} for ~{20%minutes}.
"#;

        let result = parse_recipe(content);
        assert!(result.is_ok());

        let parsed = result.unwrap();
        assert_eq!(parsed.ingredients.len(), 2);
        assert_eq!(parsed.cookware.len(), 1);
        assert_eq!(parsed.timers.len(), 1);
        assert!(!parsed.sections.is_empty());
    }

    #[test]
    fn test_format_quantity_fractions() {
        let half = cooklang::Value::Number(cooklang::quantity::Number::Regular(0.5));
        assert_eq!(format_quantity(&half), Some("½".to_string()));

        let one_and_half = cooklang::Value::Number(cooklang::quantity::Number::Regular(1.5));
        assert_eq!(format_quantity(&one_and_half), Some("1 ½".to_string()));
    }
}
