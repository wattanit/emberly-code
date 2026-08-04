//! `read_file` (T-1): read a UTF-8 text file within the project root. Reads
//! inside the root are allowed without a prompt (Requirements §6.2); a read
//! that resolves outside the root requires an explicit per-action approval
//! (HC-4). Full content is returned; the engine truncates at ingestion.
//!
//! An optional line range (`start_line`/`end_line`, 1-based inclusive) reads
//! just a slice — the prompt-free equivalent of `sed -n 'M,Np'`, so the model
//! need not shell out (and prompt) merely to read part of a large file.

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
    /// 1-based first line to read (inclusive). When omitted, read from the top.
    #[serde(default)]
    start_line: Option<usize>,
    /// 1-based last line to read (inclusive). When omitted, read to EOF.
    #[serde(default)]
    end_line: Option<usize>,
}

/// The `read_file` tool.
pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "read_file".into(),
            description: "Read a UTF-8 text file within the project root. Returns the full \
                          contents by default; pass start_line/end_line (1-based, inclusive) to \
                          read a slice with line numbers — prefer this over `sed -n` so the read \
                          needs no approval."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path, relative to the project root or absolute."
                    },
                    "start_line": {
                        "type": "integer",
                        "description": "Optional 1-based first line to read (inclusive)."
                    },
                    "end_line": {
                        "type": "integer",
                        "description": "Optional 1-based last line to read (inclusive)."
                    }
                },
                "required": ["path"]
            }),
        }
    }

    fn describe(&self, args: &Value) -> Option<String> {
        serde_json::from_value::<ReadArgs>(args.clone())
            .ok()
            .map(|a| match (a.start_line, a.end_line) {
                (Some(s), Some(e)) => format!("read {} (lines {s}-{e})", a.path),
                (Some(s), None) => format!("read {} (from line {s})", a.path),
                (None, Some(e)) => format!("read {} (to line {e})", a.path),
                (None, None) => format!("read {}", a.path),
            })
    }

    async fn execute(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let args: ReadArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolOutcome::invalid_args(&self.spec().name, &e),
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
                let total = content.lines().count();
                let rel = display_relative(ctx.project_root(), &resolved.path);
                match (args.start_line, args.end_line) {
                    (None, None) => {
                        ToolOutcome::success(content, format!("read {rel} ({total} lines)"))
                    }
                    range => {
                        let (start, end) = resolve_range(range, total);
                        if start > total {
                            return ToolOutcome::failure(
                                format!(
                                    "{rel} has {total} lines — start_line {} is past the end",
                                    start
                                ),
                                "range out of bounds",
                            );
                        }
                        let body = slice_lines(&content, start, end);
                        ToolOutcome::success(
                            body,
                            format!("read {rel} (lines {start}-{end} of {total})"),
                        )
                    }
                }
            }
            Err(e) => {
                ToolOutcome::failure(format!("cannot read {}: {e}", args.path), "read failed")
            }
        }
    }
}

/// Clamp a (start, end) pair to a 1-based inclusive range over `total` lines,
/// defaulting a missing bound to the top or bottom and clamping the end to EOF.
fn resolve_range(range: (Option<usize>, Option<usize>), total: usize) -> (usize, usize) {
    let (start, end) = range;
    let start = start.unwrap_or(1).max(1);
    let end = end.unwrap_or(total).max(start).min(total);
    (start, end)
}

/// Extract lines `start..=end` (1-based) from `content`, prefixing each with
/// its line number and a tab — the shape `sed -n '{start},{end}p'` with `=`
/// would approximate, so a ranged read is familiar to a model reaching for sed.
fn slice_lines(content: &str, start: usize, end: usize) -> String {
    let mut out = String::new();
    for (i, line) in content.lines().enumerate() {
        let n = i + 1;
        if n < start {
            continue;
        }
        if n > end {
            break;
        }
        out.push_str(&format!("{n:>6}\t{line}"));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_range_defaults_and_clamps() {
        // Missing bounds default to top/bottom.
        assert_eq!(resolve_range((None, None), 10), (1, 10));
        assert_eq!(resolve_range((Some(3), None), 10), (3, 10));
        assert_eq!(resolve_range((None, Some(4)), 10), (1, 4));
        // End clamps to total; start clamps to >= 1.
        assert_eq!(resolve_range((Some(8), Some(99)), 10), (8, 10));
        assert_eq!(resolve_range((Some(0), None), 10), (1, 10));
        // End never precedes start.
        assert_eq!(resolve_range((Some(5), Some(2)), 10), (5, 5));
    }

    #[test]
    fn slice_lines_numbers_and_trims_to_range() {
        let content = "a\nb\nc\nd\ne\n";
        let out = slice_lines(content, 2, 4);
        assert_eq!(out, "     2\tb\n     3\tc\n     4\td\n");
    }
}
