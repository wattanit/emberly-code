//! `memory` (T-13, FR-6): read and write durable memory entries that survive
//! across sessions. Harness-managed persistence (like the transcript and trust
//! store, not an agent filesystem write) — the model supplies content fields,
//! never a path; the engine derives the filename from `name` within the fixed
//! scope directory. It does not widen HC-4 and is **not permission-gated** (§6,
//! FR-6 honesty clause). Project-scope memory loads only under a trusted root.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::ctx::ToolCtx;
use crate::memory::{slug, MemoryOp, MemoryOutcome, MemoryRequest, MemoryScope};
use crate::tool::{Tool, ToolOutcome, ToolSpec};

#[derive(Deserialize)]
struct MemoryArgs {
    op: MemoryOp,
    scope: MemoryScope,
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default, rename = "type")]
    type_: Option<String>,
    #[serde(default)]
    body: Option<String>,
}

/// The `memory` tool.
pub struct MemoryTool;

impl MemoryTool {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for MemoryTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for MemoryTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "memory".into(),
            description: "Read and write durable memory entries that persist across sessions. \
                 Two scopes: 'user' (global, always loaded) and 'project' (loaded only under \
                 a trusted root). Only the one-line index is standing context; entry bodies \
                 load via op 'recall'. Harness-managed persistence — supply content fields, \
                 never a path. Not permission-gated."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "op": {
                        "type": "string",
                        "enum": ["write", "update", "remove", "recall"],
                        "description": "What to do: 'write' (create new), 'update' (create or \
                                        overwrite), 'remove' (delete), 'recall' (get body)."
                    },
                    "scope": {
                        "type": "string",
                        "enum": ["user", "project"],
                        "description": "Which store: 'user' (global) or 'project' (trusted root only)."
                    },
                    "name": {
                        "type": "string",
                        "description": "Entry name (slugified to a filename; no paths allowed)."
                    },
                    "description": {
                        "type": "string",
                        "description": "One-line summary (shown in the pinned index)."
                    },
                    "type": {
                        "type": "string",
                        "description": "Optional category tag."
                    },
                    "body": {
                        "type": "string",
                        "description": "The full entry content (markdown). Loaded on demand via recall."
                    }
                },
                "required": ["op", "scope", "name"]
            }),
        }
    }

    fn describe(&self, args: &Value) -> Option<String> {
        let args: MemoryArgs = serde_json::from_value(args.clone()).ok()?;
        let scope = match args.scope {
            MemoryScope::User => "user",
            MemoryScope::Project => "project",
        };
        match args.op {
            MemoryOp::Write | MemoryOp::Update => {
                Some(format!("remember · {scope} · {}", args.name))
            }
            MemoryOp::Remove => Some(format!("forget · {scope} · {}", args.name)),
            MemoryOp::Recall => Some(format!("recall · {scope} · {}", args.name)),
        }
    }

    async fn execute(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let args: MemoryArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolOutcome::invalid_args(&self.spec().name, &e),
        };

        // Early name validation (HC-4 boundary): reject path escapes before
        // reaching the engine.
        if let Err(e) = slug(&args.name) {
            return ToolOutcome::failure(e.0, "invalid name");
        }

        let scope_label = match args.scope {
            MemoryScope::User => "user",
            MemoryScope::Project => "project",
        };

        let req = MemoryRequest {
            op: args.op,
            scope: args.scope,
            name: args.name,
            description: args.description,
            type_: args.type_,
            body: args.body,
        };

        match ctx.memory_op(req).await {
            Ok(MemoryOutcome::Written { user, project }) => ToolOutcome::success(
                format!("Memory updated. Index: {user} user, {project} project entries."),
                format!("remembered · {scope_label} · {user}u {project}p"),
            ),
            Ok(MemoryOutcome::Recalled {
                body: Some(body),
                origin,
            }) => {
                let origin_label = match origin {
                    MemoryScope::User => "user",
                    MemoryScope::Project => "project",
                };
                ToolOutcome::success(body, format!("recalled · {origin_label}"))
            }
            Ok(MemoryOutcome::Recalled { body: None, .. }) => ToolOutcome::failure(
                "No memory entry found with that name in this scope.",
                "not found",
            ),
            Ok(MemoryOutcome::Rejected { reason }) => ToolOutcome::failure(reason.clone(), reason),
            Err(_) => ToolOutcome::failure(
                "Memory could not be updated (the engine is unreachable).",
                "engine unreachable",
            ),
        }
    }
}
