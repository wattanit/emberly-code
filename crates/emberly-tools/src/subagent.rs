//! The subagent gate as seen by tools (FR-9, T-18–T-21, Tech Spec §8.4).
//!
//! The four multi-agent tools (`spawn_agents`, `message_agent`, `list_agents`,
//! `end_agent`) let the primary agent delegate bounded, independent work to
//! subagents it creates, converses with, and ends. A subagent is a nested
//! `Engine` instance (Tech Spec §8.4) — this module only defines the contract
//! the tools call through; `emberly-core` implements [`SubagentGate`] by
//! constructing and driving those nested engines. Until that engine wiring
//! lands (Tech Spec §8.4's Phase 2), [`DropSubagentGate`] fails every call
//! closed, exactly like [`crate::memory::DropMemoryGate`] and
//! [`crate::scratch::DropScratchGate`].
//!
//! No tool call here supplies a filesystem path or bypasses the permission/
//! sandbox/workspace-trust model: every subagent's own tool calls are
//! governed by the exact same rules as the primary agent's (Requirements
//! FR-9 honesty clause) — this module only carries names, prompts, and ids.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// One subagent to create, as part of a (possibly multi-agent) batch spawn
/// (T-18). `system_prompt` is the model-authored persona/task layer inserted
/// into the harness's own baked-in tool-use scaffold (Requirements FR-9) —
/// never a bare replacement of it. `profile`/`model` select an already-
/// configured provider profile (P-8), defaulting to the primary agent's own
/// when omitted. `tools` names a subset of the primary agent's own available
/// tools; when omitted, the subagent gets the full set minus the four
/// multi-agent tools themselves (the structural depth bound, Requirements
/// §2.2) — a requested name absent from the primary agent's own registry is
/// a spawn-time structured failure (HC-6), never a silent grant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentSpawnSpec {
    pub name: String,
    pub system_prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
}

/// The `spawn_agents` request: one or more subagents to create and run
/// concurrently in a single call (T-18) — the harness's fan-out primitive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentSpawnBatch {
    pub agents: Vec<SubagentSpawnSpec>,
}

/// How one subagent in a batch spawn concluded its first turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentSpawnOutcome {
    /// The subagent reached a natural stop and answered.
    Answered(String),
    /// The subagent is still running past the per-call spawn timeout; it
    /// remains alive and addressable via `message_agent`/`list_agents`
    /// (Requirements T-18) — never canceled.
    StillRunning,
    /// The subagent could not be spawned or did not complete (e.g. the
    /// `max_concurrent` ceiling, an internal provider error). HC-6 data,
    /// never a panic; one subagent's failure never aborts the others in the
    /// same batch.
    Failed(String),
}

/// One subagent's result within a `spawn_agents` batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentSpawnResult {
    pub id: String,
    pub name: String,
    pub outcome: SubagentSpawnOutcome,
}

/// The `message_agent` request: a further prompt to a specific, still-alive
/// subagent (T-19) — the "issue another prompt" half of a multi-turn
/// delegation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentMessageRequest {
    pub id: String,
    pub message: String,
}

/// The outcome of a `message_agent` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentMessageOutcome {
    /// The subagent ran its next turn to completion and answered.
    Replied(String),
    /// No alive subagent has this id (unknown, or already ended). HC-6 data,
    /// never a crash or a silent no-op.
    NotFound,
    /// The subagent's turn errored (e.g. an internal provider error).
    Failed(String),
}

/// A currently alive subagent, as `list_agents` (T-20) reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentListEntry {
    pub id: String,
    pub name: String,
    pub status: SubagentStatus,
}

/// A subagent's current status (Tech Spec §3.1's `SubagentStatus` event
/// carries the same values).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentStatus {
    Running,
    AwaitingPermission,
    Done,
    TimedOut,
    Error,
}

/// The outcome of an `end_agent` call (T-21).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentEndOutcome {
    /// The subagent was ended and its resources freed.
    Ended,
    /// No alive subagent has this id (unknown, or already ended). HC-6 data,
    /// never a crash.
    NotFound,
}

/// The subagent gate is unreachable (engine gone), or the multi-agent
/// subsystem is disabled (`[agents] enabled = false`, Tech Spec §8.4).
/// Fail-closed: every tool maps this to a
/// [`ToolOutcome::failure`](crate::tool::ToolOutcome::failure) (HC-6), never
/// a panic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentError;

/// The gate the four multi-agent tools call through (T-18–T-21, Tech Spec
/// §8.4). Implemented by the engine, which constructs and drives each
/// subagent as a nested `Engine` instance; injected into
/// [`ToolCtx`](crate::ctx::ToolCtx). No method here ever grants a subagent a
/// capability, permission, or safety posture the primary agent's own session
/// lacks (Requirements FR-9 honesty clause) — that boundary is enforced by
/// the implementation, not expressible in this trait's signature alone.
#[async_trait]
pub trait SubagentGate: Send + Sync {
    /// Create one or more subagents and run each to its first natural stop
    /// (or a bounded per-call timeout), concurrently (T-18).
    async fn spawn_agents(
        &self,
        req: SubagentSpawnBatch,
    ) -> Result<Vec<SubagentSpawnResult>, SubagentError>;

    /// Send a further prompt to a specific, still-alive subagent and run its
    /// next turn to completion (T-19).
    async fn message_agent(
        &self,
        req: SubagentMessageRequest,
    ) -> Result<SubagentMessageOutcome, SubagentError>;

    /// Enumerate currently alive subagents (T-20).
    async fn list_agents(&self) -> Result<Vec<SubagentListEntry>, SubagentError>;

    /// End a subagent and free its resources (T-21).
    async fn end_agent(&self, id: String) -> Result<SubagentEndOutcome, SubagentError>;
}

/// The default gate installed by [`ToolCtx::new`](crate::ctx::ToolCtx::new):
/// fails every call closed. The engine replaces it with a real,
/// nested-`Engine`-backed gate via
/// [`with_subagent_gate`](crate::ctx::ToolCtx::with_subagent_gate); contexts
/// that never spawn subagents (most tests, and any context built before the
/// engine wiring lands) keep this safe no-op.
pub(crate) struct DropSubagentGate;

#[async_trait]
impl SubagentGate for DropSubagentGate {
    async fn spawn_agents(
        &self,
        _req: SubagentSpawnBatch,
    ) -> Result<Vec<SubagentSpawnResult>, SubagentError> {
        Err(SubagentError)
    }

    async fn message_agent(
        &self,
        _req: SubagentMessageRequest,
    ) -> Result<SubagentMessageOutcome, SubagentError> {
        Err(SubagentError)
    }

    async fn list_agents(&self) -> Result<Vec<SubagentListEntry>, SubagentError> {
        Err(SubagentError)
    }

    async fn end_agent(&self, _id: String) -> Result<SubagentEndOutcome, SubagentError> {
        Err(SubagentError)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn drop_gate_fails_every_call_closed() {
        let gate = DropSubagentGate;
        assert!(gate
            .spawn_agents(SubagentSpawnBatch { agents: vec![] })
            .await
            .is_err());
        assert!(gate
            .message_agent(SubagentMessageRequest {
                id: "a".into(),
                message: "hi".into(),
            })
            .await
            .is_err());
        assert!(gate.list_agents().await.is_err());
        assert!(gate.end_agent("a".into()).await.is_err());
    }
}
