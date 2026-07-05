//! The [`Tool`] trait and its result types (Tech Spec §5.1, HC-6).
//!
//! The trait is transport-agnostic: a built-in tool and a future MCP adapter
//! that proxies JSON-RPC both implement `Tool`, and nothing else in the engine
//! changes (T-7). `execute` never returns a harness-level error — every
//! outcome, success or failure, is structured data for the model (HC-6).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ctx::ToolCtx;

/// A tool's advertised interface: what the model sees when deciding to call
/// it. The engine converts this into the provider's tool-schema type (the
/// `tools` and `providers` crates do not depend on each other; the engine
/// bridges them).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    /// Tool name the model calls (e.g. `read_file`).
    pub name: String,
    /// Human/model-facing description. Behavior-critical configuration,
    /// versionable per model family (C-4) — real descriptions arrive with the
    /// tools in group 4.
    pub description: String,
    /// JSON Schema for the tool's arguments.
    pub input_schema: Value,
}

/// The result of running a tool, always handed to the model as data (HC-6).
///
/// `ok == false` is a *structured failure* (file not found, no edit match,
/// command timeout, permission denied) — the model is expected to read it and
/// recover. It is never surfaced as a harness error. The full `content` is
/// returned here; truncation-at-ingestion (group 5) is applied by the engine
/// when the result is appended to the conversation, not by the tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolOutcome {
    /// Whether the tool succeeded.
    pub ok: bool,
    /// The payload for the model: success output, or the reason for failure.
    pub content: String,
    /// One-line summary for the UI (`ToolFinished` summary).
    pub summary: String,
}

impl ToolOutcome {
    /// A successful result.
    #[must_use]
    pub fn success(content: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            ok: true,
            content: content.into(),
            summary: summary.into(),
        }
    }

    /// A structured failure (HC-6). `content` must state precisely what went
    /// wrong so the model can recover.
    #[must_use]
    pub fn failure(content: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            ok: false,
            content: content.into(),
            summary: summary.into(),
        }
    }

    /// A failure produced because the user denied the action (Requirements
    /// §6.6). Returned to the model as data so it can route around it.
    #[must_use]
    pub fn denied(what: &str) -> Self {
        Self::failure(
            format!("The user denied permission to {what}."),
            "denied by user",
        )
    }
}

/// A callable tool. Implementations are `Send + Sync` so the engine can hold
/// them behind `Arc<dyn Tool>` and call them from its async task.
#[async_trait]
pub trait Tool: Send + Sync {
    /// The tool's name, description, and argument schema.
    fn spec(&self) -> ToolSpec;

    /// Run the tool. Any error is returned as a failure [`ToolOutcome`], never
    /// as `Err` (HC-6). Actions requiring permission must go through
    /// [`ToolCtx::authorize`] — tools cannot bypass the gate.
    async fn execute(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome;
}
