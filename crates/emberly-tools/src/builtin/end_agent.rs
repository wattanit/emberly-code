//! `end_agent` (T-21): explicitly terminate a subagent and free its
//! resources before the owning session itself ends (FR-9). Every subagent
//! still alive when the session ends is ended with it — no subagent
//! outlives its session.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::ctx::ToolCtx;
use crate::subagent::SubagentEndOutcome;
use crate::tool::{Tool, ToolOutcome, ToolSpec};

#[derive(Deserialize)]
struct EndAgentArgs {
    id: String,
}

/// The `end_agent` tool.
pub struct EndAgentTool;

impl EndAgentTool {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for EndAgentTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for EndAgentTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "end_agent".into(),
            description: "Explicitly end a subagent and free its resources, once you no longer \
                 need it. Ending an id that is unknown or already ended returns a structured \
                 failure naming the reason; you don't need to end every subagent yourself — any \
                 still alive when this session ends are ended with it."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "The subagent's id, as returned by spawn_agents."
                    }
                },
                "required": ["id"]
            }),
        }
    }

    fn describe(&self, args: &Value) -> Option<String> {
        let args: EndAgentArgs = serde_json::from_value(args.clone()).ok()?;
        Some(format!("end_agent · {}", args.id))
    }

    async fn execute(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let args: EndAgentArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolOutcome::invalid_args(&self.spec().name, &e),
        };

        match ctx.end_agent(args.id.clone()).await {
            Ok(SubagentEndOutcome::Ended) => ToolOutcome::success(
                format!("Ended subagent '{}'.", args.id),
                format!("agent · {} · ended", args.id),
            ),
            Ok(SubagentEndOutcome::NotFound) => ToolOutcome::failure(
                format!(
                    "No alive subagent with id '{}'. It may not exist, or it has already ended.",
                    args.id
                ),
                "no such agent",
            ),
            Err(_) => ToolOutcome::failure(
                "Subagents are not available (the engine is unreachable or the multi-agent \
                 subsystem is disabled)."
                    .to_string(),
                "agents unavailable",
            ),
        }
    }
}
