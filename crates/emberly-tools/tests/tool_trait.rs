//! Coverage for the tool trait, context, gate, and registry (Phase 1,
//! group 3). Uses a dummy `EchoTool` and allow/deny fake gates — the real
//! tools arrive in group 4.
//!
//! No `.unwrap()`/`.expect()`: async tests return `()` and assert directly.

use std::sync::Arc;

use async_trait::async_trait;
use emberly_tools::{
    PermissionGate, PermissionOutcome, PermissionRequest, PlainSandbox, Sandbox, Tool, ToolCtx,
    ToolOutcome, ToolRegistry, ToolSpec, TruncateConfig,
};
use serde_json::{json, Value};

/// A tool that authorizes, then echoes its `text` argument.
struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "echo".into(),
            description: "Echo the text argument back.".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "required": ["text"]
            }),
        }
    }

    async fn execute(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let request = PermissionRequest {
            tool: "echo".into(),
            summary: "echo text".into(),
            detail: "echo the provided text".into(),
            affected_paths: vec![],
            outside_root: false,
        };
        if !ctx.authorize(request).await.is_allowed() {
            return ToolOutcome::denied("echo the text");
        }
        match args.get("text").and_then(Value::as_str) {
            Some(text) => ToolOutcome::success(text.to_string(), "echoed"),
            None => ToolOutcome::failure("missing required argument: text", "bad args"),
        }
    }
}

struct AllowGate;
#[async_trait]
impl PermissionGate for AllowGate {
    async fn authorize(&self, _request: PermissionRequest) -> PermissionOutcome {
        PermissionOutcome::Allow
    }
}

struct DenyGate;
#[async_trait]
impl PermissionGate for DenyGate {
    async fn authorize(&self, _request: PermissionRequest) -> PermissionOutcome {
        PermissionOutcome::Deny
    }
}

fn ctx_with(gate: Arc<dyn PermissionGate>) -> ToolCtx {
    let sandbox: Arc<dyn Sandbox> = Arc::new(PlainSandbox);
    ToolCtx::new("/proj", TruncateConfig::default(), gate, sandbox)
}

#[tokio::test]
async fn allowed_tool_runs_and_succeeds() {
    let ctx = ctx_with(Arc::new(AllowGate));
    let outcome = EchoTool.execute(json!({ "text": "สวัสดี" }), &ctx).await;
    assert!(outcome.ok);
    assert_eq!(outcome.content, "สวัสดี");
    assert_eq!(outcome.summary, "echoed");
}

#[tokio::test]
async fn denied_tool_returns_failure_outcome_not_error() {
    // HC-6 / §6.6: a denial is structured data (ok == false), never a crash.
    let ctx = ctx_with(Arc::new(DenyGate));
    let outcome = EchoTool.execute(json!({ "text": "hi" }), &ctx).await;
    assert!(!outcome.ok);
    assert!(outcome.content.contains("denied"));
    assert_eq!(outcome.summary, "denied by user");
}

#[tokio::test]
async fn tool_failure_is_data() {
    // Bad arguments → a failure outcome the model can read, not an Err.
    let ctx = ctx_with(Arc::new(AllowGate));
    let outcome = EchoTool.execute(json!({ "wrong": 1 }), &ctx).await;
    assert!(!outcome.ok);
    assert!(outcome.content.contains("text"));
}

#[test]
fn registry_registers_looks_up_and_lists_specs() {
    let mut registry = ToolRegistry::new();
    assert!(registry.is_empty());
    registry.register(Arc::new(EchoTool));

    assert_eq!(registry.len(), 1);
    assert!(registry.get("echo").is_some());
    assert!(registry.get("nonexistent").is_none());

    let specs = registry.specs();
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].name, "echo");
    assert_eq!(registry.names(), vec!["echo".to_string()]);
}

#[test]
fn registry_replaces_same_name() {
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool));
    registry.register(Arc::new(EchoTool));
    assert_eq!(registry.len(), 1, "same name replaces, not duplicates");
}

#[test]
fn ctx_exposes_root_and_truncate_config() {
    let ctx = ctx_with(Arc::new(AllowGate));
    assert_eq!(ctx.project_root().to_string_lossy(), "/proj");
    assert_eq!(ctx.truncate_config().max_lines, 400);
    assert_eq!(ctx.truncate_config().head_lines, 150);
}
