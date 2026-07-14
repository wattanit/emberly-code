//! The task-list gate as seen by tools (T-11, Tech Spec §5.2).
//!
//! The `todo` tool maintains an explicit, ordered task list as pure engine
//! state — model-authored planning made visible to the user. Like the ask-user
//! and recall gates, it is a pure engine round trip — no filesystem, no network
//! — so it bypasses the sandbox and is **not permission-gated** (§6). The engine
//! stores the list, emits it to frontends, and records it in the transcript;
//! the gate is an ack-only oneshot (simpler than `ask`, closer to `recall`).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// The status of a single task item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Done,
}

/// One item in the model-maintained task list (T-11).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskItem {
    /// What the model plans to do (model-authored).
    pub text: String,
    /// Where the item stands.
    pub status: TaskStatus,
}

/// The task-list gate is unreachable (engine gone). Fail-closed: the tool maps
/// this to a [`ToolOutcome::failure`](crate::tool::ToolOutcome::failure)
/// (HC-6), never a panic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskListError;

/// The gate the `todo` tool calls to replace the full task list in engine state
/// (T-11, Tech Spec §5.2). Implemented by the engine; injected into
/// [`ToolCtx`](crate::ctx::ToolCtx). The model always sends the **complete
/// list** — the engine replaces rather than merges (Tech Spec §5.2).
#[async_trait]
pub trait TaskListGate: Send + Sync {
    /// Replace the engine's task list with `items`. Returns `Ok(())` once the
    /// engine has stored the list; `Err` if the engine is unreachable
    /// (fail-closed, HC-6).
    async fn set_task_list(&self, items: Vec<TaskItem>) -> Result<(), TaskListError>;
}

/// The default gate installed by [`ToolCtx::new`](crate::ctx::ToolCtx::new):
/// silently discards every set. The engine replaces it with a real
/// channel-backed gate via
/// [`with_task_list_gate`](crate::ctx::ToolCtx::with_task_list_gate);
/// contexts that never track tasks (most tests) keep this safe no-op.
pub(crate) struct DropTaskListGate;

#[async_trait]
impl TaskListGate for DropTaskListGate {
    async fn set_task_list(&self, _items: Vec<TaskItem>) -> Result<(), TaskListError> {
        Ok(())
    }
}
