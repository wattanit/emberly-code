//! `grep` (T-6): search file contents under the project root for a regex, a
//! first-party wrapper over the ripgrep libraries (`grep-searcher` +
//! `grep-regex`, walked by `ignore`). Root-confined, skips `.git/`, and honors
//! `.gitignore`. Read-only and root-scoped, so it runs without a prompt.

use async_trait::async_trait;
use grep_regex::RegexMatcher;
use grep_searcher::sinks::UTF8;
use grep_searcher::SearcherBuilder;
use ignore::WalkBuilder;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::ctx::ToolCtx;
use crate::path::{display_relative, resolve_in_root};
use crate::tool::{Tool, ToolOutcome, ToolSpec};

/// Cap on matching lines returned; the engine also truncates at ingestion.
const MAX_MATCHES: usize = 1000;

#[derive(Deserialize)]
struct GrepArgs {
    /// The regular expression to search for (Rust `regex` syntax).
    pattern: String,
    /// Optional subdirectory (relative to the root) to search under; defaults
    /// to the whole project root.
    #[serde(default)]
    path: Option<String>,
}

/// The `grep` tool.
pub struct GrepTool;

#[async_trait]
impl Tool for GrepTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "grep".into(),
            description: "Search file contents under the project root for a regular expression, \
                          reporting `path:line:text`. Skips .git/ and gitignored files. \
                          Read-only and root-scoped, so it runs without a permission prompt — \
                          prefer it over shelling out to `grep`/`rg` via bash."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Regular expression (Rust regex syntax)." },
                    "path": { "type": "string", "description": "Optional subdirectory (relative to the project root) to search under; defaults to the root." }
                },
                "required": ["pattern"]
            }),
        }
    }

    fn describe(&self, args: &Value) -> Option<String> {
        serde_json::from_value::<GrepArgs>(args.clone())
            .ok()
            .map(|a| format!("grep {}", a.pattern))
    }

    async fn execute(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let args: GrepArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolOutcome::failure(format!("invalid arguments: {e}"), "bad args"),
        };

        let matcher = match RegexMatcher::new(&args.pattern) {
            Ok(m) => m,
            Err(e) => return ToolOutcome::failure(format!("invalid regex: {e}"), "bad pattern"),
        };

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

        let mut out = String::new();
        let mut matches: usize = 0;
        let mut truncated = false;
        let mut searcher = SearcherBuilder::new().line_number(true).build();
        let root = ctx.project_root().to_path_buf();

        let walker = WalkBuilder::new(&base)
            .hidden(false)
            // Honor .gitignore whether or not a .git dir is present.
            .require_git(false)
            .filter_entry(|entry| entry.file_name() != ".git")
            .build();
        'walk: for entry in walker {
            let Ok(entry) = entry else { continue };
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            if matches >= MAX_MATCHES {
                truncated = true;
                break;
            }
            let rel = display_relative(&root, entry.path());
            let result = searcher.search_path(
                &matcher,
                entry.path(),
                UTF8(|line_number, line| {
                    out.push_str(&format!("{rel}:{line_number}:{line}"));
                    matches += 1;
                    // Stop this file (and, via the outer guard, the walk) at the cap.
                    Ok(matches < MAX_MATCHES)
                }),
            );
            // A per-file search error (e.g. a permission quirk) is skipped, not
            // fatal — the rest of the tree is still searched.
            if result.is_err() {
                continue;
            }
            if matches >= MAX_MATCHES {
                truncated = true;
                break 'walk;
            }
        }

        if truncated {
            out.push_str(&format!("… (truncated at {MAX_MATCHES} matches)\n"));
        }
        if matches == 0 {
            return ToolOutcome::success(
                format!("no matches for {}", args.pattern),
                "grep: 0 matches",
            );
        }
        ToolOutcome::success(out, format!("grep: {matches} match(es)"))
    }
}
