#!/usr/bin/env python3
"""
GitHub Cooklang Repository Finder

Searches GitHub for repositories containing Cooklang recipe files (.cook)
and adds them to the federation feed configuration.

Usage:
    python3 scripts/find-cooklang-repos.py [--token TOKEN] [--limit LIMIT] [--max-pages N] [--randomize] [--dry-run]

Options:
    --token TOKEN      GitHub personal access token (REQUIRED for Code Search API)
    --limit LIMIT      Maximum number of repositories to add (default: 10)
    --max-pages N      Maximum API pages to fetch, 100 results/page (default: 10)
    --randomize        Randomize selection instead of sorting by stars
    --dry-run          Show what would be added without modifying feeds.yaml
    --added-by USER    GitHub username to credit (default: @bot)

Note:
    The GitHub Code Search API requires authentication.
    Create a token at: https://github.com/settings/tokens
    Uses query: extension:cook (API syntax differs from web search)
"""

import argparse
import json
import random
import sys
import time
from datetime import date
from pathlib import Path
from typing import List, Dict, Optional
from urllib.request import Request, urlopen
from urllib.error import HTTPError, URLError
from urllib.parse import quote
import ssl
import yaml


class GitHubSearcher:
    """Handles GitHub API searches for Cooklang repositories."""

    def __init__(self, token: Optional[str] = None):
        self.token = token
        self.api_base = "https://api.github.com"

    def _make_request(self, url: str) -> Dict:
        """Make a request to GitHub API."""
        headers = {
            "Accept": "application/vnd.github+json",
            "X-GitHub-Api-Version": "2022-11-28"
        }
        if self.token:
            headers["Authorization"] = f"Bearer {self.token}"

        req = Request(url, headers=headers)

        # Create SSL context that handles certificate verification
        # For macOS users: if you get SSL errors, run:
        # /Applications/Python\ 3.*/Install\ Certificates.command
        context = ssl.create_default_context()

        try:
            with urlopen(req, context=context) as response:
                return json.loads(response.read().decode())
        except HTTPError as e:
            error_body = e.read().decode()
            print(f"GitHub API Error: {e.code} - {error_body}", file=sys.stderr)
            raise
        except URLError as e:
            if "CERTIFICATE_VERIFY_FAILED" in str(e):
                print("\n⚠️  SSL Certificate Error!", file=sys.stderr)
                print("On macOS, you may need to install certificates:", file=sys.stderr)
                print("  Run: /Applications/Python\\ 3.*/Install\\ Certificates.command", file=sys.stderr)
                print("\nAlternatively, install the 'requests' library for better SSL handling:", file=sys.stderr)
                print("  pip3 install requests", file=sys.stderr)
                print("\nRetrying with relaxed SSL verification...\n", file=sys.stderr)

                # Retry with unverified context as fallback
                context = ssl._create_unverified_context()
                with urlopen(req, context=context) as response:
                    return json.loads(response.read().decode())
            raise

    def search_repos_with_cook_files(self, max_repos: int = 100, max_pages: int = 10) -> List[Dict]:
        """
        Search GitHub for repositories containing .cook files.
        Uses the query: 'extension:cook'

        Args:
            max_repos: Maximum number of unique repositories to return
            max_pages: Maximum number of API pages to fetch (max 100 results per page)
        """
        # Search for .cook files using extension search
        # Note: GitHub Code Search API uses different syntax than web search
        query = "extension:cook"
        encoded_query = quote(query)

        print(f"Searching GitHub with query: {query}")

        repos = {}
        page = 1
        per_page = 100  # GitHub's maximum per page

        while page <= max_pages and len(repos) < max_repos:
            url = f"{self.api_base}/search/code?q={encoded_query}&per_page={per_page}&page={page}"

            try:
                results = self._make_request(url)
            except Exception as e:
                print(f"Error on page {page}: {e}", file=sys.stderr)
                break

            total_count = results.get("total_count", 0)
            items = results.get("items", [])

            if page == 1:
                print(f"GitHub found {total_count} total code files")
                print(f"Fetching up to {min(max_pages * per_page, total_count)} results across {max_pages} pages...")

            if not items:
                print(f"No more results after page {page - 1}")
                break

            # Extract unique repositories from code search results
            for item in items:
                repo = item.get("repository", {})
                repo_full_name = repo.get("full_name")
                if repo_full_name and repo_full_name not in repos:
                    repos[repo_full_name] = {
                        "url": repo.get("html_url"),
                        "full_name": repo_full_name,
                        "description": repo.get("description", ""),
                        "default_branch": repo.get("default_branch", "main"),
                        "stars": repo.get("stargazers_count", 0),
                        "language": repo.get("language", ""),
                    }

                    # Stop if we've reached the max repos
                    if len(repos) >= max_repos:
                        break

            print(f"  Page {page}: Found {len(items)} files, {len(repos)} unique repos so far")
            page += 1

            # Be nice to GitHub's rate limiting
            time.sleep(1)

        print(f"Found {len(repos)} unique repositories with .cook files")
        return list(repos.values())


class FeedManager:
    """Manages the feeds.yaml configuration file."""

    def __init__(self, config_path: Path):
        self.config_path = config_path
        self.config = self._load_config()
        self.header_lines = self._extract_header()
        self.footer_lines = self._extract_footer()

    def _load_config(self) -> Dict:
        """Load the feeds.yaml configuration."""
        with open(self.config_path, 'r') as f:
            return yaml.safe_load(f)

    def _extract_header(self) -> List[str]:
        """Extract header comments from the original file."""
        header = []
        with open(self.config_path, 'r') as f:
            for line in f:
                stripped = line.strip()
                if stripped.startswith('#') or not stripped:
                    header.append(line.rstrip())
                elif stripped.startswith('version:'):
                    break
                else:
                    break
        return header

    def _extract_footer(self) -> List[str]:
        """Extract validation section and other content after feeds."""
        footer = []
        in_validation = False
        with open(self.config_path, 'r') as f:
            for line in f:
                stripped = line.strip()
                if stripped.startswith('# Validation configuration') or in_validation:
                    footer.append(line.rstrip())
                    in_validation = True
                elif stripped.startswith('validation:'):
                    footer.append(line.rstrip())
                    in_validation = True
        return footer

    def _escape_yaml_string(self, s: str) -> str:
        """Escape quotes and special characters in YAML string."""
        if s is None:
            return ""
        # Escape double quotes
        return s.replace('"', '\\"')

    def save_config(self):
        """Save the configuration back to feeds.yaml with preserved formatting."""
        with open(self.config_path, 'w') as f:
            # Write header comments
            if self.header_lines:
                for line in self.header_lines:
                    f.write(f"{line}\n")
                f.write("\n")

            # Write version
            f.write(f"version: {self.config.get('version', 1)}\n\n")

            # Write feeds
            f.write("feeds:\n")
            feeds = self.config.get("feeds", [])
            for i, feed in enumerate(feeds):
                # Add comment for GitHub repository feed
                if i == 0:
                    f.write("  # GitHub repository feed\n")

                f.write(f'  - url: "{self._escape_yaml_string(feed.get("url"))}"\n')
                f.write(f'    title: "{self._escape_yaml_string(feed.get("title"))}"\n')
                f.write(f'    feed_type: {feed.get("feed_type")}\n')

                # Add branch if present
                branch = feed.get("branch")
                if branch:
                    f.write(f'    branch: "{branch}"\n')

                f.write(f'    enabled: {str(feed.get("enabled", True)).lower()}\n')
                f.write('    tags:\n')
                for tag in feed.get("tags", []):
                    f.write(f'      - {tag}\n')
                f.write(f'    notes: "{self._escape_yaml_string(feed.get("notes"))}"\n')
                f.write(f'    added_by: "{self._escape_yaml_string(feed.get("added_by"))}"\n')
                f.write(f'    added_at: "{self._escape_yaml_string(feed.get("added_at"))}"\n')

                # Add spacing between entries
                if i < len(feeds) - 1:
                    f.write("\n")

            # Write footer (validation section)
            if self.footer_lines:
                f.write("\n\n\n")
                for line in self.footer_lines:
                    f.write(f"{line}\n")

    def is_feed_exists(self, repo_url: str) -> bool:
        """Check if a feed with this URL already exists."""
        feeds = self.config.get("feeds", [])
        return any(feed.get("url") == repo_url for feed in feeds)

    def add_feed(self, repo: Dict, added_by: str = "@bot"):
        """Add a new feed to the configuration."""
        if self.is_feed_exists(repo["url"]):
            print(f"  ⏭️  Skipping {repo['full_name']} (already exists)")
            return False

        new_feed = {
            "url": repo["url"],
            "title": repo["description"] or f"{repo['full_name']} recipes",
            "feed_type": "github",
            "branch": repo["default_branch"],
            "enabled": True,
            "tags": ["cookbook", "github"],
            "notes": f"Found via GitHub search (⭐ {repo['stars']})",
            "added_by": added_by,
            "added_at": str(date.today()),
        }

        # Ensure feeds list exists
        if "feeds" not in self.config:
            self.config["feeds"] = []

        self.config["feeds"].append(new_feed)
        print(f"  ✅ Added {repo['full_name']} (⭐ {repo['stars']})")
        return True


def main():
    parser = argparse.ArgumentParser(
        description="Search GitHub for Cooklang repositories and add them to feeds.yaml"
    )
    parser.add_argument(
        "--token",
        help="GitHub personal access token (REQUIRED - Code Search API requires authentication)",
        default=None
    )
    parser.add_argument(
        "--limit",
        type=int,
        default=10,
        help="Maximum number of repositories to add to feeds.yaml (default: 10)"
    )
    parser.add_argument(
        "--max-pages",
        type=int,
        default=10,
        help="Maximum API pages to fetch (100 results/page, default: 10 = up to 1000 results)"
    )
    parser.add_argument(
        "--randomize",
        action="store_true",
        help="Randomize selection instead of sorting by stars (discover different repos on each run)"
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Show what would be added without modifying feeds.yaml"
    )
    parser.add_argument(
        "--added-by",
        default="@bot",
        help="GitHub username to credit for additions (default: @bot)"
    )

    args = parser.parse_args()

    # Check for token
    if not args.token:
        print("⚠️  WARNING: GitHub Code Search API requires authentication!", file=sys.stderr)
        print("Please provide a GitHub token with --token option.", file=sys.stderr)
        print("Create one at: https://github.com/settings/tokens", file=sys.stderr)
        print("The token only needs 'public_repo' scope.\n", file=sys.stderr)
        sys.exit(1)

    # Determine config path
    config_path = Path(__file__).parent.parent / "config" / "feeds.yaml"
    if not config_path.exists():
        print(f"Error: Config file not found at {config_path}", file=sys.stderr)
        sys.exit(1)

    print(f"Using config file: {config_path}")
    print()

    # Initialize components
    searcher = GitHubSearcher(token=args.token)
    feed_manager = FeedManager(config_path)

    # Search for repositories
    try:
        # Fetch more results than we need so we can prioritize by stars
        max_repos_to_fetch = args.limit * 5  # Fetch 5x more to have better selection
        repos = searcher.search_repos_with_cook_files(
            max_repos=max_repos_to_fetch,
            max_pages=args.max_pages
        )
    except Exception as e:
        print(f"Error searching GitHub: {e}", file=sys.stderr)
        sys.exit(1)

    if not repos:
        print("No repositories found.")
        return

    # Sort or randomize repos
    if args.randomize:
        print("Randomizing repository selection...")
        random.shuffle(repos)
        repos = repos[:args.limit]
        print()
        print(f"Processing {len(repos)} randomly selected repositories...")
    else:
        # Sort repos by stars (descending) to prioritize popular ones
        repos.sort(key=lambda r: r["stars"], reverse=True)
        repos = repos[:args.limit]
        print()
        print(f"Processing top {len(repos)} repositories (sorted by stars)...")
    print()

    # Add feeds
    added_count = 0
    for repo in repos:
        if args.dry_run:
            if not feed_manager.is_feed_exists(repo["url"]):
                print(f"  [DRY RUN] Would add {repo['full_name']} (⭐ {repo['stars']})")
                added_count += 1
            else:
                print(f"  [DRY RUN] Would skip {repo['full_name']} (already exists)")
        else:
            if feed_manager.add_feed(repo, added_by=args.added_by):
                added_count += 1

    # Save changes
    if not args.dry_run and added_count > 0:
        feed_manager.save_config()
        print()
        print(f"✨ Successfully added {added_count} new feed(s) to {config_path}")
    elif args.dry_run:
        print()
        print(f"[DRY RUN] Would add {added_count} new feed(s)")
    else:
        print()
        print("No new feeds to add.")


if __name__ == "__main__":
    main()
