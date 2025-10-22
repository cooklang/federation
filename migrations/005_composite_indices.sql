-- Additional composite indices for query optimization
-- Based on N+1 query fixes and common query patterns

-- Composite index for feed queries (feed_id + published_at)
-- This optimizes queries like: SELECT * FROM recipes WHERE feed_id = ? ORDER BY published_at DESC
CREATE INDEX IF NOT EXISTS idx_recipes_feed_published ON recipes(feed_id, published_at DESC);

-- Composite index for recipe_tags junction table
-- This optimizes the batch loading query in get_tags_for_recipes()
-- that joins recipe_tags with tags: WHERE recipe_id IN (...)
CREATE INDEX IF NOT EXISTS idx_recipe_tags_composite ON recipe_tags(recipe_id, tag_id);
