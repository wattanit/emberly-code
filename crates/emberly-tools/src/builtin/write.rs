//! `write_file` (T-2): create or replace a file within the project root.
//! Refuses any path under `.git/` at the tool layer (HC-5), independent of the
//! OS sandbox. Creates parent directories only inside the root. Always asks
//! for permission (writes inside the root: ask, per Requirements §6.2).

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::ctx::ToolCtx;
use crate::diff::{line_deltas, unified_diff};
use crate::path::{display_relative, is_under_git_dir, resolve_in_root};
use crate::permission::PermissionRequest;
use crate::tool::{FileChange, Tool, ToolOutcome, ToolSpec};

#[derive(Deserialize)]
struct WriteArgs {
    path: String,
    content: String,
}

/// The `write_file` tool.
pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "write_file".into(),
            description: "Create a new file or replace an existing file's contents within the \
                          project root."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path, relative to the project root or absolute." },
                    "content": { "type": "string", "description": "Full file contents to write." }
                },
                "required": ["path", "content"]
            }),
        }
    }

    async fn execute(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let args: WriteArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolOutcome::failure(format!("invalid arguments: {e}"), "bad args"),
        };

        let resolved = match resolve_in_root(ctx.project_root(), &args.path) {
            Ok(r) => r,
            Err(e) => return ToolOutcome::failure(e, "path error"),
        };

        // HC-5: hard refusal, no ask option.
        if is_under_git_dir(&resolved.path) {
            return ToolOutcome::failure(
                format!(
                    "refusing to write under .git/: {} (only git may modify git data)",
                    args.path
                ),
                "refused: .git/",
            );
        }

        let old = tokio::fs::read_to_string(&resolved.path)
            .await
            .unwrap_or_default();
        let rel = display_relative(ctx.project_root(), &resolved.path);

        let detail = if old.is_empty() {
            format!("Create {rel} ({} lines)", args.content.lines().count())
        } else {
            unified_diff(&rel, &old, &args.content)
        };
        let request = PermissionRequest {
            tool: "write_file".into(),
            summary: format!("write {rel}"),
            detail,
            affected_paths: vec![resolved.path.clone()],
            outside_root: resolved.outside_root,
        };
        if !ctx.authorize(request).await.is_allowed() {
            return ToolOutcome::denied(&format!("write {}", args.path));
        }

        if let Some(parent) = resolved.path.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return ToolOutcome::failure(
                    format!("cannot create parent directories for {}: {e}", args.path),
                    "write failed",
                );
            }
        }

        if let Err(e) = tokio::fs::write(&resolved.path, &args.content).await {
            return ToolOutcome::failure(
                format!("cannot write {}: {e}", args.path),
                "write failed",
            );
        }

        let (adds, dels) = line_deltas(&old, &args.content);
        ToolOutcome::success(
            format!("Wrote {rel} (+{adds} -{dels})."),
            format!("wrote {rel} (+{adds} -{dels})"),
        )
        .with_file_change(FileChange {
            path: rel,
            adds,
            dels,
        })
    }
}
