//! The recall gate as seen by tools (T-10, Tech Spec §5.2, §7).
//!
//! The `recall` tool retrieves earlier turns that were elided from the sent
//! context by the adaptive window. Like the ask-user gate, it is a pure
//! engine round trip — no filesystem, no network — so it bypasses the sandbox
//! yet still flows through the `Tool` trait. `emberly-core` implements
//! [`RecallGate`]: it reads the in-memory conversation, slices the requested
//! turn range, reduces tool results via `reduce_output`, and hands
//! back the rendered text — never raw JSONL (T-10).

use async_trait::async_trait;

/// The result of recalling a turn range, as the tool sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecallOutcome {
    /// The recalled turns, rendered as readable text with tool results
    /// reduced (T-10). `count` is the number of turns found.
    Turns { content: String, count: usize },
    /// No turns matched the requested range (out of bounds or retired by
    /// compaction). A valid, structured outcome — the model reads it and
    /// proceeds (HC-6).
    Empty,
}

/// The gate the `recall` tool calls to retrieve elided turns from the engine's
/// in-memory conversation (T-10, Tech Spec §5.2/§7). Implemented by the
/// engine; injected into [`ToolCtx`](crate::ctx::ToolCtx). Tools ask *through*
/// it so they never touch the engine's state directly.
#[async_trait]
pub trait RecallGate: Send + Sync {
    /// Retrieve turns `from`–`to` (inclusive stable turn numbers, as shown in
    /// the elision marker). Never fails: an out-of-range request resolves to
    /// [`RecallOutcome::Empty`], not an error (HC-6).
    async fn recall(&self, from: usize, to: usize) -> RecallOutcome;
}

/// The default gate installed by [`ToolCtx::new`](crate::ctx::ToolCtx::new):
/// returns `Empty` for every request. The engine replaces it with a real
/// channel-backed gate via
/// [`with_recall_gate`](crate::ctx::ToolCtx::with_recall_gate); contexts that
/// never recall (most tests) keep this safe no-op.
pub(crate) struct DeclineRecallGate;

#[async_trait]
impl RecallGate for DeclineRecallGate {
    async fn recall(&self, _from: usize, _to: usize) -> RecallOutcome {
        RecallOutcome::Empty
    }
}
