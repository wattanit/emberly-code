//! The permission gate as seen by tools (Tech Spec §5.1, Requirements §6).
//!
//! A tool describes what it wants to do ([`PermissionRequest`]) and receives a
//! yes/no ([`PermissionOutcome`]) — it never sees the rule engine or the UI.
//! `emberly-core` implements [`PermissionGate`]: it enriches the request with
//! the matched-rule reason, consults the rule layer, drives the UI round trip
//! over the channels, logs the transcript events, and collapses the user's
//! richer choice into allow/deny.

use std::path::PathBuf;

use async_trait::async_trait;

/// What a tool wants to do, in the tool's own vocabulary. The gate turns this
/// into the richer UI rendering (adding the "why", which the rule layer
/// decides).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRequest {
    /// Tool requesting authorization (e.g. `bash`).
    pub tool: String,
    /// One-line summary (e.g. `run: cargo test`).
    pub summary: String,
    /// The full content the user must see: the complete command or diff
    /// (Design §5 — never truncated to fit).
    pub detail: String,
    /// Filesystem paths the action affects.
    pub affected_paths: Vec<PathBuf>,
    /// Whether any affected path is outside the project root (HC-4). The gate
    /// must never auto-approve these, regardless of rules or mode.
    pub outside_root: bool,
}

/// The gate's yes/no answer to a tool. This is the tool-facing collapse of the
/// user's richer decision (allow-once / allow-session / …), which stays in the
/// engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionOutcome {
    Allow,
    Deny,
}

impl PermissionOutcome {
    #[must_use]
    pub fn is_allowed(self) -> bool {
        matches!(self, Self::Allow)
    }
}

/// The gate a tool calls to request execution. Implemented by the engine;
/// injected into [`ToolCtx`](crate::ctx::ToolCtx). Tools request *through* it
/// and cannot bypass it (Tech Spec §5.1).
#[async_trait]
pub trait PermissionGate: Send + Sync {
    /// Decide whether the action may proceed. Never fails: a decision is
    /// always produced (a denial is [`PermissionOutcome::Deny`], not an error).
    async fn authorize(&self, request: PermissionRequest) -> PermissionOutcome;
}
