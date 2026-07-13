//! The agent loop (Tech Spec §2, §3). The engine is one async task owning all
//! mutable session state (the conversation view, token accounting, the
//! permission-id counter). It talks to a frontend only over channels: [`UiEvent`]
//! out, [`Command`] in. There is no shared mutable state.
//!
//! Phase 1 scope: drive a scripted or live provider through completion →
//! tool-call → tool-result iterations, gate tool actions, truncate results at
//! ingestion, and handle cancellation. Sandbox rules (Phase 2), retries
//! (Phase 3), and transcript persistence (Phase 5) layer on later.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use emberly_providers::{
    CompletionRequest, CompletionStream, ContentBlock, Effort, Message, Provider, ProviderError,
    RetryPolicy, Role, StreamEvent, ToolCallId, ToolSchema,
};
use emberly_sandbox::{Decision, Mode, Query, RuleEngine};
use emberly_tools::{
    reduce_output, truncate_output, AskUserOutcome, PermissionOutcome, PermissionRequest, RecallOutcome,
    Reduction, Sandbox, ToolCtx, ToolRegistry, TruncateConfig,
};
use futures::StreamExt;
use time::OffsetDateTime;
use tokio::sync::mpsc;

use crate::command::Command;
use crate::event::UiEvent;
use crate::factory::{ConfigReloader, ProviderFactory};
use crate::gate::{AskGate, AskUserAsk, ChannelGate, MemoryAsk, MemoryGateImpl, PermissionAsk, RecallAsk, RecallGateImpl, TaskListAsk, TaskListGateImpl};
use crate::id::{AskId, PermissionId, SessionId};
use crate::memory::MemoryStore;
use crate::transcript::{
    CompactTrigger, ConfigProvenance, FileTranscript, NoopSink, TranscriptEvent,
    TranscriptRecord, TranscriptSink,
};
use crate::types::{LoopResolution, PermissionRendering, SandboxStatus, TokenUsage};
use crate::view_cache::{view_cache_path, ViewCache, VIEW_CACHE_VERSION};

/// The session title is the first user message, clipped to this many chars
/// (Tech Spec §16 — the heuristic v1 title).
const TITLE_CLIP: usize = 60;

/// Reserve this many tokens for model output when computing context usage,
/// or the model's max output, whichever is smaller (Tech Spec §7).
const OUTPUT_RESERVE: u64 = 8_000;

/// The loop-breaking guardrail's tunables (S-5, Tech Spec §7). Defaults are
/// initial — tune with use. `enabled = false` turns the guardrail off entirely.
#[derive(Debug, Clone, Copy)]
pub struct LoopConfig {
    pub enabled: bool,
    /// Trip when this many consecutive turns repeat the *same* tool-call
    /// signature with no progress (default 3).
    pub repeat_window: usize,
    /// Trip when this many consecutive turns make no progress, even if the calls
    /// vary (an absolute cap; default 6).
    pub max_no_progress_turns: usize,
}

impl Default for LoopConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            repeat_window: 3,
            max_no_progress_turns: 6,
        }
    }
}

/// Adaptive context-window and compaction configuration (FR-3, Tech Spec
/// §7/§8). Defaults are placeholders — tune with real long sessions (Tech Spec
/// §16, Requirements §13). `Copy` so it is cheap to pass into send-time views.
#[derive(Debug, Clone, Copy)]
pub struct ContextConfig {
    /// How many trailing non-pinned turns are sent to the provider (FR-3, Tech
    /// Spec §7). Older turns are elided from the sent context behind one
    /// marker; they stay in the transcript and the user's scrollback (HC-7).
    pub window_turns: usize,
    /// How many trailing messages `/compact` keeps verbatim (Tech Spec §7).
    /// Also the tail manual and automatic compaction (Phase 3) both keep.
    pub keep_recent_turns: usize,
    /// Whether automatic compaction is enabled (FR-4, Tech Spec §7/§8). When
    /// `true` (default), the engine schedules a compaction at the next clean
    /// boundary once context usage crosses `auto_compact_threshold`. Set
    /// `false` to compact manually only.
    pub auto_compact: bool,
    /// The context-usage fraction that triggers automatic compaction (FR-4,
    /// Tech Spec §7/§8). Default `0.85`. In `(0.0, 1.0]`; an out-of-range
    /// value is a config error, not a silent clamp.
    pub auto_compact_threshold: f64,
    /// Whether the model-maintained task list is pinned in the sent system
    /// prompt so "what's left" survives compaction and windowing (T-11, Tech
    /// Spec §7, Requirements §13 resolved). Default `true`; set `false` to
    /// omit the list from the sent context.
    pub pin_task_list: bool,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            window_turns: 40,
            keep_recent_turns: 6,
            auto_compact: true,
            auto_compact_threshold: 0.85,
            pin_task_list: true,
        }
    }
}

/// Memory config (FR-6, Tech Spec §8.1). Resolved from `[memory]` config.
#[derive(Debug, Clone)]
pub struct MemoryConfig {
    /// Whether the memory system is enabled (default `true`). When `false`,
    /// memory ops are rejected and no index is pinned.
    pub enabled: bool,
    /// Soft warn threshold for index growth (Tech Spec §16, initial). Does not
    /// truncate — only emits a one-time dim harness line.
    pub max_index_entries: usize,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_index_entries: 50,
        }
    }
}

/// Everything needed to construct an [`Engine`].
pub struct EngineConfig {
    pub provider: Arc<dyn Provider>,
    pub tools: ToolRegistry,
    pub project_root: PathBuf,
    pub model: String,
    pub system: Option<String>,
    /// Tool-call explanations (T-9, Tech Spec §5.4). When true, an optional
    /// `explanation` property is injected into every tool's schema and the
    /// prompt instruction is appended; when false, both are omitted so the model
    /// is never prompted and no tokens are spent (Requirements T-9). Fixed for
    /// the engine's life (a live config reload does not change it).
    pub tool_explanations: bool,
    /// True when workspace trust was *newly* granted at startup this launch
    /// (FR-1) — the engine records a `trust_decision` at session start. A
    /// silently-already-trusted session leaves this false.
    pub trust_granted: bool,
    /// Loop-breaking guardrail tunables (S-5, Tech Spec §7).
    pub loop_config: LoopConfig,
    /// Adaptive context-window + compaction config (FR-3, Tech Spec §7/§8).
    pub context: ContextConfig,
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
    /// True when the resumed session had been compacted, so the compaction
    /// summary at `conversation[1]` is pinned in the sent context (FR-3
    /// windowing composition with compaction).
    pub compacted: bool,
    /// Cached derived state from a prior run (FR-5 fast path). `Some` when a
    /// valid `-view.json` was loaded at launch — the engine seeds turn_map,
    /// accounting, and compaction state directly from it instead of deriving
    /// them. `None` for fresh sessions and fallback-replay resumes.
    pub initial_cache: Option<ViewCache>,
    /// True when this resume fell back to transcript replay (no valid cache).
    /// Drives the one dimmed harness-voice Notice on start (Design §8.6).
    pub replayed: bool,
    /// A `/compact` summarization-prompt override (P-7); `None` uses the
    /// built-in default.
    pub summary_prompt: Option<String>,
    /// Builds a provider for a profile name on an in-session switch (C-6).
    /// `None` disables switching (e.g. the offline placeholder session).
    pub provider_factory: Option<Arc<dyn ProviderFactory>>,
    /// Re-reads config + prompts from disk on an in-app edit (C-5). `None`
    /// disables live reload (the edit still lands on disk for the next session).
    pub config_reloader: Option<Arc<dyn ConfigReloader>>,
    /// Maximum image file size in bytes for the `read_image` tool (Tech Spec
    /// §5.2, default 5 MiB).
    pub image_max_bytes: usize,
    /// Memory config (FR-6, Tech Spec §8.1).
    pub memory: MemoryConfig,
    /// User-global memory directory (`~/.config/emberly/memory/`). Always `Some`
    /// when a home directory exists.
    pub user_memory_dir: Option<PathBuf>,
    /// Project memory directory (`<root>/.agents/memory/`). `None` on an
    /// untrusted root — structural trust-gating (FR-1, Tech Spec §6.7).
    pub project_memory_dir: Option<PathBuf>,
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

/// The derived session state passed to [`Engine::adopt_session`] — either
/// freshly computed or restored from the view cache (FR-5).
struct AdoptedState {
    turn_map: Vec<usize>,
    next_turn: usize,
    compacted: bool,
    session_usage: TokenUsage,
    session_cost_usd: f64,
    context_tokens_authoritative: Option<u64>,
    original_task_recorded: bool,
}

impl AdoptedState {
    /// Defaults for a fresh session (empty conversation, zero accounting).
    fn fresh() -> Self {
        Self {
            turn_map: Vec::new(),
            next_turn: 0,
            compacted: false,
            session_usage: TokenUsage::default(),
            session_cost_usd: 0.0,
            context_tokens_authoritative: None,
            original_task_recorded: false,
        }
    }

    /// Derive from a replayed conversation (FR-5 fallback path).
    fn replayed(conversation: &[Message], compacted: bool) -> Self {
        let (turn_map, next_turn) = build_turn_map(conversation);
        Self {
            turn_map,
            next_turn,
            compacted,
            session_usage: TokenUsage::default(),
            session_cost_usd: 0.0,
            context_tokens_authoritative: None,
            original_task_recorded: true,
        }
    }

    /// Restore from a valid view cache (FR-5 fast path). Returns the derived
    /// state and the cached conversation so the caller can pass both to
    /// [`Engine::adopt_session`] without a partial-move conflict.
    fn from_cache(cache: ViewCache) -> (Self, Vec<Message>) {
        let conversation = cache.conversation;
        let state = Self {
            turn_map: cache.turn_map,
            next_turn: cache.next_turn,
            compacted: cache.compacted,
            session_usage: cache.session_usage,
            session_cost_usd: cache.session_cost_usd,
            context_tokens_authoritative: cache.context_tokens_authoritative,
            original_task_recorded: cache.original_task_recorded,
        };
        (state, conversation)
    }
}

/// Outcome of running one tool call.
enum ToolCallResult {
    Completed(emberly_tools::ToolOutcome),
    Canceled,
}

/// Inject the optional `explanation` string property into a tool's JSON schema
/// (T-9, Tech Spec §5.4). Additive and **never** added to `required`, so a call
/// that omits it is valid; tools ignore it (no `deny_unknown_fields`). Only
/// touches object schemas with a `properties` map — a schema without one is
/// left untouched.
fn inject_explanation_property(schema: &mut serde_json::Value) {
    let Some(props) = schema.get_mut("properties").and_then(|p| p.as_object_mut()) else {
        return;
    };
    props.entry("explanation").or_insert_with(|| {
        serde_json::json!({
            "type": "string",
            "description": "Optional: one short line on what this call does and \
                            why, only when the intent is not self-evident.",
        })
    });
}

/// Extract the model's tool-call explanation from the call arguments (T-9).
/// Returns `None` when absent or blank so the UI shows no empty caption.
fn explanation_from_args(args: &serde_json::Value) -> Option<String> {
    let text = args.get("explanation")?.as_str()?.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// What one tool-call turn did, for the loop guardrail's signature (S-5). Reset
/// each turn; consumed at the turn boundary by [`Engine::evaluate_loop`].
#[derive(Default)]
struct TurnObservation {
    /// `(tool_name, normalized-args)` per call, order-independent (sorted before
    /// hashing).
    calls: Vec<(String, String)>,
    /// Concatenated tool-result content, hashed to detect identical results.
    result_content: String,
    /// Project-relative paths modified this turn.
    files: Vec<String>,
}

/// Normalize a tool call's args for the loop signature (S-5): drop the
/// T-9-injected `explanation` (a caption change is not progress and must not
/// mask a repeat), then serialize. `serde_json`'s map is key-sorted, so equal
/// args hash equal regardless of the model's key order.
fn normalize_args(args: &serde_json::Value) -> String {
    let mut a = args.clone();
    if let Some(obj) = a.as_object_mut() {
        obj.remove("explanation");
    }
    a.to_string()
}

/// A stable within-process hash (S-5 signature). Deterministic across a session,
/// which is all the guardrail compares.
fn stable_hash(s: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// Identify turn boundaries for context windowing (FR-3, Tech Spec §7). Each
/// turn starts at a [`Role::User`] message; `Role::Assistant` and `Role::Tool`
/// messages belong to the current turn. The first message always starts a turn
/// (even when it is not `Role::User`), so a post-compaction tail that begins
/// with an assistant message is never split from its tool results. Returns the
/// starting index of each turn, relative to the input slice.
fn group_turn_starts(messages: &[Message]) -> Vec<usize> {
    let mut starts = Vec::new();
    for (i, msg) in messages.iter().enumerate() {
        if msg.role == Role::User || starts.is_empty() {
            starts.push(i);
        }
    }
    starts
}

/// Build a parallel turn-number map for a conversation (FR-3, Tech Spec
/// §7/§16). Each message gets the number of the turn it belongs to. Turn 0
/// is the first message (the pinned original task); each subsequent
/// [`Role::User`] message increments the counter. Returns `(turn_map,
/// next_turn)` so the engine can continue assigning monotonic numbers.
fn build_turn_map(conversation: &[Message]) -> (Vec<usize>, usize) {
    let mut turn_map = Vec::with_capacity(conversation.len());
    let mut current_turn: usize = 0;
    let mut next_turn: usize = 1;
    for (i, msg) in conversation.iter().enumerate() {
        if i > 0 && msg.role == Role::User {
            current_turn = next_turn;
            next_turn += 1;
        }
        turn_map.push(current_turn);
    }
    // If the conversation is empty, next_turn starts at 0 (turn 0 is the next
    // message to arrive).
    if conversation.is_empty() {
        next_turn = 0;
    }
    (turn_map, next_turn)
}

/// The audit label for a loop resolution (S-5, Tech Spec §3.2).
fn resolution_label(r: &LoopResolution) -> String {
    match r {
        LoopResolution::Resume => "resume".into(),
        LoopResolution::Stop => "stop".into(),
        LoopResolution::Steer(_) => "steer".into(),
    }
}

/// A permission ask awaiting the user's answer: the id shown to the frontend,
/// the oneshot the blocked tool waits on, and the original request (kept so an
/// "allow for session"/"always allow" answer can be turned into a grant).
struct PendingAsk {
    id: PermissionId,
    reply: tokio::sync::oneshot::Sender<PermissionOutcome>,
    request: PermissionRequest,
}

/// An `ask_user` question awaiting the user's answer (T-8): the id shown to the
/// frontend, the question/options (kept for the transcript record written on
/// resolution), and the oneshot the blocked tool waits on.
struct PendingUserAsk {
    id: AskId,
    question: String,
    options: Vec<String>,
    reply: tokio::sync::oneshot::Sender<AskUserOutcome>,
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
    /// Whether tool-call explanations are enabled (T-9); see [`EngineConfig`].
    tool_explanations: bool,
    /// Newly-granted workspace trust to record at session start (FR-1).
    trust_granted: bool,
    /// Loop-breaking guardrail state (S-5, Tech Spec §7).
    loop_config: LoopConfig,
    /// Adaptive context-window + compaction config (FR-3, Tech Spec §7/§8).
    context: ContextConfig,
    /// What the in-flight tool-call turn did (reset per turn).
    turn_obs: TurnObservation,
    /// Cumulative modified-file paths across the session (progress if a turn
    /// adds a new one).
    loop_seen_files: HashSet<String>,
    /// Cumulative distinct tool-result hashes (progress if a turn adds a new
    /// one).
    loop_seen_results: HashSet<u64>,
    /// Tool signature of the previous no-progress turn.
    loop_last_sig: Option<u64>,
    /// Consecutive no-progress turns with the *same* tool signature.
    loop_same_sig_streak: usize,
    /// Consecutive no-progress turns (any signature).
    loop_no_progress_streak: usize,
    truncate: TruncateConfig,
    retry: RetryPolicy,
    gate: Arc<ChannelGate>,
    /// The ask-user gate (T-8), installed into every `ToolCtx` so the
    /// `ask_user` tool can block on a frontend round trip.
    ask_gate: Arc<AskGate>,
    /// The recall gate (T-10), installed into every `ToolCtx` so the `recall`
    /// tool can retrieve elided turns from the in-memory conversation.
    recall_gate: Arc<RecallGateImpl>,
    /// The task-list gate (T-11), installed into every `ToolCtx` so the `todo`
    /// tool can replace the full task list in engine state.
    task_list_gate: Arc<TaskListGateImpl>,
    /// The memory gate (T-13), installed into every `ToolCtx` so the `memory`
    /// tool can read and write durable memory entries.
    memory_gate: Arc<MemoryGateImpl>,
    events_tx: mpsc::Sender<UiEvent>,
    conversation: Vec<Message>,
    /// Parallel to `conversation`: the stable monotonic turn number of each
    /// message (FR-3, Tech Spec §7/§16). `Role::User` messages start a new
    /// turn (incrementing `next_turn`); assistant/tool messages inherit the
    /// current turn. Compaction retires numbers but never shifts surviving
    /// ones, so "turn 12" means the same thing all session.
    turn_map: Vec<usize>,
    /// The next turn number to assign (monotonic; never decremented).
    next_turn: usize,
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
    /// Monotonic id source for `ask_user` questions (T-8).
    next_ask_id: u64,
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
    /// True once a compaction has run (live or resumed), so the summary at
    /// `conversation[1]` is pinned in the sent context (FR-3 windowing).
    compacted: bool,
    /// True when the resume fell back to transcript replay (FR-5 slow path).
    /// Drives the one dimmed Notice on engine start (Design §8.6).
    replayed: bool,
    /// Set when `/compact` or the auto-trigger requests a compaction;
    /// performed at the next clean boundary (Tech Spec §7). `Manual` outranks
    /// `Auto` — a user `/compact` is never downgraded (FR-4).
    pending_compaction: Option<CompactTrigger>,
    /// Hysteresis latch for the auto-trigger (FR-4, Tech Spec §7): after an
    /// auto-compaction fires the latch disarms and stays disarmed until usage
    /// has fallen below the threshold and re-crossed it (no thrash).
    auto_compact_armed: bool,
    /// Optional `/compact` prompt override (P-7).
    summary_prompt: Option<String>,
    /// Builds a provider on an in-session model switch (C-6); `None` disables it.
    provider_factory: Option<Arc<dyn ProviderFactory>>,
    /// Re-reads config on an in-app edit (C-5); `None` disables live reload.
    config_reloader: Option<Arc<dyn ConfigReloader>>,
    /// The model-maintained task list (T-11). Pure engine state — the engine
    /// stores the last full replace, emits updates to frontends, and records
    /// them in the transcript (HC-7). Reset on a new session.
    task_list: Vec<emberly_tools::TaskItem>,
    /// Maximum image file size in bytes (Tech Spec §5.2). Threaded to the
    /// `read_image` tool via `ToolCtx`.
    image_max_bytes: usize,
    /// Memory config (FR-6, Tech Spec §8.1).
    memory_config: MemoryConfig,
    /// The durable memory store (FR-6, T-13). `None` when memory is disabled
    /// or no home directory exists.
    memory_store: Option<MemoryStore>,
    /// Cached user-global memory index text for pinning (Tech Spec §7).
    memory_user_index: String,
    /// Cached project memory index text for pinning (empty when untrusted).
    memory_project_index: String,
    /// Whether the max_index_entries soft-cap warning has been emitted this
    /// session (Tech Spec §16 — warn once, do not truncate).
    memory_warn_emitted: bool,
}

impl Engine {
    /// Build an engine and the receivers for its internal permission-ask and
    /// ask-user channels. The caller passes both straight back into
    /// [`run`](Engine::run); they are opaque otherwise.
    #[must_use]
    #[allow(clippy::type_complexity)]
    pub fn new(
        config: EngineConfig,
        events_tx: mpsc::Sender<UiEvent>,
    ) -> (
        Self,
        mpsc::Receiver<PermissionAsk>,
        mpsc::Receiver<AskUserAsk>,
        mpsc::Receiver<RecallAsk>,
        mpsc::Receiver<TaskListAsk>,
        mpsc::Receiver<MemoryAsk>,
    ) {
        let (asks_tx, asks_rx) = mpsc::channel(crate::channels::DEFAULT_CHANNEL_CAPACITY);
        let (user_asks_tx, user_asks_rx) = mpsc::channel(crate::channels::DEFAULT_CHANNEL_CAPACITY);
        let (recall_tx, recall_rx) = mpsc::channel(crate::channels::DEFAULT_CHANNEL_CAPACITY);
        let (task_list_tx, task_list_rx) = mpsc::channel(crate::channels::DEFAULT_CHANNEL_CAPACITY);
        let (memory_tx, memory_rx) = mpsc::channel(crate::channels::DEFAULT_CHANNEL_CAPACITY);
        let memory_store = build_memory_store(&config);
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
        // Build the turn map from the initial conversation — or restore it
        // directly from the cache when the FR-5 fast path loaded (FR-5).
        let (turn_map, next_turn) = match &config.initial_cache {
            Some(c) => (c.turn_map.clone(), c.next_turn),
            None => build_turn_map(&config.initial_conversation),
        };
        let session_usage = config
            .initial_cache
            .as_ref()
            .map_or(TokenUsage::default(), |c| c.session_usage);
        let session_cost_usd = config
            .initial_cache
            .as_ref()
            .map_or(0.0, |c| c.session_cost_usd);
        let context_tokens_authoritative = config
            .initial_cache
            .as_ref()
            .and_then(|c| c.context_tokens_authoritative);
        let original_task_recorded = config
            .initial_cache
            .as_ref()
            .map_or(config.resuming, |c| c.original_task_recorded);
        let mut engine = Self {
            provider: config.provider,
            tools: config.tools,
            project_root: config.project_root,
            model: config.model,
            effort: seed_effort,
            system: config.system,
            tool_explanations: config.tool_explanations,
            trust_granted: config.trust_granted,
            loop_config: config.loop_config,
            context: config.context,
            turn_obs: TurnObservation::default(),
            loop_seen_files: HashSet::new(),
            loop_seen_results: HashSet::new(),
            loop_last_sig: None,
            loop_same_sig_streak: 0,
            loop_no_progress_streak: 0,
            truncate: config.truncate,
            retry: config.retry,
            gate: Arc::new(ChannelGate { asks: asks_tx }),
            ask_gate: Arc::new(AskGate { asks: user_asks_tx }),
            recall_gate: Arc::new(RecallGateImpl { asks: recall_tx }),
            task_list_gate: Arc::new(TaskListGateImpl { asks: task_list_tx }),
            memory_gate: Arc::new(MemoryGateImpl { asks: memory_tx }),
            events_tx,
            conversation: config.initial_conversation,
            turn_map,
            next_turn,
            session_usage,
            session_cost_usd,
            context_tokens_authoritative,
            next_permission_id: 0,
            next_ask_id: 0,
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
            original_task_recorded,
            resuming: config.resuming,
            compacted: config.compacted,
            replayed: config.replayed,
            pending_compaction: None,
            auto_compact_armed: true,
            summary_prompt: config.summary_prompt,
            provider_factory: config.provider_factory,
            config_reloader: config.config_reloader,
            task_list: Vec::new(),
            image_max_bytes: config.image_max_bytes,
            memory_config: config.memory.clone(),
            memory_store,
            memory_user_index: String::new(),
            memory_project_index: String::new(),
            memory_warn_emitted: false,
        };
        // Load memory indexes at session start (Tech Spec §8.1).
        engine.refresh_memory_indexes();
        let (user_count, project_count) = engine
            .memory_store
            .as_ref()
            .map_or((0, 0), |s| s.status_counts());
        let _ = engine
            .events_tx
            .try_send(UiEvent::MemoryStatus {
                user: user_count,
                project: project_count,
            });
        (engine, asks_rx, user_asks_rx, recall_rx, task_list_rx, memory_rx)
    }

    /// Run the engine until the command channel closes. Idle between turns,
    /// waiting for a `UserInput`; a turn owns `commands_rx`/`asks_rx` for its
    /// duration (permission answers and cancellation arrive through them).
    pub async fn run(
        mut self,
        mut commands_rx: mpsc::Receiver<Command>,
        mut asks_rx: mpsc::Receiver<PermissionAsk>,
        mut user_asks_rx: mpsc::Receiver<AskUserAsk>,
        mut recall_rx: mpsc::Receiver<RecallAsk>,
        mut task_rx: mpsc::Receiver<TaskListAsk>,
        mut memory_rx: mpsc::Receiver<MemoryAsk>,
    ) {
        if self.resuming {
            // Continuing an existing transcript: no fresh session_start, but
            // surface the restored context size right away (Design §8.4).
            // When the resume fell back to transcript replay (no valid cache),
            // say so in one dimmed line — speech about the slow path only
            // (Design §8.6). The fast path is silent.
            if self.replayed {
                self.emit(UiEvent::Notice {
                    message: "Rebuilding the session from its transcript…".into(),
                })
                .await;
            }
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
        // Record a newly-granted workspace-trust decision right after session
        // start (FR-1, Tech Spec §3.2). Already-trusted launches record nothing.
        if self.trust_granted {
            self.write_transcript(TranscriptEvent::TrustDecision {
                path: self.project_root.display().to_string(),
                trusted: true,
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
                    self.push_conversation_message(Message::user_text(text));
                    self.emit_context_usage().await;
                    self.run_turn(&mut commands_rx, &mut asks_rx, &mut user_asks_rx, &mut recall_rx, &mut task_rx, &mut memory_rx)
                        .await;
                    // The engine is idle again; let the frontend stop its
                    // "working" affordance (Design §6.3).
                    self.emit(UiEvent::TurnEnded).await;
                    // A `/compact` sent mid-turn or an auto-trigger request
                    // runs now, at the clean boundary (every tool_use has its
                    // tool_result — Tech Spec §7).
                    if let Some(trigger) = self.pending_compaction.take() {
                        self.compact(trigger).await;
                    }
                    self.write_view_cache();
                }
                // No turn is running while idle; these are strays or no-ops here.
                Command::Cancel
                | Command::PermissionAnswer { .. }
                | Command::AskUserAnswer { .. }
                | Command::ResolveLoop { .. } => {}
                // Idle is already a clean boundary — compact immediately.
                Command::Compact => {
                    self.compact(CompactTrigger::Manual).await;
                    self.write_view_cache();
                }
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
        // Write the cache one final time so it reflects the final transcript
        // (the SessionEnd line grew the file; without this the cache would be
        // stale on the next resume — FR-5).
        self.write_view_cache();
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
        self.adopt_session(
            session_id,
            path,
            Vec::new(),
            AdoptedState::fresh(),
            false,
        );
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
    /// Tries the derived view cache first (FR-5 fast path); falls back to a
    /// full transcript replay when the cache is absent or stale.
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

        // FR-5: try the cache first. A valid cache restores the full derived
        // state directly — no per-line re-tokenization. The fast path is silent
        // (Design §8.6).
        if let Some(cache) = crate::resume::try_load_view_cache(&path) {
            let (state, conversation) = AdoptedState::from_cache(cache);
            self.adopt_session(session_id, path, conversation, state, true);
        } else {
            // Fallback: replay the transcript, re-deriving the view. Announce
            // the slow path in one dimmed harness-voice line (Design §8.6).
            self.emit(UiEvent::Notice {
                message: "Rebuilding the session from its transcript…".into(),
            })
            .await;
            let conversation = crate::resume::rebuild_conversation(&loaded.records);
            let compacted = crate::resume::has_compaction(&loaded.records);
            self.adopt_session(
                session_id,
                path,
                conversation.clone(),
                AdoptedState::replayed(&conversation, compacted),
                true,
            );
        }
        self.emit_context_usage().await;
    }

    /// Reset session-scoped state to a freshly adopted session and publish the
    /// new transcript path to the shared handle so the host's panic/exit path
    /// names the current session (HC-3). Carries the full derived state —
    /// turn map, compaction flag, and token accounting — so both the cache
    /// fast path (FR-5) and the replay fallback restore correct turn state
    /// (fixing the pre-existing `adopt_session` gap).
    fn adopt_session(
        &mut self,
        session_id: SessionId,
        path: PathBuf,
        conversation: Vec<Message>,
        state: AdoptedState,
        resuming: bool,
    ) {
        self.session_id = session_id;
        self.conversation = conversation;
        self.turn_map = state.turn_map;
        self.next_turn = state.next_turn;
        self.compacted = state.compacted;
        self.original_task_recorded = state.original_task_recorded;
        self.resuming = resuming;
        self.replayed = false;
        self.pending_compaction = None;
        self.auto_compact_armed = true;
        self.session_usage = state.session_usage;
        self.session_cost_usd = state.session_cost_usd;
        self.context_tokens_authoritative = state.context_tokens_authoritative;
        self.next_permission_id = 0;
        self.task_list.clear();
        // Reload memory indexes for the new session (user-global unchanged,
        // project re-pointed to the new root). The store reads from disk, so a
        // resumed session re-reads the current store (Tech Spec §8.1).
        self.refresh_memory_indexes();
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

    /// Queue a compaction for the next clean boundary (Tech Spec §7). Manual
    /// outranks Auto — a user `/compact` is never downgraded to `auto` (FR-4).
    fn request_compaction(&mut self, trigger: CompactTrigger) {
        if matches!(self.pending_compaction, Some(CompactTrigger::Manual)) {
            return;
        }
        self.pending_compaction = Some(trigger);
    }

    /// Compaction at a clean boundary (Tech Spec §7). Replaces the middle
    /// of the conversation — everything after the pinned original task and
    /// before the last `context.keep_recent_turns` messages — with a model-written summary,
    /// keeping the session usable when context grows. The pinned content
    /// (system prompt, original task) is never compacted; the JSONL log is
    /// untouched (the compaction is recorded as one event, replayed on resume).
    /// `trigger` records whether the user or the FR-4 threshold initiated it.
    async fn compact(&mut self, trigger: CompactTrigger) {
        // Pinned = the original task (the system prompt lives outside the
        // conversation). Keep the tail verbatim; summarize the middle.
        let pinned = usize::from(!self.conversation.is_empty());
        let len = self.conversation.len();
        let keep = self.context.keep_recent_turns.min(len.saturating_sub(pinned));
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
        // Turn map is rebuilt in parallel: pinned turns keep their numbers, the
        // summary gets the next monotonic number, and the tail retains its
        // original numbers (FR-3: compaction retires numbers, never shifts them).
        let mut rebuilt = Vec::with_capacity(2 + keep);
        let mut rebuilt_turns = Vec::with_capacity(2 + keep);
        rebuilt.extend(self.conversation[..from].iter().cloned());
        rebuilt_turns.extend(self.turn_map[..from].iter().copied());
        let summary_turn = self.next_turn;
        rebuilt.push(Message::user_text(summary.clone()));
        rebuilt_turns.push(summary_turn);
        self.next_turn += 1;
        rebuilt.extend(self.conversation[to..].iter().cloned());
        rebuilt_turns.extend(self.turn_map[to..].iter().copied());
        self.conversation = rebuilt;
        self.turn_map = rebuilt_turns;
        self.compacted = true;

        self.write_transcript(TranscriptEvent::Compaction {
            summary,
            replaced_from: u32::try_from(from).unwrap_or(u32::MAX),
            replaced_to: u32::try_from(to).unwrap_or(u32::MAX),
            trigger,
        });
        let turns_compacted = to - from;
        self.emit(UiEvent::CompactionStatus {
            message: match trigger {
                CompactTrigger::Manual => {
                    format!("Compacted {turns_compacted} turns into a summary.")
                }
                CompactTrigger::Auto => format!(
                    "Context was near full — compacted {turns_compacted} turns to keep going."
                ),
            },
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
        user_asks_rx: &mut mpsc::Receiver<AskUserAsk>,
        recall_rx: &mut mpsc::Receiver<RecallAsk>,
        task_rx: &mut mpsc::Receiver<TaskListAsk>,
        memory_rx: &mut mpsc::Receiver<MemoryAsk>,
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
                        .run_tool_calls(
                            tool_calls,
                            commands_rx,
                            asks_rx,
                            user_asks_rx,
                            recall_rx,
                            task_rx,
                            memory_rx,
                        )
                        .await
                        .is_canceled()
                    {
                        return;
                    }
                    // S-5: before issuing the next provider call, check whether
                    // the loop is re-treading without progress. On a trip, hand
                    // control to the user (resume / stop / steer) — never spin on.
                    if let Some(reason) = self.evaluate_loop() {
                        match self.await_loop_resolution(commands_rx, reason).await {
                            LoopResolution::Resume => self.reset_loop_window(),
                            LoopResolution::Stop => return,
                            LoopResolution::Steer(text) => {
                                self.reset_loop_window();
                                self.record_user_message(&text);
                                self.push_conversation_message(Message::user_text(text));
                                self.emit_context_usage().await;
                            }
                        }
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
                        Command::Compact => self.request_compaction(CompactTrigger::Manual),
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
    #[allow(clippy::too_many_arguments)]
    async fn run_tool_calls(
        &mut self,
        tool_calls: Vec<PendingToolCall>,
        commands_rx: &mut mpsc::Receiver<Command>,
        asks_rx: &mut mpsc::Receiver<PermissionAsk>,
        user_asks_rx: &mut mpsc::Receiver<AskUserAsk>,
        recall_rx: &mut mpsc::Receiver<RecallAsk>,
        task_rx: &mut mpsc::Receiver<TaskListAsk>,
        memory_rx: &mut mpsc::Receiver<MemoryAsk>,
    ) -> ToolCallResult {
        // Start a fresh loop-signature observation for this turn (S-5).
        self.turn_obs = TurnObservation::default();
        let mut iter = tool_calls.into_iter();
        while let Some(call) = iter.next() {
            match self
                .run_one_tool_call(&call, commands_rx, asks_rx, user_asks_rx, recall_rx, task_rx, memory_rx)
                .await
            {
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

    /// Fold the just-completed tool-call turn into the loop-guardrail state and
    /// decide whether to halt (S-5, Tech Spec §7). Returns the halt reason when
    /// the loop is re-treading without progress, else `None`. Consumes the
    /// turn's observation.
    fn evaluate_loop(&mut self) -> Option<String> {
        if !self.loop_config.enabled {
            return None;
        }
        let obs = std::mem::take(&mut self.turn_obs);
        // A turn with no tool calls can't loop; treat it as progress-neutral.
        if obs.calls.is_empty() {
            return None;
        }
        // Order-independent tool signature.
        let mut parts: Vec<String> = obs
            .calls
            .iter()
            .map(|(name, args)| format!("{name}\u{0}{args}"))
            .collect();
        parts.sort();
        let tool_sig = stable_hash(&parts.join("\n"));
        let result_hash = stable_hash(&obs.result_content);

        // Progress = a newly-modified file OR a not-seen-before result.
        let new_file = obs.files.iter().any(|f| !self.loop_seen_files.contains(f));
        let new_result = !self.loop_seen_results.contains(&result_hash);
        for f in obs.files {
            self.loop_seen_files.insert(f);
        }
        self.loop_seen_results.insert(result_hash);

        if new_file || new_result {
            self.reset_loop_window();
            self.loop_last_sig = Some(tool_sig);
            return None;
        }

        // No progress this turn.
        self.loop_no_progress_streak += 1;
        if self.loop_last_sig == Some(tool_sig) {
            self.loop_same_sig_streak += 1;
        } else {
            self.loop_same_sig_streak = 1;
        }
        self.loop_last_sig = Some(tool_sig);

        let cfg = self.loop_config;
        if self.loop_same_sig_streak >= cfg.repeat_window {
            Some("the last few steps repeated without progress".into())
        } else if self.loop_no_progress_streak >= cfg.max_no_progress_turns {
            Some("several steps in a row made no progress".into())
        } else {
            None
        }
    }

    /// Reset the no-progress counters (S-5). Called on genuine progress and when
    /// the user chooses to resume, so "keep going" doesn't instantly re-trip.
    fn reset_loop_window(&mut self) {
        self.loop_same_sig_streak = 0;
        self.loop_no_progress_streak = 0;
        self.loop_last_sig = None;
    }

    /// Surface the halt (harness voice) and park until the user decides
    /// (S-5, Design §8.5): resume / stop / steer. Cancel or a departed frontend
    /// resolves to stop — the guardrail never quietly resumes. Records one
    /// `loop_halt` transcript event with the chosen resolution.
    async fn await_loop_resolution(
        &mut self,
        commands_rx: &mut mpsc::Receiver<Command>,
        reason: String,
    ) -> LoopResolution {
        self.emit(UiEvent::LoopHalted {
            reason: reason.clone(),
        })
        .await;
        let resolution = loop {
            match commands_rx.recv().await {
                Some(Command::ResolveLoop { resolution }) => break resolution,
                // Esc/Ctrl-C while halted = stop here.
                Some(Command::Cancel) => break LoopResolution::Stop,
                // A mode toggle applies immediately; keep waiting for a decision.
                Some(Command::SetMode { mode }) => self.set_mode(mode).await,
                // Queue a compaction for after we resume (if we do).
                Some(Command::Compact) => self.request_compaction(CompactTrigger::Manual),
                // Strays (permission/ask answers with no pending prompt): ignore.
                Some(_) => {}
                // Frontend gone: stop, fail-safe (never spin unattended).
                None => break LoopResolution::Stop,
            }
        };
        self.write_transcript(TranscriptEvent::LoopHalt {
            reason,
            resolution: Some(resolution_label(&resolution)),
        });
        resolution
    }

    /// Run one tool call, driving its execution concurrently with permission
    /// asks and cancellation.
    #[allow(clippy::too_many_arguments)]
    async fn run_one_tool_call(
        &mut self,
        call: &PendingToolCall,
        commands_rx: &mut mpsc::Receiver<Command>,
        asks_rx: &mut mpsc::Receiver<PermissionAsk>,
        user_asks_rx: &mut mpsc::Receiver<AskUserAsk>,
        recall_rx: &mut mpsc::Receiver<RecallAsk>,
        task_rx: &mut mpsc::Receiver<TaskListAsk>,
        memory_rx: &mut mpsc::Receiver<MemoryAsk>,
    ) -> ToolCallResult {
        let args = serde_json::from_str(&call.args).unwrap_or(serde_json::Value::Null);

        // Record the (tool, normalized-args) for the loop signature (S-5),
        // including unknown-tool attempts (a loop can re-tread those too).
        if self.loop_config.enabled {
            self.turn_obs
                .calls
                .push((call.name.clone(), normalize_args(&args)));
        }

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
        // T-9: the model's caption for a non-obvious call rides in `args`
        // (§5.4); surface it to the frontend. Absent/empty → None (no caption).
        let explanation = explanation_from_args(&args);
        self.emit(UiEvent::ToolStarted {
            call_id: call.id.clone(),
            tool: call.name.clone(),
            summary,
            explanation,
        })
        .await;

        let ctx = self.make_ctx();
        let mut exec = Box::pin(tool.execute(args, &ctx));
        let mut pending: Vec<PendingAsk> = Vec::new();
        let mut pending_user: Vec<PendingUserAsk> = Vec::new();
        let mut commands_open = true;

        loop {
            tokio::select! {
                outcome = &mut exec => return ToolCallResult::Completed(outcome),
                Some(ask) = asks_rx.recv() => self.on_permission_ask(ask, &mut pending).await,
                Some(ask) = user_asks_rx.recv() => self.on_user_ask(ask, &mut pending_user).await,
                Some(recall) = recall_rx.recv() => self.on_recall(recall).await,
                Some(task) = task_rx.recv() => self.on_task_list_set(task).await,
                Some(mem) = memory_rx.recv() => self.on_memory_op(mem).await,
                command = commands_rx.recv(), if commands_open => match command {
                    Some(Command::PermissionAnswer { id, decision }) => {
                        self.answer_permission(id, decision, &mut pending).await;
                    }
                    Some(Command::AskUserAnswer { id, answer }) => {
                        self.answer_user_ask(id, answer, &mut pending_user).await;
                    }
                    Some(Command::Cancel) => return ToolCallResult::Canceled,
                    // Queue a compaction for the clean boundary (Tech Spec §7).
                    Some(Command::Compact) => self.request_compaction(CompactTrigger::Manual),
                    // A mode toggle applies immediately to later asks this turn.
                    Some(Command::SetMode { mode }) => self.set_mode(mode).await,
                    Some(_) => {}
                    None => {
                        // No more input (frontend gone): resolve anything pending
                        // as the safe default (deny / decline) and stop watching
                        // commands, so we never hang on an answer that cannot
                        // arrive.
                        commands_open = false;
                        for p in pending.drain(..) {
                            let _ = p.reply.send(PermissionOutcome::Deny);
                        }
                        for p in pending_user.drain(..) {
                            self.record_ask(&p.question, &p.options, None);
                            let _ = p.reply.send(AskUserOutcome::Declined);
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

    /// Handle an `ask_user` question from the tool (T-8): mint an id, surface it
    /// to the frontend, and stash the pending question. The transcript record is
    /// written on resolution (question + answer together), so an unanswered
    /// question that is later declined is still recorded once.
    async fn on_user_ask(&mut self, ask: AskUserAsk, pending: &mut Vec<PendingUserAsk>) {
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
    async fn answer_user_ask(
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
    fn record_ask(&mut self, question: &str, options: &[String], answer: Option<String>) {
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
    async fn on_recall(&self, ask: RecallAsk) {
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
    async fn on_task_list_set(&mut self, ask: TaskListAsk) {
        let TaskListAsk { items, reply } = ask;
        self.task_list = items.clone();
        self.emit(UiEvent::TaskListUpdated { items }).await;
        self.write_transcript(TranscriptEvent::TaskList {
            items: self.task_list.clone(),
        });
        let _ = reply.send(Ok(()));
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
        // Accumulate this result into the turn's loop signature (S-5): identical
        // repeated results are a no-progress signal.
        if self.loop_config.enabled {
            self.turn_obs.result_content.push_str(&outcome.content);
        }
        // Salient reduction (FR-2) runs before the size backstop (§5.3), gated
        // on `truncate.reduce`. Both are deterministic, no-model-call transforms.
        let reduced = if self.truncate.reduce {
            reduce_output(&call.name, &outcome.content)
        } else {
            Reduction {
                content: outcome.content.clone(),
                reduced: false,
                withheld: String::new(),
            }
        };
        let truncation = truncate_output(&reduced.content, &self.truncate);

        // Durable record: the model-visible (possibly reduced + truncated)
        // output, plus a sidecar holding the full output when either layer
        // withheld content (Requirements §8.1/§8.5, HC-7).
        let withheld = reduced.reduced || truncation.truncated;
        let full_output_ref = if withheld {
            self.transcript.sidecar(&call.id, &outcome.content)
        } else {
            None
        };
        self.write_transcript(TranscriptEvent::ToolResult {
            call_id: call.id.clone(),
            ok: outcome.ok,
            output: truncation.content.clone(),
            truncated: withheld,
            full_output_ref,
        });

        self.push_conversation_message(Message::tool_result(
            call.id.clone(),
            truncation.content,
            !outcome.ok,
        ));

        // An image from `read_image` (P-11): append a `ContentBlock::Image` as
        // a synthetic user message so both adapters can carry it — Anthropic in
        // a user-role image block, OpenAI as an `image_url` part (whose tool
        // role cannot hold images, Tech Spec §4.2). The bytes are NOT in the
        // transcript (HC-7); the `tool_call` recorded the path, and the block
        // lives only in the live conversation (re-derived from disk on resume —
        // honestly absent if the file is gone).
        if let Some(image) = outcome.image {
            self.push_conversation_message(Message {
                role: Role::User,
                content: vec![ContentBlock::Image {
                    media_type: image.media_type,
                    data: image.data,
                }],
            });
        }

        self.emit(UiEvent::ToolFinished {
            call_id: call.id.clone(),
            ok: outcome.ok,
            summary: outcome.summary,
            preview: result_preview(&outcome.content),
        })
        .await;

        if let Some(change) = outcome.file_change {
            // A newly-modified file is the strongest progress signal (S-5).
            if self.loop_config.enabled {
                self.turn_obs.files.push(change.path.clone());
            }
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
        self.push_conversation_message(
            Message::tool_result(call.id.clone(), canceled, true),
        );
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
        self.push_conversation_message(Message {
            role: Role::Assistant,
            content,
        });
    }

    /// Push a message to the conversation and update the turn map (FR-3 turn
    /// numbering). `Role::User` messages start a new turn; assistant/tool
    /// messages inherit the current turn number.
    fn push_conversation_message(&mut self, msg: Message) {
        let turn = if self.conversation.is_empty() || msg.role == Role::User {
            let t = self.next_turn;
            self.next_turn += 1;
            t
        } else {
            *self.turn_map.last().unwrap_or(&0)
        };
        self.conversation.push(msg);
        self.turn_map.push(turn);
    }

    /// The number of messages at the front of the conversation that are always
    /// sent and never windowed away (FR-3, Tech Spec §7): the original task
    /// (position 0) plus the compaction summary (position 1) when one is active.
    fn pinned_count(&self) -> usize {
        if self.conversation.is_empty() {
            return 0;
        }
        // The original task is always pinned at position 0.
        let mut n = 1;
        // After compaction (live or resumed), the summary at position 1 is
        // also pinned (Tech Spec §7: windowing never drops the summary).
        if self.compacted {
            n += 1;
        }
        n.min(self.conversation.len())
    }

    /// The windowed view of the conversation for sending to the provider
    /// (FR-3, Tech Spec §7). A **pure view** — never mutates
    /// `self.conversation` (HC-7). Keeps the pinned prefix and the last
    /// `context.window_turns` turns; replaces older turns with one synthetic
    /// elision marker. Turns are grouped at clean boundaries: every
    /// `ContentBlock::ToolUse` keeps its matching `Message::tool_result`, so
    /// the sent list stays provider-valid.
    fn windowed_messages(&self) -> Vec<Message> {
        let pinned = self.pinned_count();
        let total = self.conversation.len();
        if total <= pinned {
            return self.conversation.clone();
        }

        let turns = group_turn_starts(&self.conversation[pinned..]);
        let elided = turns.len().saturating_sub(self.context.window_turns);
        if elided == 0 {
            return self.conversation.clone();
        }

        // Index into self.conversation where the first kept turn begins.
        let keep_from = if elided < turns.len() {
            pinned + turns[elided]
        } else {
            // window_turns = 0: elide every non-pinned turn.
            total
        };

        // The turn range being elided, using stable monotonic turn numbers
        // (FR-3, Tech Spec §7/§16) so the marker names a range `recall` can
        // take.
        let first_turn_msg = pinned + turns[0];
        let last_turn_msg = if elided < turns.len() {
            pinned + turns[elided] - 1
        } else {
            total - 1
        };
        let first_turn = self.turn_map[first_turn_msg];
        let last_turn = self.turn_map[last_turn_msg];

        let mut result =
            Vec::with_capacity(pinned + 1 + total.saturating_sub(keep_from));
        result.extend(self.conversation[..pinned].iter().cloned());
        result.push(Message::user_text(format!(
            "[turns {first_turn}–{last_turn} elided from context \
             — still in the session transcript; use recall to retrieve them]"
        )));
        result.extend(self.conversation[keep_from..].iter().cloned());
        result
    }

    fn build_request(&self) -> CompletionRequest {
        let tools = self
            .tools
            .specs()
            .into_iter()
            .map(|spec| {
                let mut input_schema = spec.input_schema;
                // T-9: inject the optional `explanation` property at the single
                // ToolSpec→provider point, so both wire adapters get it without
                // any per-adapter code (Tech Spec §5.4). Off → nothing added.
                if self.tool_explanations {
                    inject_explanation_property(&mut input_schema);
                }
                ToolSchema {
                    name: spec.name,
                    description: spec.description,
                    input_schema,
                }
            })
            .collect();
        CompletionRequest {
            model: self.model.clone(),
            // T-9: append the explanation instruction only when the feature is
            // on, so with it off the model is never asked and no tokens are
            // spent. Kept out of the stored `self.system` so a config reload
            // (which replaces it) stays orthogonal to this toggle.
            system: self.effective_system(),
            messages: self.windowed_messages(),
            tools,
            max_output_tokens: Some(self.provider.model_info().max_output_tokens),
            temperature: None,
            // The session's active reasoning effort (P-9). The adapter maps it
            // to the provider's control or drops it when unsupported.
            effort: self.effort,
        }
    }

    /// The outgoing system prompt: the stored base (base prompt + project
    /// instructions) with the tool-call explanation instruction appended when
    /// enabled (T-9), and the task-list block appended when the list is non-empty
    /// and pinning is on (T-11, Tech Spec §7).
    fn effective_system(&self) -> Option<String> {
        let base = if self.tool_explanations {
            let instruction = crate::prompts::tool_explanation();
            match &self.system {
                Some(b) => Some(format!("{b}\n\n{instruction}")),
                None => Some(instruction.to_string()),
            }
        } else {
            self.system.clone()
        };

        let base = if self.context.pin_task_list && !self.task_list.is_empty() {
            let block = render_task_list_block(&self.task_list);
            match &base {
                Some(b) => Some(format!("{b}\n\n{block}")),
                None => Some(block),
            }
        } else {
            base
        };

        // Pin the memory index (FR-6, Tech Spec §7/§8.1). Only the one-line
        // index is standing context; entry bodies load via the `recall` op
        // (progressive disclosure). Project memory is absent on an untrusted
        // root (Design §4.9).
        if self.memory_config.enabled {
            let block = render_memory_block(
                &self.memory_user_index,
                &self.memory_project_index,
            );
            if !block.is_empty() {
                match &base {
                    Some(b) => Some(format!("{b}\n\n{block}")),
                    None => Some(block),
                }
            } else {
                base
            }
        } else {
            base
        }
    }

    fn make_ctx(&self) -> ToolCtx {
        ToolCtx::new(
            self.project_root.clone(),
            self.truncate,
            self.gate.clone(),
            self.sandbox_spawn.clone(),
        )
        .with_ask_gate(self.ask_gate.clone())
        .with_recall_gate(self.recall_gate.clone())
        .with_task_list_gate(self.task_list_gate.clone())
        .with_memory_gate(self.memory_gate.clone())
        .with_vision(self.provider.model_info().vision)
        .with_image_max_bytes(self.image_max_bytes)
    }

    /// Reload the memory index strings from the store into the cached fields.
    fn refresh_memory_indexes(&mut self) {
        if let Some(store) = &self.memory_store {
            self.memory_user_index = store.user_index();
            self.memory_project_index = store.project_index();
        } else {
            self.memory_user_index.clear();
            self.memory_project_index.clear();
        }
    }

    /// Handle a memory op from the `memory` tool (T-13, FR-6). Re-validates the
    /// name, routes to the store, refreshes indexes, and emits `MemoryStatus`.
    async fn on_memory_op(&mut self, ask: MemoryAsk) {
        // Compute the result first so the immutable borrow of the store ends
        // before the mutable refresh + emit.
        let computed = if self.memory_config.enabled {
            self.memory_store.as_ref().map(|store| store.execute(&ask.req))
        } else {
            None
        };
        let outcome = match computed {
            Some(result) => {
                self.refresh_memory_indexes();
                let (user_count, project_count) = self
                    .memory_store
                    .as_ref()
                    .map_or((0, 0), |s| s.status_counts());
                self.emit(UiEvent::MemoryStatus {
                    user: user_count,
                    project: project_count,
                })
                .await;
                // Soft-cap warn (Tech Spec §16): warn once when the combined
                // index exceeds `max_index_entries`. Do not truncate.
                let total = user_count + project_count;
                if !self.memory_warn_emitted
                    && total > self.memory_config.max_index_entries
                {
                    self.memory_warn_emitted = true;
                    self.emit(UiEvent::Notice {
                        message: format!(
                            "memory index has {total} entries (soft cap {}) — consider trimming or consolidating",
                            self.memory_config.max_index_entries
                        ),
                    })
                    .await;
                }
                Ok(result)
            }
            None => Ok(emberly_tools::MemoryOutcome::Rejected {
                reason: "memory is disabled".into(),
            }),
        };
        let _ = ask.reply.send(outcome);
    }

    /// Resolve a turn-number range to the messages it contains (T-10, FR-3).
    /// Returns the engine's normalized, in-memory messages for those turns —
    /// never raw JSONL. `from`/`to` are inclusive stable turn numbers as shown
    /// in the elision marker. Returns an empty vec if the range matches no
    /// turns (e.g. out of bounds or turns retired by compaction).
    #[must_use]
    pub fn recall_turns(&self, from: usize, to: usize) -> Vec<Message> {
        if from > to || self.turn_map.is_empty() {
            return Vec::new();
        }
        let mut result = Vec::new();
        for (i, msg) in self.conversation.iter().enumerate() {
            let turn = self.turn_map[i];
            if turn >= from && turn <= to {
                result.push(msg.clone());
            }
        }
        result
    }

    fn take_permission_id(&mut self) -> PermissionId {
        let id = PermissionId(self.next_permission_id);
        self.next_permission_id += 1;
        id
    }

    fn take_ask_id(&mut self) -> AskId {
        let id = AskId(self.next_ask_id);
        self.next_ask_id += 1;
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

    /// Write the derived conversation-state cache best-effort (FR-5, Tech Spec
    /// §3.2a). Called after the view settles — a turn completes, a compaction
    /// runs. Losing this file loses nothing: the replay fallback is always
    /// correct (HC-7 subordination). Any I/O or serialization error is
    /// swallowed (HC-3 — never a panic); the cache is never relied upon.
    fn write_view_cache(&self) {
        let transcript_path = {
            let guard = match self.active_session_path.read() {
                Ok(g) => g,
                Err(_) => return,
            };
            guard.clone()
        };
        let byte_len = match std::fs::metadata(&transcript_path) {
            Ok(m) => m.len(),
            Err(_) => return, // no transcript file (e.g. NoopSink in tests)
        };
        let cache = ViewCache {
            version: VIEW_CACHE_VERSION,
            session_id: self.session_id,
            conversation: self.conversation.clone(),
            turn_map: self.turn_map.clone(),
            next_turn: self.next_turn,
            compacted: self.compacted,
            original_task_recorded: self.original_task_recorded,
            session_usage: self.session_usage,
            session_cost_usd: self.session_cost_usd,
            context_tokens_authoritative: self.context_tokens_authoritative,
            transcript_byte_len: byte_len,
        };
        let cache_path = view_cache_path(&transcript_path);
        let json = match serde_json::to_string(&cache) {
            Ok(j) => j,
            Err(_) => return,
        };
        let _ = std::fs::write(&cache_path, json);
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
    /// chars/4 estimate before the first `Usage`. Also checks the FR-4
    /// automatic-compaction threshold and queues a compaction when crossed.
    async fn emit_context_usage(&mut self) {
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

        // Automatic compaction threshold check (FR-4, Tech Spec §7). The latch
        // prevents thrash: after an auto-compaction fires the latch disarms and
        // stays disarmed until usage drops below the threshold, then re-crosses
        // it. The compaction itself drops usage well below the line, so
        // re-arming is natural.
        if self.context.auto_compact {
            let pct_f = f64::from(u8::try_from(pct).unwrap_or(100));
            if pct_f >= self.context.auto_compact_threshold * 100.0 {
                if self.auto_compact_armed {
                    self.request_compaction(CompactTrigger::Auto);
                    self.auto_compact_armed = false;
                }
            } else {
                // Usage below threshold: re-arm the latch.
                self.auto_compact_armed = true;
            }
        }

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
        let mut total = self.system.as_deref().map(count).unwrap_or(0);        // Count the windowed sent view (FR-3, Design §8.6), not the full
        // in-memory conversation — so usage reflects what the provider
        // actually receives. The elision marker is included because it rides
        // in the sent messages.
        let messages = self.windowed_messages();
        for message in &messages {
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
                    // An image's token cost is not chars/4 of its base64. Until
                    // an authoritative provider-reported usage arrives (P-6),
                    // estimate a fixed per-image cost rather than inflating the
                    // budget with raw base64 length (initial; tune with use,
                    // Requirements §13).
                    ContentBlock::Image { .. } => IMAGE_TOKEN_ESTIMATE,
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
                // An image is summarized by its media type, not its bytes.
                ContentBlock::Image { media_type, .. } => {
                    format!("[image: {media_type}]")
                }
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

/// Render recalled messages as readable text for the model (T-10). Tool
/// results are reduced via Phase 1's `reduce_output` so recall costs tokens
/// proportional to what is recalled, never the raw output size. Never raw
/// JSONL — the rendered form is the model-facing view (T-10).
fn render_recall(messages: &[Message], reduce: bool) -> String {
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
                ContentBlock::ToolUse { name, input, .. } => {
                    format!("[tool call: {name} {input}]")
                }
                ContentBlock::ToolResult { content, .. } => {
                    // Reduce tool results via Phase 1's salient reduction
                    // (T-10: never raw JSONL; proportional to recalled
                    // content, not the original raw size).
                    if reduce {
                        let r = reduce_output("", content);
                        r.content
                    } else {
                        content.clone()
                    }
                }
                ContentBlock::Reasoning { .. } => String::new(),
                ContentBlock::Image { media_type, .. } => {
                    format!("[image: {media_type}]")
                }
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

/// Rough per-image token cost for the context-budget estimate (P-6). An image's
/// cost is not chars/4 of its base64; until authoritative provider-reported usage
/// arrives, this fixed estimate avoids inflating the budget. Initial; tune with
/// use (Requirements §13, Tech Spec §16).
const IMAGE_TOKEN_ESTIMATE: u64 = 765;

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

/// Render the current task list as a compact block for the pinned system prompt
/// (T-11, Tech Spec §7). One line per item with a text status token, so the
/// standing context cost is trivial.
fn render_task_list_block(items: &[emberly_tools::TaskItem]) -> String {
    let mut lines = String::from("## Current task list\n");
    for item in items {
        let token = match item.status {
            emberly_tools::TaskStatus::Pending => "[pending]",
            emberly_tools::TaskStatus::InProgress => "[in_progress]",
            emberly_tools::TaskStatus::Done => "[done]",
        };
        lines.push_str(&format!("- {token} {}\n", item.text));
    }
    lines
}

/// Render the memory index as a pinned block for the system prompt (FR-6, Tech
/// Spec §7/§8.1). Only the one-line index is standing context; bodies load via
/// the `recall` op. Returns an empty string when both indexes are empty.
fn render_memory_block(user_index: &str, project_index: &str) -> String {
    let mut block = String::new();
    if !user_index.is_empty() {
        block.push_str("## Memory (user)\n");
        block.push_str(user_index);
    }
    if !project_index.is_empty() {
        block.push_str("## Memory (project)\n");
        block.push_str(project_index);
    }
    block
}

/// Build the memory store from the engine config (FR-6, Tech Spec §8.1). Returns
/// `None` when memory is disabled or no home directory exists.
fn build_memory_store(config: &EngineConfig) -> Option<MemoryStore> {
    if !config.memory.enabled {
        return None;
    }
    let user_dir = config.user_memory_dir.as_ref()?;
    Some(MemoryStore::new(
        user_dir.clone(),
        config.project_memory_dir.clone(),
    ))
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

#[cfg(test)]
mod tests {
    use super::{explanation_from_args, inject_explanation_property, normalize_args};
    use serde_json::json;

    #[test]
    fn normalize_args_strips_explanation_so_a_caption_is_not_progress() {
        // Same call, different T-9 caption → identical loop signature (S-5): a
        // caption change must neither fake progress nor mask a repeat.
        let a = normalize_args(&json!({ "path": "x", "explanation": "first try" }));
        let b = normalize_args(&json!({ "path": "x", "explanation": "second try" }));
        assert_eq!(a, b);
        assert!(!a.contains("explanation"));
        // A genuinely different arg changes the signature.
        let c = normalize_args(&json!({ "path": "y" }));
        assert_ne!(a, c);
    }

    #[test]
    fn injects_optional_explanation_never_required() {
        let mut schema = json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"],
        });
        inject_explanation_property(&mut schema);
        assert_eq!(schema["properties"]["explanation"]["type"], "string");
        // Never added to `required` — a call omitting it stays valid (§5.4).
        assert_eq!(schema["required"], json!(["path"]));
    }

    #[test]
    fn injection_is_idempotent_and_skips_schemas_without_properties() {
        // A model that already sent an `explanation` property is not clobbered.
        let mut has = json!({ "properties": { "explanation": { "type": "number" } } });
        inject_explanation_property(&mut has);
        assert_eq!(has["properties"]["explanation"]["type"], "number");
        // A schema with no `properties` map is left untouched (no panic).
        let mut bare = json!({ "type": "string" });
        inject_explanation_property(&mut bare);
        assert_eq!(bare, json!({ "type": "string" }));
    }

    #[test]
    fn explanation_extracted_only_when_present_and_nonblank() {
        assert_eq!(
            explanation_from_args(&json!({ "explanation": "raise log level" })),
            Some("raise log level".to_string())
        );
        assert_eq!(explanation_from_args(&json!({ "explanation": "  " })), None);
        assert_eq!(explanation_from_args(&json!({ "path": "x" })), None);
        assert_eq!(explanation_from_args(&serde_json::Value::Null), None);
    }
}
