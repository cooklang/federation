//! Conditional HTTP request handling (issue #9).
//!
//! A 304 Not Modified is the *expected* response when we send If-None-Match /
//! If-Modified-Since to a server whose content hasn't changed. It must be a
//! normal outcome, never an error - otherwise well-behaved feeds get marked
//! broken on the first crawl after a successful one.

use federation::crawler::fetcher::{FetchOutcome, Fetcher};

fn fetcher() -> Fetcher {
    Fetcher::new("FederationTest/1.0".to_string(), 5_242_880).unwrap()
}

#[tokio::test]
async fn returns_not_modified_when_server_answers_304_to_if_none_match() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/feed.xml")
        .match_header("if-none-match", "\"etag-v1\"")
        .with_status(304)
        .create_async()
        .await;

    let outcome = fetcher()
        .fetch_with_conditions(
            &format!("{}/feed.xml", server.url()),
            Some("\"etag-v1\""),
            None,
        )
        .await
        .expect("304 is a normal conditional-request outcome, not a fetch error");

    assert!(
        matches!(outcome, FetchOutcome::NotModified),
        "expected NotModified, got {outcome:?}"
    );
    mock.assert_async().await;
}

#[tokio::test]
async fn returns_not_modified_when_server_answers_304_to_if_modified_since() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/feed.xml")
        .match_header("if-modified-since", "Wed, 15 Apr 2026 10:22:42 GMT")
        .with_status(304)
        .create_async()
        .await;

    let outcome = fetcher()
        .fetch_with_conditions(
            &format!("{}/feed.xml", server.url()),
            None,
            Some("Wed, 15 Apr 2026 10:22:42 GMT"),
        )
        .await
        .expect("304 is a normal conditional-request outcome, not a fetch error");

    assert!(matches!(outcome, FetchOutcome::NotModified));
    mock.assert_async().await;
}

#[tokio::test]
async fn returns_fetched_content_with_caching_headers_on_200() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/feed.xml")
        .with_status(200)
        .with_header("content-type", "application/atom+xml")
        .with_header("etag", "\"etag-v2\"")
        .with_header("last-modified", "Wed, 15 Apr 2026 10:22:42 GMT")
        .with_body("<feed/>")
        .create_async()
        .await;

    let outcome = fetcher()
        .fetch_with_conditions(&format!("{}/feed.xml", server.url()), None, None)
        .await
        .unwrap();

    match outcome {
        FetchOutcome::Fetched(result) => {
            assert_eq!(result.content, "<feed/>");
            assert_eq!(result.etag.as_deref(), Some("\"etag-v2\""));
            assert_eq!(
                result.last_modified.as_deref(),
                Some("Wed, 15 Apr 2026 10:22:42 GMT")
            );
        }
        FetchOutcome::NotModified => panic!("expected Fetched, got NotModified"),
    }
    mock.assert_async().await;
}

#[tokio::test]
async fn real_http_errors_are_still_errors() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/feed.xml")
        .with_status(500)
        .create_async()
        .await;

    let result = fetcher()
        .fetch_with_conditions(&format!("{}/feed.xml", server.url()), None, None)
        .await;

    assert!(
        result.is_err(),
        "a 500 must still surface as an error so the feed is marked unhealthy"
    );
    mock.assert_async().await;
}
