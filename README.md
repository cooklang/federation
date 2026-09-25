# Cooklang Federation

A federated search system for Cooklang recipes that allows decentralized publishing and centralized discovery through RSS/Atom feeds.

## Features

- 🔍 **Unified search** with powerful query syntax powered by Tantivy
- 📡 **RSS/Atom feed crawler** with automatic updates
- 🏷️ **Advanced filtering** by tags, ingredients, time, difficulty, and more
- 🌐 **Web UI** for browsing and searching recipes
- 💻 **CLI tools** for searching, downloading, and publishing recipes
- 🔄 **Background scheduler** for automated feed crawling
- 🛡️ **Rate limiting** to protect API endpoints from abuse
- 🐳 **Docker support** for easy deployment

## Quick Start

### Using Docker (Recommended)

```bash
# Clone the repository
git clone <repository-url>
cd federation

# Start the server
docker-compose up -d

# Access the web UI
open http://localhost:3000
```

### Local Development

#### Prerequisites

- Rust 1.75 or later
- SQLite 3

#### Setup

```bash
# Clone the repository
git clone <repository-url>
cd federation

# Copy environment variables
cp .env.example .env

# Download Tailwind CSS CLI
./scripts/download-tailwind.sh

# Start development server (with Tailwind watch mode)
./scripts/dev.sh
```

The server will be available at http://localhost:3001

#### Alternative: Run without Tailwind watch mode

```bash
# Build Tailwind CSS once
./tailwindcss -i ./styles/input.css -o ./src/web/static/css/output.css

# Run database migrations
cargo run -- migrate

# Start the server
cargo run -- serve
```

## Search Query Syntax

The federation search supports powerful query syntax powered by Tantivy's QueryParser:

### Basic Search

```bash
# Search all fields
breakfast

# Search specific field
tags:breakfast
title:pasta
ingredients:tomato
difficulty:easy
```

### Advanced Queries

```bash
# Boolean operators
pasta AND tags:italian
breakfast OR brunch

# Exclusion
chocolate -tags:dessert

# Range queries
total_time:[0 TO 30]      # 30 minutes or less
servings:[4 TO 8]          # Serves 4-8 people

# Complex combinations
chocolate tags:dessert difficulty:easy
pasta AND tags:italian AND total_time:[0 TO 30]
```

### Multi-word Values

Use quotes for multi-word field values:

```bash
tags:"quick breakfast"
title:"chocolate chip cookies"
```

### Available Fields

- `title` - Recipe title
- `summary` - Recipe description
- `instructions` - Cooking instructions
- `ingredients` - Ingredient list
- `tags` - Recipe tags
- `difficulty` - Difficulty level (easy, medium, hard)
- `servings` - Number of servings
- `total_time` - Total cooking time in minutes
- `file_path` - Source file path (for GitHub recipes)

## CLI Usage

### Search for recipes

```bash
# Basic search
cargo run -- search "chocolate cookies"

# Field-specific search
cargo run -- search "tags:breakfast"

# Complex query
cargo run -- search "pasta AND tags:italian AND total_time:[0 TO 30]"
```

### Download a recipe

```bash
cargo run -- download 123 --output ./recipes
```

### Publish your recipes

```bash
# Generate an Atom feed from .cook files
cargo run -- publish --input ./my-recipes --output feed.xml
```

### Backfill recipe locales

Detect and store the locale of recipes that don't have one yet (author-declared
`locale:` metadata wins, otherwise the language is detected from the recipe text).
Touched recipes are re-indexed automatically.

```bash
# Tag only recipes that don't have a locale yet
cargo run -- backfill-locales

# Recompute the locale for every recipe, including ones already tagged
cargo run -- backfill-locales --force
```

### Clean up recipes

Fix titles of GitHub recipes indexed by earlier versions (declared `title:`
metadata, else a readable version of the file name) and delete recipes whose
content duplicates an older one (the same file at two paths, a fork, a mirror
feed). The search index is updated in step. Safe to rerun.

```bash
cargo run -- cleanup
```

## API Endpoints

The API is read-only JSON. Errors are `{"error": "message"}` with a 4xx/5xx
status. `/api/*` is rate-limited per client IP (see `API_RATE_LIMIT`).

### Health & Status
- `GET /health` - Health check
- `GET /ready` - Readiness check
- `GET /api/stats` - Totals: `total_recipes`, `total_feeds`, `total_tags`, `total_ingredients`, `active_feeds`

### Search
- `GET /api/search` - Search recipes. Every term in `q` must match
  (`vegan tags:dessert` is vegan **and** dessert); use `OR` for alternatives and
  `-` to exclude. Words are stemmed, so `cake` finds "cakes".

  | Param | Example | Meaning |
  |---|---|---|
  | `q` | `q=pasta tags:italian` | Query string (field syntax as above) |
  | `locale` | `locale=de` | Language; `en` also matches `en-US` |
  | `tags` | `tags=vegan,dessert` | Has **all** tags (stemmed, case-insensitive) |
  | `include_ingredients` | `include_ingredients=garlic,lemon` | Uses **all** ingredients |
  | `exclude_ingredients` | `exclude_ingredients=peanut` | Uses **none** of them |
  | `max_time` | `max_time=30` | Total time ≤ minutes |
  | `min_servings`, `max_servings` | `min_servings=2&max_servings=6` | Inclusive range |
  | `difficulty` | `difficulty=easy` | Exact, case-insensitive |
  | `feed_id` | `feed_id=12` | Only this feed |
  | `sort` | `sort=newest` | `relevance` (default) or `newest` |
  | `page`, `limit` | `page=2&limit=20` | Paging (limit max 100) |

  Invalid numbers, an unknown `sort` or a malformed `q` return `400`. Each
  result card has `id`, `title`, `summary`, `tags`, `locale`,
  `total_time_minutes`, `servings`, `difficulty`, `image_url` and
  `feed: {id, title}`. Any field except `id`, `title` and `tags` may be null.
- `GET /api/facets?tag_limit=200` - Tag, language and difficulty values with
  recipe counts, for filter UIs. Cached for 5 minutes; `tag_limit` defaults to
  200, max 1000.

### Recipes
- `GET /api/recipes/:id` - Recipe details, including `locale` (e.g. `"de"`,
  `"en-US"`) and `locale_source` (`"declared"` if set via a Cooklang `locale:`
  key, or `"detected"` if inferred from the recipe text)
- `GET /api/recipes/:id/download` - Download .cook file

### Feeds
- `GET /api/feeds` - List feeds (`page`, `limit`, `status`)
- `GET /api/feeds/:id` - Get feed details

Feeds are registered through `config/feeds.yaml`, not the API.

## Configuration

Environment variables (see `.env.example`):

| Variable | Description | Default |
|----------|-------------|---------|
| `DATABASE_URL` | Database connection string | `sqlite:./data/federation.db` |
| `HOST` | Server host | `0.0.0.0` |
| `PORT` | Server port | `3000` |
| `EXTERNAL_URL` | External URL for CLI | `http://localhost:3000` |
| `API_RATE_LIMIT` | API requests per second | `100` |
| `CRAWLER_INTERVAL` | Seconds between feed updates | `3600` |
| `MAX_FEED_SIZE` | Maximum feed size in bytes | `5242880` (5MB) |
| `MAX_RECIPE_SIZE` | Maximum recipe size in bytes | `1048576` (1MB) |
| `RATE_LIMIT` | Crawler requests per second per domain | `1` |
| `INDEX_PATH` | Search index directory | `./data/index` |
| `RUST_LOG` | Logging level | `info,federation=debug` |

## Upgrading

### Recipe Hub API release (search index rebuild required)

This release adds `feed_id`, `indexed_at`, `image_url` and `feed_title` to the
Tantivy schema and makes `servings` and `total_time` indexed, for the new
structured search filters, `sort=newest` and richer result cards. As with
earlier schema changes, **the server refuses to start** against an index built
by a previous version.

Rebuild before starting `serve`:

```bash
rm -rf data/index   # or your configured INDEX_PATH
federation backfill-locales --force
```

With Docker Compose, stop the app first, then run the rebuild in a one-off container:

```bash
docker compose stop app
rm -rf data/index
docker compose run --rm app federation backfill-locales --force
docker compose up -d app
```

`backfill-locales --force` is the full rebuild: it re-indexes every recipe with
its tags, ingredients and feed title, fills missing servings, total time
and difficulty from the recipe's Cooklang metadata, and fills the ingredient
list of every recipe that has none stored from its Cooklang content (recipes
that already have ingredients keep them). (`federation reindex <url>` is a
different command: it deletes one feed's recipes from the database and
re-crawls that feed, and it does not rebuild the search index.) Recipes with
no stored content are not indexed by the rebuild and will not appear in
search results or facet counts until their content is fetched again.

Ship this whole branch as one release: production only needs one index
rebuild, not one per commit.

Other behaviour changes:

- **Feed recipes are indexed as they are crawled.** Before, recipes from
  RSS/Atom feeds (and their `<category>` tags) reached the search index only
  through `backfill-locales`. Existing feed recipes reach the index through
  the rebuild above; nothing further is needed for them.
- **Servings, total time and difficulty come from Cooklang metadata.**
  GitHub and feed recipes now store `servings`, `time` (or `prep time` +
  `cook time`) and `difficulty` from their `.cook` metadata, so the
  `max_time`, `min_servings`/`max_servings` and `difficulty` filters and the
  difficulty facet also match them. Before, those columns stayed empty for
  GitHub recipes, and for feed recipes unless the feed entry set them. The
  values live in the database, not only in the index, so existing recipes
  get them from the full rebuild above: `backfill-locales --force` fills
  empty columns from each recipe's stored content, writes them to the
  database and indexes them. No re-crawl is needed. A re-crawl would not
  help anyway, because the GitHub indexer skips files whose SHA has not
  changed.
- **Feed recipes store their ingredients.** Before, only GitHub recipes had
  an ingredient list, so the `include_ingredients`/`exclude_ingredients`
  filters silently ignored feed recipes (`exclude_ingredients=peanut` still
  returned feed recipes with peanuts). The crawler now stores ingredients
  from each `.cook` file the same way the GitHub indexer does, and keeps the
  stored list when an updated file fails to parse. Existing feed recipes get
  their ingredient lists from the full rebuild above: `backfill-locales
  --force` fills them from each recipe's stored content, writes them to the
  database and indexes them. No re-crawl is needed.
- **Rate limiting is per client and matches `API_RATE_LIMIT`.** It used to be
  one bucket for everyone that refilled one request every `API_RATE_LIMIT`
  seconds. Now each client gets `API_RATE_LIMIT` requests per second with
  bursts of twice that. If you run behind a reverse proxy, the proxy must
  connect to this server from a loopback or private address, and it must
  overwrite (not append to) the `X-Forwarded-For` header with the real client
  address — e.g. in nginx, `proxy_set_header X-Forwarded-For $remote_addr;`.
  Otherwise the first hop of `X-Forwarded-For` is trusted as the client
  address, and a client can set that header itself to pick its own
  rate-limit bucket.
- **A malformed `q` returns `400`** with the parser's message instead of `500`.
- The website search form has the same filters as the API.

### Search quality release (search index rebuild required)

The text analyzer changed (stemming, plain-text instructions), which changes
the Tantivy schema. Delete the index, rebuild it, then repair recipes indexed
by the previous GitHub indexer:

```bash
rm -rf data/index   # or your configured INDEX_PATH
federation backfill-locales --force
federation cleanup
```

`--force` re-indexes every recipe, not only those without a locale. `cleanup`
rewrites slug-style titles from the recipes' own metadata and removes
duplicate recipes, and keeps the index in step.

### Recipe locale field (search index rebuild required)

This release adds a `locale` field to the Tantivy search schema (used to tag
and filter recipes by language). Tantivy pins field ids to the schema stored
on disk, so a search index built before this change is incompatible — **the
server now refuses to start** against a mismatched index, with an error
telling you what to do.

Before deploying this version, delete the existing index and rebuild it:

```bash
rm -rf data/index   # or your configured INDEX_PATH
federation backfill-locales --force
```

`backfill-locales` runs pending database migrations and re-indexes every
recipe it touches; with `--force` that is every recipe, so this single command
rebuilds the search index and backfills locales in one step. Run it before
starting `serve` again.

## Production Build

To build the project for production:

```bash
# Build everything (CSS + Rust binary)
./scripts/build.sh
```

This will:
1. Build Tailwind CSS with minification
2. Build the Rust binary in release mode

The output will be:
- Binary: `./target/release/federation`
- Minified CSS: `./src/web/static/css/output.css`

To run in production:

```bash
# Set environment variables
export DATABASE_URL="sqlite:./data/federation.db"
export PORT=3000

# Run the server
./target/release/federation serve
```

## Development

### Running Tests

```bash
cargo test
```

### Linting

```bash
cargo clippy -- -D warnings
```

### Database Migrations

Migrations are located in the `migrations/` directory and are automatically applied on server startup.

## Architecture

- **Web Framework**: Axum with Tokio async runtime
- **Database**: SQLite (PostgreSQL compatible)
- **Search Engine**: Tantivy full-text search
- **Feed Parsing**: feed-rs for RSS/Atom
- **Recipe Parsing**: cooklang-rs
- **Templates**: Askama with Tailwind CSS
- **CLI**: Clap for command-line interface
- **Rate Limiting**: tower-governor for API protection

## Production Deployment

For production use, it's recommended to deploy this service behind a reverse proxy (nginx, Caddy, Traefik, etc.) that:

1. Terminates TLS/SSL
2. Sets proper `X-Forwarded-For` headers for accurate rate limiting
3. Provides additional DDoS protection
4. Handles load balancing if running multiple instances

Rate limiting works best when proper IP information is available via reverse proxy headers.

## Publishing Your Recipes

To make your recipes discoverable:

1. Create `.cook` files in a directory
2. Generate an Atom feed:
   ```bash
   cargo run -- publish --input ./recipes --output feed.xml
   ```
3. Host the feed and .cook files at a public URL
4. Add your feed to a federation server:
   ```bash
   curl -X POST http://localhost:3000/api/feeds \
     -H "Content-Type: application/json" \
     -d '{"url": "https://your-site.com/feed.xml"}'
   ```

## License

See LICENSE file for details.

## Contributing

Contributions are welcome! Please see CONTRIBUTING.md for guidelines.
