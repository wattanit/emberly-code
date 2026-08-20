//! The agent loop (Tech Spec §2, §3). The engine is one async task owning all
//! mutable session state (the conversation view, token accounting, the
//! permission-id counter). It talks to a frontend only over channels: [`UiEvent`]
//! out, [`Command`] in. There is no shared mutable state.
//!
//! Drives a scripted or live provider through completion → tool-call →
//! tool-result iterations, gates tool actions against the sandbox rule
//! engine, truncates results at ingestion, retries transient stream drops,
//! persists the transcript, and handles cancellation.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use emberly_providers::{
    CompletionRequest, CompletionStream, ContentBlock, Effort, Message, Provider, ProviderError,
    ProviderId, RetryPolicy, Role, StreamEvent, ToolCallId, ToolSchema,
};
use emberly_sandbox::{Decision, Mode, Query, Rule, RuleEngine};
use emberly_tools::{
    encode_image_bytes, reduce_output, truncate_output, AskUserGate, AskUserOutcome,
    PermissionGate, PermissionOutcome, PermissionRequest, RecallOutcome, Reduction, Sandbox,
    SubagentEndOutcome, SubagentError, SubagentListEntry, SubagentMessageOutcome,
    SubagentMessageRequest, SubagentSpawnBatch, SubagentSpawnOutcome, SubagentSpawnResult,
    SubagentSpawnSpec, SubagentStatus, ToolCtx, ToolRegistry, TruncateConfig,
};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tokio::sync::mpsc;

use crate::command::Command;
use crate::event::UiEvent;
use crate::export::{collect_subagent_transcripts, render_session_html};
use crate::factory::{ConfigReloader, ProviderFactory};
use crate::gate::{
    AskUserAsk, Gate, MemoryAsk, PermissionAsk, RecallAsk, SkillAsk, SubagentAsk,
    SubagentAskUserGate, SubagentPermissionGate, TaskListAsk,
};
use crate::id::{AskId, PermissionId, SessionId};
use crate::memory::MemoryStore;
use crate::scratch::ScratchStore;
use crate::skills::{render_catalog, ShadowNotice, SkillCatalog};
use crate::transcript::{
    CompactTrigger, ConfigProvenance, FileTranscript, NoopSink, TranscriptEvent, TranscriptRecord,
    TranscriptSink,
};
use crate::types::{
    AttachedImage, AttachedImageMeta, CheckResult, GateResolution, LoopResolution,
    McpConnectionOutcome, PermissionRendering, SandboxStatus, TokenUsage,
};
use crate::view_cache::{view_cache_path, ViewCache, VIEW_CACHE_VERSION};

/// The session title is the first user message, clipped to this many chars
/// (Tech Spec §16 — the heuristic v1 title).
const TITLE_CLIP: usize = 60;

/// Reserve this many tokens for model output when computing context usage,
/// or the model's max output, whichever is smaller (Tech Spec §7).
const OUTPUT_RESERVE: u64 = 8_000;

/// How often the idle loop checks alive subagents against
/// `AgentsConfig::idle_timeout_secs` (Tech Spec §8.4). A fixed, coarse sweep
/// interval — independent of the configured timeout itself — is simplest and
/// keeps the check cheap; a subagent is reaped at most this long after it
/// actually goes idle, never sooner.
const IDLE_REAP_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

/// The loop-breaking guardrail's tunables (S-5, Tech Spec §7). Defaults are
/// initial — tune with use. `enabled = false` turns the guardrail off entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// A single pass/fail command check the completion gate runs before the loop
/// may declare a task done (S-6, Tech Spec §7). Passes iff the command exits
/// with `expect_exit`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletionCheck {
    pub name: String,
    pub command: String,
    pub expect_exit: i32,
}

/// The completion gate's tunables (S-6, Tech Spec §7). The gate is inert —
/// behaves exactly as no gate at all — until at least one [`CompletionCheck`]
/// is registered, regardless of `enabled`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletionConfig {
    pub enabled: bool,
    /// Failed completion attempts allowed before the engine halts to the user
    /// (default 3).
    pub max_attempts: usize,
}

impl Default for CompletionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_attempts: 3,
        }
    }
}

/// Adaptive context-window and compaction configuration (FR-3, Tech Spec
/// §7/§8). Defaults are placeholders — tune with real long sessions (Tech Spec
/// §16, Requirements §13). `Copy` so it is cheap to pass into send-time views.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContextConfig {
    /// How many trailing non-pinned turns are sent to the provider (FR-3, Tech
    /// Spec §7). Older turns are elided from the sent context behind one
    /// marker; they stay in the transcript and the user's scrollback (HC-7).
    pub window_turns: usize,
    /// How many trailing turns `/compact` keeps verbatim (Tech Spec §7). Also
    /// the tail that manual and automatic compaction both keep. A turn
    /// boundary, not a raw message count (see [`group_turn_starts`]) — so a
    /// turn with several tool calls is kept or summarized as one unit, never
    /// split mid-`(tool_use, tool_result)`.
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
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// Skills config (FR-7, Tech Spec §8.2). Resolved from `[skills]` config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillsConfig {
    /// Whether the skill system is enabled (default `true`). When `false`, no
    /// catalog is built or pinned, and the `skill` tool returns failures.
    pub enabled: bool,
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Multi-agent subsystem config (FR-9, Tech Spec §8.4). Resolved from
/// `[agents]` config. `max_depth` (a subagent may never itself spawn a
/// subagent, Requirements §2.2) is deliberately **not** a field here — it is
/// a Rust-level structural guarantee (a subagent's tool registry never
/// contains the four multi-agent tools), not a tunable that could be
/// misconfigured away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentsConfig {
    /// Whether the multi-agent subsystem is enabled (default `true`). When
    /// `false`, the four multi-agent tools return a structured failure
    /// instead of spawning anything.
    pub enabled: bool,
    /// The ceiling on subagents alive at once per session (default `3`).
    /// Exceeding it from `spawn_agents` fails the excess names, not the whole
    /// batch (HC-6).
    pub max_concurrent: usize,
    /// How long `spawn_agents`/`message_agent` wait for a subagent's turn
    /// before reporting it `still running` rather than canceling it (default
    /// 600s).
    pub spawn_timeout_secs: u64,
    /// How long a subagent may go without a `message_agent` call before it is
    /// reclaimed as idle (default 1800s). Tech Spec §16 open item: automatic
    /// reaping against this value is not yet wired into the run loop in this
    /// phase — only `end_agent` and session end currently reclaim a
    /// subagent; the value is threaded through so it is meaningful the moment
    /// reaping lands.
    pub idle_timeout_secs: u64,
}

impl Default for AgentsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_concurrent: 3,
            spawn_timeout_secs: 600,
            idle_timeout_secs: 1800,
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
    /// is never prompted and no tokens are spent (Requirements T-9). Live —
    /// an in-app `/config` reload picks up a changed value (C-5).
    pub tool_explanations: bool,
    /// True when workspace trust was *newly* granted at startup this launch
    /// (FR-1) — the engine records a `trust_decision` at session start. A
    /// silently-already-trusted session leaves this false.
    pub trust_granted: bool,
    /// Loop-breaking guardrail tunables (S-5, Tech Spec §7).
    pub loop_config: LoopConfig,
    /// Completion-gate tunables (S-6, Tech Spec §7).
    pub completion_config: CompletionConfig,
    /// Completion checks registered at startup, from `[[completion.check]]`
    /// config (S-6, Tech Spec §7/§8).
    pub completion_checks: Vec<CompletionCheck>,
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
    /// The `provider =` / `model =` selectors config.toml currently names as
    /// the default to use, distinct from the *active* `provider`/`model`
    /// above (which only changes on an explicit `/model` switch). Retained
    /// so a `/config` reload (C-5) can notice the configured selection moved
    /// and tell the user exactly which `/model` command would follow it —
    /// reload never auto-switches the active session.
    pub configured_provider: Option<String>,
    pub configured_model: Option<String>,
    /// OS confinement status at startup (Requirements §6.7).
    pub sandbox: SandboxStatus,
    /// The permission rule engine (Requirements §6.1): built-in defaults plus
    /// global + project rules, with the bash allowlist already toggled for the
    /// sandbox status. Session grants accrue in-memory during the run.
    pub rules: RuleEngine,
    /// The raw config-sourced rules (global + project `permissions.toml`,
    /// pre-builtin/pre-grant) that `rules` was built from — retained so a
    /// `/config` reload (C-5) can tell whether the rule set actually changed
    /// and rebuild just the config-sourced portion via
    /// [`RuleEngine::reload_config_rules`].
    pub rule_specs: Vec<Rule>,
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
    /// Maximum images attachable to one prompt via `Command::AttachImage`
    /// (FR-10, Tech Spec §8, default 4).
    pub image_max_attachments: usize,
    /// Maximum document file size in bytes for the `read_document` tool (Tech
    /// Spec §5.2, default 32 MiB).
    pub document_max_bytes: usize,
    /// The outcome of connecting to each configured MCP server (FR-11, Tech
    /// Spec §5.6/§8.5) — connecting already happened (composition-root
    /// logic, mirroring `web_search`, before this config is built); the
    /// engine reports it once at session start via
    /// `UiEvent::McpServerConnected`/`McpServerFailed` and a
    /// `TranscriptEvent::McpConnection` (extends HC-7).
    pub mcp_connections: Vec<McpConnectionOutcome>,
    /// Memory config (FR-6, Tech Spec §8.1).
    pub memory: MemoryConfig,
    /// User-global memory directory (`~/.config/emberly/memory/`). Always `Some`
    /// when a home directory exists.
    pub user_memory_dir: Option<PathBuf>,
    /// Project memory directory (`<root>/.agents/memory/`). `None` on an
    /// untrusted root — structural trust-gating (FR-1, Tech Spec §6.7).
    pub project_memory_dir: Option<PathBuf>,
    /// Skills config (FR-7, Tech Spec §8.2).
    pub skills: SkillsConfig,
    /// User-global skills directory (`~/.config/emberly/skills/`). Always `Some`
    /// when a home directory exists.
    pub user_skills_dir: Option<PathBuf>,
    /// Project skills directory (`<root>/.agents/skills/`). `None` on an
    /// untrusted root — structural trust-gating (FR-1, Tech Spec §6.7).
    pub project_skills_dir: Option<PathBuf>,
    /// Multi-agent subsystem config (FR-9, Tech Spec §8.4).
    pub agents: AgentsConfig,
    /// Override the permission gate this engine's `ToolCtx`s use, in place of
    /// a fresh `Gate<PermissionAsk>` over a channel this engine owns (Tech
    /// Spec §8.4). `None` (every top-level session) is the production path;
    /// `Some` is how a subagent's tool calls are proxied to the *root*
    /// engine's own rule state instead of getting an independent copy — the
    /// same override-seam shape as `sandbox_spawn` above.
    pub external_permission_gate: Option<Arc<dyn PermissionGate>>,
    /// The `ask_user` (T-8) analogue of `external_permission_gate`: proxies a
    /// subagent's question to the root engine's own ask-user round trip
    /// instead of opening a second, competing prompt (Tech Spec §8.4). `None`
    /// is the production path for a top-level session.
    pub external_ask_gate: Option<Arc<dyn AskUserGate>>,
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
    /// A mid-stream provider error — retried per `RetryPolicy`, then surfaced.
    Errored(ProviderError),
    /// The stream ended without a `Done` — a dropped connection.
    Dropped,
}

/// The assistant output accumulated while draining one completion stream: the
/// answer text and, distinct from it, the reasoning trail (P-10) plus the
/// opaque signature to replay it on later turns.
///
/// If a field is ever added here for a new kind of streamed content, revisit
/// `push_assistant_message`'s guard (issue #13): it only checks `text` and the
/// caller's `tool_calls`, so a turn carrying nothing but the new field would
/// silently commit a message no provider adapter can serialize.
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
    // Boxed: `ToolOutcome` grew past clippy's large-enum-variant threshold
    // once it carried both an optional image and an optional document
    // payload (P-11/P-12); `Canceled` carries no data at all.
    Completed(Box<emberly_tools::ToolOutcome>),
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

/// Parse a streamed tool call's accumulated argument buffer. Every tool's
/// input schema is `"type": "object"`, so `null` is never a legitimate whole
/// argument value — whatever produced it (an empty buffer that never
/// streamed any argument text, a backend that lazily emits the literal text
/// `null` in place of real arguments, or unparseable garbage), it collapses
/// to `{}` here rather than surfacing as `Value::Null`. That keeps a
/// required-field tool's error readable ("missing field", not "expected
/// struct, found null") and keeps `null` from ever being replayed to the
/// provider as a malformed tool call in the conversation history.
fn parse_tool_args(raw: &str) -> serde_json::Value {
    let value = if raw.trim().is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_str(raw).unwrap_or(serde_json::Value::Null)
    };
    if value.is_null() {
        serde_json::Value::Object(serde_json::Map::new())
    } else {
        value
    }
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

/// The audit label for a completion-gate resolution (S-6, Tech Spec §3.2).
fn gate_resolution_label(r: &GateResolution) -> String {
    match r {
        GateResolution::Resume => "resume".into(),
        GateResolution::Stop => "stop".into(),
        GateResolution::Steer(_) => "steer".into(),
        GateResolution::Finish => "finish".into(),
    }
}

/// Whether a completion attempt may terminate the turn or must re-open the
/// loop (S-6, Tech Spec §7).
enum CompletionGateOutcome {
    /// No checks registered, or every registered check passed.
    Terminate,
    /// At least one check failed; the failure was appended to the
    /// conversation and the turn must continue.
    ReOpen,
}

/// Render failing completion checks as agent-world content the model reads
/// and reacts to (HC-6, Design §8.7) — the check name and its structured
/// reason, exactly as an ordinary tool result; the harness never editorializes.
fn render_gate_failure(failing: &[CheckResult]) -> String {
    let mut body = String::from("completion check failed:\n");
    for result in failing {
        body.push_str(&result.name);
        body.push_str(": ");
        body.push_str(&result.reason);
        body.push('\n');
    }
    body
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

/// The receiving end of every channel a turn has to service: the frontend's
/// commands plus one per tool→engine gate (Tech Spec §5.1). Bundled because the
/// set is an invariant, not a coincidence — every level of the turn call chain
/// needs *all* of it, since a tool blocked on any gate stalls the turn until
/// the engine answers. Bundling keeps those signatures short and confines a new
/// gate to this struct and the select loops that read it, instead of to every
/// signature along the chain.
///
/// Held as `&mut` references rather than by value so the same bundle can be
/// rebuilt cheaply at each call site while [`Engine::run`] keeps ownership of
/// the receivers for its own between-turns select loop.
struct TurnChannels<'a> {
    commands: &'a mut mpsc::Receiver<Command>,
    asks: &'a mut mpsc::Receiver<PermissionAsk>,
    user_asks: &'a mut mpsc::Receiver<AskUserAsk>,
    recall: &'a mut mpsc::Receiver<RecallAsk>,
    task: &'a mut mpsc::Receiver<TaskListAsk>,
    memory: &'a mut mpsc::Receiver<MemoryAsk>,
    skill: &'a mut mpsc::Receiver<SkillAsk>,
    subagent: &'a mut mpsc::Receiver<SubagentAsk>,
}

/// SIGKILLs a completion check's entire process group on drop (S-6, mirrors
/// the `bash` tool's group-kill guard, S-4): fires on a timeout or when the
/// engine drops the check future, reaping any grandchildren an `sh -c` spawns
/// that `kill_on_drop` (leader-only) would leave behind. Disarmed after a
/// clean wait.
struct CompletionCheckKillGuard {
    #[cfg_attr(not(unix), allow(dead_code))]
    pgid: Option<i32>,
}

impl CompletionCheckKillGuard {
    fn arm(child_pid: Option<u32>) -> Self {
        Self {
            pgid: child_pid.and_then(|p| i32::try_from(p).ok()),
        }
    }

    fn disarm(&mut self) {
        self.pgid = None;
    }
}

impl Drop for CompletionCheckKillGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pgid) = self.pgid {
            if let Some(pid) = rustix::process::Pid::from_raw(pgid) {
                // Best-effort: an already-exited group yields ESRCH, ignored.
                let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
            }
        }
    }
}

/// Which provider and model the session talks to, and how (P-8, P-9, C-6).
/// Grouped because a model switch has to move all of it at once, and the
/// "configured default" pair is only meaningful next to the active selection it
/// is compared against.
struct ProviderState {
    client: Arc<dyn Provider>,
    model: String,
    /// The active reasoning-effort level for subsequent turns (C-6/P-9). Seeded
    /// from the model's `default_effort`, changed by `SetEffort`, re-seeded on a
    /// model switch. `None` sends no effort (the provider's own default).
    effort: Option<Effort>,
    system: Option<String>,
    label: String,
    /// The config-configured default `provider =` / `model =` selection, as
    /// of the last time it was observed (startup or the last `/config`
    /// reload) — lets a reload detect "the configured default moved" even
    /// though the active session provider is never auto-switched.
    configured_provider: Option<String>,
    configured_model: Option<String>,
    /// Builds a provider on an in-session model switch (C-6); `None` disables it.
    factory: Option<Arc<dyn ProviderFactory>>,
}

/// The conversation and its turn numbering. Grouped because `turn_map` is
/// index-parallel to `messages` — an invariant that only holds if they are
/// updated together, which is easier to see when they cannot be reached apart.
struct HistoryState {
    messages: Vec<Message>,
    /// Parallel to `messages`: the stable monotonic turn number of each
    /// message (FR-3, Tech Spec §7/§16). `Role::User` messages start a new
    /// turn (incrementing `next_turn`); assistant/tool messages inherit the
    /// current turn. Compaction retires numbers but never shifts surviving
    /// ones, so "turn 12" means the same thing all session.
    turn_map: Vec<usize>,
    /// The next turn number to assign (monotonic; never decremented).
    next_turn: usize,
}

/// Context economy: the window/compaction config plus the live state deciding
/// when compaction fires (FR-3, FR-4, Tech Spec §7/§8).
struct ContextState {
    /// Adaptive context-window + compaction config (FR-3, Tech Spec §7/§8).
    config: ContextConfig,
    /// Most recent authoritative prompt-token count = current context size.
    /// `None` until the first provider `Usage`; then it drives the context %.
    tokens_authoritative: Option<u64>,
    /// True once a compaction has run (live or resumed), so the summary at
    /// `history.messages[1]` is pinned in the sent context (FR-3 windowing).
    compacted: bool,
    /// Set when `/compact` or the auto-trigger requests a compaction;
    /// performed at the next clean boundary (Tech Spec §7). `Manual` outranks
    /// `Auto` — a user `/compact` is never downgraded (FR-4).
    pending: Option<CompactTrigger>,
    /// Hysteresis latch for the auto-trigger (FR-4, Tech Spec §7): after an
    /// auto-compaction fires the latch disarms and stays disarmed until usage
    /// has fallen below the threshold and re-crossed it (no thrash).
    auto_armed: bool,
    /// Optional `/compact` prompt override (P-7).
    summary_prompt: Option<String>,
}

/// Loop-breaking guardrail state (S-5, Tech Spec §7). All of it is one
/// sliding-window judgement about whether the agent is making progress.
struct GuardrailState {
    config: LoopConfig,
    /// What the in-flight tool-call turn did (reset per turn).
    turn_obs: TurnObservation,
    /// Cumulative modified-file paths across the session (progress if a turn
    /// adds a new one).
    seen_files: HashSet<String>,
    /// Cumulative distinct tool-result hashes (progress if a turn adds a new
    /// one).
    seen_results: HashSet<u64>,
    /// Tool signature of the previous no-progress turn.
    last_sig: Option<u64>,
    /// Consecutive no-progress turns with the *same* tool signature.
    same_sig_streak: usize,
    /// Consecutive no-progress turns (any signature).
    no_progress_streak: usize,
}

/// Completion-gate state (S-6, Tech Spec §7).
struct CompletionState {
    /// Completion-gate tunables.
    config: CompletionConfig,
    /// Registered completion checks the gate evaluates on a completion attempt.
    /// Populated from config at startup; empty means the gate is inert.
    checks: Vec<CompletionCheck>,
    /// Consecutive failed completion attempts this session. Reset on a new
    /// session and when the gate passes or is resolved to try again.
    attempts: usize,
}

/// The tool→engine gates, each installed into every `ToolCtx` so a tool can
/// reach engine-owned state without owning any of it (Tech Spec §5.1).
struct Gates {
    /// The permission gate (Requirements §6). A trait object rather than the
    /// concrete `Gate<PermissionAsk>` so a subagent's engine can install
    /// `SubagentPermissionGate` here instead (Tech Spec §8.4,
    /// `EngineConfig::external_permission_gate`) — the production path (every
    /// top-level session) is still exactly `Gate::new(..)`.
    permission: Arc<dyn PermissionGate>,
    /// The raw sender behind `permission` **only when this is the production
    /// `Gate::new(..)` path** — i.e. always, for a top-level session. Kept
    /// alongside the trait object so `engine::subagents` can hand a subagent
    /// a proxy pointed at *this* engine's own permission channel (Tech Spec
    /// §8.4) without reaching through the trait object, which erases the
    /// concrete sender.
    permission_tx: mpsc::Sender<PermissionAsk>,
    /// The ask-user gate (T-8), so the `ask_user` tool can block on a frontend
    /// round trip. A trait object for the same reason as `permission`.
    ask: Arc<dyn AskUserGate>,
    /// The raw sender behind `ask`, for the same reason as `permission_tx`.
    ask_tx: mpsc::Sender<AskUserAsk>,
    /// The recall gate (T-10), so the `recall` tool can retrieve elided turns
    /// from the in-memory conversation.
    recall: Arc<Gate<RecallAsk>>,
    /// The task-list gate (T-11), so the `todo` tool can replace the full task
    /// list in engine state.
    task_list: Arc<Gate<TaskListAsk>>,
    /// The memory gate (T-13), so the `memory` tool can read and write durable
    /// memory entries.
    memory: Arc<Gate<MemoryAsk>>,
    /// The skill gate (T-15), so the `skill` tool can load instruction bodies.
    skill: Arc<Gate<SkillAsk>>,
    /// The scratch gate (T-17), so the `scratch_write` tool can write into
    /// this session's disposable working directory. Unlike the other gates,
    /// this one acts directly rather than through a channel to the engine loop
    /// — a scratch write has no side effect on any other engine-owned state
    /// (FR-8, Tech Spec §8.3).
    scratch: Arc<ScratchStore>,
    /// The subagent gate (T-18–T-21), so the four multi-agent tools can reach
    /// this engine's own subagent registry (Tech Spec §8.4). A subagent's own
    /// tool registry never contains these four tools (the structural depth
    /// bound, Requirements §2.2), so a subagent's own copy of this gate is
    /// simply never exercised.
    subagent: Arc<Gate<SubagentAsk>>,
    /// The raw sender behind `subagent`, for the same reason as
    /// `permission_tx`/`ask_tx`: a spawned subagent's own per-subagent driver
    /// task (`engine::subagents`) reports its token/cost usage back through
    /// this same channel via `SubagentAsk::ReportUsage`, a fire-and-forget
    /// notification outside the `SubagentGate` trait surface.
    subagent_tx: mpsc::Sender<SubagentAsk>,
}

/// Which session this is, where it is recorded, and the per-session totals
/// (HC-7). A session switch replaces all of it together.
struct SessionState {
    /// Durable transcript sink (HC-7). Written per event; a `NoopSink` when no
    /// session file is configured.
    transcript: Box<dyn TranscriptSink>,
    id: SessionId,
    /// Where to create a new/resumed transcript on an in-session switch.
    dir: PathBuf,
    /// Shared with the host so the panic/exit path tracks the current session.
    active_path: Arc<RwLock<PathBuf>>,
    /// Cumulative billed tokens this session (summed per request — each
    /// request's input is billed, so this is the cost basis, not the context
    /// size).
    usage: TokenUsage,
    /// Running session cost estimate in USD (only when pricing is configured).
    cost_usd: f64,
    /// Whether the first (pinned, `original_task`) user message has been
    /// recorded — also gates the one-time `session_title`.
    original_task_recorded: bool,
    /// True when this run resumed an existing transcript (skips `session_start`).
    resuming: bool,
    /// True when the resume fell back to transcript replay (FR-5 slow path).
    /// Drives the one dimmed Notice on engine start (Design §8.6).
    replayed: bool,
    /// True when the transcript has been appended to since the last view-cache
    /// write, so the sidecar no longer matches the log the staleness guard
    /// compares it against (Tech Spec §3.2a). Set by every `write_transcript`,
    /// cleared by `write_view_cache`; the idle loop flushes on it so an
    /// out-of-turn write (a model switch, an effort change) can never strand
    /// the cache and force a replay that loses the session's accounting
    /// (issue #18).
    cache_dirty: bool,
}

/// The safety layers (Requirements §6): what the OS enforces, what the rules
/// decide, and the mode that may only ever relax an ask.
struct SafetyState {
    sandbox: SandboxStatus,
    /// The permission rule engine consulted by the gate.
    rules: RuleEngine,
    /// The raw config-sourced rules `rules` was built from — retained so a
    /// `/config` reload (C-5) can detect a change and rebuild just that
    /// portion via [`RuleEngine::reload_config_rules`].
    rule_specs: Vec<Rule>,
    /// The current auto-accept mode (§6.4). Starts [`Mode::Normal`]; changed
    /// only through [`Engine::set_mode`], which gates auto tiers on `sandbox`.
    mode: Mode,
    /// How bash spawns children: confined via the self-exec shim when the OS
    /// sandbox is active, directly when degraded. Built from `sandbox`.
    spawn: Arc<dyn Sandbox>,
}

/// Durable memory (FR-6, T-13, Tech Spec §8.1).
struct MemoryState {
    config: MemoryConfig,
    /// The durable memory store. `None` when memory is disabled or no home
    /// directory exists.
    store: Option<MemoryStore>,
    /// Retained from [`EngineConfig`] so a `/config` reload (C-5) can rebuild
    /// `store` without needing a full `EngineConfig`.
    user_dir: Option<PathBuf>,
    /// Retained from [`EngineConfig`] for the same reason as `user_dir`.
    project_dir: Option<PathBuf>,
    /// Cached user-global memory index text for pinning (Tech Spec §7).
    user_index: String,
    /// Cached project memory index text for pinning (empty when untrusted).
    project_index: String,
    /// Whether the max_index_entries soft-cap warning has been emitted this
    /// session (Tech Spec §16 — warn once, do not truncate).
    warn_emitted: bool,
}

/// The skill system (FR-7, T-15, Tech Spec §8.2).
struct SkillState {
    config: SkillsConfig,
    /// The skill catalog. `None` when skills are disabled or no home directory
    /// exists.
    catalog: Option<SkillCatalog>,
    /// Retained from [`EngineConfig`] so a `/config` reload (C-5) can rebuild
    /// `catalog` without needing a full `EngineConfig`.
    user_dir: Option<PathBuf>,
    /// Retained from [`EngineConfig`] for the same reason as `user_dir`.
    project_dir: Option<PathBuf>,
    /// The built skill metadata for pinning + `SkillsAvailable`.
    metas: Vec<emberly_tools::SkillMeta>,
    /// Cached catalog text for pinning (Tech Spec §7).
    catalog_text: String,
    /// Shadow notices for `emberly config show` (Tech Spec §8.2).
    shadows: Vec<ShadowNotice>,
}

/// The agent engine.
///
/// The state above is grouped into sub-structs by concern rather than held as
/// one flat list. This is not cosmetic: 62 of the fields belonged to ten
/// clusters that are always read and written together (a model switch, a
/// session switch, one compaction decision), and flattening them made it
/// possible to update one and forget its siblings. The engine is still the
/// single owner of all of it — the grouping changes reachability, not
/// ownership (A-1).
pub struct Engine {
    provider: ProviderState,
    history: HistoryState,
    context: ContextState,
    guardrail: GuardrailState,
    completion: CompletionState,
    gates: Gates,
    session: SessionState,
    safety: SafetyState,
    memory: MemoryState,
    skills: SkillState,
    tools: ToolRegistry,
    project_root: PathBuf,
    /// Whether tool-call explanations are enabled (T-9); see [`EngineConfig`].
    tool_explanations: bool,
    /// Newly-granted workspace trust to record at session start (FR-1).
    trust_granted: bool,
    truncate: TruncateConfig,
    retry: RetryPolicy,
    events_tx: mpsc::Sender<UiEvent>,
    next_permission_id: u64,
    /// Monotonic id source for `ask_user` questions (T-8).
    next_ask_id: u64,
    config_provenance: Vec<ConfigProvenance>,
    /// Re-reads config on an in-app edit (C-5); `None` disables live reload.
    config_reloader: Option<Arc<dyn ConfigReloader>>,
    /// The model-maintained task list (T-11). Pure engine state — the engine
    /// stores the last full replace, emits updates to frontends, and records
    /// them in the transcript (HC-7). Reset on a new session.
    task_list: Vec<emberly_tools::TaskItem>,
    /// Maximum image file size in bytes (Tech Spec §5.2). Threaded to the
    /// `read_image` tool via `ToolCtx`.
    image_max_bytes: usize,
    /// Maximum images attachable to one prompt via `Command::AttachImage`
    /// (FR-10, Tech Spec §8).
    image_max_attachments: usize,
    /// Images staged by `Command::AttachImage` for the prompt currently being
    /// composed (FR-10, Design §4.14), drained into the next
    /// `Command::UserInput`'s content. Engine-owned staging state so the
    /// `UserInput` command's shape never changed for this feature.
    pending_attachments: Vec<AttachedImage>,
    /// Maximum document file size in bytes (Tech Spec §5.2). Threaded to the
    /// `read_document` tool via `ToolCtx`.
    document_max_bytes: usize,
    /// MCP server connection outcomes awaiting their one-time startup report
    /// (FR-11, Tech Spec §5.6/§8.5) — drained by `run()`'s first tick, never
    /// touched again for the life of the session (connecting again only
    /// happens on `/reload`, handled inline where that command is processed).
    mcp_connections: Vec<McpConnectionOutcome>,
    /// The multi-agent subsystem's own state (FR-9, Tech Spec §8.4): every
    /// currently alive subagent, keyed by the id the model addresses it by.
    agents: AgentState,
}

// The `Engine` impl is spread across these modules by topic, each holding its
// own `impl Engine` block — an inherent impl only has to live in the same crate
// as the type. Each module sees this one's private items, but not a sibling's,
// so a method called from another of these modules is marked `pub(super)`.
// None of this is public API.
mod asks;
mod context;
mod guardrail;
mod mcp;
mod memory;
mod permissions;
mod runtime_config;
mod session;
mod skills;
mod subagents;
mod turn;

use subagents::AgentState;

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
        mpsc::Receiver<SkillAsk>,
        mpsc::Receiver<SubagentAsk>,
    ) {
        let (asks_tx, asks_rx) = mpsc::channel(crate::channels::DEFAULT_CHANNEL_CAPACITY);
        let (user_asks_tx, user_asks_rx) = mpsc::channel(crate::channels::DEFAULT_CHANNEL_CAPACITY);
        let (recall_tx, recall_rx) = mpsc::channel(crate::channels::DEFAULT_CHANNEL_CAPACITY);
        let (task_list_tx, task_list_rx) = mpsc::channel(crate::channels::DEFAULT_CHANNEL_CAPACITY);
        let (memory_tx, memory_rx) = mpsc::channel(crate::channels::DEFAULT_CHANNEL_CAPACITY);
        let (skill_tx, skill_rx) = mpsc::channel(crate::channels::DEFAULT_CHANNEL_CAPACITY);
        let (subagent_tx, subagent_rx) = mpsc::channel(crate::channels::DEFAULT_CHANNEL_CAPACITY);
        // Cloned before `asks_tx`/`user_asks_tx` are (possibly) moved into the
        // default `Gate::new(..)` below — the raw senders let a subagent's
        // derived config point its own permission/ask gates at *this* engine's
        // channel instead of building a fresh one (Tech Spec §8.4).
        let permission_tx = asks_tx.clone();
        let ask_tx = user_asks_tx.clone();
        let subagent_tx_raw = subagent_tx.clone();
        let memory_store = build_memory_store(
            config.memory.enabled,
            config.user_memory_dir.as_ref(),
            config.project_memory_dir.clone(),
        );
        let skill_catalog = build_skill_catalog(
            config.skills.enabled,
            config.user_skills_dir.as_ref(),
            config.project_skills_dir.clone(),
        );
        let scratch_store = Arc::new(ScratchStore::new(scratch_dir_for(
            &config.project_root,
            config.session_id,
        )));
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
            provider: ProviderState {
                client: config.provider,
                model: config.model,
                effort: seed_effort,
                system: config.system,
                label: config.provider_label,
                configured_provider: config.configured_provider,
                configured_model: config.configured_model,
                factory: config.provider_factory,
            },
            history: HistoryState {
                messages: config.initial_conversation,
                turn_map,
                next_turn,
            },
            context: ContextState {
                config: config.context,
                tokens_authoritative: context_tokens_authoritative,
                compacted: config.compacted,
                pending: None,
                auto_armed: true,
                summary_prompt: config.summary_prompt,
            },
            guardrail: GuardrailState {
                config: config.loop_config,
                turn_obs: TurnObservation::default(),
                seen_files: HashSet::new(),
                seen_results: HashSet::new(),
                last_sig: None,
                same_sig_streak: 0,
                no_progress_streak: 0,
            },
            completion: CompletionState {
                config: config.completion_config,
                checks: config.completion_checks,
                attempts: 0,
            },
            gates: Gates {
                permission: config
                    .external_permission_gate
                    .clone()
                    .unwrap_or_else(|| Arc::new(Gate::new(asks_tx))),
                permission_tx,
                ask: config
                    .external_ask_gate
                    .clone()
                    .unwrap_or_else(|| Arc::new(Gate::new(user_asks_tx))),
                ask_tx,
                recall: Arc::new(Gate::new(recall_tx)),
                task_list: Arc::new(Gate::new(task_list_tx)),
                memory: Arc::new(Gate::new(memory_tx)),
                skill: Arc::new(Gate::new(skill_tx)),
                scratch: scratch_store,
                subagent: Arc::new(Gate::new(subagent_tx)),
                subagent_tx: subagent_tx_raw,
            },
            session: SessionState {
                transcript: config.transcript,
                id: config.session_id,
                dir: config.sessions_dir,
                active_path: config.active_session_path,
                usage: session_usage,
                cost_usd: session_cost_usd,
                // On resume the original task already lives in the restored history.
                original_task_recorded,
                resuming: config.resuming,
                replayed: config.replayed,
                // Nothing has been appended yet this run; a `session_start`
                // (or the first turn) marks it dirty.
                cache_dirty: false,
            },
            safety: SafetyState {
                sandbox: config.sandbox,
                rules: config.rules,
                rule_specs: config.rule_specs,
                mode: Mode::Normal,
                spawn: sandbox_spawn,
            },
            memory: MemoryState {
                config: config.memory.clone(),
                store: memory_store,
                user_dir: config.user_memory_dir,
                project_dir: config.project_memory_dir,
                user_index: String::new(),
                project_index: String::new(),
                warn_emitted: false,
            },
            skills: SkillState {
                config: config.skills.clone(),
                catalog: skill_catalog,
                user_dir: config.user_skills_dir,
                project_dir: config.project_skills_dir,
                metas: Vec::new(),
                catalog_text: String::new(),
                shadows: Vec::new(),
            },
            tools: config.tools,
            project_root: config.project_root,
            tool_explanations: config.tool_explanations,
            trust_granted: config.trust_granted,
            truncate: config.truncate,
            retry: config.retry,
            events_tx,
            next_permission_id: 0,
            next_ask_id: 0,
            config_provenance: config.config_provenance,
            config_reloader: config.config_reloader,
            task_list: Vec::new(),
            image_max_bytes: config.image_max_bytes,
            image_max_attachments: config.image_max_attachments,
            pending_attachments: Vec::new(),
            document_max_bytes: config.document_max_bytes,
            mcp_connections: config.mcp_connections,
            agents: AgentState {
                config: config.agents,
                instances: std::collections::HashMap::new(),
                next_seq: 0,
            },
        };
        // Load memory indexes at session start (Tech Spec §8.1).
        engine.refresh_memory_indexes();
        let (user_count, project_count) = engine
            .memory
            .store
            .as_ref()
            .map_or((0, 0), |s| s.status_counts());
        let _ = engine.events_tx.try_send(UiEvent::MemoryStatus {
            user: user_count,
            project: project_count,
        });
        // Build the skill catalog and emit SkillsAvailable at session start
        // (Tech Spec §8.2, §3.1).
        engine.refresh_skill_catalog();
        let _ = engine.events_tx.try_send(UiEvent::SkillsAvailable {
            skills: engine.skills.metas.clone(),
        });
        (
            engine,
            asks_rx,
            user_asks_rx,
            recall_rx,
            task_list_rx,
            memory_rx,
            skill_rx,
            subagent_rx,
        )
    }

    /// Register a completion check the gate evaluates on every completion
    /// attempt (S-6, Requirements S-6). Config is the shipped registrant
    /// (`[[completion.check]]`); this seam also lets a future frontend or tool
    /// register a check programmatically.
    pub fn register_completion_check(&mut self, check: CompletionCheck) {
        self.completion.checks.push(check);
    }

    /// Run the engine until the command channel closes. Idle between turns,
    /// waiting for a `UserInput`; a turn owns `commands_rx`/`asks_rx` for its
    /// duration (permission answers and cancellation arrive through them).
    // The receivers arrive individually because the caller creates the channels
    // and keeps the senders; taking them as one struct would only move that
    // construction across the boundary. This is the last signature to list them:
    // everything below takes a [`TurnChannels`] borrow, which also lets this
    // function retain ownership for its own between-turns select.
    // Explicit `Pin<Box<dyn Future + Send>>` return, not `async fn`: a
    // subagent's own tool use can reach back into this same function
    // (`engine::subagents::spawn_one_subagent` spawning *another* nested
    // `Engine::run`, Tech Spec §8.4) — a recursive async call graph through
    // the same function. Rust's Send-auto-trait inference on an implicit
    // `impl Future` cannot resolve that cycle (verified: it cannot, even
    // with a `Box::pin` at the *call* site — only boxing the function's own
    // return type breaks it). Every existing call site is unaffected: this
    // type still implements `Future` and is passed to `tokio::spawn`
    // identically to before.
    #[allow(clippy::too_many_arguments)]
    pub fn run(
        mut self,
        mut commands_rx: mpsc::Receiver<Command>,
        mut asks_rx: mpsc::Receiver<PermissionAsk>,
        mut user_asks_rx: mpsc::Receiver<AskUserAsk>,
        mut recall_rx: mpsc::Receiver<RecallAsk>,
        mut task_rx: mpsc::Receiver<TaskListAsk>,
        mut memory_rx: mpsc::Receiver<MemoryAsk>,
        mut skill_rx: mpsc::Receiver<SkillAsk>,
        mut subagent_rx: mpsc::Receiver<SubagentAsk>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        Box::pin(async move {
            if self.session.resuming {
                // Continuing an existing transcript: no fresh session_start, but
                // surface the restored context size right away (Design §8.4).
                // When the resume fell back to transcript replay (no valid cache),
                // say so in one dimmed line — speech about the slow path only
                // (Design §8.6). The fast path is silent.
                if self.session.replayed {
                    self.emit(UiEvent::Notice {
                        message: "Rebuilding the session from its transcript…".into(),
                    })
                    .await;
                }
                self.emit_context_usage().await;
            } else {
                self.write_transcript(TranscriptEvent::SessionStart {
                    session_id: self.session.id,
                    provider: self.provider.label.clone(),
                    model: self.provider.model.clone(),
                    project_root: self.project_root.display().to_string(),
                    sandbox: self.safety.sandbox.clone(),
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
                status: self.safety.sandbox.clone(),
            })
            .await;
            if !self.safety.sandbox.is_confined() {
                self.emit(UiEvent::Notice {
                    message: degraded_notice(&self.safety.sandbox),
                })
                .await;
            }
            // Report MCP server connection outcomes that already happened
            // before this config was built (FR-11, Tech Spec §5.6/§8.5) —
            // the audit-visible half of a composition-root decision.
            self.report_mcp_connections().await;

            // Surface the initial reasoning-effort state so the sidebar and picker
            // start correct (P-9).
            self.emit_effort().await;

            // A periodic tick alongside `commands_rx` is what makes idle-reap
            // (Tech Spec §8.4, `idle_timeout_secs`) work even while genuinely
            // idle (no user input at all) — a subagent's own proxied asks are
            // already covered by `run_one_tool_call`'s own select (Tech Spec
            // §8.4's module docs on `engine::subagents`), but that select only
            // runs *during* a turn. `MissedTickBehavior::Delay` means a long
            // turn never produces a burst of catch-up ticks afterward.
            let mut idle_tick = tokio::time::interval(IDLE_REAP_CHECK_INTERVAL);
            idle_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                let command = tokio::select! {
                    cmd = commands_rx.recv() => cmd,
                    _ = idle_tick.tick() => {
                        self.reap_idle_subagents().await;
                        continue;
                    }
                };
                let Some(command) = command else { break };
                match command {
                    Command::UserInput { text } => {
                        // No provider configured (C-7): refuse before it ever
                        // becomes a turn, rather than round-tripping through the
                        // inert stand-in `Provider` the binary substitutes in
                        // this case (id "placeholder" — never a real backend, and
                        // distinct from `configured_provider`, which is just
                        // config-reload bookkeeping and legitimately `None` in
                        // tests that use a real `FakeProvider`). Not
                        // recorded/pushed — a message that never ran shouldn't
                        // burn a turn number or claim the "original task" slot;
                        // the real first message still gets both once a
                        // provider is added.
                        if self.provider.client.id() == ProviderId::new("placeholder") {
                            self.emit(UiEvent::Notice {
                                message: "no provider configured — run /model to add one before \
                                      sending a message"
                                    .into(),
                            })
                            .await;
                            self.emit(UiEvent::TurnEnded).await;
                            continue;
                        }
                        let attachments = std::mem::take(&mut self.pending_attachments);
                        let images_meta: Vec<AttachedImageMeta> =
                            attachments.iter().map(AttachedImageMeta::from).collect();
                        self.record_user_message(&text, images_meta);
                        let user_message = self.build_user_message(text, attachments);
                        self.push_conversation_message(user_message);
                        self.emit_context_usage().await;
                        self.run_turn(&mut TurnChannels {
                            commands: &mut commands_rx,
                            asks: &mut asks_rx,
                            user_asks: &mut user_asks_rx,
                            recall: &mut recall_rx,
                            task: &mut task_rx,
                            memory: &mut memory_rx,
                            skill: &mut skill_rx,
                            subagent: &mut subagent_rx,
                        })
                        .await;
                        // The engine is idle again; let the frontend stop its
                        // "working" affordance (Design §6.3).
                        self.emit(UiEvent::TurnEnded).await;
                        // A `/compact` sent mid-turn or an auto-trigger request
                        // runs now, at the clean boundary (every tool_use has its
                        // tool_result — Tech Spec §7).
                        if let Some(trigger) = self.context.pending.take() {
                            self.compact(trigger).await;
                        }
                    }
                    // No turn is running while idle; these are strays or no-ops here.
                    Command::Cancel
                    | Command::PermissionAnswer { .. }
                    | Command::AskUserAnswer { .. }
                    | Command::ResolveLoop { .. }
                    | Command::ResolveCompletionGate { .. } => {}
                    // Idle is already a clean boundary — compact immediately.
                    Command::Compact => self.compact(CompactTrigger::Manual).await,
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
                    // Inspector reads/mutations — user/TUI actions, issued at idle
                    // (a clean boundary), mirroring the config/effort commands above.
                    Command::MemoryList => self.emit_memory_entries().await,
                    Command::MemoryMutate {
                        op,
                        scope,
                        name,
                        description,
                        type_,
                        body,
                    } => {
                        // The harness performs the write via the shared validated
                        // path (FR-6) — never the TUI. `MemoryStatus` re-emits
                        // inside on success; the inspector re-issues `MemoryList` to
                        // refresh its open list.
                        let req = emberly_tools::MemoryRequest {
                            op,
                            scope,
                            name,
                            description,
                            type_,
                            body,
                        };
                        if let emberly_tools::MemoryOutcome::Rejected { reason } =
                            self.execute_memory_op(&req).await
                        {
                            // Surface a rejection (disabled memory, untrusted scope,
                            // invalid name) so the user sees why (Design §6.1).
                            self.emit(UiEvent::Notice {
                                message: format!("memory change rejected: {reason}"),
                            })
                            .await;
                        }
                    }
                    Command::MemoryView { scope, name } => self.emit_memory_body(scope, name).await,
                    Command::InspectSkill { name } => self.inspect_skill(name).await,
                    Command::InspectAgent { id } => self.inspect_agent(id).await,
                    Command::AttachImage { path } => self.attach_image(path).await,
                    Command::ExportSession { path } => self.export_session(path).await,
                }
                // The idle boundary is where the derived cache is reconciled with
                // the log (FR-5, Tech Spec §3.2a): one flush covers a completed
                // turn, a compaction, and the out-of-turn writes that have no view
                // of their own to settle — a model switch, an effort change, a mode
                // change. Gated on the flag so commands that touched no transcript
                // line (an inspector read) cost nothing (issue #18).
                if self.session.cache_dirty {
                    self.write_view_cache();
                }
            }

            // Command channel closed: the frontend is gone. Clean end of session.
            self.write_transcript(TranscriptEvent::SessionEnd { reason: None });
            // Write the cache one final time so it reflects the final transcript
            // (the SessionEnd line grew the file; without this the cache would be
            // stale on the next resume — FR-5).
            self.write_view_cache();
        })
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

    async fn emit_provider_error(&mut self, error: &ProviderError) {
        self.write_transcript(TranscriptEvent::ProviderError {
            what: "the model request failed".into(),
            why: error.to_string(),
            retry_attempt: None,
            retry_max: None,
        });
        self.emit(UiEvent::HarnessError {
            what: "the model request failed".into(),
            why: error.to_string(),
            next: "send your message again to retry".into(),
        })
        .await;
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
                // A document is summarized by its media type, not its bytes
                // (HC-2 — the harness never parses it).
                ContentBlock::Document { media_type, .. } => {
                    format!("[document: {media_type}]")
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
/// results are reduced via `reduce_output` so recall costs tokens
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
                    // Reduce tool results via the salient reduction
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
                ContentBlock::Document { media_type, .. } => {
                    format!("[document: {media_type}]")
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

/// Rough per-document token cost for the context-budget estimate (P-12). A
/// PDF's cost is not chars/4 of its base64 either; same reasoning as
/// [`IMAGE_TOKEN_ESTIMATE`]. Initial; tune with use (Requirements §13, Tech
/// Spec §16).
const DOCUMENT_TOKEN_ESTIMATE: u64 = 765;

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

/// The session's scratch directory: `<project_root>/.agents/scratch/<session-id>/`
/// (FR-8, Tech Spec §8.3). Unlike memory, there is only ever one location —
/// always inside the project, derived from the session id — so this needs no
/// config field of its own.
fn scratch_dir_for(project_root: &std::path::Path, session_id: SessionId) -> PathBuf {
    project_root
        .join(".agents")
        .join("scratch")
        .join(session_id.to_string())
}

/// Build the memory store from the engine config (FR-6, Tech Spec §8.1). Returns
/// `None` when memory is disabled or no home directory exists.
fn build_memory_store(
    enabled: bool,
    user_dir: Option<&PathBuf>,
    project_dir: Option<PathBuf>,
) -> Option<MemoryStore> {
    if !enabled {
        return None;
    }
    let user_dir = user_dir?;
    Some(MemoryStore::new(user_dir.clone(), project_dir))
}

/// Build the skill catalog (FR-7, Tech Spec §8.2). Returns `None` when skills
/// are disabled or no home directory exists. Plain params (rather than
/// `&EngineConfig`) so a `/config` reload (C-5) can rebuild it too.
fn build_skill_catalog(
    enabled: bool,
    user_dir: Option<&PathBuf>,
    project_dir: Option<PathBuf>,
) -> Option<SkillCatalog> {
    if !enabled {
        return None;
    }
    let user_dir = user_dir?;
    Some(SkillCatalog::new(user_dir.clone(), project_dir))
}

/// Enrich a tool's [`PermissionRequest`] into a UI [`PermissionRendering`] with
/// the `reason` the rule engine produced — the matched rule or the hard line —
/// shown as the dimmed "why" that teaches the model in situ (Design §5).
/// `on_behalf_of` names the subagent that raised this ask, if any (FR-9, Tech
/// Spec §8.4) — `None` for the primary agent's own requests.
fn build_rendering(
    request: &PermissionRequest,
    reason: String,
    on_behalf_of: Option<String>,
) -> PermissionRendering {
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
        on_behalf_of,
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
    use super::{
        explanation_from_args, inject_explanation_property, normalize_args, parse_tool_args,
    };
    use serde_json::json;

    #[test]
    fn parse_tool_args_empty_buffer_is_an_empty_object_not_null() {
        // A backend that ends a tool call without ever streaming argument text
        // (e.g. some local/cloud OpenAI-compatible models) leaves the buffer
        // empty. `{}` lets a no-args tool run and, for a tool with required
        // fields, fails with "missing field" rather than "expected struct,
        // found null" — and it round-trips back to the provider as `"{}"`
        // instead of the malformed literal string `"null"`.
        assert_eq!(parse_tool_args(""), json!({}));
        assert_eq!(parse_tool_args("   "), json!({}));
    }

    #[test]
    fn parse_tool_args_valid_json_parses_normally() {
        assert_eq!(
            parse_tool_args(r#"{"path":"a.txt"}"#),
            json!({ "path": "a.txt" })
        );
    }

    #[test]
    fn parse_tool_args_literal_null_text_is_an_empty_object_too() {
        // Confirmed live against qwen3.5:cloud (Ollama-hosted): rather than
        // omitting arguments or sending "", this backend streams the literal
        // 4-byte text `null` — valid JSON, so it parses straight to
        // `Value::Null` without ever hitting the empty-buffer branch above.
        assert_eq!(parse_tool_args("null"), json!({}));
    }

    #[test]
    fn parse_tool_args_garbage_also_collapses_to_an_empty_object() {
        assert_eq!(parse_tool_args("not json"), json!({}));
    }

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
