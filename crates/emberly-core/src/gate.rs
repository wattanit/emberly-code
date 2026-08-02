//! The engine's implementations of the tool-facing gate traits (Requirements
//! §6, Tech Spec §5.1, §6.1). A tool calls its gate method from inside
//! `execute`; that call hands an "ask" to the engine over a channel and awaits
//! the answer on a oneshot. The engine loop drives the tool future and the ask
//! channels concurrently, so nothing deadlocks and no state is shared behind a
//! mutex (Tech Spec §2).
//!
//! Every gate has the same shape — one mpsc sender, one ask type carrying a
//! oneshot — so the shape is written once as [`Gate<A>`] and [`ask_engine`];
//! each capability contributes only its ask type, its trait impl, and its
//! fail-closed answer. Every gate must fail *closed* when the engine is gone or
//! drops the reply unanswered, and `ask_engine` takes that fallback as a
//! required argument, so the rule is applied in one place and a new gate cannot
//! omit it. Reading the reply with `unwrap` instead of `unwrap_or` would also
//! violate HC-3.

use async_trait::async_trait;
use emberly_tools::{
    AskUserGate, AskUserOutcome, MemoryError, MemoryGate, MemoryOutcome, MemoryRequest,
    PermissionGate, PermissionOutcome, PermissionRequest, RecallGate, RecallOutcome, SkillError,
    SkillGate, SkillInvocation, TaskItem, TaskListError, TaskListGate,
};
use tokio::sync::{mpsc, oneshot};

/// The engine-side end of one capability channel, installed into every
/// [`ToolCtx`](emberly_tools::ToolCtx) as the gate for that capability. Generic
/// over the ask type because that is the only thing that differs between them.
pub(crate) struct Gate<A> {
    pub(crate) asks: mpsc::Sender<A>,
}

impl<A> Gate<A> {
    pub(crate) fn new(asks: mpsc::Sender<A>) -> Self {
        Self { asks }
    }
}

/// Send one ask to the engine and await its answer, **failing closed** to
/// `fallback` if the engine is gone or drops the reply without answering.
///
/// `make` receives the reply sender so the ask type can own it, which is what
/// lets this be generic over asks whose payloads have nothing in common.
async fn ask_engine<A, R>(
    asks: &mpsc::Sender<A>,
    fallback: R,
    make: impl FnOnce(oneshot::Sender<R>) -> A,
) -> R
where
    A: Send,
    R: Send,
{
    let (reply_tx, reply_rx) = oneshot::channel();
    if asks.send(make(reply_tx)).await.is_err() {
        return fallback;
    }
    reply_rx.await.unwrap_or(fallback)
}

/// A permission request in flight from a tool to the engine, carrying the
/// oneshot the engine replies on. Opaque to callers of the engine; they only
/// route the receiver back into [`Engine::run`](crate::engine::Engine::run).
pub struct PermissionAsk {
    pub(crate) request: PermissionRequest,
    pub(crate) reply: oneshot::Sender<PermissionOutcome>,
}

#[async_trait]
impl PermissionGate for Gate<PermissionAsk> {
    /// Fails closed to [`PermissionOutcome::Deny`] — the safe default (Design §5).
    async fn authorize(&self, request: PermissionRequest) -> PermissionOutcome {
        ask_engine(&self.asks, PermissionOutcome::Deny, |reply| PermissionAsk {
            request,
            reply,
        })
        .await
    }
}

/// An `ask_user` question in flight from a tool to the engine (T-8).
pub struct AskUserAsk {
    pub(crate) question: String,
    pub(crate) options: Vec<String>,
    pub(crate) reply: oneshot::Sender<AskUserOutcome>,
}

#[async_trait]
impl AskUserGate for Gate<AskUserAsk> {
    /// Fails closed to [`AskUserOutcome::Declined`] — no unsafe answer is
    /// invented on the user's behalf (Design §5.1).
    async fn ask(&self, question: String, options: Vec<String>) -> AskUserOutcome {
        ask_engine(&self.asks, AskUserOutcome::Declined, |reply| AskUserAsk {
            question,
            options,
            reply,
        })
        .await
    }
}

/// A `recall` request in flight from a tool to the engine (T-10).
pub struct RecallAsk {
    pub(crate) from: usize,
    pub(crate) to: usize,
    pub(crate) reply: oneshot::Sender<RecallOutcome>,
}

#[async_trait]
impl RecallGate for Gate<RecallAsk> {
    /// Fails closed to [`RecallOutcome::Empty`] — the model proceeds without
    /// the recalled turns rather than being handed something wrong.
    async fn recall(&self, from: usize, to: usize) -> RecallOutcome {
        ask_engine(&self.asks, RecallOutcome::Empty, |reply| RecallAsk {
            from,
            to,
            reply,
        })
        .await
    }
}

/// A task-list update in flight from a tool to the engine (T-11).
pub struct TaskListAsk {
    pub(crate) items: Vec<TaskItem>,
    pub(crate) reply: oneshot::Sender<Result<(), TaskListError>>,
}

#[async_trait]
impl TaskListGate for Gate<TaskListAsk> {
    /// Fails closed to `Err`; the tool maps it to a structured failure (HC-6).
    async fn set_task_list(&self, items: Vec<TaskItem>) -> Result<(), TaskListError> {
        ask_engine(&self.asks, Err(TaskListError), |reply| TaskListAsk {
            items,
            reply,
        })
        .await
    }
}

/// A memory op in flight from a tool to the engine (T-13).
pub struct MemoryAsk {
    pub(crate) req: MemoryRequest,
    pub(crate) reply: oneshot::Sender<Result<MemoryOutcome, MemoryError>>,
}

#[async_trait]
impl MemoryGate for Gate<MemoryAsk> {
    /// Fails closed to `Err`; the tool maps it to a structured failure (HC-6).
    async fn memory_op(&self, req: MemoryRequest) -> Result<MemoryOutcome, MemoryError> {
        ask_engine(&self.asks, Err(MemoryError), |reply| MemoryAsk {
            req,
            reply,
        })
        .await
    }
}

/// A skill invoke in flight from a tool to the engine (T-15).
pub struct SkillAsk {
    pub(crate) name: String,
    pub(crate) reply: oneshot::Sender<Result<Option<SkillInvocation>, SkillError>>,
}

#[async_trait]
impl SkillGate for Gate<SkillAsk> {
    /// Fails closed to `Err`; the tool maps it to a structured failure (HC-6).
    async fn invoke_skill(&self, name: String) -> Result<Option<SkillInvocation>, SkillError> {
        ask_engine(&self.asks, Err(SkillError), |reply| SkillAsk {
            name,
            reply,
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use emberly_tools::{MemoryOp, MemoryScope};

    /// A gate whose engine end is already gone: `send` fails immediately.
    fn orphaned<A>() -> Gate<A> {
        let (tx, rx) = mpsc::channel(1);
        drop(rx);
        Gate::new(tx)
    }

    fn permission_request() -> PermissionRequest {
        PermissionRequest {
            tool: "bash".into(),
            summary: "run: true".into(),
            detail: "true".into(),
            affected_paths: Vec::new(),
            outside_root: false,
        }
    }

    fn memory_request() -> MemoryRequest {
        MemoryRequest {
            op: MemoryOp::Recall,
            scope: MemoryScope::User,
            name: "x".into(),
            description: None,
            type_: None,
            body: None,
        }
    }

    // ---- the engine is gone (send fails) --------------------------------

    #[tokio::test]
    async fn every_gate_fails_closed_when_the_engine_is_gone() {
        assert!(matches!(
            orphaned::<PermissionAsk>()
                .authorize(permission_request())
                .await,
            PermissionOutcome::Deny
        ));
        assert!(matches!(
            orphaned::<AskUserAsk>()
                .ask("q".into(), vec!["a".into()])
                .await,
            AskUserOutcome::Declined
        ));
        assert!(matches!(
            orphaned::<RecallAsk>().recall(0, 1).await,
            RecallOutcome::Empty
        ));
        assert!(orphaned::<TaskListAsk>()
            .set_task_list(Vec::new())
            .await
            .is_err());
        assert!(orphaned::<MemoryAsk>()
            .memory_op(memory_request())
            .await
            .is_err());
        assert!(orphaned::<SkillAsk>()
            .invoke_skill("s".into())
            .await
            .is_err());
    }

    // ---- the engine takes the ask and never answers ---------------------

    /// The reply oneshot is dropped unanswered. Every gate must still return its
    /// safe default rather than panicking. This is the branch that makes
    /// `ask_engine`'s `unwrap_or` load-bearing, and all six gates share it, so a
    /// regression there unsafe-defaults every one of them at once.
    #[tokio::test]
    async fn every_gate_fails_closed_when_the_reply_is_dropped() {
        // Each block: the engine receives the ask, then drops it — taking the
        // reply sender with it — while the gate is still awaiting. Spelled out
        // per gate rather than generated: a gate call borrows its gate, and the
        // six return types share no trait to assert against.
        {
            let (tx, mut rx) = mpsc::channel::<PermissionAsk>(1);
            let gate = Gate::new(tx);
            let (outcome, ()) = tokio::join!(gate.authorize(permission_request()), async {
                drop(rx.recv().await);
            });
            assert!(matches!(outcome, PermissionOutcome::Deny));
        }
        {
            let (tx, mut rx) = mpsc::channel::<AskUserAsk>(1);
            let gate = Gate::new(tx);
            let (outcome, ()) = tokio::join!(gate.ask("q".into(), Vec::new()), async {
                drop(rx.recv().await);
            });
            assert!(matches!(outcome, AskUserOutcome::Declined));
        }
        {
            let (tx, mut rx) = mpsc::channel::<RecallAsk>(1);
            let gate = Gate::new(tx);
            let (outcome, ()) = tokio::join!(gate.recall(0, 1), async {
                drop(rx.recv().await);
            });
            assert!(matches!(outcome, RecallOutcome::Empty));
        }
        {
            let (tx, mut rx) = mpsc::channel::<TaskListAsk>(1);
            let gate = Gate::new(tx);
            let (outcome, ()) = tokio::join!(gate.set_task_list(Vec::new()), async {
                drop(rx.recv().await);
            });
            assert!(outcome.is_err());
        }
        {
            let (tx, mut rx) = mpsc::channel::<MemoryAsk>(1);
            let gate = Gate::new(tx);
            let (outcome, ()) = tokio::join!(gate.memory_op(memory_request()), async {
                drop(rx.recv().await);
            });
            assert!(outcome.is_err());
        }
        {
            let (tx, mut rx) = mpsc::channel::<SkillAsk>(1);
            let gate = Gate::new(tx);
            let (outcome, ()) = tokio::join!(gate.invoke_skill("s".into()), async {
                drop(rx.recv().await);
            });
            assert!(outcome.is_err());
        }
    }
}
