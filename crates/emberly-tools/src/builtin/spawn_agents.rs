//! `spawn_agents` (T-18): create one or more subagents and run each to its
//! first natural stop, concurrently — the harness's fan-out primitive for
//! delegating bounded, independent work (FR-9). Each subagent gets a
//! model-authored persona/task layered on the harness's own tool-use
//! scaffold, an optional provider profile (P-8), and an optional restricted
//! tool subset — never a superset of the primary agent's own tools.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::ctx::ToolCtx;
use crate::subagent::{SubagentSpawnBatch, SubagentSpawnOutcome, SubagentSpawnSpec};
use crate::tool::{Tool, ToolOutcome, ToolSpec};

#[derive(Deserialize)]
struct SpawnAgentSpecArgs {
    name: String,
    system_prompt: String,
    #[serde(default)]
    profile: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    tools: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct SpawnAgentsArgs {
    agents: Vec<SpawnAgentSpecArgs>,
}

/// The `spawn_agents` tool.
pub struct SpawnAgentsTool;

impl SpawnAgentsTool {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for SpawnAgentsTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SpawnAgentsTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "spawn_agents".into(),
            description: "Create one or more subagents and run each on its own task \
                 concurrently — delegate independent subtasks in one round trip instead of \
                 doing them yourself one at a time. Each subagent gets a name, a system prompt \
                 describing its persona/task, and optionally a provider profile and a \
                 restricted set of tools (never more than you yourself have available). The \
                 call returns once every subagent reaches its first stop, each tagged with its \
                 id and either its answer, a note that it's still running (it remains reachable \
                 via message_agent/list_agents), or a failure reason. A subagent cannot spawn \
                 its own subagents, and every subagent's actions are governed by the exact same \
                 permission and safety rules as your own."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agents": {
                        "type": "array",
                        "minItems": 1,
                        "description": "One or more subagents to create and run concurrently.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": {
                                    "type": "string",
                                    "description": "A short label for this subagent, used to \
                                        address it in message_agent/list_agents/end_agent."
                                },
                                "system_prompt": {
                                    "type": "string",
                                    "description": "The subagent's persona/task, in your own \
                                        words — what it should do and how. Inserted alongside \
                                        the harness's own tool-use instructions, not a \
                                        replacement of them."
                                },
                                "profile": {
                                    "type": "string",
                                    "description": "An already-configured provider profile name \
                                        for this subagent to use. Omit to use your own active \
                                        profile."
                                },
                                "model": {
                                    "type": "string",
                                    "description": "A model id within the chosen profile. Omit \
                                        to use the profile's default."
                                },
                                "tools": {
                                    "type": "array",
                                    "items": { "type": "string" },
                                    "description": "Restrict this subagent to only these tool \
                                        names (must be a subset of your own available tools). \
                                        Omit to give it your full tool set minus the multi-agent \
                                        tools themselves."
                                }
                            },
                            "required": ["name", "system_prompt"]
                        }
                    }
                },
                "required": ["agents"]
            }),
        }
    }

    fn describe(&self, args: &Value) -> Option<String> {
        let args: SpawnAgentsArgs = serde_json::from_value(args.clone()).ok()?;
        let names: Vec<&str> = args.agents.iter().map(|a| a.name.as_str()).collect();
        Some(format!("spawn_agents · {}", names.join(", ")))
    }

    async fn execute(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let args: SpawnAgentsArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolOutcome::invalid_args(&self.spec().name, &e),
        };

        if args.agents.is_empty() {
            return ToolOutcome::failure("The agents list must not be empty.", "no agents given");
        }

        let batch = SubagentSpawnBatch {
            agents: args
                .agents
                .into_iter()
                .map(|a| SubagentSpawnSpec {
                    name: a.name,
                    system_prompt: a.system_prompt,
                    profile: a.profile,
                    model: a.model,
                    tools: a.tools,
                })
                .collect(),
        };

        match ctx.spawn_agents(batch).await {
            Ok(results) => {
                let mut lines = Vec::with_capacity(results.len());
                let mut names = Vec::with_capacity(results.len());
                for r in &results {
                    names.push(r.name.clone());
                    let line = match &r.outcome {
                        SubagentSpawnOutcome::Answered(text) => {
                            format!("{} ({}): {text}", r.id, r.name)
                        }
                        SubagentSpawnOutcome::StillRunning => format!(
                            "{} ({}): still running — reachable via message_agent/list_agents",
                            r.id, r.name
                        ),
                        SubagentSpawnOutcome::Failed(reason) => {
                            format!("{} ({}): failed — {reason}", r.id, r.name)
                        }
                    };
                    lines.push(line);
                }
                ToolOutcome::success(
                    lines.join("\n"),
                    format!("spawned {} agent(s): {}", results.len(), names.join(", ")),
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
