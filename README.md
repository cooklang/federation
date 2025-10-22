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

## API Endpoints

### Health & Status
- `GET /health` - Health check
- `GET /ready` - Readiness check
- `GET /api/stats` - System statistics

### Search
- `GET /api/search?q=<query>` - Search recipes with unified query syntax
  - Examples:
    - `/api/search?q=breakfast` - Basic search
    - `/api/search?q=tags:breakfast` - Field-specific search
    - `/api/search?q=pasta%20AND%20tags:italian` - Boolean search
    - `/api/search?q=total_time:[0%20TO%2030]` - Range search

### Recipes
- `GET /api/recipes/:id` - Get recipe details
- `GET /api/recipes/:id/download` - Download .cook file

### Feeds
- `GET /api/feeds` - List all feeds
- `POST /api/feeds` - Register a new feed
- `GET /api/feeds/:id` - Get feed details
- `DELETE /api/feeds/:id` - Remove a feed

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
