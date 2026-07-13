//! The engine's implementation of the tool-facing [`PermissionGate`]
//! (Requirements §6, Tech Spec §6.1 — the rule layer; OS confinement is
//! Phase 2). A tool calls `authorize` from inside its `execute`; that call
//! hands a [`PermissionAsk`] to the engine over a channel and awaits the
//! answer on a oneshot. The engine loop drives the tool future and the ask
//! channel concurrently, so nothing deadlocks and no state is shared behind a
//! mutex (Tech Spec §2).

use async_trait::async_trait;
use emberly_tools::{
    AskUserGate, AskUserOutcome, MemoryError, MemoryGate, MemoryOutcome, MemoryRequest,
    PermissionGate, PermissionOutcome, PermissionRequest, RecallGate, RecallOutcome, TaskItem,
    TaskListError, TaskListGate,
};
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

/// An `ask_user` question in flight from a tool to the engine, carrying the
/// oneshot the engine replies on (T-8). Like [`PermissionAsk`], opaque to
/// callers of the engine — they only route the receiver back into
/// [`Engine::run`](crate::engine::Engine::run).
pub struct AskUserAsk {
    pub(crate) question: String,
    pub(crate) options: Vec<String>,
    pub(crate) reply: oneshot::Sender<AskUserOutcome>,
}

/// The ask-user gate installed into every [`ToolCtx`](emberly_tools::ToolCtx).
/// Fails closed: if the engine is gone, or the reply is dropped, the answer is
/// [`AskUserOutcome::Declined`] (the safe default — no unsafe answer, Design
/// §5.1).
pub(crate) struct AskGate {
    pub(crate) asks: mpsc::Sender<AskUserAsk>,
}

#[async_trait]
impl AskUserGate for AskGate {
    async fn ask(&self, question: String, options: Vec<String>) -> AskUserOutcome {
        let (reply_tx, reply_rx) = oneshot::channel();
        if self
            .asks
            .send(AskUserAsk {
                question,
                options,
                reply: reply_tx,
            })
            .await
            .is_err()
        {
            return AskUserOutcome::Declined;
        }
        reply_rx.await.unwrap_or(AskUserOutcome::Declined)
    }
}

/// A `recall` request in flight from a tool to the engine, carrying the
/// oneshot the engine replies on (T-10). Like [`AskUserAsk`], opaque to
/// callers of the engine.
pub struct RecallAsk {
    pub(crate) from: usize,
    pub(crate) to: usize,
    pub(crate) reply: oneshot::Sender<RecallOutcome>,
}

/// The recall gate installed into every
/// [`ToolCtx`](emberly_tools::ToolCtx). Fails closed: if the engine is gone or
/// the reply is dropped, the answer is [`RecallOutcome::Empty`] (the safe
/// default — the model proceeds without the recalled turns).
pub(crate) struct RecallGateImpl {
    pub(crate) asks: mpsc::Sender<RecallAsk>,
}

#[async_trait]
impl RecallGate for RecallGateImpl {
    async fn recall(&self, from: usize, to: usize) -> RecallOutcome {
        let (reply_tx, reply_rx) = oneshot::channel();
        if self
            .asks
            .send(RecallAsk {
                from,
                to,
                reply: reply_tx,
            })
            .await
            .is_err()
        {
            return RecallOutcome::Empty;
        }
        reply_rx.await.unwrap_or(RecallOutcome::Empty)
    }
}

/// A task-list update in flight from a tool to the engine, carrying the
/// oneshot the engine acks on (T-11). Like [`RecallAsk`], opaque to callers of
/// the engine — they only route the receiver back into
/// [`Engine::run`](crate::engine::Engine::run).
pub struct TaskListAsk {
    pub(crate) items: Vec<TaskItem>,
    pub(crate) reply: oneshot::Sender<Result<(), TaskListError>>,
}

/// The task-list gate installed into every
/// [`ToolCtx`](emberly_tools::ToolCtx). Fails closed: if the engine is gone or
/// the reply is dropped, the answer is `Err` (the safe default — the tool maps
/// it to a structured failure, HC-6).
pub(crate) struct TaskListGateImpl {
    pub(crate) asks: mpsc::Sender<TaskListAsk>,
}

#[async_trait]
impl TaskListGate for TaskListGateImpl {
    async fn set_task_list(&self, items: Vec<TaskItem>) -> Result<(), TaskListError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        if self
            .asks
            .send(TaskListAsk {
                items,
                reply: reply_tx,
            })
            .await
            .is_err()
        {
            return Err(TaskListError);
        }
        reply_rx.await.unwrap_or(Err(TaskListError))
    }
}

/// A memory op in flight from a tool to the engine, carrying the oneshot the
/// engine replies on (T-13). Like [`TaskListAsk`], opaque to callers of the
/// engine.
pub struct MemoryAsk {
    pub(crate) req: MemoryRequest,
    pub(crate) reply: oneshot::Sender<Result<MemoryOutcome, MemoryError>>,
}

/// The memory gate installed into every
/// [`ToolCtx`](emberly_tools::ToolCtx). Fails closed: if the engine is gone or
/// the reply is dropped, the answer is `Err` (the safe default — the tool maps
/// it to a structured failure, HC-6).
pub(crate) struct MemoryGateImpl {
    pub(crate) asks: mpsc::Sender<MemoryAsk>,
}

#[async_trait]
impl MemoryGate for MemoryGateImpl {
    async fn memory_op(&self, req: MemoryRequest) -> Result<MemoryOutcome, MemoryError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        if self
            .asks
            .send(MemoryAsk {
                req,
                reply: reply_tx,
            })
            .await
            .is_err()
        {
            return Err(MemoryError);
        }
        reply_rx.await.unwrap_or(Err(MemoryError))
    }
}
