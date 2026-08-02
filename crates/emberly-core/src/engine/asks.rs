//! The tool→engine gates other than permission: ask_user, recall, and
//! the task list.
//!
//! Part of the `Engine` inherent impl, split out of one 2,700-line
//! block; the engine is still the single owner of this state.

use super::*;

impl Engine {
    /// Handle an `ask_user` question from the tool (T-8): mint an id, surface it
    /// to the frontend, and stash the pending question. The transcript record is
    /// written on resolution (question + answer together), so an unanswered
    /// question that is later declined is still recorded once.
    pub(super) async fn on_user_ask(&mut self, ask: AskUserAsk, pending: &mut Vec<PendingUserAsk>) {
        let AskUserAsk {
            question,
            options,
            reply,
        } = ask;
        let id = self.take_ask_id();
        self.emit(UiEvent::AskUserRequest {
            id,
            question: question.clone(),
            options: options.clone(),
        })
        .await;
        pending.push(PendingUserAsk {
            id,
            question,
            options,
            reply,
        });
    }

    /// Resolve the user's answer to a pending `ask_user` question: record it and
    /// reply to the blocked tool. Unknown ids are ignored (a stray or
    /// already-answered question).
    pub(super) async fn answer_user_ask(
        &mut self,
        id: AskId,
        answer: crate::types::AskAnswer,
        pending: &mut Vec<PendingUserAsk>,
    ) {
        let Some(pos) = pending.iter().position(|p| p.id == id) else {
            return;
        };
        let PendingUserAsk {
            question,
            options,
            reply,
            ..
        } = pending.swap_remove(pos);

        let (recorded, outcome) = match answer {
            crate::types::AskAnswer::Answered(text) => {
                (Some(text.clone()), AskUserOutcome::Answered(text))
            }
            crate::types::AskAnswer::Declined => (None, AskUserOutcome::Declined),
        };
        self.record_ask(&question, &options, recorded);
        let _ = reply.send(outcome);
    }

    /// Write the durable `ask_user` record (HC-7). `answer` is `None` on decline.
    pub(super) fn record_ask(
        &mut self,
        question: &str,
        options: &[String],
        answer: Option<String>,
    ) {
        self.write_transcript(TranscriptEvent::AskUser {
            question: question.to_string(),
            options: options.to_vec(),
            answer,
        });
    }

    /// Handle a `recall` request from the tool (T-10): resolve the turn range
    /// to messages from the in-memory conversation, reduce tool results, and
    /// reply. A pure engine round trip — no filesystem, no network, no
    /// permission gate (§6). The tool future blocks on the oneshot reply.
    pub(super) async fn on_recall(&self, ask: RecallAsk) {
        let RecallAsk { from, to, reply } = ask;
        let messages = self.recall_turns(from, to);
        let outcome = if messages.is_empty() {
            RecallOutcome::Empty
        } else {
            let count = messages.len();
            let content = render_recall(&messages, self.truncate.reduce);
            RecallOutcome::Turns { content, count }
        };
        let _ = reply.send(outcome);
    }

    /// Handle a task-list replace from the `todo` tool (T-11): store the full
    /// list, emit the UI event for the sidebar/inline render, write the
    /// additive transcript event (HC-7), and ack the oneshot so the tool
    /// returns success only after the state is stored.
    pub(super) async fn on_task_list_set(&mut self, ask: TaskListAsk) {
        let TaskListAsk { items, reply } = ask;
        self.task_list = items.clone();
        self.emit(UiEvent::TaskListUpdated { items }).await;
        self.write_transcript(TranscriptEvent::TaskList {
            items: self.task_list.clone(),
        });
        let _ = reply.send(Ok(()));
    }
}
