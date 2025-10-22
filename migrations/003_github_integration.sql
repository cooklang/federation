-- GitHub integration tables

-- GitHub feeds - tracks GitHub repositories that are indexed
CREATE TABLE IF NOT EXISTS github_feeds (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    feed_id INTEGER NOT NULL REFERENCES feeds(id) ON DELETE CASCADE,
    repository_url TEXT NOT NULL UNIQUE,
    owner TEXT NOT NULL,
    repo_name TEXT NOT NULL,
    default_branch TEXT NOT NULL DEFAULT 'main',
    last_commit_sha TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- GitHub recipes - links recipes to their source files on GitHub
CREATE TABLE IF NOT EXISTS github_recipes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    recipe_id INTEGER NOT NULL REFERENCES recipes(id) ON DELETE CASCADE,
    github_feed_id INTEGER NOT NULL REFERENCES github_feeds(id) ON DELETE CASCADE,
    file_path TEXT NOT NULL,
    file_sha TEXT NOT NULL,
    raw_url TEXT NOT NULL,
    html_url TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(github_feed_id, file_path)
);

-- Indexes for efficient queries
CREATE INDEX IF NOT EXISTS idx_github_feeds_repo ON github_feeds(owner, repo_name);
CREATE INDEX IF NOT EXISTS idx_github_feeds_feed_id ON github_feeds(feed_id);
CREATE INDEX IF NOT EXISTS idx_github_recipes_feed ON github_recipes(github_feed_id);
CREATE INDEX IF NOT EXISTS idx_github_recipes_sha ON github_recipes(file_sha);
CREATE INDEX IF NOT EXISTS idx_github_recipes_recipe_id ON github_recipes(recipe_id);
