//! `todo` (T-11): set the model-maintained task list — an explicit, ordered,
//! user-visible list of multi-step work items. A pure engine round trip (Tech
//! Spec §5.2) — it touches no filesystem or network, so it bypasses the sandbox
//! and is **not permission-gated** (§6). The model always sends the **full
//! list** (replace, not merge); the engine stores it, emits it to frontends,
//! and records it in the transcript (HC-7).

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::ctx::ToolCtx;
use crate::task_list::{TaskItem, TaskStatus};
use crate::tool::{Tool, ToolOutcome, ToolSpec};

#[derive(Deserialize)]
struct TodoArgs {
    /// The full task list (replace semantics, Tech Spec §5.2).
    items: Vec<TaskItem>,
}

/// The `todo` tool.
pub struct TaskListTool;

impl TaskListTool {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for TaskListTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for TaskListTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "todo".into(),
            description: "Maintain an explicit, ordered task list for multi-step work. Send the \
                 FULL list every time — the list is replaced, not merged. Each item has a status: \
                 'pending', 'in_progress', or 'done'. Keep at most one item 'in_progress' at a \
                 time; mark items 'done' as you complete them. The user sees this list, so keep \
                 it current. This is session state, not persistent memory."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "items": {
                        "type": "array",
                        "description": "The complete task list. Sending this replaces the entire \
                                        previous list.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "text": {
                                    "type": "string",
                                    "description": "What needs to be done."
                                },
                                "status": {
                                    "type": "string",
                                    "enum": ["pending", "in_progress", "done"],
                                    "description": "Current status of this item."
                                }
                            },
                            "required": ["text", "status"]
                        }
                    }
                },
                "required": ["items"]
            }),
        }
    }

    fn describe(&self, args: &Value) -> Option<String> {
        let args: TodoArgs = serde_json::from_value(args.clone()).ok()?;
        let done = args
            .items
            .iter()
            .filter(|i| i.status == TaskStatus::Done)
            .count();
        if args.items.is_empty() {
            Some("task list cleared".into())
        } else {
            Some(format!("task list: {} items, {} done", args.items.len(), done))
        }
    }

    async fn execute(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let args: TodoArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolOutcome::failure(format!("invalid arguments: {e}"), "bad args"),
        };

        let count = args.items.len();
        let in_progress = args
            .items
            .iter()
            .filter(|i| i.status == TaskStatus::InProgress)
            .count();

        match ctx.set_task_list(args.items).await {
            Ok(()) => {
                let summary = if count == 0 {
                    "task list cleared".to_string()
                } else {
                    format!("task list: {count} items, {in_progress} in progress")
                };
                ToolOutcome::success(summary.clone(), summary)
            }
            Err(_) => ToolOutcome::failure(
                "The task list could not be updated (the engine is unreachable).",
                "engine unreachable",
            ),
        }
    }
}
