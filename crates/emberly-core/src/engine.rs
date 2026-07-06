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
use std::sync::Arc;

use emberly_providers::{
    CompletionRequest, CompletionStream, ContentBlock, Message, Provider, ProviderError,
    RetryPolicy, Role, StreamEvent, ToolCallId, ToolSchema,
};
use emberly_tools::{
    truncate_output, PermissionOutcome, PermissionRequest, ToolCtx, ToolRegistry, TruncateConfig,
};
use futures::StreamExt;
use time::OffsetDateTime;
use tokio::sync::mpsc;

use crate::command::Command;
use crate::event::UiEvent;
use crate::gate::{ChannelGate, PermissionAsk};
use crate::id::{PermissionId, SessionId};
use crate::transcript::{
    ConfigProvenance, NoopSink, TranscriptEvent, TranscriptRecord, TranscriptSink,
};
use crate::types::{PermissionRendering, SandboxStatus, TokenUsage};

/// The session title is the first user message, clipped to this many chars
/// (Tech Spec §16 — the heuristic v1 title).
const TITLE_CLIP: usize = 60;

/// How many trailing messages `/compact` keeps verbatim (`context.
/// keep_recent_turns`, default 6 — Tech Spec §7; config wiring is group 5).
const KEEP_RECENT: usize = 6;

/// The purpose-built summarization prompt for `/compact` (Tech Spec §7). Made
/// overridable from the prompts directory in group 5.
const SUMMARY_PROMPT: &str = "You are compacting a coding session's context. \
Summarize the conversation so work can continue with less context, covering, \
concisely and factually: the original task, key decisions made, files created \
or modified and how, the current state, and the next steps. No preamble.";

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
    /// Provider label for the transcript `session_start` (e.g. `anthropic`).
    pub provider_label: String,
    /// OS confinement status at startup (Requirements §6.7).
    pub sandbox: SandboxStatus,
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

/// Outcome of running one tool call.
enum ToolCallResult {
    Completed(emberly_tools::ToolOutcome),
    Canceled,
}

/// The agent engine.
pub struct Engine {
    provider: Arc<dyn Provider>,
    tools: ToolRegistry,
    project_root: PathBuf,
    model: String,
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
    provider_label: String,
    sandbox: SandboxStatus,
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
        let engine = Self {
            provider: config.provider,
            tools: config.tools,
            project_root: config.project_root,
            model: config.model,
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
            provider_label: config.provider_label,
            sandbox: config.sandbox,
            config_provenance: config.config_provenance,
            // On resume the original task already lives in the restored history.
            original_task_recorded: config.resuming,
            resuming: config.resuming,
            compact_requested: false,
            summary_prompt: config.summary_prompt,
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
            });
        }

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
                Command::SetMode { .. } => {
                    // Auto-accept modes are Phase 2 (gated on the sandbox).
                }
            }
        }

        // Command channel closed: the frontend is gone. Clean end of session.
        self.write_transcript(TranscriptEvent::SessionEnd { reason: None });
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
        let prompt = self.summary_prompt.as_deref().unwrap_or(SUMMARY_PROMPT);
        let request = CompletionRequest {
            model: self.model.clone(),
            system: Some(prompt.to_string()),
            messages: vec![Message::user_text(render_for_summary(messages))],
            tools: Vec::new(),
            max_output_tokens: Some(self.provider.model_info().max_output_tokens),
            temperature: None,
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

            let (end, text) = self.consume_stream(stream, commands_rx).await;
            match end {
                StreamEnd::Done { tool_calls } => {
                    self.push_assistant_message(&text, &tool_calls);
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
                    self.push_assistant_message(&text, &[]);
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
    ) -> (StreamEnd, String) {
        let mut text = String::new();
        let mut tool_calls: Vec<PendingToolCall> = Vec::new();
        let mut saw_done = false;

        let end = loop {
            tokio::select! {
                item = stream.next() => match item {
                    Some(Ok(event)) => {
                        if let Some(end) = self
                            .handle_stream_event(event, &mut text, &mut tool_calls, &mut saw_done)
                            .await
                        {
                            break end;
                        }
                    }
                    Some(Err(error)) => break StreamEnd::Errored(error),
                    None => break if saw_done { StreamEnd::Done { tool_calls: std::mem::take(&mut tool_calls) } } else { StreamEnd::Dropped },
                },
                Some(command) = commands_rx.recv() => {
                    match command {
                        Command::Cancel => break StreamEnd::Interrupted,
                        // Queue a compaction for the clean boundary (Tech Spec §7).
                        Command::Compact => self.compact_requested = true,
                        // Ignore permission answers / other commands mid-stream.
                        _ => {}
                    }
                }
            }
        };

        // The caller commits the assistant message: on Done/Interrupted it is
        // kept; on a retryable Dropped it is discarded (not stitched, §4.3).
        (end, text)
    }

    /// Apply one stream event. Returns `Some(end)` when the stream is done.
    async fn handle_stream_event(
        &mut self,
        event: StreamEvent,
        text: &mut String,
        tool_calls: &mut Vec<PendingToolCall>,
        saw_done: &mut bool,
    ) -> Option<StreamEnd> {
        match event {
            StreamEvent::TextDelta { text: delta } => {
                text.push_str(&delta);
                self.emit(UiEvent::AssistantDelta { text: delta }).await;
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
                *saw_done = true;
                return Some(StreamEnd::Done {
                    tool_calls: std::mem::take(tool_calls),
                });
            }
            // `StreamEvent` is non-exhaustive; ignore variants added later.
            _ => {}
        }
        None
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
        let mut pending: Vec<(
            PermissionId,
            tokio::sync::oneshot::Sender<PermissionOutcome>,
        )> = Vec::new();
        let mut commands_open = true;

        loop {
            tokio::select! {
                outcome = &mut exec => return ToolCallResult::Completed(outcome),
                Some(ask) = asks_rx.recv() => self.on_permission_ask(ask, &mut pending).await,
                command = commands_rx.recv(), if commands_open => match command {
                    Some(Command::PermissionAnswer { id, decision }) => {
                        if let Some(pos) = pending.iter().position(|(pid, _)| *pid == id) {
                            let (_, reply) = pending.swap_remove(pos);
                            self.write_transcript(TranscriptEvent::PermissionDecision {
                                id,
                                decision,
                                executed: None,
                            });
                            let outcome = if decision.is_allow() {
                                PermissionOutcome::Allow
                            } else {
                                PermissionOutcome::Deny
                            };
                            let _ = reply.send(outcome);
                        }
                    }
                    Some(Command::Cancel) => return ToolCallResult::Canceled,
                    // Queue a compaction for the clean boundary (Tech Spec §7).
                    Some(Command::Compact) => self.compact_requested = true,
                    Some(_) => {}
                    None => {
                        // No more input (frontend gone): deny anything pending
                        // as the safe default and stop watching commands, so we
                        // never hang on an answer that cannot arrive.
                        commands_open = false;
                        for (_, reply) in pending.drain(..) {
                            let _ = reply.send(PermissionOutcome::Deny);
                        }
                    }
                },
            }
        }
    }

    /// Turn a tool's permission ask into a UI prompt and remember its reply
    /// channel until the user answers.
    async fn on_permission_ask(
        &mut self,
        ask: PermissionAsk,
        pending: &mut Vec<(
            PermissionId,
            tokio::sync::oneshot::Sender<PermissionOutcome>,
        )>,
    ) {
        let id = self.take_permission_id();
        let rendering = build_rendering(&ask.request);
        self.write_transcript(TranscriptEvent::PermissionRequest {
            id,
            rendering: rendering.clone(),
        });
        self.emit(UiEvent::PermissionRequest { id, rendering })
            .await;
        pending.push((id, ask.reply));
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

    /// Build the assistant message for the turn: text plus any tool-use blocks.
    /// Records the complete assistant message and each requested tool call to
    /// the transcript (Tech Spec §3.2).
    fn push_assistant_message(&mut self, text: &str, tool_calls: &[PendingToolCall]) {
        if text.is_empty() && tool_calls.is_empty() {
            return;
        }
        if !text.is_empty() {
            self.write_transcript(TranscriptEvent::AssistantMessage {
                text: text.to_string(),
            });
        }
        let mut content = Vec::new();
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
        }
    }

    fn make_ctx(&self) -> ToolCtx {
        ToolCtx::new(self.project_root.clone(), self.truncate, self.gate.clone())
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

/// Enrich a tool's [`PermissionRequest`] into a UI [`PermissionRendering`],
/// adding the reason the prompt appeared (Design §5 — teaches the model in
/// situ). Phase 1 uses a minimal reason; the rule engine refines it (Phase 2).
fn build_rendering(request: &PermissionRequest) -> PermissionRendering {
    let reason = if request.outside_root {
        "this action affects files OUTSIDE the project root".to_string()
    } else {
        format!("{} requires your approval", request.tool)
    };
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
