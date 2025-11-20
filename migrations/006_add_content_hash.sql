-- Add content hash column for deduplication
ALTER TABLE recipes ADD COLUMN content_hash TEXT;

-- Index for fast duplicate lookups
CREATE INDEX idx_recipes_content_hash ON recipes(content_hash);
