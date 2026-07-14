//! `web_search` (T-14, Tech Spec §5.5): search the web through a harness-owned,
//! configurable, provider-agnostic backend.
//!
//! Modeled on `builtin/bash.rs` (the permission-gated precedent) — but HTTP is
//! first-party from the harness process, never a sandboxed child. The tool holds
//! its own resolved [`SearchClient`] (like a provider struct); its only engine
//! interaction is the ordinary `ctx.authorize` permission call (no engine
//! round-trip gate, no `ctx.sandbox()`).
//!
//! Results are tagged **untrusted web content** (Design §4.10) via
//! [`ToolOutcome::with_untrusted`] so the TUI renders them as fetched web data
//! with visible source URLs — never harness or assistant voice.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::ctx::ToolCtx;
use crate::permission::PermissionRequest;
use crate::search::SearchClient;
use crate::tool::{Tool, ToolOutcome, ToolSpec};

#[derive(Deserialize)]
struct WebSearchArgs {
    query: String,
    #[serde(default)]
    count: Option<usize>,
}

/// The `web_search` tool (T-14). Holds the resolved [`SearchClient`], built by
/// the binary composition root. Registered only when `search.enabled = true`
/// and an endpoint is configured (Tech Spec §5.5, group 4).
pub struct WebSearchTool {
    client: SearchClient,
}

impl WebSearchTool {
    /// Build a tool backed by a resolved search client. The binary constructs
    /// this after resolving config; tests pass a client pointed at a mock.
    #[must_use]
    pub fn new(client: SearchClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "web_search".into(),
            description: "Search the web via the configured search backend. Results include \
                          title, URL, and a short snippet each. Requires permission — the \
                          request reaches the internet and the results are untrusted web \
                          content."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The search query."
                    },
                    "count": {
                        "type": "integer",
                        "description": "Maximum number of results (optional; capped by configuration)."
                    }
                },
                "required": ["query"]
            }),
        }
    }

    fn describe(&self, args: &Value) -> Option<String> {
        serde_json::from_value::<WebSearchArgs>(args.clone())
            .ok()
            .map(|a| format!("search: {}", a.query))
    }

    async fn execute(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let args: WebSearchArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolOutcome::failure(format!("invalid arguments: {e}"), "bad args"),
        };

        // Authorize first (Tech Spec §6.1). `outside_root` MUST be `false` — a
        // web search is an ordinary, lower-stakes gated action, never the
        // reserved outside-root safety band (Design §5.2, structural fact 3).
        // Never call `ctx.sandbox()` — egress is harness-process HTTP (§5.5).
        let request = PermissionRequest {
            tool: "web_search".into(),
            summary: format!("search: {}", args.query),
            detail: format!(
                "query: {}\nbackend: {}\nreaches the internet; results are untrusted",
                args.query,
                self.client.endpoint()
            ),
            affected_paths: vec![],
            outside_root: false,
        };
        if !ctx.authorize(request).await.is_allowed() {
            return ToolOutcome::denied("search the web");
        }

        match self.client.search(&args.query, args.count).await {
            Ok(results) if results.is_empty() => ToolOutcome::success(
                format!("No results found for: {}\n", args.query),
                format!("searched: {:?} (0 results)", args.query),
            ),
            Ok(results) => {
                let n = results.len();
                let mut body = format!("Search results for: {}\n\n", args.query);
                for (i, result) in results.iter().enumerate() {
                    body.push_str(&format!(
                        "{}. {}\n   {}\n   {}\n\n",
                        i + 1,
                        result.title,
                        result.url,
                        result.snippet
                    ));
                }
                ToolOutcome::success(body, format!("searched: {:?} ({n} results)", args.query))
                    .with_untrusted()
            }
            Err(e) => {
                // A connect/non-2xx/parse failure is a structured HC-6 failure
                // the model can react to — never a panic (Tech Spec §5.5).
                ToolOutcome::failure(format!("Web search failed: {e}"), "search failed")
            }
        }
    }
}
