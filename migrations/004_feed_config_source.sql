-- Add source column to track feed origin

ALTER TABLE feeds ADD COLUMN source TEXT DEFAULT 'config'
    CHECK(source IN ('config', 'manual', 'disabled'));

-- Create index for efficient source queries
CREATE INDEX IF NOT EXISTS idx_feeds_source ON feeds(source);
