#!/usr/bin/env python3
"""
GitHub Cooklang Repository Finder

Searches GitHub for repositories containing Cooklang recipe files (.cook),
filters out anything that is not an actual recipe collection, and appends the
survivors to the federation feed configuration.

Usage:
    python3 scripts/find-cooklang-repos.py [--token TOKEN] [--limit LIMIT] [--max-pages N] [--dry-run]

Options:
    --token TOKEN      GitHub personal access token (REQUIRED for Code Search API)
    --limit LIMIT      Maximum number of repositories to add (default: 10)
    --max-pages N      Maximum API pages per size bucket, 100 results/page (default: 10)
    --min-recipes N    Minimum real recipes a repo must have (default: 3)
    --randomize        Randomize selection instead of ranking by recipe count
    --dry-run          Show what would be added without modifying feeds.yaml
    --added-by USER    GitHub username to credit (default: @bot)

Note:
    The GitHub Code Search API requires authentication.
    Create a token at: https://github.com/settings/tokens
    Uses query: extension:cook (API syntax differs from web search)
"""

import argparse
import json
import os
import random
import re
import ssl
import sys
import time
from datetime import date
from pathlib import Path
from typing import List, Dict, Optional, Tuple
from urllib.error import HTTPError, URLError
from urllib.parse import quote
from urllib.request import Request, urlopen

import yaml


# --- Quality filtering -------------------------------------------------------
#
# The code search only tells us a repo contains a .cook file. That is a weak
# signal: Cooklang parsers, editor plugins and spec repos are full of .cook test
# fixtures, and plenty of repos carry a single sample file. Those are not
# cookbooks and should never reach the feed, so every candidate is checked
# against the rules below before it is added.

# Directories where .cook files are fixtures rather than recipes someone cooks.
TEST_DIR_RE = re.compile(
    r'(^|/)(tests?|testing|test[-_]?data|test[-_]files?|fixtures?|__tests__|'
    r'spec|specs|canonical|examples?|samples?|demos?|benches?|benchmarks?)(/|$)', re.I)

# Descriptions/topics that mark a repo as Cooklang tooling rather than a cookbook.
TOOLING_RE = re.compile(
    r'\b(parser|parsing|lexer|grammar|tree-sitter|syntax highlight\w*|language server|'
    r'\blsp\b|linter|formatter|bindings?|\bsdk\b|library for|npm package|crate for|'
    r'plugin for|extension for|vscode|vs code|neovim|\bnvim\b|emacs|sublime text|'
    r'obsidian plugin|home assistant|\bcli tool\b|command.?line tool|implementation of '
    r'(the )?cooklang|cooklang spec|specification)\b', re.I)

# Source files, used to tell a cookbook from a codebase that ships sample recipes.
CODE_EXTENSIONS = {
    'rs', 'py', 'ts', 'tsx', 'js', 'jsx', 'go', 'java', 'swift', 'kt', 'kts',
    'c', 'cpp', 'h', 'hpp', 'rb', 'php', 'cs', 'scala', 'ex', 'exs', 'lua',
    'vim', 'el', 'dart', 'zig',
}


# Cooklang marks ingredients, cookware and timers with braces: @flour{200%g},
# #saucepan{}, ~{5%minutes}. Nothing else that ships as ".cook" looks like this.
COOKLANG_RE = re.compile(
    r'[@#][^\s@#~{}][^\n@#~{}]*\{[^}\n]*\}|~[^\s{}]*\{[^}\n]*\}')

# ".cook" is ALSO the extension of Peter Miller's `cook` build tool. Its files
# appear in srecord, aegis, libexplain, the UCSD p-system tools and their many
# forks, none of which have anything to do with food.
BUILDFILE_RE = re.compile(
    r'(^\s*/\*)|(\bif\s*\[)|(\[(fromto|collect|match_mask|defined|addsuffix|'
    r'stringset|filter|resolve|dirname|basename)\b)|(#include-cooked)|'
    r'(^\s*cascade\s)|(^\s*set\s+\w+\s*;)', re.M)


def looks_like_recipe(text: str) -> bool:
    """Whether a .cook file is a Cooklang recipe rather than a build file."""
    if not text.strip():
        return False
    body = re.sub(r'^---\n.*?\n---\n', '', text, flags=re.S)  # strip frontmatter
    if BUILDFILE_RE.search(body):
        return False
    return bool(COOKLANG_RE.search(body)) and len(body.strip()) >= 80


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
            if e.code in (403, 429) and "rate limit" in error_body.lower():
                print("  Rate limited by GitHub, waiting 30s...", file=sys.stderr)
                time.sleep(30)
                return self._make_request(url)
            print(f"GitHub API Error: {e.code} - {error_body}", file=sys.stderr)
            raise
        except URLError as e:
            if "CERTIFICATE_VERIFY_FAILED" in str(e):
                print("\n⚠️  SSL Certificate Error!", file=sys.stderr)
                print("On macOS, you may need to install certificates:", file=sys.stderr)
                print("  Run: /Applications/Python\\ 3.*/Install\\ Certificates.command", file=sys.stderr)
                print("\nRetrying with relaxed SSL verification...\n", file=sys.stderr)

                # Retry with unverified context as fallback
                context = ssl._create_unverified_context()
                with urlopen(req, context=context) as response:
                    return json.loads(response.read().decode())
            raise

    def _count(self, query: str) -> int:
        """How many code results a query has, without fetching them."""
        url = f"{self.api_base}/search/code?q={quote(query)}&per_page=1"
        try:
            result = self._make_request(url)
        except Exception:
            return 0
        time.sleep(2)
        return result.get("total_count", 0)

    def _harvest(self, query: str, repos: Dict, max_repos: int, max_pages: int):
        """Page through one query, collecting the repositories behind the hits."""
        page = 1
        while page <= max_pages and len(repos) < max_repos:
            url = (f"{self.api_base}/search/code?q={quote(query)}"
                   f"&per_page=100&page={page}")
            try:
                results = self._make_request(url)
            except Exception as e:
                print(f"    Error on page {page}: {e}", file=sys.stderr)
                return
            time.sleep(2)

            items = results.get("items", [])
            if not items:
                return

            for item in items:
                repo = item.get("repository", {})
                full_name = repo.get("full_name")
                if full_name and full_name not in repos:
                    # Code search returns only a stub repository object; the real
                    # metadata is fetched later in enrich().
                    repos[full_name] = {
                        "full_name": full_name,
                        "url": repo.get("html_url"),
                    }
                    if len(repos) >= max_repos:
                        break

            print(f"    Page {page}: {len(items)} files, {len(repos)} unique repos so far")
            if len(items) < 100:
                return
            page += 1

    def search_repos_with_cook_files(self, max_repos: int = 100,
                                     max_pages: int = 10) -> List[Dict]:
        """
        Search GitHub for repositories containing .cook files.

        Code search returns at most 1000 results per query, far fewer than the
        number of .cook files on GitHub, so the search is split into file-size
        buckets and each bucket is narrowed until it fits under that cap.
        """
        repos: Dict[str, Dict] = {}

        def split(lo: int, hi: Optional[int], depth: int = 0):
            if len(repos) >= max_repos:
                return
            rng = f"{lo}..{hi}" if hi is not None else f">{lo}"
            query = f"extension:cook size:{rng}"
            total = self._count(query)
            print(f"  {'  ' * depth}size:{rng} -> {total} files")
            if total == 0:
                return
            # Under the cap (or as narrow as it is worth splitting): fetch it.
            if total <= 1000 or hi is None or hi - lo <= 1 or depth >= 8:
                self._harvest(query, repos, max_repos, max_pages)
                return
            mid = (lo + hi) // 2
            split(lo, mid, depth + 1)
            split(mid + 1, hi, depth + 1)

        print("Searching GitHub for .cook files (partitioned by file size)")
        split(0, 10000)
        split(10000, None)

        print(f"Found {len(repos)} unique repositories with .cook files")
        return list(repos.values())

    def fetch_raw_file(self, full_name: str, branch: str,
                       path: str) -> Tuple[bool, str]:
        """
        Fetch a file from raw.githubusercontent.com.

        Returns (downloaded, text). The flag matters: a flaky download must not
        be read as "this is not a recipe", which would reject good repos at
        random.
        """
        url = ("https://raw.githubusercontent.com/"
               f"{full_name}/{quote(branch)}/{quote(path)}")
        for attempt in range(3):
            try:
                req = Request(url, headers={"User-Agent": "cooklang-federation"})
                with urlopen(req, timeout=30) as response:
                    return True, response.read(20000).decode("utf-8", "replace")
            except HTTPError as e:
                if e.code == 404:
                    return False, ""      # genuinely absent, not transient
            except Exception:
                pass
            time.sleep(1 + attempt * 2)
        return False, ""

    def enrich(self, repo: Dict) -> Optional[Dict]:
        """
        Fill in the metadata code search does not return, and inspect the file
        tree. Returns None if the repository is no longer reachable.
        """
        full_name = repo["full_name"]
        try:
            meta = self._make_request(f"{self.api_base}/repos/{full_name}")
        except Exception:
            return None

        branch = meta.get("default_branch") or "main"
        owner = full_name.split("/")[0]
        try:
            owner_meta = self._make_request(f"{self.api_base}/users/{owner}")
        except Exception:
            owner_meta = {}
        repo.update({
            "owner_name": (owner_meta.get("name") or "").strip() or owner,
            "url": meta.get("html_url") or repo.get("url"),
            "description": (meta.get("description") or "").strip(),
            "stars": meta.get("stargazers_count", 0),
            "default_branch": branch,
            "archived": meta.get("archived", False),
            "fork": meta.get("fork", False),
            "topics": meta.get("topics", []),
        })

        try:
            tree = self._make_request(
                f"{self.api_base}/repos/{full_name}/git/trees/{quote(branch)}?recursive=1")
        except Exception:
            return None
        if "tree" not in tree:
            return None

        cook_files, code_files = [], 0
        for node in tree["tree"]:
            if node.get("type") != "blob":
                continue
            path = node["path"]
            if path.lower().endswith(".cook"):
                cook_files.append(path)
            elif path.rsplit(".", 1)[-1].lower() in CODE_EXTENSIONS and "." in path:
                code_files += 1

        repo["cook_files"] = cook_files
        repo["recipes"] = [p for p in cook_files
                           if not TEST_DIR_RE.search(os.path.dirname(p))]
        repo["code_files"] = code_files
        return repo


def is_recipe_collection(repo: Dict, searcher: GitHubSearcher,
                         min_recipes: int) -> Tuple[bool, str]:
    """Decide whether a repository is a genuine collection of recipes."""
    owner = repo["full_name"].split("/")[0]

    if repo.get("fork"):
        return False, "fork"
    if owner.lower() == "cooklang":
        return False, "Cooklang project tooling"

    haystack = f'{repo["full_name"]} {repo.get("description", "")} ' \
               f'{" ".join(repo.get("topics", []))}'
    tooling = TOOLING_RE.search(haystack)
    if tooling:
        return False, f"looks like tooling ('{tooling.group(0)}')"

    cook_files = repo.get("cook_files", [])
    recipes = repo.get("recipes", [])
    if not cook_files:
        return False, "no .cook files"
    if not recipes:
        return False, "all .cook files are test fixtures"
    if len(recipes) < min_recipes:
        return False, f"only {len(recipes)} recipe(s), need {min_recipes}"

    # Read a spread of files to confirm they are recipes, not stub fixtures or
    # build scripts.
    n = len(recipes)
    sample = [recipes[i] for i in sorted({0, n // 4, n // 2, 3 * n // 4, n - 1})
              if i < n]
    downloaded = recipe_like = 0
    for path in sample:
        ok, text = searcher.fetch_raw_file(
            repo["full_name"], repo["default_branch"], path)
        if not ok:
            continue
        downloaded += 1
        if looks_like_recipe(text):
            recipe_like += 1

    if downloaded == 0:
        return False, "could not download any .cook file to check"
    if recipe_like == 0:
        return False, ("sampled .cook files are not Cooklang "
                       "(likely `cook` build-tool files)")

    return True, ""


class FeedManager:
    """Manages the feeds.yaml configuration file."""

    def __init__(self, config_path: Path):
        self.config_path = config_path
        self.config = self._load_config()
        self.pending: List[str] = []

    def _load_config(self) -> Dict:
        """Load the feeds.yaml configuration."""
        with open(self.config_path, 'r') as f:
            return yaml.safe_load(f)

    @staticmethod
    def _escape_yaml_string(s: Optional[str]) -> str:
        """Escape backslashes and quotes for a double-quoted YAML scalar."""
        if s is None:
            return ""
        return s.replace("\\", "\\\\").replace('"', '\\"')

    def is_feed_exists(self, repo_url: str) -> bool:
        """Check if a feed with this URL already exists."""
        target = (repo_url or "").rstrip("/").lower()
        return any((feed.get("url") or "").rstrip("/").lower() == target
                   for feed in self.config.get("feeds", []))

    # What the repository name suggests the collection should be called.
    COLLECTION_NOUNS = [
        ("cookbook", "Cookbook"),
        ("recipe-book", "Recipe Book"),
        ("recipebook", "Recipe Book"),
        ("recipes-book", "Recipe Book"),
        ("recipe", "Recipes"),
        ("cooking", "Cooking Recipes"),
        ("cook", "Cookbook"),
        ("menu", "Menu"),
        ("meal", "Meal Plans"),
    ]

    def _title_for(self, repo: Dict) -> str:
        """
        Build a readable title: "<Owner>'s <Collection>".

        GitHub descriptions make poor titles - most repos have none, and the
        ones that do often carry typos, URLs, or words that mean nothing out of
        context. The description is kept in `notes` instead.
        """
        owner = repo.get("owner_name") or repo["full_name"].split("/")[0]
        name = repo["full_name"].split("/")[1].lower()

        noun = "Cooklang Recipes"
        for needle, label in self.COLLECTION_NOUNS:
            if needle in name:
                noun = label
                break

        possessive = f"{owner}'" if owner.endswith("s") else f"{owner}'s"
        return f"{possessive} {noun}"

    @staticmethod
    def _notes_for(repo: Dict) -> str:
        """Measured facts, plus the repo's own description when it has one."""
        note = (f'Found via GitHub search ({len(repo["recipes"])} recipes, '
                f'⭐ {repo["stars"]})')
        description = (repo.get("description") or "").strip()
        if description:
            if len(description) > 150:
                description = description[:147].rsplit(" ", 1)[0] + "..."
            note += f" - {description}"
        return note

    def add_feed(self, repo: Dict, added_by: str = "@bot") -> bool:
        """Queue a new feed entry for writing."""
        if self.is_feed_exists(repo["url"]):
            print(f"  ⏭️  Skipping {repo['full_name']} (already exists)")
            return False

        escape = self._escape_yaml_string
        self.pending.append("\n".join([
            f'  - url: "{escape(repo["url"])}"',
            f'    title: "{escape(self._title_for(repo))}"',
            f'    feed_type: github',
            f'    branch: "{escape(repo["default_branch"])}"',
            f'    enabled: true',
            f'    tags:',
            f'      - cookbook',
            f'      - github',
            f'    notes: "{escape(self._notes_for(repo))}"',
            f'    added_by: "{escape(added_by)}"',
            f'    added_at: "{date.today()}"',
        ]))
        print(f"  ✅ Added {repo['full_name']} "
              f"({len(repo['recipes'])} recipes, ⭐ {repo['stars']})")
        return True

    def save_config(self):
        """
        Append the queued entries to feeds.yaml.

        The file is edited rather than regenerated: rewriting it would drop
        comments and the disabled_at/disabled_by/disabled_reason fields that
        the config validator requires on every disabled feed.
        """
        if not self.pending:
            return

        text = self.config_path.read_text()
        marker = "# Validation configuration"
        if marker in text:
            index = text.index(marker)
            head, tail = text[:index].rstrip("\n"), text[index:]
            new_text = head + "\n\n" + "\n\n".join(self.pending) + "\n\n\n\n" + tail
        else:
            new_text = text.rstrip("\n") + "\n\n" + "\n\n".join(self.pending) + "\n"

        self.config_path.write_text(new_text)

        # Fail loudly rather than leave a corrupted config behind.
        reloaded = yaml.safe_load(self.config_path.read_text())
        urls = [f["url"] for f in reloaded["feeds"]]
        assert len(urls) == len(set(urls)), "duplicate feed URLs introduced"


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
        help="Maximum API pages per size bucket (100 results/page, default: 10)"
    )
    parser.add_argument(
        "--min-recipes",
        type=int,
        default=3,
        help="Minimum number of real recipes a repo must contain (default: 3)"
    )
    parser.add_argument(
        "--randomize",
        action="store_true",
        help="Randomize selection instead of ranking by recipe count"
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

    token = args.token or os.environ.get("GITHUB_TOKEN")
    if not token:
        print("⚠️  WARNING: GitHub Code Search API requires authentication!", file=sys.stderr)
        print("Please provide a GitHub token with --token or $GITHUB_TOKEN.", file=sys.stderr)
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

    searcher = GitHubSearcher(token=token)
    feed_manager = FeedManager(config_path)

    # Search for repositories. Most candidates are rejected by the quality
    # filter, so cast a much wider net than the number we intend to add.
    try:
        candidates = searcher.search_repos_with_cook_files(
            max_repos=max(args.limit * 20, 100),
            max_pages=args.max_pages
        )
    except Exception as e:
        print(f"Error searching GitHub: {e}", file=sys.stderr)
        sys.exit(1)

    if not candidates:
        print("No repositories found.")
        return

    candidates = [c for c in candidates if not feed_manager.is_feed_exists(c["url"])]
    print(f"\n{len(candidates)} candidate(s) not already in feeds.yaml")
    print("Checking which ones are real recipe collections...\n")

    if args.randomize:
        random.shuffle(candidates)

    accepted, rejected = [], 0
    for candidate in candidates:
        if len(accepted) >= args.limit and not args.randomize:
            # Ranking needs the whole pool, so only stop early once we have a
            # comfortable surplus to rank.
            if len(accepted) >= args.limit * 3:
                break

        repo = searcher.enrich(candidate)
        if repo is None:
            rejected += 1
            print(f"  ❌ {candidate['full_name']}: not accessible")
            continue

        ok, reason = is_recipe_collection(repo, searcher, args.min_recipes)
        if ok:
            accepted.append(repo)
            print(f"  ✔️  {repo['full_name']}: {len(repo['recipes'])} recipes, "
                  f"⭐ {repo['stars']}")
        else:
            rejected += 1
            print(f"  ❌ {repo['full_name']}: {reason}")

    print(f"\n{len(accepted)} recipe collection(s), {rejected} rejected")

    if args.randomize:
        random.shuffle(accepted)
    else:
        accepted.sort(key=lambda r: (-len(r["recipes"]), -r["stars"], r["full_name"]))
    accepted = accepted[:args.limit]

    print(f"\nAdding {len(accepted)} repositories...\n")

    added_count = 0
    for repo in accepted:
        if args.dry_run:
            print(f"  [DRY RUN] Would add {repo['full_name']} "
                  f"({len(repo['recipes'])} recipes, ⭐ {repo['stars']})")
            added_count += 1
        elif feed_manager.add_feed(repo, added_by=args.added_by):
            added_count += 1

    if args.dry_run:
        print()
        print(f"[DRY RUN] Would add {added_count} new feed(s)")
    elif added_count > 0:
        feed_manager.save_config()
        print()
        print(f"✨ Successfully added {added_count} new feed(s) to {config_path}")
    else:
        print()
        print("No new feeds to add.")


if __name__ == "__main__":
    main()
