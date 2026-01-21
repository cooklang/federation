-- Add columns to support caching of recipe content fetches
-- content_etag: ETag header from recipe content fetch for conditional requests
-- content_last_modified: Last-Modified header from recipe content fetch
-- feed_entry_updated: The <updated> timestamp from the feed entry for comparison

ALTER TABLE recipes ADD COLUMN content_etag TEXT;
ALTER TABLE recipes ADD COLUMN content_last_modified TIMESTAMP;
ALTER TABLE recipes ADD COLUMN feed_entry_updated TIMESTAMP;

-- Index for efficient lookup when checking if entry has been updated
CREATE INDEX IF NOT EXISTS idx_recipes_feed_entry_updated ON recipes(feed_id, external_id, feed_entry_updated);
