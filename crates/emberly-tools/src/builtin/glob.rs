//! `glob` (T-5): list files under the project root matching a glob pattern.
//! Root-confined, skips `.git/`, and honors `.gitignore` by default (via the
//! `ignore` walker). Read-only and root-scoped, so — like an in-root read — it
//! runs without a permission prompt.

use async_trait::async_trait;
use globset::Glob;
use ignore::WalkBuilder;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::ctx::ToolCtx;
use crate::path::{display_relative, resolve_in_root};
use crate::tool::{Tool, ToolOutcome, ToolSpec};

/// Cap the number of paths returned so an over-broad pattern can't flood the
/// context; the engine also truncates at ingestion. Truncation is flagged.
const MAX_MATCHES: usize = 1000;

#[derive(Deserialize)]
struct GlobArgs {
    pattern: String,
    /// Optional subdirectory (relative to the root) to search under; the
    /// pattern is matched against paths relative to it. Defaults to the root.
    #[serde(default)]
    path: Option<String>,
}

/// The `glob` tool.
pub struct GlobTool;

#[async_trait]
impl Tool for GlobTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "glob".into(),
            description: "Find files under the project root whose path matches a glob pattern \
                          (e.g. `**/*.rs`, `src/**/mod.rs`). Skips .git/ and gitignored files."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Glob pattern, matched against paths relative to the search directory. Supports `*`, `?`, `**`, `[...]`." },
                    "path": { "type": "string", "description": "Optional subdirectory (relative to the project root) to search under; defaults to the root." }
                },
                "required": ["pattern"]
            }),
        }
    }

    fn describe(&self, args: &Value) -> Option<String> {
        serde_json::from_value::<GlobArgs>(args.clone())
            .ok()
            .map(|a| format!("glob {}", a.pattern))
    }

    async fn execute(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let args: GlobArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolOutcome::failure(format!("invalid arguments: {e}"), "bad args"),
        };

        let matcher = match Glob::new(&args.pattern) {
            Ok(glob) => glob.compile_matcher(),
            Err(e) => {
                return ToolOutcome::failure(format!("invalid glob pattern: {e}"), "bad pattern")
            }
        };

        // Resolve (and confine) the search base. `.` searches the whole root.
        let base = match resolve_in_root(ctx.project_root(), args.path.as_deref().unwrap_or(".")) {
            Ok(r) if r.outside_root => {
                return ToolOutcome::failure(
                    "refusing to search outside the project root",
                    "outside root",
                )
            }
            Ok(r) => r.path,
            Err(e) => return ToolOutcome::failure(e, "path error"),
        };

        let mut matches: Vec<String> = Vec::new();
        let mut truncated = false;
        // hidden(false) so dotfiles are searchable, but prune `.git/` explicitly
        // (HC-5 belt-and-braces); `.gitignore` is honored by default.
        let walker = WalkBuilder::new(&base)
            .hidden(false)
            // Honor .gitignore whether or not a .git dir is present.
            .require_git(false)
            .filter_entry(|entry| entry.file_name() != ".git")
            .build();
        for entry in walker {
            let Ok(entry) = entry else { continue };
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            // Match the pattern against the path relative to the search base.
            let rel_to_base = entry.path().strip_prefix(&base).unwrap_or(entry.path());
            if matcher.is_match(rel_to_base) {
                if matches.len() >= MAX_MATCHES {
                    truncated = true;
                    break;
                }
                matches.push(display_relative(ctx.project_root(), entry.path()));
            }
        }

        matches.sort();
        let count = matches.len();
        let mut body = matches.join("\n");
        if truncated {
            body.push_str(&format!("\n… (truncated at {MAX_MATCHES} matches)"));
        }
        if count == 0 {
            body = format!("no files match {}", args.pattern);
        }
        ToolOutcome::success(body, format!("glob: {count} match(es)"))
    }
}
