# Federation Scripts

This directory contains utility scripts for managing the Cooklang Federation.

## find-cooklang-repos.py

Searches GitHub for repositories containing Cooklang recipe files (`.cook`), filters out anything that is not a genuine recipe collection, and adds the rest to the federation feed configuration.

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

# Comprehensive search: inspect everything and add the top 100 collections
python3 scripts/find-cooklang-repos.py --limit 100 --token YOUR_TOKEN

# Only accept substantial cookbooks
python3 scripts/find-cooklang-repos.py --limit 20 --min-recipes 10 --token YOUR_TOKEN

# Randomize selection to discover different repos on each run
python3 scripts/find-cooklang-repos.py --limit 20 --randomize --token YOUR_TOKEN

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
  - The script inspects far more candidates than this, since most are rejected

- `--min-recipes N` - Minimum real recipes a repo must contain (default: 3)

- `--max-pages N` - Maximum number of API pages to fetch per size bucket (default: 10)
  - Each page contains up to 100 results, and 10 pages is the API's hard cap per query
  - The search runs many buckets, so the total number of files scanned is far higher

- `--randomize` - Randomize repository selection instead of ranking by recipe count
  - Perfect for discovering diverse repositories across multiple runs
  - Each run will select different repositories from the pool
  - Combine with `--max-pages` to ensure a large pool to choose from

- `--dry-run` - Preview what would be added without modifying `feeds.yaml`

- `--added-by USERNAME` - GitHub username to credit for additions (default: @bot)

### How It Works

1. Searches the GitHub Code Search API for `.cook` files
2. Splits the search into file-size buckets so it can see past the API's
   1000-results-per-query cap (there are ~23,000 `.cook` files on GitHub)
3. Extracts unique repositories from the search results
4. Drops repositories already present in `feeds.yaml`
5. **Filters out everything that is not a real recipe collection** (see below)
6. Ranks survivors by recipe count, then stars (or shuffles, with `--randomize`)
7. Appends the top N to `config/feeds.yaml`

### Quality Filtering

A repository containing a `.cook` file is *not* necessarily a cookbook. Cooklang
parsers, editor plugins and the spec repo are full of `.cook` test fixtures, and
many repos carry a single sample file. Earlier versions of this script added all
of them. A candidate is now rejected unless it passes every check:

| Check | Rejects |
|---|---|
| Not a fork | Duplicate copies of other people's cookbooks |
| Not in the `cooklang` org | `cooklang/cookcli`, `cooklang/spec`, `cooklang/cooklang-rs`, ... |
| Description/topics free of tooling keywords | Parsers, LSPs, tree-sitter grammars, VSCode/Neovim/Obsidian plugins |
| Has `.cook` files outside `tests/`, `fixtures/`, `spec/`, `examples/`, ... | Parser test suites |
| At least `--min-recipes` real recipes (default 3) | Repos with one sample file |
| Sampled `.cook` files carry real Cooklang syntax (`@flour{200%g}`, `#pan{}`, `~{5%min}`) | Stub fixtures, and files that merely mention `@` |
| Sampled files are not `cook` build scripts | See below |

#### The `.cook` extension collision

`.cook` is **also** the extension used by [Peter Miller's `cook` build tool](https://sourceforge.net/projects/cook/),
whose build scripts live in the `etc/` directory of `srecord`, `aegis`,
`libexplain`, `fstrcmp`, the UCSD p-system tools and their many forks. A GitHub
search for `.cook` files returns all of them, and they have nothing to do with
food:

```
/*
 * srecord - manipulate eprom load files
 * Copyright (C) 1998-2000, 2003, 2004, 2006-2014 Peter Miller
 */
if [not [defined integration-build-targets]] then
    integration-build-targets = ;
```

Recipes are told apart by requiring genuine Cooklang syntax — an ingredient,
cookware or timer with braces — and rejecting anything carrying build-script
constructs (`if [`, `[fromto ...]`, `#include-cooked`, C comment blocks).

A download that fails is retried, and never counted as "not a recipe" — a flaky
network would otherwise reject good repositories at random.

Use `--dry-run` to see the accept/reject decision for every candidate before
anything is written.

### Example Output

```
Using config file: /path/to/config/feeds.yaml

Searching GitHub for .cook files (partitioned by file size)
  size:0..10000 -> 22936 files
    size:0..5000 -> 10712 files
      size:0..2500 -> 5168 files
      ...
    Page 1: 100 files, 45 unique repos so far
    Page 2: 100 files, 78 unique repos so far
Found 412 unique repositories with .cook files

336 candidate(s) not already in feeds.yaml
Checking which ones are real recipe collections...

  ✔️  user/awesome-recipes: 142 recipes, ⭐ 12
  ❌ someone/cooklang-vim: looks like tooling ('vim')
  ❌ someone/parser-tests: all .cook files are test fixtures
  ❌ someone/hello-world: only 1 recipe(s), need 3
  ✔️  chef/meal-prep: 61 recipes, ⭐ 3
  ...

48 recipe collection(s), 288 rejected

Adding 20 repositories...

  ✅ Added user/awesome-recipes (142 recipes, ⭐ 12)
  ✅ Added chef/meal-prep (61 recipes, ⭐ 3)
  ...

✨ Successfully added 20 new feed(s) to config/feeds.yaml
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
- Repositories are ranked by recipe count, then star count, unless `--randomize` is used
- New entries are **appended** to `feeds.yaml`; the file is never regenerated, so
  comments and the `disabled_at`/`disabled_by` fields on disabled feeds survive
- **Randomization tip**: Run the script multiple times with `--randomize` to build a diverse collection
  - Each run will select different repositories from the available pool
  - Combine with `--max-pages 25` to maximize the pool size
- The GitHub Code Search API:
  - **Requires authentication** (token is mandatory)
  - Rate limit: 30 requests per minute with authentication
  - The script adds a 2-second delay between requests and backs off on 403/429
  - A full search takes tens of minutes; the repo metadata and file-tree lookups
    use the separate 5000/hour core rate limit
- Each repository is added with:
  - Automatic branch detection (main/master)
  - Tags: `cookbook`, `github`
  - A readable title of the form `<Owner>'s <Collection>`, built from the owner's
    display name and the repository name. GitHub descriptions are *not* used as
    titles - most repos have none, and those that do often carry typos, URLs or
    words meaningless out of context. The description goes into `notes` instead.
  - Recipe count and star count in notes
  - Current date as `added_at`
