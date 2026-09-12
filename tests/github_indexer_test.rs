//! End-to-end behaviour of the GitHub indexer against a fake GitHub API.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use federation::db::{self, init_pool, run_migrations};
use federation::github::{GitHubConfig, GitHubIndexer};
use federation::indexer::{SearchIndex, SearchQuery};
use mockito::{Matcher, Mock, ServerGuard};
use serde_json::json;

/// A GitHub repository whose contents we control.
struct FakeRepo {
    server: ServerGuard,
    owner: String,
    repo: String,
    mocks: Vec<Mock>,
    commit: u32,
}

fn sha(text: &str) -> String {
    let mut h = DefaultHasher::new();
    text.hash(&mut h);
    format!("{:016x}", h.finish())
}

impl FakeRepo {
    async fn new(owner: &str, repo: &str) -> Self {
        Self {
            server: mockito::Server::new_async().await,
            owner: owner.to_string(),
            repo: repo.to_string(),
            mocks: Vec::new(),
            commit: 0,
        }
    }

    fn url(&self) -> String {
        format!("https://github.com/{}/{}", self.owner, self.repo)
    }

    fn config(&self) -> GitHubConfig {
        GitHubConfig {
            api_base_url: self.server.url(),
            raw_base_url: self.server.url(),
            ..GitHubConfig::default()
        }
    }

    /// Replace the repository contents with `files` (path, content) in a new commit.
    async fn set_files(&mut self, files: &[(&str, &str)]) {
        for mock in self.mocks.drain(..) {
            mock.remove_async().await;
        }
        self.commit += 1;
        let commit_sha = format!("commit{}", self.commit);
        let tree_sha = format!("tree{}", self.commit);
        let api = format!("/repos/{}/{}", self.owner, self.repo);

        let repo_json = json!({
            "id": 1, "name": self.repo, "full_name": format!("{}/{}", self.owner, self.repo),
            "owner": {"login": self.owner, "id": 1}, "default_branch": "main",
            "description": null, "html_url": self.url(), "archived": false
        });
        self.mount("GET", &api, repo_json.to_string()).await;

        let reference = json!({
            "ref": "refs/heads/main", "node_id": "n", "url": "u",
            "object": {"sha": commit_sha, "type": "commit", "url": "u"}
        });
        self.mount(
            "GET",
            &format!("{api}/git/refs/heads/main"),
            reference.to_string(),
        )
        .await;

        let commit = json!({
            "sha": commit_sha, "url": "u", "html_url": "u",
            "commit": {"message": "m", "tree": {"sha": tree_sha, "url": "u"}}
        });
        self.mount(
            "GET",
            &format!("{api}/commits/{commit_sha}"),
            commit.to_string(),
        )
        .await;

        let entries: Vec<_> = files
            .iter()
            .map(|(path, content)| {
                json!({"path": path, "mode": "100644", "sha": sha(content), "size": content.len(), "type": "blob"})
            })
            .collect();
        let tree = json!({"sha": tree_sha, "url": "u", "tree": entries, "truncated": false});
        self.mount(
            "GET",
            &format!("{api}/git/trees/{tree_sha}"),
            tree.to_string(),
        )
        .await;

        for (path, content) in files {
            // The client percent-encodes the request path, so the mock must too.
            let raw = format!(
                "/{}/{}/main/{}",
                self.owner,
                self.repo,
                path.replace(' ', "%20")
            );
            self.mount("GET", &raw, content.to_string()).await;
        }
    }

    async fn mount(&mut self, method: &str, path: &str, body: String) {
        let mock = self
            .server
            .mock(method, path)
            .match_query(Matcher::Any)
            .with_status(200)
            .with_body(body)
            .create_async()
            .await;
        self.mocks.push(mock);
    }
}

struct Harness {
    pool: db::DbPool,
    search: Arc<SearchIndex>,
    _dir: tempfile::TempDir,
}

impl Harness {
    async fn new() -> Self {
        let pool = init_pool("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let search = Arc::new(SearchIndex::new(dir.path()).unwrap());
        Self {
            pool,
            search,
            _dir: dir,
        }
    }

    fn indexer(&self, repo: &FakeRepo) -> GitHubIndexer {
        GitHubIndexer::new(repo.config(), self.pool.clone(), self.search.clone()).unwrap()
    }

    async fn titles(&self) -> Vec<String> {
        let mut titles: Vec<String> = db::recipes::list_all_recipes(&self.pool, 100, 0)
            .await
            .unwrap()
            .into_iter()
            .map(|r| r.title)
            .collect();
        titles.sort();
        titles
    }

    fn search_ids(&self, q: &str) -> Vec<i64> {
        let query = SearchQuery {
            q: q.to_string(),
            page: 1,
            limit: 10,
            locale: None,
        };
        let mut ids: Vec<i64> = self
            .search
            .search(&query, 100)
            .unwrap()
            .results
            .iter()
            .map(|r| r.recipe_id)
            .collect();
        ids.sort();
        ids
    }
}

#[tokio::test]
async fn title_comes_from_metadata_or_a_humanized_filename() {
    let h = Harness::new().await;
    let mut repo = FakeRepo::new("alice", "recipes").await;
    repo.set_files(&[
        (
            "recipes/2025-12-01-tiramisu-brownies.cook",
            "---\ntitle: Tiramisu Brownies Deluxe\n---\nMix @flour{1%cup}.\n",
        ),
        (
            "recipes/2025-12-04-cottage_cheese-gnocchi.cook",
            "Boil @gnocchi{} until they float.\n",
        ),
    ])
    .await;

    h.indexer(&repo).add_repository(&repo.url()).await.unwrap();

    assert_eq!(
        h.titles().await,
        vec!["Cottage Cheese Gnocchi", "Tiramisu Brownies Deluxe"]
    );
}

#[tokio::test]
async fn a_changed_file_refreshes_title_content_and_image() {
    let h = Harness::new().await;
    let mut repo = FakeRepo::new("alice", "recipes").await;
    repo.set_files(&[("Cake.cook", "Bake @flour{1%cup}.\n")])
        .await;
    let feed_id = h.indexer(&repo).add_repository(&repo.url()).await.unwrap();
    assert_eq!(h.titles().await, vec!["Cake"]);
    let before = db::recipes::list_all_recipes(&h.pool, 100, 0)
        .await
        .unwrap()
        .remove(0);

    repo.set_files(&[
        (
            "Cake.cook",
            "---\ntitle: Lemon Cake\n---\nBake @lemon{1}.\n",
        ),
        ("Cake.png", "png"),
    ])
    .await;
    h.indexer(&repo).index_repository(feed_id).await.unwrap();

    let after = db::recipes::get_recipe(&h.pool, before.id).await.unwrap();
    assert_eq!(after.title, "Lemon Cake");
    assert!(after.content.unwrap().contains("@lemon"));
    assert!(after.image_url.unwrap().ends_with("/Cake.png"));
    assert_ne!(after.content_hash, before.content_hash);
    assert_eq!(h.search_ids("lemon"), vec![before.id]);
    assert!(
        h.search_ids("flour").is_empty(),
        "stale content must leave the index"
    );
}

#[tokio::test]
async fn a_file_removed_from_the_repository_is_removed_from_db_and_search() {
    let h = Harness::new().await;
    let mut repo = FakeRepo::new("alice", "recipes").await;
    repo.set_files(&[
        ("old/Caesar Salad.cook", "Toss @lettuce{}.\n"),
        ("Soup.cook", "Simmer @stock{}.\n"),
    ])
    .await;
    let feed_id = h.indexer(&repo).add_repository(&repo.url()).await.unwrap();
    assert_eq!(h.search_ids("lettuce").len(), 1);

    repo.set_files(&[("Soup.cook", "Simmer @stock{}.\n")]).await;
    h.indexer(&repo).index_repository(feed_id).await.unwrap();

    assert_eq!(h.titles().await, vec!["Soup"]);
    assert!(h.search_ids("lettuce").is_empty());
    let github_rows = db::github::list_github_recipes_by_feed(&h.pool, feed_id)
        .await
        .unwrap();
    assert_eq!(github_rows.len(), 1);
    assert_eq!(github_rows[0].file_path, "Soup.cook");
}

#[tokio::test]
async fn identical_recipe_at_a_second_path_is_indexed_once() {
    let h = Harness::new().await;
    let mut repo = FakeRepo::new("alice", "recipes").await;
    let content = "---\ntitle: Tiramisu Brownies\n---\nLayer @mascarpone{}.\n";
    // Many copies, so that concurrent processing cannot slip a duplicate past
    // the check by luck of timing.
    let paths: Vec<String> = (0..20)
        .map(|i| format!("build{i}/recipes/tiramisu-brownies.cook"))
        .collect();
    let files: Vec<(&str, &str)> = paths.iter().map(|p| (p.as_str(), content)).collect();
    repo.set_files(&files).await;

    h.indexer(&repo).add_repository(&repo.url()).await.unwrap();

    assert_eq!(h.titles().await, vec!["Tiramisu Brownies"]);
    assert_eq!(h.search_ids("mascarpone").len(), 1);
}

#[tokio::test]
async fn identical_recipe_in_a_fork_is_indexed_once() {
    let h = Harness::new().await;
    let content = "---\ntitle: Tiramisu Brownies\n---\nLayer @mascarpone{}.\n";
    let mut upstream = FakeRepo::new("alice", "recipes").await;
    upstream
        .set_files(&[("tiramisu-brownies.cook", content)])
        .await;
    let mut fork = FakeRepo::new("bob", "recipes").await;
    fork.set_files(&[("tiramisu-brownies.cook", content)]).await;

    h.indexer(&upstream)
        .add_repository(&upstream.url())
        .await
        .unwrap();
    h.indexer(&fork).add_repository(&fork.url()).await.unwrap();

    assert_eq!(h.titles().await, vec!["Tiramisu Brownies"]);
    assert_eq!(h.search_ids("mascarpone").len(), 1);
}
