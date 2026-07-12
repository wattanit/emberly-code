//! `recall` (T-10): retrieve turns that were elided from the sent context by
//! the adaptive window (FR-3). A pure engine round trip (Tech Spec §5.2) — it
//! touches no filesystem or network, so it bypasses the sandbox and is **not
//! permission-gated** (§6). The engine returns the normalized, reduced messages
//! for the requested range — never raw JSONL — so recall costs tokens
//! proportional to what is recalled, not the transcript's raw size (T-10).

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::ctx::ToolCtx;
use crate::recall::RecallOutcome;
use crate::tool::{Tool, ToolOutcome, ToolSpec};

#[derive(Deserialize)]
struct RecallArgs {
    /// The first turn number to recall (inclusive), as shown in the elision
    /// marker (e.g. `turns 5–16 elided`).
    from_turn: usize,
    /// The last turn number to recall (inclusive). If omitted, only the single
    /// turn `from_turn` is recalled.
    #[serde(default)]
    to_turn: Option<usize>,
}

/// The `recall` tool.
pub struct RecallTool;

#[async_trait]
impl Tool for RecallTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "recall".into(),
            description: "Retrieve earlier conversation turns that were elided from the current \
                 context window. Use the turn numbers from the elision marker (e.g. if the \
                 marker says 'turns 5–16 elided', request those turns). Returns the turns in \
                 reduced form — enough to understand what happened without the full raw output. \
                 This is not file I/O; it reads the session's own memory, so it is always \
                 available and never needs permission."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "from_turn": {
                        "type": "integer",
                        "description": "The first turn number to recall (inclusive)."
                    },
                    "to_turn": {
                        "type": "integer",
                        "description": "The last turn number to recall (inclusive). \
                                        If omitted, only from_turn is recalled."
                    }
                },
                "required": ["from_turn"]
            }),
        }
    }

    fn describe(&self, args: &Value) -> Option<String> {
        serde_json::from_value::<RecallArgs>(args.clone())
            .ok()
            .map(|a| match a.to_turn {
                Some(to) if to != a.from_turn => format!("recall turns {}–{}", a.from_turn, to),
                _ => format!("recall turn {}", a.from_turn),
            })
    }

    async fn execute(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let args: RecallArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolOutcome::failure(format!("invalid arguments: {e}"), "bad args"),
        };

        let to = args.to_turn.unwrap_or(args.from_turn);

        match ctx.recall(args.from_turn, to).await {
            RecallOutcome::Turns { content, count } => {
                ToolOutcome::success(content, format!("recalled {count} turns"))
            }
            // An empty recall is a valid outcome — the model reads it and
            // proceeds (HC-6). The range may have been retired by compaction.
            RecallOutcome::Empty => ToolOutcome::success(
                "No turns found in the requested range. They may have been \
                 compacted into a summary, or the range is out of bounds.",
                "no turns found",
            ),
        }
    }
}
