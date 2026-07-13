//! `skill` (T-15, FR-7): invoke a skill by name — load its `SKILL.md` body and
//! bundled resource paths into the tool result. The model discovers skills via
//! the pinned catalog (metadata standing); this tool loads the body on demand
//! (progressive disclosure, Tech Spec §7/§8.2).
//!
//! This tool **reads instruction text only — it executes nothing**. Running a
//! skill's bundled script is a separate ordinary `bash` call under the full
//! permission/sandbox model (§6, FR-7 honesty clause). It is **not
//! permission-gated** — it reads from trust-resolved dirs, like `recall`.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::ctx::ToolCtx;
use crate::memory::slug;
use crate::tool::{Tool, ToolOutcome, ToolSpec};

#[derive(Deserialize)]
struct SkillArgs {
    name: String,
}

/// The `skill` tool.
pub struct SkillTool;

impl SkillTool {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for SkillTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SkillTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "skill".into(),
            description: "Invoke a skill by name to load its instruction body (SKILL.md) and \
                 list its bundled resource paths. Only metadata for all skills is in context; \
                 the body loads on invoke (progressive disclosure). This tool reads instruction \
                 text only — it executes nothing. Running a skill's bundled script is a separate \
                 ordinary bash call under the full permission/sandbox model. Not permission-gated."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "The skill name (as shown in the skill catalog)."
                    }
                },
                "required": ["name"]
            }),
        }
    }

    fn describe(&self, args: &Value) -> Option<String> {
        let args: SkillArgs = serde_json::from_value(args.clone()).ok()?;
        Some(format!("skill · {}", args.name))
    }

    async fn execute(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let args: SkillArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolOutcome::failure(format!("invalid arguments: {e}"), "bad args"),
        };

        // Defense in depth: sanitize the name via slug before reaching the
        // engine (same boundary as the memory tool, HC-4).
        let name = match slug(&args.name) {
            Ok(s) => s,
            Err(e) => return ToolOutcome::failure(e.0, "invalid name"),
        };

        match ctx.invoke_skill(name.clone()).await {
            Ok(Some(invocation)) => {
                let origin_label = match invocation.origin {
                    crate::skills::SkillOrigin::User => "user",
                    crate::skills::SkillOrigin::Project => "project",
                };

                // Content = the SKILL.md body followed by the resource list.
                let mut content = invocation.body;
                if !invocation.resources.is_empty() {
                    content.push_str("\n\nBundled resources:\n");
                    for path in &invocation.resources {
                        content.push_str(&format!("- {path}\n"));
                    }
                }

                // Summary carries origin for the Design §4.9 tool line.
                let summary = format!("skill · {name} · {origin_label}");
                ToolOutcome::success(content, summary)
            }
            Ok(None) => ToolOutcome::failure(
                "No skill found with that name in the catalog.",
                "skill not found",
            ),
            Err(_) => ToolOutcome::failure(
                "The skill could not be loaded (the engine is unreachable).",
                "engine unreachable",
            ),
        }
    }
}
