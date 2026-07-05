//! The engine's implementation of the tool-facing [`PermissionGate`]
//! (Requirements §6, Tech Spec §6.1 — the rule layer; OS confinement is
//! Phase 2). A tool calls `authorize` from inside its `execute`; that call
//! hands a [`PermissionAsk`] to the engine over a channel and awaits the
//! answer on a oneshot. The engine loop drives the tool future and the ask
//! channel concurrently, so nothing deadlocks and no state is shared behind a
//! mutex (Tech Spec §2).

use async_trait::async_trait;
use emberly_tools::{PermissionGate, PermissionOutcome, PermissionRequest};
use tokio::sync::{mpsc, oneshot};

/// A permission request in flight from a tool to the engine, carrying the
/// oneshot the engine replies on. Opaque to callers of the engine; they only
/// route the receiver back into [`Engine::run`](crate::engine::Engine::run).
pub struct PermissionAsk {
    pub(crate) request: PermissionRequest,
    pub(crate) reply: oneshot::Sender<PermissionOutcome>,
}

/// The gate installed into every [`ToolCtx`](emberly_tools::ToolCtx). Sending
/// fails closed: if the engine is gone, or the reply is dropped, the answer is
/// [`PermissionOutcome::Deny`] (the safe default, Design §5).
pub(crate) struct ChannelGate {
    pub(crate) asks: mpsc::Sender<PermissionAsk>,
}

#[async_trait]
impl PermissionGate for ChannelGate {
    async fn authorize(&self, request: PermissionRequest) -> PermissionOutcome {
        let (reply_tx, reply_rx) = oneshot::channel();
        if self
            .asks
            .send(PermissionAsk {
                request,
                reply: reply_tx,
            })
            .await
            .is_err()
        {
            return PermissionOutcome::Deny;
        }
        reply_rx.await.unwrap_or(PermissionOutcome::Deny)
    }
}
