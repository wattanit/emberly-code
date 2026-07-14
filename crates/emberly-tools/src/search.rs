//! Web-search client, normalized types, per-adapter response parsers, and
//! auth (T-14, Tech Spec §5.5).
//!
//! The `reqwest::Client` is injected, so tests run over plain HTTP (mirror the
//! provider clients). [`SearchAuth`] mirrors the provider `Auth` enum with the
//! addition of a `Query` scheme (append `?param=<key>` to the URL) — new
//! search-side, since `emberly-tools` does not depend on `emberly-providers`
//! (G-24 candidate if the Spec §4.5 auth set should gain `query` provider-side).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Normalized types
// ---------------------------------------------------------------------------

/// A single normalized search hit (Tech Spec §5.5). All adapter parsers
/// produce this shape; the tool renders it as untrusted web content
/// (Design §4.10).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// An error from the search backend. Mapped to a structured `ToolOutcome`
/// failure by the tool (HC-6) — the model can react and recover.
#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    /// Transport/connection failure (DNS, TCP, TLS, dropped socket).
    #[error("connection failed: {0}")]
    Connect(String),

    /// Authentication rejected (401/403).
    #[error("authentication failed")]
    Auth,

    /// A non-success HTTP status not covered by a more specific variant.
    #[error("search returned HTTP {status}: {message}")]
    Http { status: u16, message: String },

    /// The response body could not be parsed as the adapter's expected shape.
    #[error("failed to parse search response: {0}")]
    Parse(String),
}

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

/// How a search client authenticates its requests (Tech Spec §5.5). Mirrors
/// the provider `Auth` enum with a `Query` scheme that appends
/// `?param=<key>` to the URL — new search-side (§4.5 `Auth` lacks `query`).
///
/// The key value is resolved from config (env / `keys.toml`) before it reaches
/// here, so no secret-loading logic lives in this module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchAuth {
    /// No authentication.
    None,
    /// `Authorization: Bearer <key>`. An empty key sends no header (the
    /// keyless-server path).
    Bearer(String),
    /// A custom header carrying the key verbatim.
    Header { name: String, value: String },
    /// Append `?param=<value>` to the request URL (the new scheme, Tech Spec
    /// §5.5 — e.g. `?key=…`).
    Query { param: String, value: String },
}

impl SearchAuth {
    /// Attach this scheme to an outgoing request (mirror `Auth::apply`,
    /// `emberly-providers/src/auth.rs:25`).
    pub fn apply(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self {
            SearchAuth::None => builder,
            SearchAuth::Bearer(key) if key.is_empty() => builder,
            SearchAuth::Bearer(key) => builder.bearer_auth(key),
            SearchAuth::Header { name, value } => builder.header(name.as_str(), value),
            SearchAuth::Query { param, value } => {
                builder.query(&[(param.as_str(), value.as_str())])
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Per-adapter response parsers (the search analogue of §4.2 wire parsers)
// ---------------------------------------------------------------------------

/// Dispatch to a per-adapter response parser. Each normalizes a service's JSON
/// into `Vec<SearchResult>` and caps to `max`. A genuinely new shape is a new
/// parser; a new service is just a profile (P-8, the property to preserve).
pub fn parse_results(
    adapter: &str,
    body: &Value,
    max: usize,
) -> Result<Vec<SearchResult>, SearchError> {
    match adapter {
        "brave" => Ok(parse_brave(body, max)),
        "tavily" => Ok(parse_tavily(body, max)),
        "searxng" => Ok(parse_searxng(body, max)),
        "json" => Ok(parse_json(body, max)),
        other => Err(SearchError::Parse(format!(
            "unknown search adapter: {other}"
        ))),
    }
}

/// Brave Search API: results under `web.results[]`, snippet in `description`.
fn parse_brave(body: &Value, max: usize) -> Vec<SearchResult> {
    let results = body
        .get("web")
        .and_then(|w| w.get("results"))
        .and_then(Value::as_array);
    results_from(results, max)
}

/// Tavily Search API: results at top-level `results[]`, snippet in `content`.
fn parse_tavily(body: &Value, max: usize) -> Vec<SearchResult> {
    results_from(body.get("results").and_then(Value::as_array), max)
}

/// SearXNG: results at top-level `results[]`, snippet in `content`.
fn parse_searxng(body: &Value, max: usize) -> Vec<SearchResult> {
    results_from(body.get("results").and_then(Value::as_array), max)
}

/// Generic JSON parser for services not otherwise covered (Tech Spec §5.5).
/// Tries common array locations and field names; the escape hatch for any
/// service whose response does not match a named adapter.
fn parse_json(body: &Value, max: usize) -> Vec<SearchResult> {
    let array = body
        .as_array()
        .or_else(|| body.get("results").and_then(Value::as_array))
        .or_else(|| body.get("data").and_then(Value::as_array))
        .or_else(|| body.get("items").and_then(Value::as_array))
        .or_else(|| {
            body.get("web")
                .and_then(|w| w.get("results"))
                .and_then(Value::as_array)
        });
    results_from(array, max)
}

/// Collect and cap results from an optional JSON array, skipping entries
/// without a URL (the essential field — a hit without a link is not useful).
fn results_from(array: Option<&Vec<Value>>, max: usize) -> Vec<SearchResult> {
    match array {
        Some(arr) => arr.iter().filter_map(extract_result).take(max).collect(),
        None => Vec::new(),
    }
}

/// Normalize a single JSON object into a [`SearchResult`]. Returns `None` if
/// no URL field is present. Title and snippet default to the URL and empty
/// string respectively when the service omits them.
fn extract_result(obj: &Value) -> Option<SearchResult> {
    let url = first_str(obj, &["url", "link", "href", "dest"])?;
    let title = first_str(obj, &["title", "name", "heading"]).unwrap_or_else(|| url.clone());
    let snippet = first_str(
        obj,
        &["snippet", "description", "content", "text", "excerpt"],
    )
    .unwrap_or_default();
    Some(SearchResult {
        title,
        url,
        snippet,
    })
}

/// The first non-empty string among the given keys in a JSON object.
fn first_str(obj: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| obj.get(*key).and_then(Value::as_str).map(str::to_owned))
}

// ---------------------------------------------------------------------------
// Thin client
// ---------------------------------------------------------------------------

/// Thin first-party search client over an injected `reqwest::Client` (Tech Spec
/// §5.5). Built by the binary composition root with resolved config; only the
/// configured `endpoint` is ever reached. The tool holds this struct (mirror
/// the provider structs); its only engine interaction is the ordinary
/// `ctx.authorize` permission call (no engine round-trip gate).
pub struct SearchClient {
    client: reqwest::Client,
    endpoint: String,
    adapter: String,
    auth: SearchAuth,
    max_results: usize,
}

impl SearchClient {
    /// Build a client with resolved config. The `reqwest::Client` is injected;
    /// the TLS/crypto backend is chosen at wiring time and tests pass a
    /// plain-HTTP client.
    #[must_use]
    pub fn new(
        client: reqwest::Client,
        endpoint: impl Into<String>,
        adapter: impl Into<String>,
        auth: SearchAuth,
        max_results: usize,
    ) -> Self {
        Self {
            client,
            endpoint: endpoint.into(),
            adapter: adapter.into(),
            auth,
            max_results,
        }
    }

    /// The configured endpoint URL (for display in permission prompts — never
    /// the key).
    #[must_use]
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Execute a search query, returning normalized results capped at
    /// `max_results` (or `count`, whichever is smaller). First-party HTTP from
    /// the harness process — the tool never touches the sandbox (Tech Spec
    /// §5.5, §2).
    pub async fn search(
        &self,
        query: &str,
        count: Option<usize>,
    ) -> Result<Vec<SearchResult>, SearchError> {
        let limit = count.unwrap_or(self.max_results).min(self.max_results);
        let builder = self.build_request(query, limit);
        let builder = self.auth.apply(builder);
        let response = builder
            .send()
            .await
            .map_err(|e| SearchError::Connect(e.to_string()))?;
        let response = Self::check_response(response).await?;
        let body: Value = response
            .json()
            .await
            .map_err(|e| SearchError::Parse(e.to_string()))?;
        parse_results(&self.adapter, &body, limit)
    }

    /// Build the base request per adapter: GET for brave/searxng/json, POST for
    /// tavily (each service's documented request shape).
    fn build_request(&self, query: &str, count: usize) -> reqwest::RequestBuilder {
        match self.adapter.as_str() {
            "tavily" => self.client.post(&self.endpoint).json(&json!({
                "query": query,
                "max_results": count,
            })),
            "searxng" => self
                .client
                .get(&self.endpoint)
                .query(&[("q", query), ("format", "json")]),
            _ => self.client.get(&self.endpoint).query(&[("q", query)]),
        }
    }

    /// Map a non-2xx response into a typed error (mirror
    /// `wire::check_response`, `emberly-providers/src/wire.rs:19`).
    async fn check_response(response: reqwest::Response) -> Result<reqwest::Response, SearchError> {
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let code = status.as_u16();
        let body = response.text().await.unwrap_or_default();
        Err(match code {
            401 | 403 => SearchError::Auth,
            _ => SearchError::Http {
                status: code,
                message: clip(&body),
            },
        })
    }
}

/// Clip a response body for error diagnostics (mirror `wire::clip`).
fn clip(body: &str) -> String {
    let trimmed = body.trim();
    trimmed.chars().take(500).collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- SearchAuth.apply --------------------------------------------------

    /// A header's value as a string, or `None` if absent/non-ASCII.
    fn header(req: &reqwest::Request, name: &str) -> Option<String> {
        req.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    }

    /// Build a POST request with `auth` applied and return it for inspection.
    /// (No `unwrap`/`expect` — the crate denies them; `match` + `panic!`.)
    fn apply(auth: &SearchAuth) -> reqwest::Request {
        use std::sync::Once;
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            let _ = rustls_rustcrypto::provider().install_default();
        });
        let builder = reqwest::Client::new().post("http://localhost/");
        match auth.apply(builder).build() {
            Ok(req) => req,
            Err(e) => panic!("failed to build request: {e}"),
        }
    }

    #[test]
    fn auth_none_sets_nothing() {
        let req = apply(&SearchAuth::None);
        assert_eq!(header(&req, "authorization"), None);
    }

    #[test]
    fn auth_bearer_sets_authorization() {
        let req = apply(&SearchAuth::Bearer("sk-abc".into()));
        assert_eq!(
            header(&req, "authorization").as_deref(),
            Some("Bearer sk-abc")
        );
    }

    #[test]
    fn auth_empty_bearer_sets_no_header() {
        let req = apply(&SearchAuth::Bearer(String::new()));
        assert_eq!(header(&req, "authorization"), None);
    }

    #[test]
    fn auth_header_sets_custom_header() {
        let req = apply(&SearchAuth::Header {
            name: "X-Api-Key".into(),
            value: "secret".into(),
        });
        assert_eq!(header(&req, "x-api-key").as_deref(), Some("secret"));
    }

    #[test]
    fn auth_query_appends_to_url() {
        // Query params end up on the request URL; verify the param is present.
        let builder = reqwest::Client::new().get("http://localhost/");
        let applied = SearchAuth::Query {
            param: "key".into(),
            value: "abc".into(),
        }
        .apply(builder);
        let url = applied
            .try_clone()
            .and_then(|b| b.build().ok())
            .map(|r| r.url().to_string());
        match url {
            Some(u) => assert!(u.contains("key=abc"), "URL was: {u}"),
            None => panic!("could not build request to inspect URL"),
        }
    }

    // -- Parser tests ------------------------------------------------------

    #[test]
    fn parse_brave_extracts_results() {
        let body = json!({
            "web": {
                "results": [
                    {"title": "Rust", "url": "https://rust-lang.org", "description": "Fearless concurrency"},
                    {"title": "Cargo", "url": "https://doc.rust-lang.org/cargo", "description": "Package manager"}
                ]
            }
        });
        let results = parse_brave(&body, 10);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Rust");
        assert_eq!(results[0].snippet, "Fearless concurrency");
    }

    #[test]
    fn parse_brave_caps_at_max() {
        let body = json!({
            "web": {
                "results": [
                    {"title": "A", "url": "https://a.example", "description": "a"},
                    {"title": "B", "url": "https://b.example", "description": "b"},
                    {"title": "C", "url": "https://c.example", "description": "c"},
                ]
            }
        });
        let results = parse_brave(&body, 2);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "A");
        assert_eq!(results[1].title, "B");
    }

    #[test]
    fn parse_brave_empty_when_no_results_key() {
        let body = json!({"web": {}});
        let results = parse_brave(&body, 10);
        assert!(results.is_empty());
    }

    #[test]
    fn parse_brave_skips_entries_without_url() {
        let body = json!({
            "web": {
                "results": [
                    {"title": "Good", "url": "https://good.example", "description": "yes"},
                    {"title": "No URL", "description": "skip me"}
                ]
            }
        });
        let results = parse_brave(&body, 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Good");
    }

    #[test]
    fn parse_tavily_extracts_results() {
        let body = json!({
            "results": [
                {"title": "Rust", "url": "https://rust-lang.org", "content": "Systems language"},
                {"title": "Learn", "url": "https://doc.rust-lang.org", "content": "The book"}
            ]
        });
        let results = parse_tavily(&body, 10);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].snippet, "Systems language");
    }

    #[test]
    fn parse_searxng_extracts_results() {
        let body = json!({
            "results": [
                {"title": "Rust", "url": "https://rust-lang.org", "content": "Systems language"}
            ]
        });
        let results = parse_searxng(&body, 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Rust");
    }

    #[test]
    fn parse_json_finds_results_array() {
        let body = json!({
            "results": [
                {"title": "Test", "url": "https://example.com", "snippet": "A snippet"}
            ]
        });
        let results = parse_json(&body, 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].snippet, "A snippet");
    }

    #[test]
    fn parse_json_finds_items_array() {
        let body = json!({
            "items": [
                {"name": "Test", "href": "https://example.com", "text": "A text"}
            ]
        });
        let results = parse_json(&body, 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Test");
        assert_eq!(results[0].url, "https://example.com");
        assert_eq!(results[0].snippet, "A text");
    }

    #[test]
    fn parse_json_bare_array() {
        let body = json!([
            {"title": "Direct", "url": "https://direct.example", "content": "bare"}
        ]);
        let results = parse_json(&body, 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Direct");
    }

    #[test]
    fn parse_results_dispatches_correctly() {
        let brave =
            json!({"web": {"results": [{"title": "B", "url": "https://b", "description": "d"}]}});
        let tavily = json!({"results": [{"title": "T", "url": "https://t", "content": "c"}]});

        let r = parse_results("brave", &brave, 10).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(r[0].title, "B");

        let r = parse_results("tavily", &tavily, 10).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(r[0].title, "T");
    }

    #[test]
    fn parse_results_unknown_adapter_is_error() {
        let body = json!({});
        match parse_results("yahoo", &body, 10) {
            Err(SearchError::Parse(msg)) => assert!(msg.contains("yahoo"), "msg: {msg}"),
            other => panic!("expected Parse error, got {other:?}"),
        }
    }

    #[test]
    fn parse_defaults_title_to_url_when_missing() {
        let body = json!([{"url": "https://no-title.example", "content": "stuff"}]);
        let results = parse_json(&body, 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "https://no-title.example");
        assert_eq!(results[0].snippet, "stuff");
    }

    #[test]
    fn parse_empty_snippet_defaults_to_empty_string() {
        let body = json!([{"title": "T", "url": "https://u"}]);
        let results = parse_json(&body, 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].snippet, "");
    }
}
