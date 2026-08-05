//! `edit_file` (T-3): exact string match-and-replace (not line-number based).
//! Failure messages distinguish *no match* from *N matches found*, and the
//! no-match case includes the closest existing line as a hint — model recovery
//! quality depends on this precision.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::ctx::ToolCtx;
use crate::diff::{closest_region_hint, line_deltas, unified_diff};
use crate::path::{display_relative, is_under_git_dir, resolve_in_root};
use crate::permission::PermissionRequest;
use crate::tool::{FileChange, Tool, ToolOutcome, ToolSpec};

#[derive(Deserialize)]
struct EditArgs {
    path: String,
    old_string: String,
    new_string: String,
    #[serde(default)]
    replace_all: bool,
}

/// The `edit_file` tool.
pub struct EditFileTool;

#[async_trait]
impl Tool for EditFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "edit_file".into(),
            description: "Replace an exact string in a file. By default `old_string` must match \
                          exactly once; set `replace_all` to replace every occurrence."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path, relative to the project root or absolute." },
                    "old_string": { "type": "string", "description": "Exact text to find." },
                    "new_string": { "type": "string", "description": "Replacement text." },
                    "replace_all": { "type": "boolean", "description": "Replace all occurrences (default false)." }
                },
                "required": ["path", "old_string", "new_string"]
            }),
        }
    }

    fn describe(&self, args: &Value) -> Option<String> {
        serde_json::from_value::<EditArgs>(args.clone())
            .ok()
            .map(|a| format!("edit {}", a.path))
    }

    async fn execute(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let args: EditArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolOutcome::invalid_args(&self.spec().name, &e),
        };

        if args.old_string.is_empty() {
            return ToolOutcome::failure("old_string must not be empty", "bad args");
        }

        let resolved = match resolve_in_root(ctx.project_root(), &args.path) {
            Ok(r) => r,
            Err(e) => return ToolOutcome::failure(e, "path error"),
        };

        if is_under_git_dir(ctx.project_root(), &resolved.path) {
            return ToolOutcome::failure(
                format!(
                    "refusing to edit under .git/: {} (only git may modify git data)",
                    args.path
                ),
                "refused: .git/",
            );
        }

        let content = match tokio::fs::read_to_string(&resolved.path).await {
            Ok(c) => c,
            Err(e) => {
                return ToolOutcome::failure(
                    format!("cannot read {}: {e}", args.path),
                    "read failed",
                )
            }
        };

        let matches = content.matches(&args.old_string).count();
        match matches {
            0 => {
                let mut message = format!("no match for old_string in {}", args.path);
                if let Some(hint) = closest_region_hint(&content, &args.old_string) {
                    message.push_str(&format!("\n{hint}"));
                }
                return ToolOutcome::failure(message, "no match");
            }
            n if n > 1 && !args.replace_all => {
                return ToolOutcome::failure(
                    format!(
                        "found {n} matches for old_string in {}; add surrounding context to make \
                         it unique, or set replace_all=true",
                        args.path
                    ),
                    format!("{n} matches"),
                );
            }
            _ => {}
        }

        let new_content = if args.replace_all {
            content.replace(&args.old_string, &args.new_string)
        } else {
            content.replacen(&args.old_string, &args.new_string, 1)
        };

        let rel = display_relative(ctx.project_root(), &resolved.path);
        let request = PermissionRequest {
            tool: "edit_file".into(),
            summary: format!("edit {rel}"),
            detail: unified_diff(&rel, &content, &new_content),
            affected_paths: vec![resolved.path.clone()],
            outside_root: resolved.outside_root,
        };
        if !ctx.authorize(request).await.is_allowed() {
            return ToolOutcome::denied(&format!("edit {}", args.path));
        }

        if let Err(e) = tokio::fs::write(&resolved.path, &new_content).await {
            return ToolOutcome::failure(
                format!("cannot write {}: {e}", args.path),
                "write failed",
            );
        }

        let (adds, dels) = line_deltas(&content, &new_content);
        let diff = unified_diff(&rel, &content, &new_content);
        ToolOutcome::success(
            format!("Edited {rel} (+{adds} -{dels})."),
            format!("edited {rel} (+{adds} -{dels})"),
        )
        .with_file_change(FileChange {
            path: rel,
            adds,
            dels,
            diff: Some(diff),
        })
    }
}
