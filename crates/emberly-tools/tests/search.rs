//! Client tests for the search backend (Phase 5, group 1), driven against a
//! `wiremock` server over **plain HTTP** — no API key, no TLS, deterministic.
//! Proves request shape, auth application, non-2xx mapping, and parsing without
//! touching the network (mirror `emberly-providers/tests/live_clients.rs`).
//!
//! No `.unwrap()`/`.expect()`: tests thread `Result` or `panic!` with context.

use emberly_tools::{SearchAuth, SearchClient, SearchError};
use serde_json::json;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A reqwest client with the pure-Rust crypto provider installed once (mirror
/// `live_clients.rs:20`).
fn http_client() -> reqwest::Client {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let _ = rustls_rustcrypto::provider().install_default();
    });
    reqwest::Client::new()
}

#[tokio::test]
async fn brave_search_returns_results() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .and(query_param("q", "rust language"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "web": {
                "results": [
                    {"title": "Rust", "url": "https://rust-lang.org", "description": "Fearless"},
                    {"title": "Learn", "url": "https://doc.rust-lang.org", "description": "Book"}
                ]
            }
        })))
        .mount(&server)
        .await;

    let client = SearchClient::new(
        http_client(),
        format!("{}/search", server.uri()),
        "brave",
        SearchAuth::None,
        5,
    );
    let results = client
        .search("rust language", None)
        .await
        .unwrap_or_else(|e| panic!("search failed: {e}"));

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].title, "Rust");
    assert_eq!(results[0].url, "https://rust-lang.org");
    assert_eq!(results[0].snippet, "Fearless");
}

#[tokio::test]
async fn searxng_sends_format_json_param() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .and(query_param("q", "hello"))
        .and(query_param("format", "json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [
                {"title": "Hi", "url": "https://hi.example", "content": "greeting"}
            ]
        })))
        .mount(&server)
        .await;

    let client = SearchClient::new(
        http_client(),
        format!("{}/search", server.uri()),
        "searxng",
        SearchAuth::None,
        5,
    );
    let results = client
        .search("hello", None)
        .await
        .unwrap_or_else(|e| panic!("search failed: {e}"));
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, "Hi");
}

#[tokio::test]
async fn tavily_posts_json_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/search"))
        .and(wiremock::matchers::body_string_contains("rust async"))
        .and(wiremock::matchers::body_string_contains("max_results"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [
                {"title": "Tokio", "url": "https://tokio.rs", "content": "Async runtime"}
            ]
        })))
        .mount(&server)
        .await;

    let client = SearchClient::new(
        http_client(),
        format!("{}/search", server.uri()),
        "tavily",
        SearchAuth::None,
        5,
    );
    let results = client
        .search("rust async", None)
        .await
        .unwrap_or_else(|e| panic!("search failed: {e}"));
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, "Tokio");
}

#[tokio::test]
async fn bearer_auth_sets_authorization_header() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .and(header("authorization", "Bearer test-key-123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": []
        })))
        .mount(&server)
        .await;

    let client = SearchClient::new(
        http_client(),
        format!("{}/search", server.uri()),
        "searxng",
        SearchAuth::Bearer("test-key-123".into()),
        5,
    );
    let results = client
        .search("test", None)
        .await
        .unwrap_or_else(|e| panic!("search failed: {e}"));
    assert!(results.is_empty());
}

#[tokio::test]
async fn header_auth_sets_custom_header() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .and(header("x-subscription-token", "sub-abc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "web": {"results": []}
        })))
        .mount(&server)
        .await;

    let client = SearchClient::new(
        http_client(),
        format!("{}/search", server.uri()),
        "brave",
        SearchAuth::Header {
            name: "X-Subscription-Token".into(),
            value: "sub-abc".into(),
        },
        5,
    );
    let results = client
        .search("test", None)
        .await
        .unwrap_or_else(|e| panic!("search failed: {e}"));
    assert!(results.is_empty());
}

#[tokio::test]
async fn query_auth_appends_key_param() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .and(query_param("q", "test"))
        .and(query_param("key", "query-secret"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": []
        })))
        .mount(&server)
        .await;

    let client = SearchClient::new(
        http_client(),
        format!("{}/search", server.uri()),
        "searxng",
        SearchAuth::Query {
            param: "key".into(),
            value: "query-secret".into(),
        },
        5,
    );
    let results = client
        .search("test", None)
        .await
        .unwrap_or_else(|e| panic!("search failed: {e}"));
    assert!(results.is_empty());
}

#[tokio::test]
async fn caps_at_max_results() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [
                {"title": "1", "url": "https://1.example", "content": "a"},
                {"title": "2", "url": "https://2.example", "content": "b"},
                {"title": "3", "url": "https://3.example", "content": "c"},
                {"title": "4", "url": "https://4.example", "content": "d"},
                {"title": "5", "url": "https://5.example", "content": "e"}
            ]
        })))
        .mount(&server)
        .await;

    let client = SearchClient::new(
        http_client(),
        format!("{}/search", server.uri()),
        "json",
        SearchAuth::None,
        3,
    );
    let results = client
        .search("test", None)
        .await
        .unwrap_or_else(|e| panic!("search failed: {e}"));
    assert_eq!(results.len(), 3);
    assert_eq!(results[0].title, "1");
    assert_eq!(results[2].title, "3");
}

#[tokio::test]
async fn count_param_caps_below_max_results() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [
                {"title": "1", "url": "https://1.example", "content": "a"},
                {"title": "2", "url": "https://2.example", "content": "b"},
                {"title": "3", "url": "https://3.example", "content": "c"}
            ]
        })))
        .mount(&server)
        .await;

    let client = SearchClient::new(
        http_client(),
        format!("{}/search", server.uri()),
        "json",
        SearchAuth::None,
        5,
    );
    let results = client
        .search("test", Some(2))
        .await
        .unwrap_or_else(|e| panic!("search failed: {e}"));
    assert_eq!(results.len(), 2);
}

#[tokio::test]
async fn count_above_max_is_clamped_to_max() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [
                {"title": "1", "url": "https://1.example", "content": "a"},
                {"title": "2", "url": "https://2.example", "content": "b"}
            ]
        })))
        .mount(&server)
        .await;

    let client = SearchClient::new(
        http_client(),
        format!("{}/search", server.uri()),
        "json",
        SearchAuth::None,
        2,
    );
    // count=10 but max_results=2 → only 2 results
    let results = client
        .search("test", Some(10))
        .await
        .unwrap_or_else(|e| panic!("search failed: {e}"));
    assert_eq!(results.len(), 2);
}

#[tokio::test]
async fn server_error_maps_to_http_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
        .mount(&server)
        .await;

    let client = SearchClient::new(
        http_client(),
        format!("{}/search", server.uri()),
        "json",
        SearchAuth::None,
        5,
    );
    match client.search("test", None).await {
        Err(SearchError::Http { status, .. }) => assert_eq!(status, 500),
        other => panic!("expected Http error, got {other:?}"),
    }
}

#[tokio::test]
async fn auth_error_maps_to_auth_variant() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
        .mount(&server)
        .await;

    let client = SearchClient::new(
        http_client(),
        format!("{}/search", server.uri()),
        "json",
        SearchAuth::None,
        5,
    );
    match client.search("test", None).await {
        Err(SearchError::Auth) => {}
        other => panic!("expected Auth error, got {other:?}"),
    }
}

#[tokio::test]
async fn empty_results_is_not_an_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"results": []})))
        .mount(&server)
        .await;

    let client = SearchClient::new(
        http_client(),
        format!("{}/search", server.uri()),
        "json",
        SearchAuth::None,
        5,
    );
    let results = client
        .search("nothing matches", None)
        .await
        .unwrap_or_else(|e| panic!("search failed: {e}"));
    assert!(results.is_empty());
}
