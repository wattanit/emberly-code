//! `scratch_write` (T-17, FR-8): write a temporary file into the session's
//! disposable scratch space. Harness-managed persistence (like the transcript
//! and memory store, not an agent filesystem write) — the model supplies
//! content fields, never a path; the engine derives the real path within the
//! fixed per-session scratch directory. It does not widen HC-4 and is **not
//! permission-gated** (§6, FR-8 honesty clause).

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::ctx::ToolCtx;
use crate::scratch::{validate_name, ScratchOutcome, ScratchRequest};
use crate::tool::{Tool, ToolOutcome, ToolSpec};

#[derive(Deserialize)]
struct ScratchArgs {
    name: String,
    content: String,
}

/// The `scratch_write` tool.
pub struct ScratchWriteTool;

impl ScratchWriteTool {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for ScratchWriteTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ScratchWriteTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "scratch_write".into(),
            description: "Write a temporary file into this session's disposable scratch \
                 space: a working directory for scripts, intermediate output, or notes that \
                 are not part of the project itself. Supply content fields, never a path — the \
                 harness resolves the real location. Never committed to the user's version \
                 control, never auto-deleted, and not permission-gated. Read a scratch file \
                 back with read_file once you know the name you used."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "File name within the scratch directory — a flat name \
                            (no paths), e.g. 'analysis.py'. Extension and case are preserved."
                    },
                    "content": {
                        "type": "string",
                        "description": "The full file content. Creates or replaces the file."
                    }
                },
                "required": ["name", "content"]
            }),
        }
    }

    fn describe(&self, args: &Value) -> Option<String> {
        let args: ScratchArgs = serde_json::from_value(args.clone()).ok()?;
        Some(format!("scratch · {}", args.name))
    }

    async fn execute(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let args: ScratchArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolOutcome::failure(format!("invalid arguments: {e}"), "bad args"),
        };

        // Early name validation (HC-4 boundary): reject path escapes before
        // reaching the engine.
        if let Err(e) = validate_name(&args.name) {
            return ToolOutcome::failure(e.0, "invalid name");
        }

        let req = ScratchRequest {
            name: args.name,
            content: args.content,
        };

        match ctx.scratch_write(req).await {
            Ok(ScratchOutcome::Written { name, bytes }) => ToolOutcome::success(
                format!("Wrote {bytes} bytes to scratch/{name}."),
                format!("scratched · {name} · {bytes}B written"),
            ),
            Ok(ScratchOutcome::Rejected { reason }) => ToolOutcome::failure(reason.clone(), reason),
            Err(_) => ToolOutcome::failure(
                "Scratch space could not be written (the engine is unreachable).",
                "engine unreachable",
            ),
        }
    }
}
