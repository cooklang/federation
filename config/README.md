# Cooklang Federation - Feed Configuration

This directory contains the configuration for all feeds indexed by the Cooklang Federation.

## Overview

Feeds are managed via the `feeds.yaml` file using a GitOps workflow. This means:
- All feeds are version-controlled
- Changes require pull request review
- CI automatically validates feed configurations
- Full audit trail via Git history

## Feed Types

The federation supports two types of feeds:

### 1. RSS/Atom Feeds (`feed_type: web`)
Traditional RSS or Atom feeds containing Cooklang recipes.

**Example:**
```yaml
- url: "https://example.com/recipes/feed.xml"
  title: "Example Recipe Blog"
  feed_type: web
  enabled: true
  tags:
    - baking
    - desserts
  notes: "High-quality dessert recipes"
  added_by: "@username"
  added_at: "2025-10-13"
```

### 2. GitHub Repositories (`feed_type: github`)
GitHub repositories containing Cooklang recipe files (.cook).

**Example:**
```yaml
- url: "https://github.com/owner/cookbook"
  title: "Owner's Personal Cookbook"
  feed_type: github
  branch: "main"  # Optional: specify branch (defaults to "main")
  enabled: true
  tags:
    - cookbook
    - github
  notes: "Personal recipe collection"
  added_by: "@owner"
  added_at: "2025-10-13"
```

**Note:** If the repository uses a different default branch (e.g., `master`), specify it with the `branch` field.

## Adding a New Feed

### 1. Fork the Repository
Fork the Cooklang Federation repository to your GitHub account.

### 2. Edit `feeds.yaml`
Add your feed entry to the `feeds:` section:

```yaml
feeds:
  # ... existing feeds ...

  - url: "YOUR_FEED_URL_HERE"
    title: "Your Feed Title"
    feed_type: web  # or 'github' for GitHub repos
    enabled: true
    tags:
      - your-category
      - another-tag
    notes: "Brief description of your feed"
    added_by: "@yourusername"
    added_at: "2025-10-13"  # Use current date
```

### 3. Validate Locally (Optional)
If you have Rust installed, you can validate your changes locally:

```bash
cargo run --bin validate-feeds -- config/feeds.yaml
```

### 4. Submit Pull Request
1. Commit your changes:
   ```bash
   git add config/feeds.yaml
   git commit -m "Add feed: Your Feed Title"
   ```

2. Push to your fork:
   ```bash
   git push origin main
   ```

3. Open a pull request on GitHub

### 5. Wait for CI Validation
The CI will automatically:
- Validate YAML syntax
- Check for duplicate URLs
- Verify feed URL is accessible (for RSS/Atom feeds)
- Ensure GitHub repository exists (for GitHub feeds)
- Check for private IPs or localhost URLs
- Verify feed count limits

### 6. PR Review and Merge
A maintainer will review your PR and may request changes. Once approved and merged, your feed will be automatically indexed on the next deployment.

## Feed Configuration Fields

### Required Fields

| Field | Type | Description |
|-------|------|-------------|
| `url` | string | Feed URL (RSS/Atom feed URL or GitHub repo URL) |
| `title` | string | Human-readable feed title |
| `added_by` | string | GitHub username of contributor (e.g., `@username`) |
| `added_at` | string | Date added (YYYY-MM-DD format) |

### Optional Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `feed_type` | enum | `web` | Feed type: `web` (RSS/Atom) or `github` (GitHub repo) |
| `branch` | string | `"main"` | Branch to index (GitHub feeds only) |
| `enabled` | boolean | `true` | Whether feed is active |
| `tags` | array | `[]` | Categorization tags |
| `notes` | string | `null` | Additional context or description |

### Disabled Feed Fields

When disabling a feed, add these fields:

| Field | Type | Description |
|-------|------|-------------|
| `disabled_at` | string | Date disabled (YYYY-MM-DD) |
| `disabled_by` | string | Who disabled it (e.g., `@admin`) |
| `disabled_reason` | string | Why it was disabled (optional but recommended) |

**Example:**
```yaml
- url: "https://broken-site.example.com/feed.xml"
  title: "Broken Feed"
  feed_type: web
  enabled: false
  tags:
    - disabled
  notes: "Site appears to be down"
  added_by: "@originaluser"
  added_at: "2025-10-01"
  disabled_at: "2025-10-13"
  disabled_by: "@admin"
  disabled_reason: "Site returns 404 errors"
```

## Validation Rules

### URL Validation
- Must be valid HTTP/HTTPS URL
- Must have a valid host
- Cannot be localhost or private IP addresses
- GitHub feeds must be from github.com

### Deny Patterns
The following URL patterns are automatically rejected:
- `*localhost*` - Local development URLs
- `*127.0.0.1*` - Loopback address
- `*192.168.*` - Private IPv4 range
- `*10.0.*` - Private IPv4 range
- `*.local` - mDNS local domains

### Feed Limits
- Maximum feeds: 1000 (configurable in `validation.max_feeds`)

### Duplicate Detection
- No two feeds can have the same URL
- Case-sensitive URL comparison

## Tags

Tags help categorize and discover feeds. Recommended tags:

**By Type:**
- `cookbook` - Personal recipe collections
- `blog` - Recipe blogs
- `github` - GitHub repositories
- `professional` - Professional chef content

**By Cuisine:**
- `italian`, `french`, `chinese`, `mexican`, etc.

**By Category:**
- `baking`, `desserts`, `vegan`, `vegetarian`, `meat`, etc.

**By Difficulty:**
- `beginner`, `intermediate`, `advanced`

Feel free to create new tags as needed. Use lowercase, hyphenated names.

## Troubleshooting

### "Duplicate feed URL" Error
- Check if your feed URL already exists in `feeds.yaml`
- Note: URLs are case-sensitive

### "URL matches deny pattern" Error
- Your URL contains a private IP or localhost
- Feeds must be publicly accessible

### "GitHub feed must be from github.com" Error
- Only GitHub repositories are supported for `feed_type: github`
- For GitLab, Bitbucket, etc., use self-hosted RSS feeds

### "Invalid protocol" Error
- Only `https` and `http` protocols are allowed
- No `ftp`, `file`, or other protocols

### CI Validation Fails
- Check the CI logs in your pull request
- Most errors have clear messages explaining the issue
- Fix the issue and push new commits; CI will re-run

## Questions?

- Open an issue on GitHub
- Ask in discussions
- Tag a maintainer in your PR

## Configuration Version

Current config version: **1**

If the config format changes, the version will be bumped and migration guides provided.
