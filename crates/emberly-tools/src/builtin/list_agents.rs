//! `list_agents` (T-20): enumerate currently alive subagents — situational
//! awareness when an id has fallen out of the working context window (FR-3)
//! or is simply forgotten, the multi-agent analogue of the task list (T-11).

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::ctx::ToolCtx;
use crate::subagent::SubagentStatus;
use crate::tool::{Tool, ToolOutcome, ToolSpec};

/// The `list_agents` tool.
pub struct ListAgentsTool;

impl ListAgentsTool {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for ListAgentsTool {
    fn default() -> Self {
        Self::new()
    }
}

fn status_label(status: SubagentStatus) -> &'static str {
    match status {
        SubagentStatus::Running => "running",
        SubagentStatus::AwaitingPermission => "awaiting permission",
        SubagentStatus::Done => "done",
        SubagentStatus::TimedOut => "timed out",
        SubagentStatus::Error => "error",
    }
}

#[async_trait]
impl Tool for ListAgentsTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "list_agents".into(),
            description: "List every currently alive subagent, with its id, name, and status. \
                 Use this if you've lost track of a subagent's id, or to check on delegated \
                 work before deciding whether to message or end an agent."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
        }
    }

    fn describe(&self, _args: &Value) -> Option<String> {
        Some("list_agents".into())
    }

    async fn execute(&self, _args: Value, ctx: &ToolCtx) -> ToolOutcome {
        match ctx.list_agents().await {
            Ok(entries) if entries.is_empty() => {
                ToolOutcome::success("No subagents are currently alive.", "no agents alive")
            }
            Ok(entries) => {
                let lines: Vec<String> = entries
                    .iter()
                    .map(|e| format!("{} ({}): {}", e.id, e.name, status_label(e.status)))
                    .collect();
                ToolOutcome::success(
                    lines.join("\n"),
                    format!("{} agent(s) alive", entries.len()),
                )
            }
            Err(_) => ToolOutcome::failure(
                "Subagents are not available (the engine is unreachable or the multi-agent \
                 subsystem is disabled)."
                    .to_string(),
                "agents unavailable",
            ),
        }
    }
}
