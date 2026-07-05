//! `read_file` (T-1): read a UTF-8 text file within the project root. Reads
//! inside the root are allowed without a prompt (Requirements §6.2); a read
//! that resolves outside the root requires an explicit per-action approval
//! (HC-4). Full content is returned; the engine truncates at ingestion.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::ctx::ToolCtx;
use crate::path::{display_relative, resolve_in_root};
use crate::permission::PermissionRequest;
use crate::tool::{Tool, ToolOutcome, ToolSpec};

#[derive(Deserialize)]
struct ReadArgs {
    path: String,
}

/// The `read_file` tool.
pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "read_file".into(),
            description: "Read a UTF-8 text file within the project root and return its contents."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path, relative to the project root or absolute."
                    }
                },
                "required": ["path"]
            }),
        }
    }

    async fn execute(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let args: ReadArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolOutcome::failure(format!("invalid arguments: {e}"), "bad args"),
        };

        let resolved = match resolve_in_root(ctx.project_root(), &args.path) {
            Ok(r) => r,
            Err(e) => return ToolOutcome::failure(e, "path error"),
        };

        // HC-4: reading outside the project root requires explicit approval.
        if resolved.outside_root {
            let request = PermissionRequest {
                tool: "read_file".into(),
                summary: format!("read {} (outside project root)", args.path),
                detail: format!(
                    "Read a file OUTSIDE the project root:\n{}",
                    resolved.path.display()
                ),
                affected_paths: vec![resolved.path.clone()],
                outside_root: true,
            };
            if !ctx.authorize(request).await.is_allowed() {
                return ToolOutcome::denied(&format!("read {}", args.path));
            }
        }

        match tokio::fs::read_to_string(&resolved.path).await {
            Ok(content) => {
                let lines = content.lines().count();
                let rel = display_relative(ctx.project_root(), &resolved.path);
                ToolOutcome::success(content, format!("read {rel} ({lines} lines)"))
            }
            Err(e) => {
                ToolOutcome::failure(format!("cannot read {}: {e}", args.path), "read failed")
            }
        }
    }
}
