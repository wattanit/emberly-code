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
    /// versionable per model family (C-4).
    pub description: String,
    /// JSON Schema for the tool's arguments.
    pub input_schema: Value,
}

/// A file created or modified by a tool, with line deltas. Carried on a
/// successful [`ToolOutcome`] so the engine can emit `FileModified` for the
/// sidebar's modified-files list (Design §3.1) — tools have no event channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    /// Path relative to the project root.
    pub path: String,
    pub adds: u32,
    pub dels: u32,
    /// A unified diff of this change (we own both sides — no external diff
    /// binary), for the frontend to render inline and in the diff overlay
    /// (Design §4.2). `None` when a tool reports a change without one.
    pub diff: Option<String>,
}

/// An image payload carried on a [`ToolOutcome`] so the engine can append a
/// `ContentBlock::Image` to the conversation (P-11, T-12). `data` is the
/// base64-encoded file bytes; `media_type` is the MIME string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageContent {
    pub media_type: String,
    pub data: String,
}

/// A document payload carried on a [`ToolOutcome`] so the engine can append a
/// `ContentBlock::Document` to the conversation (P-12, T-16). `data` is the
/// base64-encoded file bytes; `media_type` is always `application/pdf`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentContent {
    pub media_type: String,
    pub data: String,
}

/// The result of running a tool, always handed to the model as data (HC-6).
///
/// `ok == false` is a *structured failure* (file not found, no edit match,
/// command timeout, permission denied) — the model is expected to read it and
/// recover. It is never surfaced as a harness error. The full `content` is
/// returned here; truncation-at-ingestion is applied by the engine when the
/// result is appended to the conversation, not by the tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolOutcome {
    /// Whether the tool succeeded.
    pub ok: bool,
    /// The payload for the model: success output, or the reason for failure.
    pub content: String,
    /// One-line summary for the UI (`ToolFinished` summary).
    pub summary: String,
    /// A file change to surface, if this tool wrote or edited a file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_change: Option<FileChange>,
    /// An image to append to the conversation (P-11, T-12). When `Some`, the
    /// engine adds a `ContentBlock::Image` alongside the text `tool_result`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<ImageContent>,
    /// A document to append to the conversation (P-12, T-16). When `Some`,
    /// the engine adds a `ContentBlock::Document` alongside the text
    /// `tool_result`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<DocumentContent>,
    /// Whether the result is **untrusted web content** (T-14, Design §4.10).
    /// When `true`, the TUI renders it as fetched web data with visible source
    /// URLs — never in harness or assistant voice. Reusable by a future
    /// web-fetch source, not tied to the `web_search` tool name.
    #[serde(default)]
    pub untrusted: bool,
}

impl ToolOutcome {
    /// A successful result.
    #[must_use]
    pub fn success(content: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            ok: true,
            content: content.into(),
            summary: summary.into(),
            file_change: None,
            image: None,
            document: None,
            untrusted: false,
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
            file_change: None,
            image: None,
            document: None,
            untrusted: false,
        }
    }

    /// Attach a file change to a (typically successful) outcome.
    #[must_use]
    pub fn with_file_change(mut self, change: FileChange) -> Self {
        self.file_change = Some(change);
        self
    }

    /// Attach an image payload so the engine appends a `ContentBlock::Image`
    /// to the conversation (P-11, T-12).
    #[must_use]
    pub fn with_image(mut self, image: ImageContent) -> Self {
        self.image = Some(image);
        self
    }

    /// Attach a document payload so the engine appends a
    /// `ContentBlock::Document` to the conversation (P-12, T-16).
    #[must_use]
    pub fn with_document(mut self, document: DocumentContent) -> Self {
        self.document = Some(document);
        self
    }

    /// Mark the result as untrusted web content so the TUI renders it with the
    /// §4.10 untrusted-content styling (T-14, Design §4.10).
    #[must_use]
    pub fn with_untrusted(mut self) -> Self {
        self.untrusted = true;
        self
    }

    /// A failure produced because the user (or the rule engine on their
    /// behalf) denied the action (Requirements §6.6). Returned to the model
    /// as data, not a crash — but this project's sandbox is deliberately
    /// strict, and a denial is a decision, not an obstacle. The nudge
    /// against retrying or working around it is the general form of the
    /// same idea as `chained_git_hint` in `bash.rs`: point the model at the
    /// harness's actual intent instead of letting it spend the turn hunting
    /// for a way through.
    #[must_use]
    pub fn denied(what: &str) -> Self {
        Self::failure(
            format!(
                "The user denied permission to {what}. Don't keep trying to work \
                 around this — tell the user what you needed to do and why, and \
                 ask them to do it themselves if it's still necessary."
            ),
            "denied by user",
        )
    }

    /// A tool call whose arguments failed to deserialize against its schema
    /// (HC-6). Some backends send a call with missing, empty, or nonsense
    /// arguments; naming *whose* mistake it was and telling the model to
    /// call again — rather than a bare "invalid type: null" — gives it what
    /// it needs to correct itself on the next attempt instead of repeating
    /// the same bad call or stalling silently.
    #[must_use]
    pub fn invalid_args(tool_name: &str, error: &serde_json::Error) -> Self {
        Self::failure(
            format!(
                "Your last `{tool_name}` call had invalid arguments and did not run: {error}. \
                 Call `{tool_name}` again with arguments matching its schema."
            ),
            "bad args",
        )
    }
}

/// A callable tool. Implementations are `Send + Sync` so the engine can hold
/// them behind `Arc<dyn Tool>` and call them from its async task.
#[async_trait]
pub trait Tool: Send + Sync {
    /// The tool's name, description, and argument schema.
    fn spec(&self) -> ToolSpec;

    /// A one-line, human-readable description of *this* invocation, from its
    /// arguments — e.g. `run: cargo test`, `read src/main.rs`. Shown as the
    /// tool-activity label the moment a call starts, before any output exists
    /// (Design §6.3). Defaults to `None`, and the engine falls back to the
    /// tool's name.
    fn describe(&self, _args: &Value) -> Option<String> {
        None
    }

    /// Run the tool. Any error is returned as a failure [`ToolOutcome`], never
    /// as `Err` (HC-6). Actions requiring permission must go through
    /// [`ToolCtx::authorize`] — tools cannot bypass the gate.
    async fn execute(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_args_names_the_tool_and_tells_the_model_to_retry() {
        // A bare "invalid type: null" leaves the model no clue that its own
        // last call is what's wrong; this must name the tool and instruct a
        // retry so the model corrects its arguments instead of repeating the
        // same bad call or stalling silently.
        let error = match serde_json::from_str::<serde_json::Value>("not json") {
            Err(e) => e,
            Ok(_) => panic!("\"not json\" is not valid JSON"),
        };
        let outcome = ToolOutcome::invalid_args("read_file", &error);
        assert!(!outcome.ok);
        assert!(outcome.content.contains("read_file"));
        assert!(outcome.content.contains("Call `read_file` again"));
    }

    #[test]
    fn denied_names_what_was_refused_and_points_at_the_user_not_a_workaround() {
        // A bare "permission denied" invites the model to spend the turn
        // hunting for another way in; this must name what was refused and
        // steer it toward asking the user instead of retrying or routing
        // around the block (the general form of `chained_git_hint`).
        let outcome = ToolOutcome::denied("write config.toml");
        assert!(!outcome.ok);
        assert_eq!(outcome.summary, "denied by user");
        assert!(outcome.content.contains("write config.toml"));
        assert!(outcome
            .content
            .contains("Don't keep trying to work around this"));
        assert!(outcome.content.contains("ask them to do it themselves"));
    }
}
