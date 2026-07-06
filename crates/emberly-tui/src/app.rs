//! The rich TUI's local view-model (Tech Spec §9). The engine owns session
//! state; the frontend keeps a *projection* of it, updated by [`UiEvent`]s and
//! read by the renderer each frame. Keeping this a plain data structure with a
//! pure [`App::apply_event`] reducer is what lets the sidebar/status logic be
//! unit-tested without a terminal (§14; group 11).
//!
//! Group 1 establishes the state and the event/key plumbing with a minimal
//! render; the real layout (group 4), markdown (group 5), diffs (group 6),
//! permission prompt (group 7), and palette (group 8) fill it in.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use emberly_core::{
    Command, PermissionDecision, PermissionId, PermissionRendering, SandboxStatus, ToolCallId,
    UiEvent,
};

use crate::editor::LineEditor;
use crate::theme::Theme;

/// One rendered item in the conversation flow. Group 5 enriches assistant text
/// with the markdown pass; group 6 adds diffs.
#[derive(Debug, Clone, PartialEq)]
pub enum ConvItem {
    /// A prompt the user submitted.
    User(String),
    /// Accumulated assistant text for one turn (deltas append to it).
    Assistant(String),
    /// A tool invocation and its outcome.
    Tool {
        call_id: ToolCallId,
        tool: String,
        summary: String,
        /// `None` while running; `Some(ok)` once finished.
        done: Option<bool>,
    },
    /// A harness-world line (error, retry) — rendered out-of-band from the
    /// conversation voice (Design §6.1).
    Notice(String),
}

/// A file the agent created or changed this session (sidebar list, Design §3.1).
#[derive(Debug, Clone, PartialEq)]
pub struct ModifiedFile {
    pub path: String,
    pub adds: u32,
    pub dels: u32,
}

/// Session identity for the sidebar/header (Design §3.1).
#[derive(Debug, Clone, Default)]
pub struct SessionInfo {
    pub title: String,
    pub provider: String,
    pub model: String,
    pub project_root: String,
}

/// What a key press asked the frontend loop to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Nothing observable (e.g. a plain edit of the input buffer).
    None,
    /// Send this command to the engine.
    Command(Command),
    /// Begin a clean shutdown.
    Quit,
}

/// The complete view-model the renderer reads.
pub struct App {
    pub session: SessionInfo,
    pub conversation: Vec<ConvItem>,
    /// True between the first `AssistantDelta` and `AssistantDone` of a turn.
    pub streaming: bool,
    /// The grapheme-aware input editor (multi-line, history, Thai-correct
    /// cursor motion). See [`crate::editor`].
    pub editor: LineEditor,
    pub context_pct: u8,
    pub context_tokens: u64,
    pub cost_usd: f64,
    pub cost_known: bool,
    /// `None` until the engine reports confinement status (Phase 2).
    pub sandbox: Option<SandboxStatus>,
    pub mode: emberly_core::Mode,
    pub modified_files: Vec<ModifiedFile>,
    /// The permission prompt currently awaiting an answer, if any. While set,
    /// the prompt owns the screen and normal input is suspended (Design §5).
    pub pending_permission: Option<(PermissionId, PermissionRendering)>,
    pub sidebar_visible: bool,
    /// The active theme (Design §2). One source the renderer reads; swapping it
    /// (mode/light-fallback later) is a value change, not a refactor.
    pub theme: Theme,
}

impl App {
    #[must_use]
    pub fn new(session: SessionInfo) -> Self {
        Self {
            session,
            conversation: Vec::new(),
            streaming: false,
            editor: LineEditor::new(),
            context_pct: 0,
            context_tokens: 0,
            cost_usd: 0.0,
            cost_known: false,
            sandbox: None,
            mode: emberly_core::Mode::default(),
            modified_files: Vec::new(),
            pending_permission: None,
            sidebar_visible: true,
            theme: Theme::rich(),
        }
    }

    /// Fold one engine event into the view-model. Pure over `self` — no I/O — so
    /// a canned event sequence can be asserted against the resulting state.
    pub fn apply_event(&mut self, event: UiEvent) {
        match event {
            UiEvent::AssistantDelta { text } => {
                if self.streaming {
                    if let Some(ConvItem::Assistant(buf)) = self.conversation.last_mut() {
                        buf.push_str(&text);
                        return;
                    }
                }
                self.streaming = true;
                self.conversation.push(ConvItem::Assistant(text));
            }
            UiEvent::AssistantDone => self.streaming = false,
            UiEvent::ToolStarted {
                call_id,
                tool,
                summary,
            } => {
                self.streaming = false;
                self.conversation.push(ConvItem::Tool {
                    call_id,
                    tool,
                    summary,
                    done: None,
                });
            }
            UiEvent::ToolFinished {
                call_id,
                ok,
                summary,
            } => {
                if let Some(ConvItem::Tool {
                    done, summary: s, ..
                }) = self.find_tool_mut(&call_id)
                {
                    *done = Some(ok);
                    if !summary.is_empty() {
                        *s = summary;
                    }
                }
            }
            UiEvent::PermissionRequest { id, rendering } => {
                self.pending_permission = Some((id, rendering));
            }
            UiEvent::ContextUsage { pct, tokens } => {
                self.context_pct = pct;
                self.context_tokens = tokens;
            }
            UiEvent::CostEstimate { usd, .. } => {
                self.cost_usd = usd;
                self.cost_known = true;
            }
            UiEvent::SandboxStatus { status } => self.sandbox = Some(status),
            UiEvent::ModeChanged { mode } => self.mode = mode,
            UiEvent::HarnessError { what, why, next } => {
                self.conversation
                    .push(ConvItem::Notice(format!("error: {what} — {why}. {next}")));
            }
            UiEvent::Retrying {
                attempt,
                max_attempts,
                delay_ms,
                reason,
            } => {
                self.conversation.push(ConvItem::Notice(format!(
                    "retrying ({attempt}/{max_attempts}) in {delay_ms}ms — {reason}"
                )));
            }
            UiEvent::SessionMeta {
                title,
                provider,
                model,
                project_root,
                ..
            } => {
                self.session = SessionInfo {
                    title,
                    provider,
                    model,
                    project_root,
                };
            }
            UiEvent::FileModified { path, adds, dels } => self.upsert_modified(path, adds, dels),
            UiEvent::CompactionStatus { message } => {
                self.conversation.push(ConvItem::Notice(message));
            }
            // `#[non_exhaustive]`: unknown future events are ignored, not fatal.
            _ => {}
        }
    }

    fn find_tool_mut(&mut self, call_id: &ToolCallId) -> Option<&mut ConvItem> {
        self.conversation
            .iter_mut()
            .rev()
            .find(|item| matches!(item, ConvItem::Tool { call_id: c, .. } if c == call_id))
    }

    fn upsert_modified(&mut self, path: String, adds: u32, dels: u32) {
        if let Some(existing) = self.modified_files.iter_mut().find(|f| f.path == path) {
            existing.adds = adds;
            existing.dels = dels;
        } else {
            self.modified_files.push(ModifiedFile { path, adds, dels });
        }
    }

    /// Handle a key press, returning the action for the loop to carry out.
    ///
    /// While a permission prompt is open it owns the keyboard: only the
    /// deliberate allow keys approve, and everything else (including Enter and
    /// Esc) denies — deny is the safe default (Design §5). The full prompt
    /// screen and scrolling arrive in group 7; the guarantees hold from now.
    pub fn on_key(&mut self, key: KeyEvent) -> Action {
        if let Some((id, _)) = self.pending_permission.as_ref().map(|(i, r)| (*i, r)) {
            return self.answer_permission(id, key);
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        match key.code {
            // Ctrl-D on an empty line quits; on a non-empty line it deletes
            // forward (readline convention).
            KeyCode::Char('d') if ctrl => {
                if self.editor.is_empty() {
                    return Action::Quit;
                }
                self.editor.delete();
                Action::None
            }
            KeyCode::Char('c') if ctrl => {
                if self.editor.is_empty() {
                    Action::Quit
                } else {
                    self.editor.clear();
                    Action::None
                }
            }
            KeyCode::Char('b') if ctrl => {
                self.sidebar_visible = !self.sidebar_visible;
                Action::None
            }
            // Emacs-style line editing.
            KeyCode::Char('a') if ctrl => self.edit(|e| e.home()),
            KeyCode::Char('e') if ctrl => self.edit(|e| e.end()),
            KeyCode::Char('k') if ctrl => self.edit(|e| e.kill_to_end()),
            KeyCode::Char('w') if ctrl => self.edit(|e| e.delete_word_back()),
            // Shift+Enter (where the terminal reports it) inserts a newline;
            // plain Enter submits.
            KeyCode::Enter if shift => self.edit(|e| e.newline()),
            KeyCode::Enter => match self.editor.submit() {
                Some(text) => Action::Command(Command::UserInput { text }),
                None => Action::None,
            },
            KeyCode::Backspace => self.edit(|e| e.backspace()),
            KeyCode::Delete => self.edit(|e| e.delete()),
            KeyCode::Left if ctrl => self.edit(|e| e.word_left()),
            KeyCode::Right if ctrl => self.edit(|e| e.word_right()),
            KeyCode::Left => self.edit(|e| e.left()),
            KeyCode::Right => self.edit(|e| e.right()),
            KeyCode::Home => self.edit(|e| e.home()),
            KeyCode::End => self.edit(|e| e.end()),
            // Up/Down move between logical lines; at the top/bottom edge they
            // step through input history instead.
            KeyCode::Up => {
                if !self.editor.up() {
                    self.editor.history_prev();
                }
                Action::None
            }
            KeyCode::Down => {
                if !self.editor.down() {
                    self.editor.history_next();
                }
                Action::None
            }
            KeyCode::Char(c) => self.edit(|e| e.insert_char(c)),
            _ => Action::None,
        }
    }

    /// Run an editor mutation and report nothing observable to the loop.
    fn edit(&mut self, f: impl FnOnce(&mut LineEditor)) -> Action {
        f(&mut self.editor);
        Action::None
    }

    /// Insert pasted text (bracketed paste) into the input, unless a permission
    /// prompt is open — nothing may be typed into a decision (Design §5).
    pub fn on_paste(&mut self, text: &str) {
        if self.pending_permission.is_none() {
            self.editor.insert_str(text);
        }
    }

    fn answer_permission(&mut self, id: PermissionId, key: KeyEvent) -> Action {
        let decision = match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => PermissionDecision::AllowOnce,
            KeyCode::Char('s') | KeyCode::Char('S') => PermissionDecision::AllowForSession,
            // Enter, Esc, and anything else deny (Design §5 — deny is default).
            _ => PermissionDecision::Deny,
        };
        self.pending_permission = None;
        Action::Command(Command::PermissionAnswer { id, decision })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use emberly_core::ToolCallId;

    fn app() -> App {
        App::new(SessionInfo::default())
    }

    #[test]
    fn assistant_deltas_accumulate_into_one_item() {
        let mut a = app();
        a.apply_event(UiEvent::AssistantDelta { text: "Hel".into() });
        a.apply_event(UiEvent::AssistantDelta { text: "lo".into() });
        a.apply_event(UiEvent::AssistantDone);
        assert_eq!(a.conversation, vec![ConvItem::Assistant("Hello".into())]);
        assert!(!a.streaming);
    }

    #[test]
    fn tool_finished_marks_the_matching_start() {
        let mut a = app();
        let id = ToolCallId::new("c1");
        a.apply_event(UiEvent::ToolStarted {
            call_id: id.clone(),
            tool: "bash".into(),
            summary: "run: ls".into(),
        });
        a.apply_event(UiEvent::ToolFinished {
            call_id: id.clone(),
            ok: true,
            summary: "exit 0".into(),
        });
        match &a.conversation[0] {
            ConvItem::Tool { done, summary, .. } => {
                assert_eq!(*done, Some(true));
                assert_eq!(summary, "exit 0");
            }
            other => panic!("expected a tool item, got {other:?}"),
        }
    }

    #[test]
    fn modified_files_upsert_by_path() {
        let mut a = app();
        a.apply_event(UiEvent::FileModified {
            path: "a.rs".into(),
            adds: 1,
            dels: 0,
        });
        a.apply_event(UiEvent::FileModified {
            path: "a.rs".into(),
            adds: 3,
            dels: 2,
        });
        assert_eq!(a.modified_files.len(), 1);
        assert_eq!(a.modified_files[0].adds, 3);
        assert_eq!(a.modified_files[0].dels, 2);
    }

    #[test]
    fn permission_defaults_to_deny_on_enter() {
        let mut a = app();
        a.apply_event(UiEvent::PermissionRequest {
            id: PermissionId(1),
            rendering: PermissionRendering {
                tool: "bash".into(),
                summary: "run: rm -rf x".into(),
                detail: "rm -rf x".into(),
                affected_paths: vec![],
                outside_root: false,
                reason: "bash asks".into(),
            },
        });
        let action = a.on_key(KeyEvent::from(KeyCode::Enter));
        assert_eq!(
            action,
            Action::Command(Command::PermissionAnswer {
                id: PermissionId(1),
                decision: PermissionDecision::Deny,
            })
        );
        assert!(a.pending_permission.is_none());
    }

    #[test]
    fn permission_allows_only_on_deliberate_key() {
        let mut a = app();
        a.apply_event(UiEvent::PermissionRequest {
            id: PermissionId(2),
            rendering: PermissionRendering {
                tool: "bash".into(),
                summary: "run: ls".into(),
                detail: "ls".into(),
                affected_paths: vec![],
                outside_root: false,
                reason: "bash asks".into(),
            },
        });
        let action = a.on_key(KeyEvent::from(KeyCode::Char('y')));
        assert_eq!(
            action,
            Action::Command(Command::PermissionAnswer {
                id: PermissionId(2),
                decision: PermissionDecision::AllowOnce,
            })
        );
    }

    #[test]
    fn enter_submits_user_input() {
        let mut a = app();
        a.on_key(KeyEvent::from(KeyCode::Char('h')));
        a.on_key(KeyEvent::from(KeyCode::Char('i')));
        let action = a.on_key(KeyEvent::from(KeyCode::Enter));
        assert_eq!(
            action,
            Action::Command(Command::UserInput { text: "hi".into() })
        );
        assert!(a.editor.is_empty());
    }
}
