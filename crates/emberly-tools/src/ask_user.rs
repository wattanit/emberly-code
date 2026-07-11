//! The ask-user gate as seen by tools (T-8, Tech Spec §5.2, Design §5.1).
//!
//! The `ask_user` tool asks the user a question and blocks until they answer.
//! Like the permission gate, it is a pure engine↔frontend round trip — no
//! filesystem, no network — so it bypasses the sandbox yet still flows through
//! the `Tool` trait. `emberly-core` implements [`AskUserGate`]: it drives the
//! UI round trip over the channels, logs the transcript event, and hands back
//! the user's answer or a structured decline. Unlike the permission gate, its
//! answer carries data (the chosen option or free text), and its safe outcome
//! is [`Declined`](AskUserOutcome::Declined), never an "allow/deny".

use async_trait::async_trait;

/// The user's answer to an `ask_user` question, as the tool sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskUserOutcome {
    /// The user answered: free text, or the label of a chosen option.
    Answered(String),
    /// The user dismissed the question without answering. Returned to the
    /// model as a structured decline so it can proceed or stop (T-8, Design
    /// §5.1 — there is no unsafe default; a dismiss is an explicit decline).
    Declined,
}

/// The gate the `ask_user` tool calls to put a question to the user and block
/// until it is answered. Implemented by the engine; injected into
/// [`ToolCtx`](crate::ctx::ToolCtx). Tools ask *through* it (Tech Spec §5.2).
#[async_trait]
pub trait AskUserGate: Send + Sync {
    /// Present `question` (with optional discrete `options`) and wait for the
    /// user. Never fails: a dismissed prompt resolves to
    /// [`AskUserOutcome::Declined`], not an error (HC-6).
    async fn ask(&self, question: String, options: Vec<String>) -> AskUserOutcome;
}

/// The default gate installed by [`ToolCtx::new`](crate::ctx::ToolCtx::new):
/// declines every question. The engine replaces it with a real channel-backed
/// gate via [`with_ask_gate`](crate::ctx::ToolCtx::with_ask_gate); contexts
/// that never ask (most tests) keep this safe no-op.
pub(crate) struct DeclineAskGate;

#[async_trait]
impl AskUserGate for DeclineAskGate {
    async fn ask(&self, _question: String, _options: Vec<String>) -> AskUserOutcome {
        AskUserOutcome::Declined
    }
}
