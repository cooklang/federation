# Federation Scripts

This directory contains utility scripts for managing the Cooklang Federation.

## find-cooklang-repos.py

Searches GitHub for repositories containing Cooklang recipe files (`.cook`) and adds them to the federation feed configuration.

### Prerequisites

```bash
# Install Python dependencies
pip3 install -r scripts/requirements.txt
```

**macOS SSL Certificate Issue:**
If you encounter SSL certificate errors on macOS, run this command to install certificates:
```bash
/Applications/Python\ 3.*/Install\ Certificates.command
```

Or use the full path for your Python version, e.g.:
```bash
/Applications/Python\ 3.11/Install\ Certificates.command
```

### Usage

```bash
# Basic usage (dry-run to see what would be added)
# NOTE: Replace YOUR_GITHUB_TOKEN with your actual token
python3 scripts/find-cooklang-repos.py --dry-run --token YOUR_GITHUB_TOKEN

# Add up to 20 repositories
python3 scripts/find-cooklang-repos.py --limit 20 --token YOUR_GITHUB_TOKEN

# Fetch more pages to find more repositories (100 results per page)
python3 scripts/find-cooklang-repos.py --limit 50 --max-pages 25 --token YOUR_GITHUB_TOKEN

# Specify who is adding the feeds
python3 scripts/find-cooklang-repos.py --added-by @yourusername --token YOUR_GITHUB_TOKEN

# Comprehensive search: fetch many results and add top 100 by stars
python3 scripts/find-cooklang-repos.py --limit 100 --max-pages 25 --token YOUR_TOKEN

# Randomize selection to discover different repos on each run
python3 scripts/find-cooklang-repos.py --limit 20 --max-pages 25 --randomize --token YOUR_TOKEN

# Multiple runs with randomization to build a diverse feed
python3 scripts/find-cooklang-repos.py --limit 10 --randomize --added-by @me --token YOUR_TOKEN
# Run again to get 10 different ones
python3 scripts/find-cooklang-repos.py --limit 10 --randomize --added-by @me --token YOUR_TOKEN
```

### Options

- `--token TOKEN` - **REQUIRED** - GitHub personal access token
  - The Code Search API requires authentication
  - Create a token at: https://github.com/settings/tokens
  - Only needs `public_repo` scope for public repositories
  - **The script will not work without a token**

- `--limit LIMIT` - Maximum number of repositories to add to feeds.yaml (default: 10)
  - The script fetches 5x this number to allow better selection by popularity

- `--max-pages N` - Maximum number of API pages to fetch (default: 10)
  - Each page contains up to 100 results
  - Default of 10 pages = up to 1000 code files searched
  - Increase this to search more comprehensively (e.g., 25 pages = 2500 files)

- `--randomize` - Randomize repository selection instead of sorting by stars
  - Perfect for discovering diverse repositories across multiple runs
  - Each run will select different repositories from the pool
  - Combine with `--max-pages` to ensure a large pool to choose from

- `--dry-run` - Preview what would be added without modifying `feeds.yaml`

- `--added-by USERNAME` - GitHub username to credit for additions (default: @bot)

### How It Works

1. Searches GitHub Code Search API using the query: `extension:cook`
2. Paginates through multiple pages (100 results per page) to gather comprehensive results
3. Extracts unique repositories from the search results
4. Either sorts by star count OR randomizes (if `--randomize` is used)
   - **Default**: Sorts by stars to prioritize popular repositories
   - **With --randomize**: Shuffles repositories for diverse selection across runs
5. Selects N repositories based on the `--limit` parameter
6. Checks for duplicates in the existing `feeds.yaml`
7. Adds new repositories to the feed configuration

The script fetches 5x more repositories than `--limit` to ensure a good selection pool, whether you're sorting by popularity or randomizing.

### Example Output

```
Using config file: /path/to/config/feeds.yaml

Searching GitHub with query: extension:cook
GitHub found 2500 total code files
Fetching up to 1000 results across 10 pages...
  Page 1: Found 100 files, 45 unique repos so far
  Page 2: Found 100 files, 78 unique repos so far
  Page 3: Found 100 files, 102 unique repos so far
  ...
  Page 10: Found 100 files, 248 unique repos so far
Found 248 unique repositories with .cook files

Processing top 20 repositories (sorted by stars)...

  ✅ Added user/awesome-recipes (⭐ 142)
  ✅ Added chef/meal-prep (⭐ 89)
  ⏭️  Skipping dubadub/cookbook (already exists)
  ✅ Added home-cook/dinner-ideas (⭐ 56)
  ...

✨ Successfully added 17 new feed(s) to config/feeds.yaml
```

### Search Query Details

The script uses the GitHub Code Search API with the query: `extension:cook`

This query:
- `extension:cook` - Finds all files with the `.cook` extension
- Works with GitHub's Code Search API (different from web search syntax)
- Returns files from public repositories

Note: The GitHub web search uses different syntax (like `path:*.cook @ NOT function`), but the API requires simpler queries like `extension:cook`.

### Notes

- The script automatically checks for duplicate entries before adding
- Repositories are sorted by star count (most popular first) unless `--randomize` is used
- **Randomization tip**: Run the script multiple times with `--randomize` to build a diverse collection
  - Each run will select different repositories from the available pool
  - Combine with `--max-pages 25` to maximize the pool size
- The GitHub Code Search API:
  - **Requires authentication** (token is mandatory)
  - Rate limit: 30 requests per minute with authentication
  - The script adds a 1-second delay between pages to be respectful
- Each repository is added with:
  - Automatic branch detection (main/master)
  - Tags: `cookbook`, `github`
  - Star count in notes
  - Current date as `added_at`
