//! The agent loop (Tech Spec §2, §3). The engine is one async task owning all
//! mutable session state (the conversation view, token accounting, the
//! permission-id counter). It talks to a frontend only over channels: [`UiEvent`]
//! out, [`Command`] in. There is no shared mutable state.
//!
//! Phase 1 scope: drive a scripted or live provider through completion →
//! tool-call → tool-result iterations, gate tool actions, truncate results at
//! ingestion, and handle cancellation. Sandbox rules (Phase 2), retries
//! (Phase 3), and transcript persistence (Phase 5) layer on later.

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use emberly_providers::{
    CompletionRequest, CompletionStream, ContentBlock, Effort, Message, Provider, ProviderError,
    RetryPolicy, Role, StreamEvent, ToolCallId, ToolSchema,
};
use emberly_sandbox::{Decision, Mode, Query, RuleEngine};
use emberly_tools::{
    truncate_output, PermissionOutcome, PermissionRequest, Sandbox, ToolCtx, ToolRegistry,
    TruncateConfig,
};
use futures::StreamExt;
use time::OffsetDateTime;
use tokio::sync::mpsc;

use crate::command::Command;
use crate::event::UiEvent;
use crate::factory::{ConfigReloader, ProviderFactory};
use crate::gate::{ChannelGate, PermissionAsk};
use crate::id::{PermissionId, SessionId};
use crate::transcript::{
    ConfigProvenance, FileTranscript, NoopSink, TranscriptEvent, TranscriptRecord, TranscriptSink,
};
use crate::types::{PermissionRendering, SandboxStatus, TokenUsage};

/// The session title is the first user message, clipped to this many chars
/// (Tech Spec §16 — the heuristic v1 title).
const TITLE_CLIP: usize = 60;

/// How many trailing messages `/compact` keeps verbatim (`context.
/// keep_recent_turns`, default 6 — Tech Spec §7; config wiring is group 5).
const KEEP_RECENT: usize = 6;

/// Reserve this many tokens for model output when computing context usage,
/// or the model's max output, whichever is smaller (Tech Spec §7).
const OUTPUT_RESERVE: u64 = 8_000;

/// Everything needed to construct an [`Engine`].
pub struct EngineConfig {
    pub provider: Arc<dyn Provider>,
    pub tools: ToolRegistry,
    pub project_root: PathBuf,
    pub model: String,
    pub system: Option<String>,
    pub truncate: TruncateConfig,
    /// Retry policy for retryable provider failures and mid-stream drops.
    pub retry: RetryPolicy,
    /// Session identity — backs the transcript filename (Tech Spec §3.2).
    pub session_id: SessionId,
    /// Directory holding session transcripts, so the engine can roll a new (or
    /// resumed) transcript on an in-session switch (`/new`, `/resume`).
    pub sessions_dir: PathBuf,
    /// Shared handle to the active transcript path, updated on every session
    /// switch so the host's panic/exit path always names the *current* session
    /// (HC-3). The host reads it for the abnormal-exit record and the closing
    /// summary.
    pub active_session_path: Arc<RwLock<PathBuf>>,
    /// Provider label for the transcript `session_start` (e.g. `anthropic`).
    pub provider_label: String,
    /// OS confinement status at startup (Requirements §6.7).
    pub sandbox: SandboxStatus,
    /// The permission rule engine (Requirements §6.1): built-in defaults plus
    /// global + project rules, with the bash allowlist already toggled for the
    /// sandbox status. Session grants accrue in-memory during the run.
    pub rules: RuleEngine,
    /// Override for how bash spawns children. `None` (the production path)
    /// builds the host confined-spawn from `sandbox`. Tests inject a plain
    /// spawner so they can report a confined *status* (to exercise the rule and
    /// mode layers) without the self-exec shim re-executing the test binary.
    pub sandbox_spawn: Option<Arc<dyn Sandbox>>,
    /// Non-default configuration pieces, for `session_start` provenance (C-3).
    pub config_provenance: Vec<ConfigProvenance>,
    /// Where durable events go. Defaults to [`NoopSink`] via
    /// [`EngineConfig::no_transcript`] for tests that don't assert on it.
    pub transcript: Box<dyn TranscriptSink>,
    /// Conversation to start from when resuming a session (Tech Spec §3.3);
    /// empty for a fresh session.
    pub initial_conversation: Vec<Message>,
    /// True when resuming an existing transcript: no fresh `session_start` is
    /// written and the original task is treated as already recorded.
    pub resuming: bool,
    /// A `/compact` summarization-prompt override (P-7); `None` uses the
    /// built-in default.
    pub summary_prompt: Option<String>,
    /// Builds a provider for a profile name on an in-session switch (C-6).
    /// `None` disables switching (e.g. the offline placeholder session).
    pub provider_factory: Option<Arc<dyn ProviderFactory>>,
    /// Re-reads config + prompts from disk on an in-app edit (C-5). `None`
    /// disables live reload (the edit still lands on disk for the next session).
    pub config_reloader: Option<Arc<dyn ConfigReloader>>,
}

impl EngineConfig {
    /// A [`NoopSink`] for the `transcript` field — the convenient default for
    /// callers (and tests) that don't persist a transcript.
    #[must_use]
    pub fn no_transcript() -> Box<dyn TranscriptSink> {
        Box::new(NoopSink)
    }
}

/// A tool call accumulated from the provider stream.
struct PendingToolCall {
    id: ToolCallId,
    name: String,
    args: String,
}

/// How a single completion stream ended.
enum StreamEnd {
    /// Clean finish; the model may have requested tool calls.
    Done { tool_calls: Vec<PendingToolCall> },
    /// The user canceled mid-stream.
    Interrupted,
    /// A mid-stream provider error (Phase 3 will retry; Phase 1 surfaces it).
    Errored(ProviderError),
    /// The stream ended without a `Done` — a dropped connection.
    Dropped,
}

/// The assistant output accumulated while draining one completion stream: the
/// answer text and, distinct from it, the reasoning trail (P-10) plus the
/// opaque signature to replay it on later turns.
#[derive(Default)]
struct TurnOutput {
    text: String,
    /// Reasoning/thinking text as streamed (for display + the transcript).
    reasoning: String,
    /// `(signature, redacted)` for the reasoning block, when the provider sent
    /// one. `redacted` blocks carry opaque `data` here and have no replay text.
    reasoning_signature: Option<(String, bool)>,
}

/// Outcome of running one tool call.
enum ToolCallResult {
    Completed(emberly_tools::ToolOutcome),
    Canceled,
}

/// A permission ask awaiting the user's answer: the id shown to the frontend,
/// the oneshot the blocked tool waits on, and the original request (kept so an
/// "allow for session"/"always allow" answer can be turned into a grant).
struct PendingAsk {
    id: PermissionId,
    reply: tokio::sync::oneshot::Sender<PermissionOutcome>,
    request: PermissionRequest,
}

/// The agent engine.
pub struct Engine {
    provider: Arc<dyn Provider>,
    tools: ToolRegistry,
    project_root: PathBuf,
    model: String,
    /// The active reasoning-effort level for subsequent turns (C-6/P-9). Seeded
    /// from the model's `default_effort`, changed by `SetEffort`, re-seeded on a
    /// model switch. `None` sends no effort (the provider's own default).
    effort: Option<Effort>,
    system: Option<String>,
    truncate: TruncateConfig,
    retry: RetryPolicy,
    gate: Arc<ChannelGate>,
    events_tx: mpsc::Sender<UiEvent>,
    conversation: Vec<Message>,
    /// Cumulative billed tokens this session (summed per request — each
    /// request's input is billed, so this is the cost basis, not the context
    /// size).
    session_usage: TokenUsage,
    /// Running session cost estimate in USD (only when pricing is configured).
    session_cost_usd: f64,
    /// Most recent authoritative prompt-token count = current context size.
    /// `None` until the first provider `Usage`; then it drives the context %.
    context_tokens_authoritative: Option<u64>,
    next_permission_id: u64,
    /// Durable transcript sink (HC-7). Written per event; a `NoopSink` when no
    /// session file is configured.
    transcript: Box<dyn TranscriptSink>,
    session_id: SessionId,
    /// Where to create a new/resumed transcript on an in-session switch.
    sessions_dir: PathBuf,
    /// Shared with the host so the panic/exit path tracks the current session.
    active_session_path: Arc<RwLock<PathBuf>>,
    provider_label: String,
    sandbox: SandboxStatus,
    /// The permission rule engine consulted by the gate (Requirements §6).
    rules: RuleEngine,
    /// The current auto-accept mode (Requirements §6.4). Starts [`Mode::Normal`];
    /// changed only through [`Engine::set_mode`], which gates auto tiers on the
    /// sandbox status.
    mode: Mode,
    /// How bash spawns children: confined via the self-exec shim when the OS
    /// sandbox is active, directly when degraded. Built from `sandbox`.
    sandbox_spawn: Arc<dyn Sandbox>,
    config_provenance: Vec<ConfigProvenance>,
    /// Whether the first (pinned, `original_task`) user message has been
    /// recorded — also gates the one-time `session_title`.
    original_task_recorded: bool,
    /// True when this run resumed an existing transcript (skips `session_start`).
    resuming: bool,
    /// Set when `/compact` arrives mid-turn; performed at the next clean
    /// boundary (Tech Spec §7).
    compact_requested: bool,
    /// Optional `/compact` prompt override (P-7).
    summary_prompt: Option<String>,
    /// Builds a provider on an in-session model switch (C-6); `None` disables it.
    provider_factory: Option<Arc<dyn ProviderFactory>>,
    /// Re-reads config on an in-app edit (C-5); `None` disables live reload.
    config_reloader: Option<Arc<dyn ConfigReloader>>,
}

impl Engine {
    /// Build an engine and the receiver for its internal permission-ask
    /// channel. The caller passes that receiver straight back into
    /// [`run`](Engine::run); it is opaque otherwise.
    #[must_use]
    pub fn new(
        config: EngineConfig,
        events_tx: mpsc::Sender<UiEvent>,
    ) -> (Self, mpsc::Receiver<PermissionAsk>) {
        let (asks_tx, asks_rx) = mpsc::channel(crate::channels::DEFAULT_CHANNEL_CAPACITY);
        // Capture before `config.sandbox` is moved into the struct below.
        let sandbox_spawn: Arc<dyn Sandbox> = config.sandbox_spawn.unwrap_or_else(|| {
            // Fallback (no explicit spawner): confine from the status, but with
            // no recorded git binary — nothing is `.git/`-writable. The binary
            // supplies a fully-configured spawner; this keeps the default safe.
            Arc::new(crate::spawn::HostSandbox::new(
                config.sandbox.is_confined(),
                None,
                String::new(),
            ))
        });
        // Seed the session effort from the model's declared default before the
        // provider is moved into the struct (P-9, Tech Spec §4.6).
        let seed_effort = config.provider.model_info().default_effort;
        let engine = Self {
            provider: config.provider,
            tools: config.tools,
            project_root: config.project_root,
            model: config.model,
            effort: seed_effort,
            system: config.system,
            truncate: config.truncate,
            retry: config.retry,
            gate: Arc::new(ChannelGate { asks: asks_tx }),
            events_tx,
            conversation: config.initial_conversation,
            session_usage: TokenUsage::default(),
            session_cost_usd: 0.0,
            context_tokens_authoritative: None,
            next_permission_id: 0,
            transcript: config.transcript,
            session_id: config.session_id,
            sessions_dir: config.sessions_dir,
            active_session_path: config.active_session_path,
            provider_label: config.provider_label,
            sandbox: config.sandbox,
            rules: config.rules,
            mode: Mode::Normal,
            sandbox_spawn,
            config_provenance: config.config_provenance,
            // On resume the original task already lives in the restored history.
            original_task_recorded: config.resuming,
            resuming: config.resuming,
            compact_requested: false,
            summary_prompt: config.summary_prompt,
            provider_factory: config.provider_factory,
            config_reloader: config.config_reloader,
        };
        (engine, asks_rx)
    }

    /// Run the engine until the command channel closes. Idle between turns,
    /// waiting for a `UserInput`; a turn owns `commands_rx`/`asks_rx` for its
    /// duration (permission answers and cancellation arrive through them).
    pub async fn run(
        mut self,
        mut commands_rx: mpsc::Receiver<Command>,
        mut asks_rx: mpsc::Receiver<PermissionAsk>,
    ) {
        if self.resuming {
            // Continuing an existing transcript: no fresh session_start, but
            // surface the restored context size right away (Design §8.4).
            self.emit_context_usage().await;
        } else {
            self.write_transcript(TranscriptEvent::SessionStart {
                session_id: self.session_id,
                provider: self.provider_label.clone(),
                model: self.model.clone(),
                project_root: self.project_root.display().to_string(),
                sandbox: self.sandbox.clone(),
                config_provenance: self.config_provenance.clone(),
                prompts_version: crate::prompts::VERSION,
            });
        }

        // Sandbox status is always-visible state (Requirements §6.7): surface it
        // at session start, and — when confinement is unavailable — explain the
        // degraded path once, in plain language (Design §8.2).
        self.emit(UiEvent::SandboxStatus {
            status: self.sandbox.clone(),
        })
        .await;
        if !self.sandbox.is_confined() {
            self.emit(UiEvent::Notice {
                message: degraded_notice(&self.sandbox),
            })
            .await;
        }
        // Surface the initial reasoning-effort state so the sidebar and picker
        // start correct (P-9).
        self.emit_effort().await;

        while let Some(command) = commands_rx.recv().await {
            match command {
                Command::UserInput { text } => {
                    self.record_user_message(&text);
                    self.conversation.push(Message::user_text(text));
                    self.emit_context_usage().await;
                    self.run_turn(&mut commands_rx, &mut asks_rx).await;
                    // The engine is idle again; let the frontend stop its
                    // "working" affordance (Design §6.3).
                    self.emit(UiEvent::TurnEnded).await;
                    // A `/compact` sent mid-turn runs now, at the clean boundary
                    // (every tool_use has its tool_result — Tech Spec §7).
                    if std::mem::take(&mut self.compact_requested) {
                        self.compact().await;
                    }
                }
                // No turn is running while idle; these are strays or no-ops here.
                Command::Cancel | Command::PermissionAnswer { .. } => {}
                // Idle is already a clean boundary — compact immediately.
                Command::Compact => self.compact().await,
                // Session switches are only issued at idle (the frontend gates
                // them while a turn runs), so a clean boundary is guaranteed.
                Command::NewSession { session_id } => self.start_new_session(session_id).await,
                Command::ResumeSession { session_id } => self.resume_session(session_id).await,
                Command::SetMode { mode } => self.set_mode(mode).await,
                Command::SwitchModel { profile, model } => {
                    self.switch_model(profile, model).await;
                }
                Command::SetEffort { effort } => self.set_effort(effort).await,
                Command::ReloadConfig => self.reload_config().await,
            }
        }

        // Command channel closed: the frontend is gone. Clean end of session.
        self.write_transcript(TranscriptEvent::SessionEnd { reason: None });
    }

    /// Start a fresh session in place (`/new`): end the current transcript
    /// cleanly and roll a new one under `session_id`, resetting the conversation
    /// and per-session accounting. The new transcript is created *before* the
    /// old one is ended, so a creation failure leaves the current session intact
    /// (transcript failures are never fatal — HC-7).
    async fn start_new_session(&mut self, session_id: SessionId) {
        let path = self.sessions_dir.join(format!("{session_id}.jsonl"));
        let sink = match FileTranscript::create(&self.sessions_dir, session_id) {
            Ok(file) => file,
            Err(error) => {
                self.emit(UiEvent::HarnessError {
                    what: "could not start a new session".into(),
                    why: error.to_string(),
                    next: "staying on the current session".into(),
                })
                .await;
                return;
            }
        };
        self.write_transcript(TranscriptEvent::SessionEnd { reason: None });
        self.transcript = Box::new(sink);
        self.adopt_session(session_id, path, Vec::new(), false);
        self.write_transcript(TranscriptEvent::SessionStart {
            session_id,
            provider: self.provider_label.clone(),
            model: self.model.clone(),
            project_root: self.project_root.display().to_string(),
            sandbox: self.sandbox.clone(),
            config_provenance: self.config_provenance.clone(),
            prompts_version: crate::prompts::VERSION,
        });
        self.emit_context_usage().await;
    }

    /// Resume a saved session by id (`/resume` from the picker): end the current
    /// transcript, reopen the target for append, and replace the live
    /// conversation with the one rebuilt from it. Reading the target *before*
    /// ending the current session keeps the current one intact on any failure.
    async fn resume_session(&mut self, session_id: SessionId) {
        let path = self.sessions_dir.join(format!("{session_id}.jsonl"));
        let loaded = match crate::resume::read_records(&path) {
            Ok(loaded) => loaded,
            Err(error) => {
                self.emit(UiEvent::HarnessError {
                    what: "could not read that session".into(),
                    why: error.to_string(),
                    next: "staying on the current session".into(),
                })
                .await;
                return;
            }
        };
        let conversation = crate::resume::rebuild_conversation(&loaded.records);
        let sink = match FileTranscript::open(&path) {
            Ok(file) => file,
            Err(error) => {
                self.emit(UiEvent::HarnessError {
                    what: "could not open that session for writing".into(),
                    why: error.to_string(),
                    next: "staying on the current session".into(),
                })
                .await;
                return;
            }
        };
        self.write_transcript(TranscriptEvent::SessionEnd { reason: None });
        self.transcript = Box::new(sink);
        // Resuming: the original task lives in the restored history, so no fresh
        // session_start is written (matches launch-time resume — Tech Spec §3.3).
        self.adopt_session(session_id, path, conversation, true);
        self.emit_context_usage().await;
    }

    /// Reset session-scoped state to a freshly adopted session and publish the
    /// new transcript path to the shared handle so the host's panic/exit path
    /// names the current session (HC-3).
    fn adopt_session(
        &mut self,
        session_id: SessionId,
        path: PathBuf,
        conversation: Vec<Message>,
        resuming: bool,
    ) {
        self.session_id = session_id;
        self.conversation = conversation;
        self.original_task_recorded = resuming;
        self.resuming = resuming;
        self.compact_requested = false;
        self.session_usage = TokenUsage::default();
        self.session_cost_usd = 0.0;
        self.context_tokens_authoritative = None;
        self.next_permission_id = 0;
        if let Ok(mut guard) = self.active_session_path.write() {
            *guard = path;
        }
    }

    /// Record a user message durably, tagging the first one as the pinned
    /// `original_task` and deriving the session title from it (Tech Spec §7,
    /// §16).
    fn record_user_message(&mut self, text: &str) {
        let original_task = !self.original_task_recorded;
        self.write_transcript(TranscriptEvent::UserMessage {
            text: text.to_string(),
            original_task,
        });
        if original_task {
            self.original_task_recorded = true;
            self.write_transcript(TranscriptEvent::SessionTitle {
                title: clip_title(text),
            });
        }
    }

    /// Manual `/compact` at a clean boundary (Tech Spec §7). Replaces the middle
    /// of the conversation — everything after the pinned original task and
    /// before the last [`KEEP_RECENT`] messages — with a model-written summary,
    /// keeping the session usable when context grows. The pinned content
    /// (system prompt, original task) is never compacted; the JSONL log is
    /// untouched (the compaction is recorded as one event, replayed on resume).
    async fn compact(&mut self) {
        // Pinned = the original task (the system prompt lives outside the
        // conversation). Keep the tail verbatim; summarize the middle.
        let pinned = usize::from(!self.conversation.is_empty());
        let len = self.conversation.len();
        let keep = KEEP_RECENT.min(len.saturating_sub(pinned));
        let from = pinned;
        let to = len.saturating_sub(keep);
        if to <= from {
            self.emit(UiEvent::CompactionStatus {
                message: "nothing to compact yet".into(),
            })
            .await;
            return;
        }

        self.emit(UiEvent::CompactionStatus {
            message: "compacting the conversation…".into(),
        })
        .await;

        let summary = match self.summarize(&self.conversation[from..to]).await {
            Ok(text) if !text.is_empty() => text,
            // Failure fallback (Tech Spec §7): drop the middle behind a
            // placeholder with a visible warning — a full context never yields a
            // stuck session. Recorded as a compaction so resume stays consistent.
            other => {
                let why = match other {
                    Ok(_) => "the summary came back empty".to_string(),
                    Err(error) => error.to_string(),
                };
                self.emit(UiEvent::CompactionStatus {
                    message: format!("summarization failed ({why}); truncated older context"),
                })
                .await;
                format!("[older context was truncated — summarization failed: {why}]")
            }
        };

        // Rebuild: [original task][summary as user message][recent verbatim].
        let mut rebuilt = Vec::with_capacity(2 + keep);
        rebuilt.extend(self.conversation[..from].iter().cloned());
        rebuilt.push(Message::user_text(summary.clone()));
        rebuilt.extend(self.conversation[to..].iter().cloned());
        self.conversation = rebuilt;

        self.write_transcript(TranscriptEvent::Compaction {
            summary,
            replaced_from: u32::try_from(from).unwrap_or(u32::MAX),
            replaced_to: u32::try_from(to).unwrap_or(u32::MAX),
        });
        self.emit(UiEvent::CompactionStatus {
            message: format!("compacted — kept the task, a summary, and the last {keep} messages"),
        })
        .await;
        self.emit_context_usage().await;
    }

    /// Summarize a slice of the conversation with the current provider using the
    /// purpose-built prompt (Tech Spec §7). Drains the stream collecting text;
    /// tool calls are not offered.
    async fn summarize(&self, messages: &[Message]) -> Result<String, ProviderError> {
        let prompt = self
            .summary_prompt
            .as_deref()
            .unwrap_or_else(|| crate::prompts::compact());
        let request = CompletionRequest {
            model: self.model.clone(),
            system: Some(prompt.to_string()),
            messages: vec![Message::user_text(render_for_summary(messages))],
            tools: Vec::new(),
            max_output_tokens: Some(self.provider.model_info().max_output_tokens),
            temperature: None,
            // Summarization is a fixed internal task; it does not carry the
            // session's reasoning effort.
            effort: None,
        };
        let mut stream = self.provider.stream_completion(request).await?;
        let mut text = String::new();
        while let Some(item) = stream.next().await {
            if let StreamEvent::TextDelta { text: delta } = item? {
                text.push_str(&delta);
            }
        }
        Ok(text.trim().to_string())
    }

    /// Drive completions until the model stops without requesting tools, an
    /// error/drop occurs, or the user cancels.
    async fn run_turn(
        &mut self,
        commands_rx: &mut mpsc::Receiver<Command>,
        asks_rx: &mut mpsc::Receiver<PermissionAsk>,
    ) {
        let mut drop_attempts = 0u32;
        loop {
            let stream = match self.open_stream_with_retry().await {
                Ok(stream) => stream,
                Err(error) => {
                    self.emit_provider_error(&error).await;
                    return;
                }
            };

            let (end, out) = self.consume_stream(stream, commands_rx).await;
            match end {
                StreamEnd::Done { tool_calls } => {
                    self.push_assistant_message(&out, &tool_calls);
                    self.emit_context_usage().await;
                    self.emit(UiEvent::AssistantDone).await;
                    if tool_calls.is_empty() {
                        return; // model finished its turn
                    }
                    if self
                        .run_tool_calls(tool_calls, commands_rx, asks_rx)
                        .await
                        .is_canceled()
                    {
                        return;
                    }
                    // Loop: send the tool results back for another completion.
                }
                StreamEnd::Interrupted => {
                    // Keep the partial text visible in the conversation.
                    self.push_assistant_message(&out, &[]);
                    self.emit_context_usage().await;
                    self.emit(UiEvent::AssistantDone).await;
                    return;
                }
                StreamEnd::Errored(error) => {
                    self.emit(UiEvent::AssistantDone).await;
                    self.emit_provider_error(&error).await;
                    return;
                }
                StreamEnd::Dropped => {
                    // The partial text was shown live; it is NOT stitched into
                    // the retry (partial turns are not stitched — Tech Spec
                    // §4.3). Retry the whole turn from the unchanged history.
                    drop_attempts += 1;
                    if self.retry.may_retry(drop_attempts) {
                        let delay = self.retry.delay_for(drop_attempts);
                        self.emit_retrying(
                            drop_attempts,
                            delay,
                            "the model response ended unexpectedly",
                        )
                        .await;
                        self.emit(UiEvent::AssistantDone).await;
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                    self.emit(UiEvent::AssistantDone).await;
                    self.emit(UiEvent::HarnessError {
                        what: "the model response kept ending unexpectedly".into(),
                        why: "the provider stream closed before completing, repeatedly".into(),
                        next: "send your message again to retry".into(),
                    })
                    .await;
                    return;
                }
            }
        }
    }

    /// Open a completion stream, retrying retryable pre-stream failures with
    /// backoff (Tech Spec §4.3). Every retry is surfaced (Design §6.1).
    async fn open_stream_with_retry(&mut self) -> Result<CompletionStream, ProviderError> {
        let mut attempts = 0u32;
        loop {
            attempts += 1;
            let request = self.build_request();
            match self.provider.stream_completion(request).await {
                Ok(stream) => return Ok(stream),
                Err(error) => {
                    if !error.is_retryable() || !self.retry.may_retry(attempts) {
                        return Err(error);
                    }
                    let delay = error
                        .retry_after()
                        .unwrap_or_else(|| self.retry.delay_for(attempts));
                    self.emit_retrying(attempts, delay, &error.to_string())
                        .await;
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    async fn emit_retrying(&self, attempt: u32, delay: std::time::Duration, reason: &str) {
        self.emit(UiEvent::Retrying {
            attempt,
            max_attempts: self.retry.max_attempts,
            delay_ms: u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
            reason: reason.to_string(),
        })
        .await;
    }

    /// Read one completion stream to its end, emitting text deltas and
    /// accumulating tool calls. Cancellable at each await point.
    async fn consume_stream(
        &mut self,
        mut stream: CompletionStream,
        commands_rx: &mut mpsc::Receiver<Command>,
    ) -> (StreamEnd, TurnOutput) {
        let mut out = TurnOutput::default();
        let mut tool_calls: Vec<PendingToolCall> = Vec::new();
        let mut saw_done = false;

        let end = loop {
            tokio::select! {
                item = stream.next() => match item {
                    Some(Ok(event)) => {
                        self.handle_stream_event(event, &mut out, &mut tool_calls, &mut saw_done)
                            .await;
                    }
                    Some(Err(error)) => break StreamEnd::Errored(error),
                    None => break if saw_done { StreamEnd::Done { tool_calls: std::mem::take(&mut tool_calls) } } else { StreamEnd::Dropped },
                },
                Some(command) = commands_rx.recv() => {
                    match command {
                        Command::Cancel => break StreamEnd::Interrupted,
                        // Queue a compaction for the clean boundary (Tech Spec §7).
                        Command::Compact => self.compact_requested = true,
                        // A mode toggle applies immediately, even mid-stream.
                        Command::SetMode { mode } => self.set_mode(mode).await,
                        // Ignore permission answers / other commands mid-stream.
                        _ => {}
                    }
                }
            }
        };

        // The caller commits the assistant message: on Done/Interrupted it is
        // kept; on a retryable Dropped it is discarded (not stitched, §4.3).
        (end, out)
    }

    /// Apply one stream event. Terminal events (`Done`) only set `saw_done`;
    /// the stream loop keeps draining until EOF so trailing chunks — notably
    /// the OpenAI-compatible `usage` chunk, which arrives after finish_reason —
    /// are still processed. The loop finalizes via its `None` (EOF) arm.
    async fn handle_stream_event(
        &mut self,
        event: StreamEvent,
        out: &mut TurnOutput,
        tool_calls: &mut Vec<PendingToolCall>,
        saw_done: &mut bool,
    ) {
        match event {
            StreamEvent::TextDelta { text: delta } => {
                out.text.push_str(&delta);
                self.emit(UiEvent::AssistantDelta { text: delta }).await;
            }
            StreamEvent::ReasoningDelta { text: delta } => {
                out.reasoning.push_str(&delta);
                self.emit(UiEvent::ReasoningDelta { text: delta }).await;
            }
            StreamEvent::ReasoningSignature {
                signature,
                redacted,
            } => {
                // Opaque replay token for the reasoning block (P-1); kept to
                // echo back on later tool-use turns.
                out.reasoning_signature = Some((signature, redacted));
            }
            StreamEvent::ToolCallStart { id, name } => {
                tool_calls.push(PendingToolCall {
                    id,
                    name,
                    args: String::new(),
                });
            }
            StreamEvent::ToolCallDelta { id, args_delta } => {
                if let Some(call) = tool_calls.iter_mut().find(|c| c.id == id) {
                    call.args.push_str(&args_delta);
                }
            }
            StreamEvent::ToolCallEnd { .. } => {}
            StreamEvent::Usage { usage } => {
                // Accumulate billed tokens and cost (each request's input is
                // billed), and record the prompt size as the current context.
                self.session_usage.input = self.session_usage.input.saturating_add(usage.input);
                self.session_usage.output = self.session_usage.output.saturating_add(usage.output);
                self.context_tokens_authoritative = Some(usage.input);
                if let Some(pricing) = self.provider.model_info().pricing {
                    self.session_cost_usd += pricing.estimate_usd(usage);
                }
            }
            StreamEvent::Done { stop_reason: _ } => {
                // Mark done but keep draining the stream: OpenAI-compatible
                // servers send the `usage` chunk *after* the finish_reason
                // chunk (real OpenAI does too). Breaking here would drop it and
                // leave session token counts at 0. The stream's `None` arm
                // finalizes with `StreamEnd::Done` once `saw_done` is set.
                *saw_done = true;
            }
            // `StreamEvent` is non-exhaustive; ignore variants added later.
            _ => {}
        }
    }

    /// Execute tool calls sequentially, appending each result to the
    /// conversation. Stops early on cancellation, backfilling canceled results
    /// so the conversation stays well-formed (every tool_use has a result).
    async fn run_tool_calls(
        &mut self,
        tool_calls: Vec<PendingToolCall>,
        commands_rx: &mut mpsc::Receiver<Command>,
        asks_rx: &mut mpsc::Receiver<PermissionAsk>,
    ) -> ToolCallResult {
        let mut iter = tool_calls.into_iter();
        while let Some(call) = iter.next() {
            match self.run_one_tool_call(&call, commands_rx, asks_rx).await {
                ToolCallResult::Completed(outcome) => self.ingest_tool_result(&call, outcome).await,
                ToolCallResult::Canceled => {
                    self.push_canceled_result(&call).await;
                    for remaining in iter {
                        self.push_canceled_result(&remaining).await;
                    }
                    return ToolCallResult::Canceled;
                }
            }
        }
        ToolCallResult::Completed(emberly_tools::ToolOutcome::success("", ""))
    }

    /// Run one tool call, driving its execution concurrently with permission
    /// asks and cancellation.
    async fn run_one_tool_call(
        &mut self,
        call: &PendingToolCall,
        commands_rx: &mut mpsc::Receiver<Command>,
        asks_rx: &mut mpsc::Receiver<PermissionAsk>,
    ) -> ToolCallResult {
        let args = serde_json::from_str(&call.args).unwrap_or(serde_json::Value::Null);

        let Some(tool) = self.tools.get(&call.name) else {
            return ToolCallResult::Completed(emberly_tools::ToolOutcome::failure(
                format!("unknown tool: {}", call.name),
                "unknown tool",
            ));
        };

        // Describe the invocation from its arguments (e.g. `run: cargo test`,
        // `read src/main.rs`) so the activity line says what is happening, not
        // just the tool name (Design §6.3).
        let summary = tool.describe(&args).unwrap_or_else(|| call.name.clone());
        self.emit(UiEvent::ToolStarted {
            call_id: call.id.clone(),
            tool: call.name.clone(),
            summary,
        })
        .await;

        let ctx = self.make_ctx();
        let mut exec = Box::pin(tool.execute(args, &ctx));
        let mut pending: Vec<PendingAsk> = Vec::new();
        let mut commands_open = true;

        loop {
            tokio::select! {
                outcome = &mut exec => return ToolCallResult::Completed(outcome),
                Some(ask) = asks_rx.recv() => self.on_permission_ask(ask, &mut pending).await,
                command = commands_rx.recv(), if commands_open => match command {
                    Some(Command::PermissionAnswer { id, decision }) => {
                        self.answer_permission(id, decision, &mut pending).await;
                    }
                    Some(Command::Cancel) => return ToolCallResult::Canceled,
                    // Queue a compaction for the clean boundary (Tech Spec §7).
                    Some(Command::Compact) => self.compact_requested = true,
                    // A mode toggle applies immediately to later asks this turn.
                    Some(Command::SetMode { mode }) => self.set_mode(mode).await,
                    Some(_) => {}
                    None => {
                        // No more input (frontend gone): deny anything pending
                        // as the safe default and stop watching commands, so we
                        // never hang on an answer that cannot arrive.
                        commands_open = false;
                        for p in pending.drain(..) {
                            let _ = p.reply.send(PermissionOutcome::Deny);
                        }
                    }
                },
            }
        }
    }

    /// Resolve a user's answer to a pending prompt: record it, apply any grant
    /// (session or persisted), and reply to the blocked tool. Unknown ids are
    /// ignored (a stray or already-answered prompt).
    async fn answer_permission(
        &mut self,
        id: PermissionId,
        decision: crate::types::PermissionDecision,
        pending: &mut Vec<PendingAsk>,
    ) {
        let Some(pos) = pending.iter().position(|p| p.id == id) else {
            return;
        };
        let PendingAsk { request, reply, .. } = pending.swap_remove(pos);

        // A grant widens future asks. HC-4: outside-root actions can never be
        // turned into a rule (only ever allowed once), so a session/project
        // grant is applied only for in-root actions.
        if decision.is_allow() && !request.outside_root {
            match decision {
                crate::types::PermissionDecision::AllowForSession => {
                    self.grant_for_session(&request);
                }
                crate::types::PermissionDecision::AlwaysAllowInProject => {
                    self.grant_for_session(&request);
                    self.persist_project_grant(&request).await;
                }
                _ => {}
            }
        }

        let executed = decision.is_allow().then(|| request.summary.clone());
        self.write_transcript(TranscriptEvent::PermissionDecision {
            id,
            decision,
            executed,
        });
        let outcome = if decision.is_allow() {
            PermissionOutcome::Allow
        } else {
            PermissionOutcome::Deny
        };
        let _ = reply.send(outcome);
    }

    /// Change the auto-accept mode (Requirements §6.4, §6.7). Auto tiers are
    /// gated on active confinement by [`Mode::resolve`]; a refused escalation
    /// leaves the mode unchanged and explains why (the type system, not this
    /// method, guarantees an auto mode is never entered while degraded).
    async fn set_mode(&mut self, requested: Mode) {
        match Mode::resolve(requested, &self.sandbox) {
            Ok(mode) => {
                if mode == self.mode {
                    return;
                }
                self.mode = mode;
                self.write_transcript(TranscriptEvent::ModeChange { mode });
                self.emit(UiEvent::ModeChanged { mode }).await;
            }
            Err(unavailable) => {
                self.emit(UiEvent::Notice {
                    message: format!(
                        "auto-accept modes need OS confinement — {} (staying in {})",
                        unavailable.reason,
                        mode_label(self.mode),
                    ),
                })
                .await;
            }
        }
    }

    /// Switch the active provider/model for subsequent turns (C-6). Runs at
    /// this clean boundary (idle) and never rewrites prior turns; a failure to
    /// build (unknown profile, missing key) is a harness-world error and the
    /// current model stays active.
    async fn switch_model(&mut self, profile: String, model: Option<String>) {
        let Some(factory) = self.provider_factory.clone() else {
            self.emit(UiEvent::Notice {
                message: "switching models is not available in this session".into(),
            })
            .await;
            return;
        };
        let model = model.unwrap_or_else(|| self.model.clone());
        match factory.build(&profile, &model) {
            Ok(choice) => {
                if choice.profile == self.provider_label && choice.model == self.model {
                    return; // no-op: already active
                }
                self.provider = choice.provider;
                self.provider_label = choice.profile.clone();
                self.model = choice.model.clone();
                self.write_transcript(TranscriptEvent::ModelSwitch {
                    provider: choice.profile.clone(),
                    model: choice.model.clone(),
                });
                self.emit(UiEvent::ModelChanged {
                    provider: choice.profile.clone(),
                    model: choice.model.clone(),
                })
                .await;
                self.emit(UiEvent::Notice {
                    message: format!("switched to {} / {}", choice.profile, choice.model),
                })
                .await;
                // Re-seed the reasoning effort to the new model's default — its
                // available levels and default differ per model (P-9). Always
                // re-emit so the sidebar/picker track the new model's levels
                // even when the default happens to match.
                self.effort = self.provider.model_info().default_effort;
                self.emit_effort().await;
            }
            Err(why) => {
                self.emit(UiEvent::HarnessError {
                    what: format!("could not switch to '{profile}'"),
                    why,
                    next: format!("staying on {} / {}", self.provider_label, self.model),
                })
                .await;
            }
        }
    }

    /// Set the session reasoning effort for subsequent turns (C-6/P-9). Logged
    /// to the transcript (HC-7) and announced — never silent. A model with no
    /// effort control still accepts the setting; the adapter drops it at the
    /// wire (P-9), so setting it is never an error.
    async fn set_effort(&mut self, effort: Effort) {
        // The engine is the authority on what a model supports (P-9): decline
        // (calmly, never an error) when the model has no control or the level
        // isn't offered, so no frontend can announce a change that won't happen.
        let levels = self.provider.model_info().effort_levels;
        if levels.is_empty() {
            self.emit(UiEvent::Notice {
                message: "this model has no reasoning-effort control".into(),
            })
            .await;
            return;
        }
        if !levels.contains(&effort) {
            self.emit(UiEvent::Notice {
                message: format!("this model does not offer '{effort}' reasoning effort"),
            })
            .await;
            return;
        }
        if self.effort == Some(effort) {
            return; // no-op: already active
        }
        self.effort = Some(effort);
        self.write_transcript(TranscriptEvent::EffortChange { effort });
        self.emit_effort().await;
        self.emit(UiEvent::Notice {
            message: format!("reasoning effort set to {effort}"),
        })
        .await;
    }

    /// Emit the current effort and the active model's available levels (P-9), so
    /// the sidebar and the effort picker stay in sync with the model.
    async fn emit_effort(&self) {
        self.emit(UiEvent::EffortChanged {
            effort: self.effort,
            available: self.provider.model_info().effort_levels,
        })
        .await;
    }

    /// Re-read config + prompts from disk and apply the live pieces to the
    /// running session (C-5): the system/compact prompts and the provider
    /// profile set (so a newly-added profile is switchable and shows in the
    /// picker). The active provider/model is left as-is — use `/model` to
    /// switch. Restart-only changes are named, not applied. Applies to
    /// subsequent turns; never rewrites prior turns or the transcript.
    async fn reload_config(&mut self) {
        let Some(reloader) = self.config_reloader.clone() else {
            self.emit(UiEvent::Notice {
                message: "config reload is not available in this session".into(),
            })
            .await;
            return;
        };
        let reloaded = match reloader.reload() {
            Ok(reloaded) => reloaded,
            Err(why) => {
                self.emit(UiEvent::HarnessError {
                    what: "could not reload config".into(),
                    why,
                    next: "keeping the current config".into(),
                })
                .await;
                return;
            }
        };

        let mut changed = Vec::new();
        if reloaded.system != self.system {
            self.system = reloaded.system;
            changed.push("system prompt");
        }
        if reloaded.summary_prompt != self.summary_prompt {
            self.summary_prompt = reloaded.summary_prompt;
            changed.push("compact prompt");
        }
        let old_profiles = self
            .provider_factory
            .as_ref()
            .map(|factory| factory.profiles())
            .unwrap_or_default();
        if reloaded.profiles != old_profiles {
            changed.push("provider profiles");
            self.emit(UiEvent::ProfilesChanged {
                profiles: reloaded.profiles.clone(),
            })
            .await;
        }
        self.provider_factory = Some(reloaded.provider_factory);

        let mut message = if changed.is_empty() {
            "reloaded config — no live changes".to_string()
        } else {
            format!("reloaded: {}", changed.join(", "))
        };
        for note in &reloaded.restart_notes {
            message.push_str("; ");
            message.push_str(note);
        }
        self.emit(UiEvent::Notice { message }).await;
    }

    /// Add an in-memory session grant from an approved request (Requirements
    /// §6.6): a bash command prefix, or a per-tool allow for file writes/edits.
    fn grant_for_session(&mut self, request: &PermissionRequest) {
        self.rules.add_session_grant(grant_rule(request));
    }

    /// Persist an approved request as a project rule in `.agents/permissions.toml`
    /// and show the user the exact line written (Requirements §6.6). Best-effort:
    /// a write failure degrades to a session-only grant with a notice, never a
    /// crash (HC-7).
    async fn persist_project_grant(&mut self, request: &PermissionRequest) {
        let path = self.project_root.join(".agents").join("permissions.toml");
        let block = grant_rule(request).to_toml_block();
        match append_rule_block(&path, &block) {
            Ok(()) => {
                self.emit(UiEvent::Notice {
                    message: format!("saved to .agents/permissions.toml:\n{}", block.trim_end()),
                })
                .await;
            }
            Err(error) => {
                self.emit(UiEvent::Notice {
                    message: format!(
                        "couldn't write .agents/permissions.toml ({error}); allowed for this session only"
                    ),
                })
                .await;
            }
        }
    }

    /// Consult the rule engine for a tool's ask (Requirements §6): `Allow` runs
    /// silently (no prompt, no UI churn), `Deny` auto-denies with the reason,
    /// and `Ask` raises the prompt as before. Every request and decision is
    /// recorded, including the auto ones (HC-7).
    async fn on_permission_ask(&mut self, ask: PermissionAsk, pending: &mut Vec<PendingAsk>) {
        let request = ask.request;
        let outcome = self.rules.evaluate(&make_query(&request), self.mode);
        let id = self.take_permission_id();
        let rendering = build_rendering(&request, outcome.reason.clone());

        match outcome.decision {
            Decision::Allow => {
                self.write_transcript(TranscriptEvent::PermissionRequest { id, rendering });
                self.write_transcript(TranscriptEvent::PermissionDecision {
                    id,
                    decision: crate::types::PermissionDecision::AllowOnce,
                    executed: Some(request.summary.clone()),
                });
                let _ = ask.reply.send(PermissionOutcome::Allow);
            }
            Decision::Deny => {
                self.write_transcript(TranscriptEvent::PermissionRequest { id, rendering });
                self.write_transcript(TranscriptEvent::PermissionDecision {
                    id,
                    decision: crate::types::PermissionDecision::Deny,
                    executed: None,
                });
                self.emit(UiEvent::Notice {
                    message: format!("auto-denied {}: {}", request.tool, outcome.reason),
                })
                .await;
                let _ = ask.reply.send(PermissionOutcome::Deny);
            }
            Decision::Ask => {
                self.write_transcript(TranscriptEvent::PermissionRequest {
                    id,
                    rendering: rendering.clone(),
                });
                self.emit(UiEvent::PermissionRequest { id, rendering })
                    .await;
                pending.push(PendingAsk {
                    id,
                    reply: ask.reply,
                    request,
                });
            }
        }
    }

    /// Truncate a tool result at ingestion (§8.1), append it to the
    /// conversation, and emit UI events (finish, file change, usage).
    async fn ingest_tool_result(
        &mut self,
        call: &PendingToolCall,
        outcome: emberly_tools::ToolOutcome,
    ) {
        let truncation = truncate_output(&outcome.content, &self.truncate);

        // Durable record: the model-visible (possibly truncated) output, plus a
        // sidecar holding the full output when truncated (Requirements §8.1).
        let full_output_ref = if truncation.truncated {
            self.transcript.sidecar(&call.id, &outcome.content)
        } else {
            None
        };
        self.write_transcript(TranscriptEvent::ToolResult {
            call_id: call.id.clone(),
            ok: outcome.ok,
            output: truncation.content.clone(),
            truncated: truncation.truncated,
            full_output_ref,
        });

        self.conversation.push(Message::tool_result(
            call.id.clone(),
            truncation.content,
            !outcome.ok,
        ));

        self.emit(UiEvent::ToolFinished {
            call_id: call.id.clone(),
            ok: outcome.ok,
            summary: outcome.summary,
            preview: result_preview(&outcome.content),
        })
        .await;

        if let Some(change) = outcome.file_change {
            self.emit(UiEvent::FileModified {
                path: change.path.clone(),
                adds: change.adds,
                dels: change.dels,
            })
            .await;
            if let Some(unified) = change.diff {
                self.emit(UiEvent::FileDiff {
                    path: change.path,
                    unified,
                })
                .await;
            }
        }
        self.emit_context_usage().await;
    }

    async fn push_canceled_result(&mut self, call: &PendingToolCall) {
        let canceled = "The user canceled before this tool ran.".to_string();
        // Keep the transcript well-formed: every tool_call has a tool_result.
        self.write_transcript(TranscriptEvent::ToolResult {
            call_id: call.id.clone(),
            ok: false,
            output: canceled.clone(),
            truncated: false,
            full_output_ref: None,
        });
        self.conversation
            .push(Message::tool_result(call.id.clone(), canceled, true));
        self.emit(UiEvent::ToolFinished {
            call_id: call.id.clone(),
            ok: false,
            summary: "canceled".into(),
            preview: String::new(),
        })
        .await;
    }

    /// Build the assistant message for the turn: reasoning (P-10), then text,
    /// then any tool-use blocks. Records the complete assistant message — with
    /// reasoning as a distinct field, never merged into the answer — and each
    /// requested tool call to the transcript (Tech Spec §3.2, §4.7).
    fn push_assistant_message(&mut self, out: &TurnOutput, tool_calls: &[PendingToolCall]) {
        let text = out.text.as_str();
        let has_reasoning = !out.reasoning.is_empty() || out.reasoning_signature.is_some();
        if text.is_empty() && tool_calls.is_empty() && !has_reasoning {
            return;
        }
        if !text.is_empty() || has_reasoning {
            self.write_transcript(TranscriptEvent::AssistantMessage {
                text: text.to_string(),
                // Recorded whatever the view key: `hidden` is a view choice, not
                // a discard (P-10, Design §4.4).
                reasoning: (!out.reasoning.is_empty()).then(|| out.reasoning.clone()),
            });
        }
        let mut content = Vec::new();
        // Reasoning must precede text/tool_use so a provider that requires the
        // thinking block echoed back accepts the turn (Anthropic ordering).
        if has_reasoning {
            let (signature, redacted) = match &out.reasoning_signature {
                Some((sig, red)) => (Some(sig.clone()), *red),
                None => (None, false),
            };
            content.push(ContentBlock::Reasoning {
                // A redacted block has no replayable text; a normal one replays
                // the exact reasoning it streamed.
                text: if redacted {
                    String::new()
                } else {
                    out.reasoning.clone()
                },
                signature,
                redacted,
            });
        }
        if !text.is_empty() {
            content.push(ContentBlock::Text {
                text: text.to_string(),
            });
        }
        for call in tool_calls {
            let input = serde_json::from_str(&call.args).unwrap_or(serde_json::Value::Null);
            self.write_transcript(TranscriptEvent::ToolCall {
                call_id: call.id.clone(),
                tool: call.name.clone(),
                args: input.clone(),
            });
            content.push(ContentBlock::ToolUse {
                id: call.id.clone(),
                name: call.name.clone(),
                input,
            });
        }
        self.conversation.push(Message {
            role: Role::Assistant,
            content,
        });
    }

    fn build_request(&self) -> CompletionRequest {
        let tools = self
            .tools
            .specs()
            .into_iter()
            .map(|spec| ToolSchema {
                name: spec.name,
                description: spec.description,
                input_schema: spec.input_schema,
            })
            .collect();
        CompletionRequest {
            model: self.model.clone(),
            system: self.system.clone(),
            messages: self.conversation.clone(),
            tools,
            max_output_tokens: Some(self.provider.model_info().max_output_tokens),
            temperature: None,
            // The session's active reasoning effort (P-9). The adapter maps it
            // to the provider's control or drops it when unsupported.
            effort: self.effort,
        }
    }

    fn make_ctx(&self) -> ToolCtx {
        ToolCtx::new(
            self.project_root.clone(),
            self.truncate,
            self.gate.clone(),
            self.sandbox_spawn.clone(),
        )
    }

    fn take_permission_id(&mut self) -> PermissionId {
        let id = PermissionId(self.next_permission_id);
        self.next_permission_id += 1;
        id
    }

    async fn emit(&self, event: UiEvent) {
        let _ = self.events_tx.send(event).await;
    }

    /// Append one durable event to the transcript (HC-7). Best-effort: the sink
    /// swallows I/O errors. Stamped with the current time at the write edge.
    fn write_transcript(&mut self, event: TranscriptEvent) {
        let record = TranscriptRecord::new(OffsetDateTime::now_utc(), event);
        self.transcript.record(&record);
    }

    async fn emit_provider_error(&self, error: &ProviderError) {
        self.emit(UiEvent::HarnessError {
            what: "the model request failed".into(),
            why: error.to_string(),
            next: "send your message again to retry".into(),
        })
        .await;
    }

    /// Emit context usage and, when pricing is configured, the running cost
    /// estimate (Requirements §8.4, P-6; Design §3.1). Uses the provider's
    /// authoritative prompt-token count once available, falling back to a
    /// chars/4 estimate before the first `Usage`.
    async fn emit_context_usage(&self) {
        let info = self.provider.model_info();
        let reserve = OUTPUT_RESERVE.min(u64::from(info.max_output_tokens));
        let budget = u64::from(info.context_window)
            .saturating_sub(reserve)
            .max(1);
        let tokens = self
            .context_tokens_authoritative
            .unwrap_or_else(|| self.context_tokens());
        let pct = ((tokens.saturating_mul(100)) / budget).min(100);
        self.emit(UiEvent::ContextUsage {
            pct: u8::try_from(pct).unwrap_or(100),
            tokens,
        })
        .await;

        // Cumulative session tokens — always available (independent of pricing).
        self.emit(UiEvent::SessionUsage {
            usage: self.session_usage,
        })
        .await;

        // Cost is only knowable with a pricing table (always labeled "est.").
        if info.pricing.is_some() {
            self.emit(UiEvent::CostEstimate {
                usage: self.session_usage,
                usd: self.session_cost_usd,
            })
            .await;
        }
    }

    fn context_tokens(&self) -> u64 {
        let count = |s: &str| self.provider.count_tokens(s).tokens;
        let mut total = self.system.as_deref().map(count).unwrap_or(0);
        for message in &self.conversation {
            for block in &message.content {
                total = total.saturating_add(match block {
                    ContentBlock::Text { text } => count(text),
                    ContentBlock::ToolUse { name, input, .. } => {
                        count(name).saturating_add(count(&input.to_string()))
                    }
                    ContentBlock::ToolResult { content, .. } => count(content),
                    // Replayed reasoning is sent back on the wire, so it counts
                    // toward the context budget (P-10).
                    ContentBlock::Reasoning { text, .. } => count(text),
                });
            }
        }
        total
    }
}

impl ToolCallResult {
    fn is_canceled(&self) -> bool {
        matches!(self, ToolCallResult::Canceled)
    }
}

/// The heuristic session title: the first user message, trimmed and clipped
/// to [`TITLE_CLIP`] characters with an ellipsis when cut (Tech Spec §16).
fn clip_title(text: &str) -> String {
    let trimmed = text.trim();
    let mut title: String = trimmed.chars().take(TITLE_CLIP).collect();
    if trimmed.chars().count() > TITLE_CLIP {
        title.push('…');
    }
    title
}

/// Render conversation messages as plain text for the summarization prompt: one
/// `role: …` block per message, tool calls and results flattened to text.
fn render_for_summary(messages: &[Message]) -> String {
    let mut out = String::new();
    for message in messages {
        let role = match message.role {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
            Role::System => "system",
        };
        for block in &message.content {
            let piece = match block {
                ContentBlock::Text { text } => text.clone(),
                ContentBlock::ToolUse { name, input, .. } => format!("[tool call: {name} {input}]"),
                ContentBlock::ToolResult { content, .. } => format!("[tool result: {content}]"),
                // Reasoning is the model's private scratch, not conversation
                // content; the summary is built from the answer, so skip it.
                ContentBlock::Reasoning { .. } => String::new(),
            };
            if !piece.is_empty() {
                out.push_str(role);
                out.push_str(": ");
                out.push_str(&piece);
                out.push('\n');
            }
        }
    }
    out
}

/// Number of result lines shown inline under a finished tool call.
const PREVIEW_LINES: usize = 8;
/// Character ceiling for the inline preview, so a single very long line cannot
/// flood the conversation.
const PREVIEW_CHARS: usize = 600;

/// A short excerpt of a tool's output for the conversation (Design §6.1): the
/// first few lines, char-capped. The full output goes to the model; this is
/// just what the user glances at.
fn result_preview(content: &str) -> String {
    let mut preview: String = content
        .lines()
        .take(PREVIEW_LINES)
        .collect::<Vec<_>>()
        .join("\n");
    if preview.chars().count() > PREVIEW_CHARS {
        preview = preview.chars().take(PREVIEW_CHARS).collect();
    }
    preview
}

/// Enrich a tool's [`PermissionRequest`] into a UI [`PermissionRendering`] with
/// the `reason` the rule engine produced — the matched rule or the hard line —
/// shown as the dimmed "why" that teaches the model in situ (Design §5).
fn build_rendering(request: &PermissionRequest, reason: String) -> PermissionRendering {
    PermissionRendering {
        tool: request.tool.clone(),
        summary: request.summary.clone(),
        detail: request.detail.clone(),
        affected_paths: request
            .affected_paths
            .iter()
            .map(|p| p.display().to_string())
            .collect(),
        outside_root: request.outside_root,
        reason,
    }
}

/// Map a tool's [`PermissionRequest`] into the neutral rule-engine [`Query`].
/// Only bash is prefix-matched, and its `detail` is exactly the command it will
/// run (set by the bash tool), so it doubles as the match subject.
fn make_query(request: &PermissionRequest) -> Query<'_> {
    Query {
        tool: &request.tool,
        command: (request.tool == "bash").then_some(request.detail.as_str()),
        outside_root: request.outside_root,
    }
}

/// The rule to grant for an approved request (Requirements §6.6): a bash command
/// prefix (the exact command, so only it and its argument variations auto-run —
/// conservative), or a per-tool allow for file writes/edits.
fn grant_rule(request: &PermissionRequest) -> emberly_sandbox::Rule {
    if request.tool == "bash" {
        emberly_sandbox::bash_session_grant(request.detail.trim())
    } else {
        emberly_sandbox::tool_session_grant(&request.tool)
    }
}

/// The one-time plain-language explanation shown when OS confinement is
/// unavailable or partial (Requirements §6.7 item 1, Design §8.2): what is off,
/// what still holds, and that the hard lines are now policy-level.
fn degraded_notice(sandbox: &SandboxStatus) -> String {
    match sandbox {
        SandboxStatus::Partial { missing, .. } => format!(
            "sandbox partially available ({missing}). Core containment is active; \
             per-action prompting works as normal."
        ),
        _ => {
            let reason = match sandbox {
                SandboxStatus::Unavailable { reason } => reason.as_str(),
                _ => "unknown",
            };
            format!(
                "no OS sandbox ({reason}). Running the honest degraded path: auto-accept \
                 modes are off and the bash allowlist is suspended (every command asks). \
                 The project-root and .git protections still hold, but as policy-level, \
                 not kernel-level, guarantees."
            )
        }
    }
}

/// A short label for a mode, for user-facing notices.
fn mode_label(mode: Mode) -> &'static str {
    match mode {
        Mode::Normal => "normal",
        Mode::AutoAcceptEdits => "auto-accept edits",
        Mode::Auto => "auto",
    }
}

/// Append a `[[rule]]` block to a permissions file, creating it (and `.agents/`)
/// if absent, with a blank line before the block.
fn append_rule_block(path: &std::path::Path, block: &str) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file)?;
    write!(file, "{block}")?;
    Ok(())
}
