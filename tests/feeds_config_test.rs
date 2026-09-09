//! The shipped config/feeds.yaml must always load.
//!
//! feeds.yaml is edited by hand and by scripts/find-cooklang-repos.py, and a
//! malformed entry only surfaces at startup, after deploy. In particular a feed
//! marked `enabled: false` has to carry disabled_at/disabled_by, which is easy
//! to miss when retiring a feed.

use federation::config::feeds::FeedConfig;

#[test]
fn shipped_feed_config_is_valid() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/config/feeds.yaml");
    let config = FeedConfig::from_file(path)
        .unwrap_or_else(|e| panic!("config/feeds.yaml is invalid: {e}"));

    assert!(!config.feeds.is_empty(), "feed config has no feeds");
    assert!(
        config.enabled_feeds().count() > 0,
        "feed config has no enabled feeds"
    );
}
