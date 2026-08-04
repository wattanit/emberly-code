//! `ask_user` (T-8): put a question to the user and block the agent loop until
//! they answer. A pure engine↔frontend round trip (Tech Spec §5.2) — it touches
//! no filesystem or network, so it bypasses the sandbox, but it is a real
//! [`Tool`] and reaches the frontend only through [`ToolCtx::ask_user`]. The
//! answer (free text or a chosen option) comes back as data; a dismissal comes
//! back as a structured decline so the model can proceed or stop (HC-6).

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::ask_user::AskUserOutcome;
use crate::ctx::ToolCtx;
use crate::tool::{Tool, ToolOutcome, ToolSpec};

#[derive(Deserialize)]
struct AskArgs {
    question: String,
    #[serde(default)]
    options: Vec<String>,
}

/// The `ask_user` tool.
pub struct AskUserTool;

#[async_trait]
impl Tool for AskUserTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "ask_user".into(),
            description: "Ask the user a question and wait for their answer before continuing. \
                 Provide `options` for a multiple-choice question, or omit them for a free-form \
                 answer. Use this only when you genuinely need the user's decision or information \
                 you cannot obtain yourself — not for routine confirmations. Returns the user's \
                 answer, or a note that they declined to answer."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "question": {
                        "type": "string",
                        "description": "The question to put to the user."
                    },
                    "options": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional discrete choices to offer. Omit for free-form input."
                    }
                },
                "required": ["question"]
            }),
        }
    }

    fn describe(&self, args: &Value) -> Option<String> {
        serde_json::from_value::<AskArgs>(args.clone())
            .ok()
            .map(|a| format!("ask: {}", a.question))
    }

    async fn execute(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let args: AskArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolOutcome::invalid_args(&self.spec().name, &e),
        };

        match ctx.ask_user(args.question, args.options).await {
            AskUserOutcome::Answered(answer) => {
                ToolOutcome::success(format!("The user answered: {answer}"), "answered")
            }
            // A decline is a valid, structured outcome (T-8), not a failure —
            // the model reads it and decides whether to proceed or stop.
            AskUserOutcome::Declined => {
                ToolOutcome::success("The user declined to answer.", "declined")
            }
        }
    }
}
