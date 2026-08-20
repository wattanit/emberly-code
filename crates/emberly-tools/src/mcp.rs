//! MCP-sourced tool contract (FR-11, Tech Spec §5.6): the transport trait an
//! MCP client implements, and the generic `Tool` proxy built over it.
//!
//! Unlike the multi-agent tools (`spawn_agents`/`message_agent`/`list_agents`/
//! `end_agent`), MCP tools are not a small fixed set behind a `*Gate` trait —
//! a connected server can advertise any number of tools, discovered only once
//! it is reached. So this module carries no `Drop*`-style fail-closed default
//! for *construction*: there is no engine-owned state to stand a tool up
//! against until a tool actually exists, and a tool only exists once
//! `emberly-core`'s connection/discovery machinery (Tech Spec §8.4/§8.5's
//! Phase 4) constructs one with a real [`McpTransport`]. What Phase 3 fixes
//! is the *contract* Phase 4 builds against: the transport shape, the
//! namespacing/collision rule, and the proxy `Tool` impl itself — fully
//! testable now against a fake transport.
//!
//! No tool call here supplies a filesystem path or bypasses the permission/
//! sandbox model: an MCP tool call *is* permission-gated, through the exact
//! same `ToolCtx::authorize` every built-in tool already calls (Tech Spec
//! §5.6) — no new gate type, no proxy gate, just the ordinary rule engine.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::ctx::ToolCtx;
use crate::permission::PermissionRequest;
use crate::tool::{Tool, ToolOutcome, ToolSpec};

/// A transport-agnostic MCP client call — one JSON-RPC 2.0 request/response
/// round trip (Tech Spec §5.6). The default implementation is a first-party,
/// newline-delimited JSON-RPC-over-stdio client (`emberly-core`'s
/// `McpClient`); a future remote transport (SSE/HTTP) is a second impl behind
/// this same trait, never a rewrite.
#[async_trait]
pub trait McpTransport: Send + Sync {
    async fn call(&self, method: &str, params: Value) -> Result<Value, McpError>;
}

/// A failure reaching or talking to an MCP server. Mapped to a structured
/// `ToolOutcome` failure by [`McpTool::execute`] (HC-6) — never a panic or a
/// bare harness error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum McpError {
    /// The transport could not reach the server at all (process spawn
    /// failure, closed pipe, connection refused).
    #[error("could not reach the server: {0}")]
    Connect(String),
    /// The server responded, but with a JSON-RPC error or a malformed
    /// response.
    #[error("the server returned an error: {0}")]
    Protocol(String),
}

/// One tool a connected MCP server advertised via `tools/list`, before
/// namespacing (Tech Spec §5.6).
#[derive(Debug, Clone, PartialEq)]
pub struct McpToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// Build `mcp__<server>__<tool>` — the namespacing convention (Requirements
/// §13 resolved, Tech Spec §5.6) that makes the owning server legible
/// directly from a tool's name, read by both the permission-prompt
/// provenance line (Design §5.3) and the tool-activity line (Design §4.15).
/// Collision with a built-in tool name is impossible by construction: no
/// built-in name carries the `mcp__` prefix.
#[must_use]
pub fn namespaced_tool_name(server: &str, tool: &str) -> String {
    format!("mcp__{server}__{tool}")
}

/// Turn one server's discovered tools into registrable [`McpTool`]s. A name
/// collision *within this server's own list* is that server's own bug,
/// surfaced as a structured connection-time failure naming both tools (Tech
/// Spec §5.6) — never a silent overwrite in whatever registry they land in.
pub fn build_mcp_tools(
    server: &str,
    specs: Vec<McpToolSpec>,
    transport: Arc<dyn McpTransport>,
) -> Result<Vec<McpTool>, McpError> {
    let mut seen = HashSet::new();
    for spec in &specs {
        if !seen.insert(spec.name.clone()) {
            return Err(McpError::Protocol(format!(
                "server '{server}' advertises more than one tool named '{}'",
                spec.name
            )));
        }
    }
    Ok(specs
        .into_iter()
        .map(|spec| McpTool {
            namespaced_name: namespaced_tool_name(server, &spec.name),
            original_name: spec.name,
            server: server.to_string(),
            description: spec.description,
            input_schema: spec.input_schema,
            transport: transport.clone(),
        })
        .collect())
}

/// A single MCP-sourced tool (Tech Spec §5.6): a thin proxy over its own
/// server's [`McpTransport`], registered into the ordinary `ToolRegistry`
/// exactly like a built-in tool (T-7) — no parallel registry, no new gate
/// type, no engine change.
pub struct McpTool {
    namespaced_name: String,
    original_name: String,
    server: String,
    description: String,
    input_schema: Value,
    transport: Arc<dyn McpTransport>,
}

#[async_trait]
impl Tool for McpTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.namespaced_name.clone(),
            description: self.description.clone(),
            input_schema: self.input_schema.clone(),
        }
    }

    fn describe(&self, _args: &Value) -> Option<String> {
        Some(format!("{} via {}", self.original_name, self.server))
    }

    async fn execute(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        // Authorize first (Tech Spec §5.6/§6.1) — an MCP tool call is
        // permission-gated exactly like a built-in's, ordinary band (Design
        // §5.3): never the reserved outside-root styling on its own, since
        // an external origin does not by itself make a call more dangerous.
        // `tool` carries the namespaced name so the prompt/rule layer (and
        // the TUI's provenance line) read the owning server directly from
        // it — no separate payload field needed.
        let request = PermissionRequest {
            tool: self.namespaced_name.clone(),
            summary: format!("{} (via MCP server {})", self.original_name, self.server),
            detail: format!(
                "MCP server: {}\ntool: {}\narguments: {args}",
                self.server, self.original_name
            ),
            affected_paths: vec![],
            outside_root: false,
        };
        if !ctx.authorize(request).await.is_allowed() {
            return ToolOutcome::denied(&format!(
                "{} via MCP server {}",
                self.original_name, self.server
            ));
        }

        let params = json!({ "name": self.original_name, "arguments": args });
        match self.transport.call("tools/call", params).await {
            // Untrusted, like web_search (T-14, Design §4.10/§4.15): a
            // result from code the user pointed the harness at, not
            // harness-authored data (HC-6 — data, never a crash).
            Ok(result) => ToolOutcome::success(
                result.to_string(),
                format!("{} (via {})", self.original_name, self.server),
            )
            .with_untrusted(),
            Err(e) => ToolOutcome::failure(e.to_string(), "mcp error"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct FakeTransport {
        response: Result<Value, McpError>,
        last_call: Mutex<Option<(String, Value)>>,
    }

    impl FakeTransport {
        fn ok(response: Value) -> Self {
            Self {
                response: Ok(response),
                last_call: Mutex::new(None),
            }
        }
        fn err(error: McpError) -> Self {
            Self {
                response: Err(error),
                last_call: Mutex::new(None),
            }
        }
    }

    #[async_trait]
    impl McpTransport for FakeTransport {
        async fn call(&self, method: &str, params: Value) -> Result<Value, McpError> {
            *self.last_call.lock().unwrap_or_else(|e| e.into_inner()) =
                Some((method.to_string(), params));
            self.response.clone()
        }
    }

    /// A `ToolCtx` whose gate always allows — for tests exercising an
    /// `McpTool` call under a genuinely granted permission (Tech Spec §5.6:
    /// an MCP call is permission-gated exactly like a built-in's).
    struct AllowGate;
    #[async_trait]
    impl crate::permission::PermissionGate for AllowGate {
        async fn authorize(
            &self,
            _req: crate::permission::PermissionRequest,
        ) -> crate::permission::PermissionOutcome {
            crate::permission::PermissionOutcome::Allow
        }
    }

    /// A `ToolCtx` whose gate always denies — for the denial test.
    struct DenyGate;
    #[async_trait]
    impl crate::permission::PermissionGate for DenyGate {
        async fn authorize(
            &self,
            _req: crate::permission::PermissionRequest,
        ) -> crate::permission::PermissionOutcome {
            crate::permission::PermissionOutcome::Deny
        }
    }

    fn allow_ctx() -> ToolCtx {
        ToolCtx::new(
            std::path::PathBuf::from("/tmp"),
            crate::ctx::TruncateConfig::default(),
            Arc::new(AllowGate),
            Arc::new(crate::sandbox::PlainSandbox),
        )
    }

    fn deny_ctx() -> ToolCtx {
        ToolCtx::new(
            std::path::PathBuf::from("/tmp"),
            crate::ctx::TruncateConfig::default(),
            Arc::new(DenyGate),
            Arc::new(crate::sandbox::PlainSandbox),
        )
    }

    /// `build_mcp_tools`, panicking with the reason on an unexpected
    /// collision — this crate denies `.expect()`/`.unwrap()` even in tests.
    fn build_ok(
        server: &str,
        specs: Vec<McpToolSpec>,
        transport: Arc<dyn McpTransport>,
    ) -> Vec<McpTool> {
        match build_mcp_tools(server, specs, transport) {
            Ok(tools) => tools,
            Err(e) => panic!("expected no collision: {e}"),
        }
    }

    fn spec(name: &str) -> McpToolSpec {
        McpToolSpec {
            name: name.into(),
            description: format!("{name} description"),
            input_schema: json!({"type": "object"}),
        }
    }

    #[test]
    fn namespacing_prefixes_with_mcp_and_server_name() {
        assert_eq!(
            namespaced_tool_name("jira", "get_issue"),
            "mcp__jira__get_issue"
        );
    }

    #[test]
    fn build_mcp_tools_namespaces_every_tool() {
        let transport: Arc<dyn McpTransport> = Arc::new(FakeTransport::ok(json!({})));
        let tools = build_ok(
            "jira",
            vec![spec("get_issue"), spec("create_issue")],
            transport,
        );
        let names: Vec<String> = tools.iter().map(|t| t.spec().name).collect();
        assert_eq!(
            names,
            vec![
                "mcp__jira__get_issue".to_string(),
                "mcp__jira__create_issue".to_string()
            ]
        );
    }

    #[test]
    fn build_mcp_tools_rejects_a_name_collision_within_one_server() {
        let transport: Arc<dyn McpTransport> = Arc::new(FakeTransport::ok(json!({})));
        let err = match build_mcp_tools(
            "jira",
            vec![spec("get_issue"), spec("get_issue")],
            transport,
        ) {
            Ok(_) => panic!("collision must be rejected"),
            Err(e) => e,
        };
        let msg = err.to_string();
        assert!(msg.contains("jira") && msg.contains("get_issue"), "{msg}");
    }

    #[tokio::test]
    async fn execute_success_is_untrusted_and_calls_tools_call() {
        let transport = Arc::new(FakeTransport::ok(json!({"result": "42"})));
        let dyn_transport: Arc<dyn McpTransport> = transport.clone();
        let tools = build_ok("jira", vec![spec("get_issue")], dyn_transport);
        let tool = &tools[0];
        let outcome = tool.execute(json!({"id": "ISSUE-1"}), &allow_ctx()).await;
        assert!(outcome.ok);
        assert!(
            outcome.untrusted,
            "an MCP result must be labeled untrusted (Design §4.15)"
        );
        let last_call = transport
            .last_call
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let Some((method, params)) = last_call.as_ref() else {
            panic!("expected a call to have been made");
        };
        assert_eq!(method, "tools/call");
        assert_eq!(params["name"], "get_issue");
        assert_eq!(params["arguments"]["id"], "ISSUE-1");
    }

    #[tokio::test]
    async fn execute_transport_failure_is_a_structured_failure_not_a_panic() {
        let transport: Arc<dyn McpTransport> = Arc::new(FakeTransport::err(McpError::Connect(
            "process exited".into(),
        )));
        let tools = build_ok("jira", vec![spec("get_issue")], transport);
        let outcome = tools[0].execute(json!({}), &allow_ctx()).await;
        assert!(!outcome.ok);
        assert!(
            outcome.content.contains("process exited"),
            "{}",
            outcome.content
        );
    }

    #[tokio::test]
    async fn execute_is_permission_gated_a_denial_is_structured_never_a_call() {
        // Tech Spec §5.6: an MCP tool call is permission-gated exactly like a
        // built-in's — a denial returns a structured failure and the
        // transport is never reached at all (HC-6, no privileged path).
        let transport = Arc::new(FakeTransport::ok(json!({"should": "never happen"})));
        let dyn_transport: Arc<dyn McpTransport> = transport.clone();
        let tools = build_ok("jira", vec![spec("get_issue")], dyn_transport);
        let outcome = tools[0].execute(json!({}), &deny_ctx()).await;
        assert!(!outcome.ok);
        assert!(
            outcome.content.to_lowercase().contains("denied"),
            "{}",
            outcome.content
        );
        assert!(
            transport
                .last_call
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_none(),
            "a denied call must never reach the transport"
        );
    }

    #[test]
    fn describe_names_the_original_tool_and_its_server() {
        let transport: Arc<dyn McpTransport> = Arc::new(FakeTransport::ok(json!({})));
        let tools = build_ok("jira", vec![spec("get_issue")], transport);
        assert_eq!(
            tools[0].describe(&json!({})),
            Some("get_issue via jira".to_string())
        );
    }
}
