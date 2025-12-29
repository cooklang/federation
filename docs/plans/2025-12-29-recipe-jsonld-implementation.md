# Recipe JSON-LD Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add Schema.org Recipe JSON-LD structured data to recipe pages for better search engine indexing.

**Architecture:** Create a new `src/web/schema.rs` module that converts `RecipeData` to Schema.org JSON-LD format. The handler passes the serialized JSON to the template, which embeds it in a `<script type="application/ld+json">` tag.

**Tech Stack:** Rust, serde_json, Askama templates

---

### Task 1: Create schema.rs module with time conversion helpers

**Files:**
- Create: `src/web/schema.rs`
- Modify: `src/web/mod.rs:4`

**Step 1: Create the schema.rs file with time conversion helpers**

Create `src/web/schema.rs`:

```rust
//! Schema.org JSON-LD generation for recipes
//! Ported from CookCLI's cooklang_to_schema.rs

use serde_json::{json, Value};

/// Extract first number from a string like "30 minutes" -> 30
fn extract_number(s: &str) -> Option<i32> {
    s.chars()
        .filter(|c| c.is_numeric())
        .collect::<String>()
        .parse::<i32>()
        .ok()
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
```

**Step 2: Add module to mod.rs**

Edit `src/web/mod.rs` to add the schema module:

```rust
// Phase 5: Web UI module
// This module provides the web interface using Askama templates

pub mod handlers;
pub mod schema;
```

**Step 3: Run tests to verify time conversion**

Run: `cargo test web::schema --no-fail-fast`

Expected: All tests pass

**Step 4: Commit**

```bash
git add src/web/schema.rs src/web/mod.rs
git commit -m "feat(schema): add time conversion helpers for JSON-LD"
```

---

### Task 2: Add recipe_to_schema_json function

**Files:**
- Modify: `src/web/schema.rs`

**Step 1: Add imports and the main conversion function**

Add to `src/web/schema.rs` after the existing code (before tests):

```rust
use super::handlers::{RecipeData, IngredientData, FeedData};
use crate::indexer::cooklang_parser::{RecipeSection, RecipeMetadata, StepItem};

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
            let steps: Vec<Value> = section.steps.iter().enumerate().map(|(i, step)| {
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
```

**Step 2: Run cargo check to verify compilation**

Run: `cargo check`

Expected: Compiles without errors

**Step 3: Commit**

```bash
git add src/web/schema.rs
git commit -m "feat(schema): add recipe_to_schema_json conversion function"
```

---

### Task 3: Update handlers.rs to generate JSON-LD

**Files:**
- Modify: `src/web/handlers.rs:166-170` (RecipeTemplate struct)
- Modify: `src/web/handlers.rs:250-280` (recipe_detail function)

**Step 1: Add schema_json field to RecipeTemplate**

Edit `src/web/handlers.rs` to modify the RecipeTemplate struct:

```rust
/// Recipe detail page template
#[derive(Template)]
#[template(path = "recipe.html")]
struct RecipeTemplate {
    recipe: RecipeData,
    schema_json: String,
}
```

**Step 2: Generate JSON-LD in recipe_detail handler**

Edit the end of the `recipe_detail` function (around line 278) to add schema generation:

Replace:
```rust
    let template = RecipeTemplate {
        recipe: recipe_data,
    };
```

With:
```rust
    // Generate Schema.org JSON-LD
    let schema = super::schema::recipe_to_schema_json(&recipe_data);
    let schema_json = serde_json::to_string_pretty(&schema)
        .unwrap_or_else(|_| "{}".to_string());

    let template = RecipeTemplate {
        recipe: recipe_data,
        schema_json,
    };
```

**Step 3: Run cargo check to verify compilation**

Run: `cargo check`

Expected: Compiles without errors

**Step 4: Commit**

```bash
git add src/web/handlers.rs
git commit -m "feat(schema): generate JSON-LD in recipe handler"
```

---

### Task 4: Update recipe.html template to embed JSON-LD

**Files:**
- Modify: `src/web/templates/recipe.html:1-5`

**Step 1: Add JSON-LD script block in the template**

Edit `src/web/templates/recipe.html` to add a new block for the head section. Add after line 1 (after `{% extends "base.html" %}`):

```html
{% extends "base.html" %}

{% block head_extra %}
{% if !schema_json.is_empty() %}
<script type="application/ld+json">
{{ schema_json|safe }}
</script>
{% endif %}
{% endblock %}

{% block title %}{% if let Some(metadata) = recipe.metadata %}{% if let Some(title) = metadata.title %}{{ title }}{% else %}{{ recipe.title }}{% endif %}{% else %}{{ recipe.title }}{% endif %} - Cooklang Federation{% endblock %}
```

**Step 2: Add head_extra block to base.html**

Edit `src/web/templates/base.html` to add the head_extra block. Insert before line 16 (before `</head>`):

```html
    {% block head_extra %}{% endblock %}
</head>
```

**Step 3: Run cargo check to verify template compilation**

Run: `cargo check`

Expected: Compiles without errors

**Step 4: Commit**

```bash
git add src/web/templates/recipe.html src/web/templates/base.html
git commit -m "feat(schema): embed JSON-LD in recipe page template"
```

---

### Task 5: Manual verification

**Step 1: Build and run the server**

Run: `cargo run`

**Step 2: View a recipe page source**

Open a recipe page in the browser, view page source (Ctrl+U / Cmd+U), and verify the JSON-LD script tag is present in the `<head>` section.

Expected: See `<script type="application/ld+json">` with valid Schema.org Recipe JSON

**Step 3: Validate with Google's Rich Results Test**

Go to: https://search.google.com/test/rich-results

Paste the recipe page URL and verify it detects the Recipe structured data.

**Step 4: Final commit with all changes**

```bash
git add -A
git commit -m "feat: add Schema.org JSON-LD to recipe pages for SEO

- Create src/web/schema.rs with JSON-LD generation logic
- Port time conversion helpers from CookCLI
- Generate JSON-LD in recipe_detail handler
- Embed in recipe.html template head
- Support HowToSection grouping for recipes with named sections"
```

---

## Summary

This plan adds Schema.org Recipe JSON-LD to recipe pages in 5 tasks:

1. **Task 1:** Create schema.rs with time conversion helpers
2. **Task 2:** Add main recipe_to_schema_json function
3. **Task 3:** Update handler to generate JSON-LD
4. **Task 4:** Update templates to embed JSON-LD
5. **Task 5:** Manual verification with Google Rich Results Test
