-- Performance optimization indices
-- Based on code review recommendations

-- Drop and recreate published_at index with DESC for better performance on recent recipes queries
DROP INDEX IF EXISTS idx_recipes_published_at;
CREATE INDEX idx_recipes_published_at ON recipes(published_at DESC);

-- Add index for common query pattern: recipes by feed and external_id
-- Note: UNIQUE constraint on (feed_id, external_id) already provides an index,
-- but we explicitly add it for clarity and to ensure optimal query performance
CREATE INDEX IF NOT EXISTS idx_recipes_feed_external ON recipes(feed_id, external_id);

-- Add index for indexed_at to track which recipes need reindexing
CREATE INDEX IF NOT EXISTS idx_recipes_indexed_at ON recipes(indexed_at);

-- Add index for recipe-tag junction table for faster lookups
CREATE INDEX IF NOT EXISTS idx_recipe_tags_recipe_id ON recipe_tags(recipe_id);
CREATE INDEX IF NOT EXISTS idx_recipe_tags_tag_id ON recipe_tags(tag_id);

-- Add index for recipe-ingredient junction table for faster lookups
CREATE INDEX IF NOT EXISTS idx_recipe_ingredients_recipe_id ON recipe_ingredients(recipe_id);
CREATE INDEX IF NOT EXISTS idx_recipe_ingredients_ingredient_id ON recipe_ingredients(ingredient_id);
