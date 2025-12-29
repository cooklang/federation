# Recipe Page JSON-LD for SEO

## Overview

Add Schema.org Recipe JSON-LD structured data to recipe pages for better search engine indexing.

## Approach

- **Location:** Embedded `<script type="application/ld+json">` in recipe page HTML `<head>`
- **Conversion logic:** Port from CookCLI's `cooklang_to_schema.rs`
- **Fields:** Essential only (no nutrition)
- **Instructions format:** HowToSection grouping with fallback to flat HowToStep list

## JSON-LD Structure

```json
{
  "@context": "https://schema.org",
  "@type": "Recipe",
  "name": "Recipe Title",
  "description": "Recipe summary/description",
  "image": "https://example.com/image.jpg",
  "author": {"@type": "Person", "name": "Author Name"},
  "url": "https://source-url.com/recipe",
  "keywords": "tag1, tag2, tag3",
  "recipeYield": "4 servings",
  "prepTime": "PT30M",
  "cookTime": "PT45M",
  "totalTime": "PT75M",
  "recipeCategory": "Main Course",
  "recipeCuisine": "Italian",
  "recipeIngredient": ["1 cup flour", "2 tbsp sugar"],
  "recipeInstructions": [
    {
      "@type": "HowToSection",
      "name": "Section Name",
      "itemListElement": [
        {"@type": "HowToStep", "text": "Step text..."}
      ]
    }
  ]
}
```

## Field Mapping

| JSON-LD Field | Primary Source | Fallback |
|---------------|----------------|----------|
| name | metadata.title | recipe.title |
| description | metadata.description | recipe.summary |
| image | image_url | - |
| author | metadata.author | feed.author |
| url | source_url | - |
| keywords | tags (comma-joined) | - |
| recipeYield | servings | - |
| prepTime | metadata.prep_time | - |
| cookTime | metadata.cook_time | - |
| totalTime | metadata.time | total_time_minutes (DB) |
| recipeCategory | metadata.course | - |
| recipeCuisine | metadata.cuisine | - |
| recipeIngredient | ingredients | - |
| recipeInstructions | parsed_sections | - |

## Implementation

### New Module: `src/web/schema.rs`

Port from CookCLI's `cooklang_to_schema.rs` with these key functions:

- `recipe_to_schema_json(data: &RecipeData) -> serde_json::Value` - Main builder
- `create_ingredients_list()` - Format ingredients as "quantity unit name"
- `add_time_fields()` - Convert time strings to ISO 8601 duration (PT30M)
- `create_instructions_list()` - Build HowToSection/HowToStep structure

### Template Change: `src/web/templates/recipe.html`

Add in `<head>` section:

```html
{% if schema_json %}
<script type="application/ld+json">
{{ schema_json | safe }}
</script>
{% endif %}
```

### Handler Change: `src/web/handlers.rs`

In `recipe_detail()`:

1. Build `RecipeData` as currently done
2. Call `recipe_to_schema_json(&recipe_data)`
3. Serialize to string, pass to template as `schema_json`

## Time Conversion

Convert time strings to ISO 8601 duration format:

- "30 minutes" → "PT30M"
- "1 hour 15 minutes" → "PT1H15M"
- "45 min" → "PT45M"

Uses regex parsing, ported from CookCLI.

## Edge Cases

- Missing image → omit `image` field
- No sections → output flat HowToStep list (no HowToSection wrapper)
- Empty ingredients → omit `recipeIngredient` field
- No author → omit `author` field

Schema.org validators handle missing optional fields gracefully.
