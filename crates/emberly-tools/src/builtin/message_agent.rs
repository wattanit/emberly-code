//! `message_agent` (T-19): send a further prompt to a specific, still-alive
//! subagent — the "issue another prompt" half of a multi-turn delegation
//! (FR-9). Runs that subagent's next turn to completion and returns its
//! response.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::ctx::ToolCtx;
use crate::subagent::{SubagentMessageOutcome, SubagentMessageRequest};
use crate::tool::{Tool, ToolOutcome, ToolSpec};

#[derive(Deserialize)]
struct MessageAgentArgs {
    id: String,
    message: String,
}

/// The `message_agent` tool.
pub struct MessageAgentTool;

impl MessageAgentTool {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for MessageAgentTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for MessageAgentTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "message_agent".into(),
            description: "Send a further prompt to a specific, still-alive subagent (by the id \
                 spawn_agents returned) and get its response, once it runs its next turn to \
                 completion. Use this to continue a conversation with a subagent you already \
                 spawned, rather than spawning a new one for every follow-up. A message to an \
                 id that is unknown or has ended returns a structured failure naming the reason."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "The subagent's id, as returned by spawn_agents."
                    },
                    "message": {
                        "type": "string",
                        "description": "The prompt to send to the subagent."
                    }
                },
                "required": ["id", "message"]
            }),
        }
    }

    fn describe(&self, args: &Value) -> Option<String> {
        let args: MessageAgentArgs = serde_json::from_value(args.clone()).ok()?;
        Some(format!("message_agent · {}", args.id))
    }

    async fn execute(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let args: MessageAgentArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolOutcome::invalid_args(&self.spec().name, &e),
        };

        let req = SubagentMessageRequest {
            id: args.id.clone(),
            message: args.message,
        };

        match ctx.message_agent(req).await {
            Ok(SubagentMessageOutcome::Replied(text)) => {
                ToolOutcome::success(text, format!("agent · {} · replied", args.id))
            }
            Ok(SubagentMessageOutcome::NotFound) => ToolOutcome::failure(
                format!(
                    "No alive subagent with id '{}'. It may not exist, or it has already ended.",
                    args.id
                ),
                "no such agent",
            ),
            Ok(SubagentMessageOutcome::Failed(reason)) => {
                ToolOutcome::failure(reason, format!("agent · {} · failed", args.id))
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
