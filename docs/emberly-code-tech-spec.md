# Emberly Code — Technical Specification

**Version:** 0.15 
**Status:** approved
**Date:** 2026-09-02
**Owner:** Wattanit
**Companion documents:** Requirements Document v0.12 (upstream contract),
Design Guideline v0.12 (upstream for all UI/UX decisions)

This document defines HOW Emberly Code is built. Requirements-level
identifiers (HC-n, FR-n, P-n, T-n, C-n, S-n, A-n) refer to the Requirements
Document. Where this spec makes a choice, the requirement it satisfies is
cited so traceability is mechanical.

---

## 1. Workspace Layout

Cargo workspace, six crates, strictly one-way dependency flow:

```
emberly/
├── crates/
│   ├── emberly-core/      # engine, event model, session, context mgmt
│   ├── emberly-providers/ # Provider trait + Anthropic, OpenAI-compat
│   ├── emberly-tools/     # Tool trait + the built-in tool suite
│   ├── emberly-sandbox/   # permission rules + OS confinement
│   ├── emberly-tui/       # ratatui frontend
│   └── emberly/           # thin binary: wiring, CLI args, supervisor
```

- Dependency direction: `emberly` → { `tui`, `core` }; `tui` → `core`;
`core` → { `providers`, `tools`, `sandbox` } via traits. `tools` and
`providers` do not know each other. Nothing depends on `tui`.
- `emberly-sandbox` is deliberately separate: it is the security-critical
code and must stay small, dependency-minimal, and separately auditable.
Its dependency list is frozen harder than the rest (see §15).
- Lint policy (HC-1, HC-3): every crate carries
`#![forbid(unsafe_code)]`. `core`, `providers`, `tools`, `sandbox`
additionally carry `#![deny(clippy::unwrap_used, clippy::expect_used)]`
enforced in CI. `anyhow` appears only in the `emberly` binary;
library crates define errors with `thiserror`.

## 2. Concurrency Model

- **Runtime:** Tokio (multi-thread runtime, but the design assumes
nothing about parallelism — see below).
- **Engine as single owner.** `emberly-core` runs the agent loop as one
async task owning ALL mutable session state (conversation view,
transcript writer, token accounting, permission grants, mode). No
`Arc<Mutex<_>>` state sharing. The engine communicates exclusively
through channels:
  - `mpsc::Sender<UiEvent>` — events out (A-1, A-3).
  - `mpsc::Receiver<Command>` — commands in (user input, permission
  answers, mode changes, `/compact`, cancel).
- **Frontends are consumers.** The TUI is a separate task consuming
`UiEvent` and producing `Command`. A future headless frontend is a
different consumer of the same channels; the engine cannot tell the
difference (A-1, A-2).
- **Child processes** (bash tool) run via `tokio::process` under a
timeout (`S-4`); each child is placed in its own process group so
timeout/cancel kills the whole tree, not just the shell. Cancellation
(Esc / Ctrl+C during a run) is a `Command`, handled by the engine at
the next await point; a running child is killed by process group.
- **Provider streaming** runs inside the engine task (one in-flight
completion at a time — the agent loop is inherently sequential).

## 3. Event Model

Two related but distinct event types (rationale: transcript durability
vs UI streaming have incompatible granularity):

### 3.1 `UiEvent` (ephemeral, high-frequency)

Non-exhaustive enum, serializable (A-3):
`AssistantDelta(String)`, `ReasoningDelta(String)` (P-10), `AssistantDone`,
`ToolStarted{.., explanation: Option<String>}` (the T-9 line, §5.4),
`ToolFinished{..}`, `PermissionRequest{id, rendering}`,
`AskUserRequest{id, question, options}` (T-8), `LoopHalted{reason}` (S-5),
`ContextUsage{pct, tokens}`, `CostEstimate{..}`,
`SandboxStatus(..)`, `ModeChanged(..)`, `ModelChanged{provider, model}` and
`EffortChanged{effort: Option<Effort>, available: Vec<Effort>}` (C-6/P-9 —
carries the model's offered levels so the picker knows its options and the
sidebar the current one; `effort`/`available` empty when the model has no
control), `HarnessError{..}`, `SessionMeta{..}`,
`FileModified{path, adds, dels}`, `CompactionStatus(..)`.

The 0.4 feature set adds (all serializable, A-3):
`TaskListUpdated{items: Vec<TaskItem>}` (T-11, the model-authored list for the
sidebar Tasks section and inline render, Design §4.7),
`SkillsAvailable{skills: Vec<SkillMeta>}` and `MemoryStatus{user: usize, project: usize}` (sidebar catalog/counts for the Skills and Memory sections,
Design §3.1/§4.9; emitted at session start and when the set changes). Memory
writes/recalls, skill invocations, image reads, and web searches are ordinary
tool calls, so they flow through the existing `ToolStarted`/`ToolFinished`
events (Design renders origin and untrusted-content labels from the tool
result payload, §4.9/§4.10) — no per-feature UiEvent for them.

The 0.4.1 feature set adds `CompletionGateHalted{failing: Vec<CheckResult>, attempts: usize}` (S-6, §7) — the harness-voice halt after bounded failed
completion attempts, awaiting a user resolution (Design §8.7). A failing check's
reason returns to the model as ordinary tool-result content (agent-world, HC-6),
so per-check results need no UiEvent — only the halt does; and a document read is
an ordinary tool call flowing through `ToolStarted`/`ToolFinished` like an image
read (Design renders the §4.11 reference line from the result payload).

The 0.5 feature set adds `SubagentSpawned{id, name, profile, model}` (FR-9,
T-18, §8.4) when a subagent is created, `SubagentStatus{id, name, status}`
(status: `running | awaiting_permission | done | timed_out | error`) on each
of its status changes, and `SubagentEnded{id, reason}` (T-21, §8.4) — the
sidebar Agents section (Design §3.1) and its per-agent inspector (Design
§4.13) render from these three. A subagent's own assistant text and tool
activity are **not** re-emitted as top-level `UiEvent`s onto the primary
session's stream (Design §4.13's "no raw concurrent streaming"); they are
recorded to the subagent's own transcript (§8.4) and served to the
inspector on demand via `Command::InspectAgent` / `UiEvent::AgentActivity`
(§8.4), mirroring the existing `InspectSkill`/`SkillBody` request-reply
pair. The primary agent's own `spawn_agents`/`message_agent`/`list_agents`/
`end_agent` calls are ordinary tool calls, so they already flow through the
existing `ToolStarted`/`ToolFinished` events like any other tool — no
separate event is needed for those.

The 0.5.1 feature set adds `McpServerConnected{name, tools: Vec<String>}`
(FR-11, §5.6/§8.5) when a server's handshake completes at session start (or
reload), `McpServerFailed{name, reason}` when it does not (Design §8.10's
harness-voice notice), and `SessionExported{path}` (FR-12, §8.6) when an
in-session export command completes. An MCP tool call itself is an ordinary
`ToolStarted`/`ToolFinished` pair like any tool (§5.6) — ordinary rendering,
no per-call event. A user-attached image needs no new `UiEvent`: it rides
the outgoing `user_message` the TUI already constructs and sends (§4.1),
rendered from that message's own content blocks (Design §4.14) rather than
a separate notification.

Workspace trust (FR-1) is **not** a `UiEvent`: it is a pre-engine gate in the
binary (§6.7), resolved before the engine loop starts and before any project
file is read into a prompt, so it never crosses the engine↔frontend channel.
(v0.5 listed a `TrustRequest{path}` UiEvent; the pre-engine gate supersedes it —
withdrawn in v0.6, see §16.)

### 3.2 `TranscriptEvent` (durable, append-only JSONL)

One JSON object per line in
`.agents/sessions/<session-id>.jsonl`. Every event carries:

```json
{"v": 1, "ts": "2026-07-06T09:14:02.113Z", "type": "...", ...}
```

- `v` — schema version, present from day one (HC-7 longevity).
- Event types: `session_start` (model, provider, config provenance,
sandbox status), `trust_decision` (path + trusted — FR-1, §6.7; written on
**accept only**, since a decline starts no session and so has no transcript to
record into — the trust store's absence of the path is the durable record of a
non-grant), `user_message`, `assistant_message` (complete, not deltas;
carries a distinct `reasoning` field when the model produced one —
P-10, §4.7), `tool_call` (with the model's `explanation` when present —
T-9, §5.4), `tool_result` (with `truncated: bool` and, when
truncated, `full_output_ref` pointing to a sidecar file under
`.agents/sessions/<id>-outputs/`), `ask_user` (question + options and
the user's answer or decline — T-8), `loop_halt` (reason + the user's
chosen resolution — S-5), `permission_request`,
`permission_decision` (what was asked, what the user answered, what
actually ran — Requirements §6.6), `mode_change`, `model_switch` /
`effort_change` (C-6/P-9), `compaction`
(summary text + replaced range + a `trigger: manual | auto` field naming
whether the user or the FR-4 threshold initiated it — additive, older
readers warn-skip it, so no `SCHEMA_VERSION` bump; absent reads as
`manual`), `session_title`, `session_end`,
`abnormal_exit` (written by the supervisor when possible), and (0.4)
`task_list` (the model's current task list after each update — T-11, §5.2;
additive, older readers warn-skip, no `SCHEMA_VERSION` bump). Memory writes,
skill invocations, image reads, and web searches need no new transcript type:
each is already a `tool_call`/`tool_result` pair (HC-7). An image read records
the project-relative path in `tool_call` args; the image bytes are re-derived
from that file when building the provider request (§4.1), so the transcript
references the image without duplicating it, and a web search records its
query and the returned results (untrusted content, Design §4.10) in the
`tool_result` like any tool. The 0.4.1 feature set adds `completion_check` (one
per gate evaluation: check name, pass/fail, and the structured reason — S-6, §7)
and `completion_gate_halt` (the failing checks, the attempt count, and the
user's resolution, including `override: true` when the user finished over a red
gate — S-6, Design §8.7); both are additive, older readers warn-skip, no
`SCHEMA_VERSION` bump. A document read needs no new transcript type — it is a
`tool_call`/`tool_result` pair recording the project-relative path in args, the
document bytes re-derived from the file when building the provider request
(§4.1) exactly as an image read. The 0.5 feature set needs no new
transcript event type at the **primary session's** level either:
`spawn_agents`/`message_agent`/`list_agents`/`end_agent` are ordinary
`tool_call`/`tool_result` pairs like any other tool (HC-7); a subagent's own
turns, tool calls, and permission events instead go to that subagent's own
transcript file (§8.4), which reuses `TranscriptEvent`/`FileTranscript`
unchanged — it is a nested session in the audit-trail sense, not a new
schema.
- The transcript is ground truth; the in-context conversation is rebuilt
from it (resume) or maintained in parallel with it (live session).
Nothing ever rewrites a transcript line (HC-7, Requirements §8.2). The
adaptive window (FR-3, §7) and tool-result reduction (FR-2, §5.3) change
only what is *sent to the provider*; both leave every transcript line
intact, so the log stays the complete audit record.

### 3.2a Derived conversation-state cache (FR-5)

A sidecar `.agents/sessions/<session-id>-view.json` holds the session's
**derived** conversation state — the current in-context view (normalized
messages), token-accounting totals, the compaction summary/range history,
and the current window bound (§7) — so a resume restores that state directly
instead of replaying and re-tokenizing the whole JSONL (Requirements FR-5).

- **Derived, never authoritative.** The cache is a pure function of the
transcript; it is rewritten in place each time the view changes (unlike the
append-only transcript, HC-7 does not apply to it — it holds no fact the
transcript lacks). Losing, corrupting, or deleting it loses nothing.
- **Staleness guard.** The cache records the transcript length in bytes and
the byte offset it was built through; on resume, if the transcript has
grown past that offset, is shorter, or the file is unreadable/parse-fails,
the cache is discarded and resume falls back to full transcript replay
(§3.3). The cache is only trusted when it provably matches the log.
- Written on the same flush cadence as the transcript is not required — a
best-effort write after each turn suffices, because the transcript replay
fallback (§3.3) is always correct. A crash mid-write is a stale cache, which
the staleness guard already rejects.

### 3.3 Resume

`emberly resume` (and the offer-on-next-launch flow, Design §8.3) restores
the conversation view. Two paths, cache-first (Requirements FR-5):

1. **Fast path (cache).** If the derived cache (§3.2a) exists and its
  staleness guard matches the transcript, load the view, accounting, and
   window state from it directly — no per-line re-tokenization. This is the
   normal resume and is what makes resume cost proportional to the working
   view, not the full history (FR-5).
2. **Fallback (replay).** Otherwise replay the transcript: the conversation
  view is reconstructed by applying `compaction` events as view
   transformations, then re-deriving the window bound. Unknown event types
   (newer `v`) are surfaced as a warning, not a crash. Replay is always
   correct on its own; the cache is only ever an optimization over it.

Design §8.6 requires the fallback to be announced in one dimmed harness-voice
line (speech about the slow path); the fast path is silent.

## 4. Provider Layer (`emberly-providers`)

### 4.1 Trait

```rust
#[async_trait]
pub trait Provider: Send + Sync {
    fn id(&self) -> ProviderId;
    fn model_info(&self) -> ModelInfo; // context window, pricing, effort levels
    async fn stream_completion(
        &self,
        req: CompletionRequest,   // messages + tool schemas + reasoning effort
    ) -> Result<CompletionStream, ProviderError>;
    fn count_tokens(&self, text: &str) -> TokenEstimate; // may be approx
}
```

`ModelInfo` additionally declares a `vision: bool` capability (P-11): the
harness reads it to decide whether an image content block can be sent to the
active model and, when it cannot, to produce the structured
unsupported-capability result (§5.2, HC-6) instead of dropping the image.
`CompletionRequest`'s normalized messages carry a
`ContentBlock::Image{media_type, data}` variant alongside text and tool blocks
(`data` is base64 of the file bytes; P-11); each adapter maps it to the
provider's native shape (§4.2), and no wire type crosses the boundary (P-1).

`ModelInfo` likewise declares a `documents: bool` capability (P-12), read to
decide whether a `ContentBlock::Document{media_type, data}` (base64 of the file
bytes, alongside image and text blocks) can be sent to the active model —
producing the structured unsupported-capability result (§5.2, HC-6) when it
cannot, never dropping the document. Each adapter maps the block to the
provider's native document shape (§4.2); no wire type crosses the boundary
(P-1). The harness never parses the document — it forwards the bytes — so no
PDF-parsing dependency enters the tree (HC-2, §12).

**User-attached images reuse the same block, no new variant (FR-10).** A
`ContentBlock::Image` inside a `user_message` transcript event's own content
list is a user attachment (Design §4.14); the identical block inside a
`tool_result` event is `read_image`'s output (T-12, Design §4.8). The two
are told apart structurally, by which event carries the block, never by a
flag — the TUI (§9) renders the former as a chip on the sent message and the
latter as a tool-activity line purely from which event it is reading. The
same `image.max_bytes`/format checks (§5.2) and the same
unsupported-capability result (HC-6) on a non-`vision` model apply to both
paths identically, since both ultimately produce the same block.

`CompletionStream` yields normalized `StreamEvent`s: `TextDelta`,
`ReasoningDelta` (model thinking, distinct from the answer — P-10),
`ReasoningSignature{signature, redacted}` (emitted once when a reasoning
block closes — the opaque provider token to replay it verbatim on later
tool-use turns, §4.7; a provider without one never emits it),
`ToolCallStart/Delta/End`, `Usage`, `Done`, `Err`. No provider wire
type crosses this boundary (P-1). A provider that does not stream
reasoning simply never emits `ReasoningDelta`.

### 4.2 Implementations

- **Anthropic Messages API** — content blocks, `tool_use`/`tool_result`
mapping, SSE streaming. Image blocks map to an `image` content block with a
base64 `source` and media type (P-11); document blocks map to a `document`
content block with a base64 `source` of media type `application/pdf` (P-12).
- **OpenAI-compatible** — `tool_calls` mapping, SSE streaming; base URL
configurable, which transitively covers Ollama, vLLM, OpenRouter,
private deployments (P-2). Image blocks map to an `image_url` content part
with a `data:` URI (P-11). Document blocks map to the endpoint's file/document
input part where it supports one; an OpenAI-compatible endpoint that does not is
declared `documents:false` (below), so `read_document` returns the
unsupported-capability result rather than sending (P-12).

Both declare `vision` in `ModelInfo` per configured model (§8 pricing/model
config gains an optional `vision` flag; default `false`, so an image is never
sent to a model not declared vision-capable — P-11). Each configured model
likewise carries an optional `documents` flag (default `false`), so a document
is never sent to a model not declared document-capable (P-12).

Both are thin first-party clients on `reqwest` (default features off,
`rustls-tls`, `json`, `stream` on) — no vendor SDK crates (P-4). Two
live implementations before release (P-3).

### 4.3 Streaming, retries, failures

- SSE parsing first-party (it is ~100 lines; existing crates add little).
- Retry policy: idempotent-safe retries with exponential backoff +
jitter on connect errors, 429, 5xx (respecting `Retry-After`); max 3
attempts; every retry surfaced as a dimmed harness-voice line
(Design §6.1) and never silent. Mid-stream drop: the partial
assistant text is kept visible, marked interrupted, and the turn is
retried as a whole (partial turns are not stitched).
- Provider failure after retries → harness-world error, session remains
live and resumable (S-3).

### 4.4 Token & cost accounting (P-6)

- Prefer authoritative usage from provider responses where offered;
between responses, estimate with chars/4 (trigger-grade accuracy is
the requirement, not exactness).
- Cost: per-model pricing table in config
(`[pricing."model-id"] input=…, output=…` per MTok), estimate =
Σ(usage × price), always labeled "est." in UI (Design §3.1).

### 4.5 Provider profiles and endpoint configuration (P-8)

The two implementations in §4.2 are **wire-format parsers**; the endpoint
each talks to is data, not code. A provider is a config profile:

```toml
[providers.<name>]
adapter  = "anthropic" | "openai"            # which §4.2 wire-format parser
base_url = "https://…"
auth     = { scheme = "bearer" | "x-api-key" | "header", header = "…", key = "<ref>" }
models   = ["model-id", …]
# optional [providers.<name>.pricing."model-id"] input=…, output=…
```

- Adding a service that speaks a wire format we already parse is a new
profile and **zero code** (P-1's normalization paying rent, P-8). Only a
genuinely new wire format needs a new `Provider` impl.
- Auth `key` is a reference resolved from env / `keys.toml` (§8), never an
inline secret; the `header`/`scheme` set covers bearer tokens, API-key
headers, and arbitrary custom headers — enough for common endpoints.
- *Example (non-normative):* the Z.ai coding plan is a profile selecting
whichever wire format it speaks, its base URL, and its auth. The harness
carries no Z.ai-specific code.

### 4.6 Reasoning effort (P-9)

- `CompletionRequest` carries `effort: Option<Effort>`, a normalized enum
(`Low | Medium | High | Max`). Each adapter maps it to the provider's
native control — the `anthropic` adapter to a thinking-budget token count,
the `openai` adapter to the `reasoning_effort` field — or drops it when the
model has no such control (a no-op, never an error — P-9).
- `ModelInfo` declares the levels a model offers and its default; the UI
(Design §3.1) offers exactly those. Effort is engine state, changed by
`Command::SetEffort`, transcript-logged like mode (§6.6).

### 4.7 Reasoning trace (P-10)

- Adapters translate provider-native thinking parts into `ReasoningDelta`;
the engine records them in the `assistant_message` transcript event as a
**distinct field**, never concatenated into the answer text.
- Where a provider requires reasoning blocks (and their opaque signatures)
to be echoed back on subsequent tool-use turns for multi-turn thinking to
work, the adapter preserves the signature and replays it per that
provider's rule. The signature travels as a `ReasoningSignature` stream
event (§4.1) and is held in the normalized conversation as a
`ContentBlock::Reasoning{text, signature, redacted}` — an opaque token the
engine never interprets, so no wire *type* crosses the boundary (P-1). The
engine places that block ahead of the turn's text/tool-use so a provider
that demands the ordering (e.g. Anthropic extended thinking) accepts the
replay; a provider with no such requirement ignores the block. Resume does
not reconstruct reasoning blocks — the signature is not persisted, and only
the live turn needs replay.

## 5. Tool Layer (`emberly-tools`)

### 5.1 Trait

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;          // name, description, JSON schema
    async fn execute(&self, args: Value, ctx: &ToolCtx)
        -> ToolOutcome;                   // never Err at harness level
}
```

`ToolOutcome` is always a structured result for the model — success
payload or failure payload (HC-6). `ToolCtx` carries project root,
sandbox handle, truncation config, and the permission gate (tools
request execution *through* the gate; they cannot bypass it). The trait
is transport-agnostic so a future MCP adapter implements `Tool` by
proxying JSON-RPC (T-7); nothing else in the engine changes.

### 5.2 Built-ins


| Tool         | Notes                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| ------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `read_file`  | Path-normalized (symlinks resolved, `..` collapsed) then root-checked. Optional `start_line`/`end_line` (1-based, inclusive) read a numbered slice — the prompt-free alternative to `sed -n`. Output truncation per §7.                                                                                                                                                                                                                                                                                                                                                                                     |
| `write_file` | Refuses `.git/` (HC-5). Creates parent dirs inside root only.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| `edit_file`  | Exact string match-and-replace. Failure messages distinguish *no match* vs *N matches found* and, for no-match, include the closest fuzzy region as a hint (T-3 — model recovery quality depends on this).                                                                                                                                                                                                                                                                                                                                                                                                  |
| `bash`       | See §6. Timeout default 120s, configurable per-call by the model up to a config ceiling. Env is a scrubbed allowlist (PATH, HOME, LANG, TERM + config additions) — secrets in the user's env are not inherited by default.                                                                                                                                                                                                                                                                                                                                                                                  |
| `glob`       | Root-confined; ignores `.git/` and honors `.gitignore` by default.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `grep`       | First-party wrapper over the `grep-searcher`/`ignore` crates (the ripgrep libraries — pure Rust, same author). Root-confined.                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| `ask_user`   | Presents a question and optional discrete options to the user and blocks the agent loop until answered (T-8). Returns the typed answer, or a structured `{declined: true}` if dismissed, so the model can proceed or stop. Touches no filesystem or network — a pure engine↔frontend round-trip — so it bypasses the sandbox but still flows through the `Tool` trait; it is a `Command`/`UiEvent` pair under the hood (§3).                                                                                                                                                                                |
| `recall`     | Returns earlier conversation turns dropped from the working window (T-10, FR-3, §7). Args: a turn range (or the id referenced by a window-elision marker). Returns the engine's **normalized, reduced** messages for that range — never raw JSONL — so recall costs tokens proportional to what is recalled, not the transcript's raw size. Reads only the current session's own history from the transcript records the engine already holds; touches no filesystem or network, so like `ask_user` it bypasses the sandbox and is not permission-gated (§6), while still flowing through the `Tool` trait. |
| `todo`       | Sets/updates the model's task list (T-11). Args: the full list of `{text, status: pending|in_progress|done}` items — the model always sends the complete list, so the engine never merges partial edits. Engine state, emitted as `TaskListUpdated` (§3.1) and logged as `task_list` (§3.2); touches no filesystem/network, so like `recall`/`ask_user` it bypasses the sandbox and is **not permission-gated** (§6).                                                                                                                                                                                       |
| `read_image` | Reads an image file within the root (T-12). Path-normalized and root-checked exactly as `read_file`; governed by the same read rules (§6.2). Detects format + dimensions (header-only, `imagesize`), base64-encodes the bytes (`base64`), and appends a `ContentBlock::Image` to the turn (P-11, §4.1). Formats: PNG, JPEG, GIF (first frame), WebP. Rejects files over `image.max_bytes` (default 5 MiB) and, on a non-`vision` model (§4.2), returns the structured unsupported-capability result (HC-6) rather than sending it.                                                                          |
| `read_document` | Reads a document file within the root (T-16). Path-normalized and root-checked exactly as `read_file`; governed by the same read rules (§6.2). Sniffs the type by magic bytes (PDF: a leading `%PDF-` — a first-party byte-prefix check, no parser, HC-2) and rejects a non-PDF or a file over `document.max_bytes` (default 32 MiB). Base64-encodes the bytes (`base64`, already locked) and appends a `ContentBlock::Document` to the turn (P-12, §4.1); reports file size and format only, never a page count or extracted text (unparsed). On a non-`documents` model (§4.2) returns the structured unsupported-capability result (HC-6) rather than sending. |
| `memory`     | Writes/updates/removes/recalls durable memory entries (T-13, §8.1). Args are **content fields, never a path** (`op`, `scope`, `name`, `description?`, `type?`, `body?`); the engine derives the filename within the fixed scope directory and rejects `..`/absolute/separator in `name`. Harness-managed persistence (like the transcript/trust store), so it does not widen HC-4 (FR-6) and is not a sandboxed filesystem write; refreshes `MemoryStatus` (§3.1).                                                                                                                                          |
| `web_search` | Searches the web via the harness-owned backend (T-14, §5.5). Args: query (+ optional count ≤ `search.max_results`). A first-party `reqwest` call to the configured search endpoint; returns `{title, url, snippet}` results marked as **untrusted web content** (Design §4.10). **Permission-gated** (default `ask`, allowlistable — §6.1); it is a harness-process network egress, not a sandboxed child (§5.5).                                                                                                                                                                                           |
| `skill`      | Invokes a skill by name (T-15, §8.2). Args: skill name. Loads that skill's `SKILL.md` body (and lists its bundled resources) into the tool result for the model; metadata for all skills is already in context (§7). Reads only from the resolved, trust-gated skill folders (§6.7); running a skill's bundled script is a separate ordinary `bash` call under the full permission/sandbox model (§6) — the `skill` tool itself only reads instruction text, so it is not separately permission-gated.                                                                                                      |
| `scratch_write` | Writes a temporary file into the session's scratch space (T-17, §8.3). Args: `{name, content}` — **content fields, never a path**, exactly the T-13 pattern: the engine derives the real path from `name` within the fixed per-session scratch directory and rejects `..`, absolute paths, and separators in `name`. Creates or replaces the named file. Harness-managed persistence, not an agent filesystem write (does not widen HC-4), so it is not sandboxed and not permission-gated. |


### 5.3 Tool-result reduction at ingestion (Requirements §8.1, FR-2)

Two deterministic, no-model-call layers run when a tool result is appended,
salient-reduction first, then the size backstop. The full untruncated output
is always written to the sidecar and referenced by `full_output_ref`, so both
layers are recoverable via `/view` (Design §4.3); neither ever touches the
transcript's recorded result (HC-7).

- **Salient reduction (FR-2).** Where a tool's output has a known salient
shape, a per-tool reducer keeps the meat and elides the noise *by meaning*,
not by position. The reducer set (initial; tune with use — Requirements
§13):
  - `bash`: keep the exit status, all of stderr, and the head+tail of stdout;
  drop repetitive progress/percentage lines (deterministic
  collapse of runs of near-identical lines). *Example (non-normative):* a
  500-line `cargo build` collapses to its warnings/errors and final
  summary.
  - `grep`: keep match lines and their file:line headers; drop nothing that
  is a hit (hits are the point).
  - `read_file`: no semantic reducer — a file read is already bounded by the
  optional `start_line`/`end_line` and the size backstop; reducing by
  meaning would risk hiding code the model asked for.
  - `glob`: keep the path list; if it exceeds the count backstop, keep head
  +tail with the elision marker.
  Reducers are pure functions `(&ToolSpec, &raw_output) -> reduced_output`
  registered per tool; a tool with no registered reducer falls straight
  through to the size backstop. `truncate.reduce = bool` (default `true`,
  §8) disables the reduction layer for users who want raw results.
- **Size backstop (Requirements §8.1).** After reduction, if output still
exceeds `truncate.max_lines` (default 400) or `truncate.max_bytes`
(default 64 KiB), keep head (default 150 lines) + tail (default 100 lines)
and insert `[... N lines elided — /view to open full output ...]`. This is
the blind head/tail defense that always applies, reducer or not.

The elision/reduction marker names what was withheld and offers `/view`; the
`/view` handoff (Design §4.3) opens the sidecar read-only in
`$VISUAL`/`$EDITOR`. A reducer must never drop information the model cannot
then recover from the sidecar — the sidecar holds the complete output, so
"recoverable" is guaranteed by construction (Requirements FR-2).

### 5.4 Tool-call explanation (T-9)

- An optional `explanation` string property is injected into every tool's
`input_schema` at the single `ToolSpec → provider` serialization point;
the system prompt instructs the model to fill it briefly and only for
calls whose intent is not self-evident (bump the prompt version). No tool
declares `deny_unknown_fields`, so the field rides in `args` and tools
ignore it — and because the transcript already records full `args`, the
explanation persists with **no transcript schema-version bump**.
- The UI renders it per Design §4.5 (`explanation: Option<String>` on the
`ToolStarted` UiEvent, §3.1).
- `ui.tool_explanations = true|false` (§8): when off, the schema property is
omitted entirely so the model is never prompted for it and no tokens are
spent (Requirements T-9).

### 5.5 Web-search backend (T-14)

Search is **harness-owned and provider-agnostic** (Requirements §1, T-14): the
`web_search` tool talks to a configured search service through a thin
first-party `reqwest` client, exactly mirroring the provider-profile pattern of
§4.5 so a new search service is config, not code.

```toml
[search]
enabled  = true
adapter  = "brave" | "tavily" | "searxng" | "json"   # response-shape parser
endpoint = "https://…"
auth     = { scheme = "bearer" | "header" | "query", header = "…", key = "<ref>" }
max_results = 5            # cap sent to the model (Design §4.10)
```

- `**adapter` is a response-shape parser**, the search analogue of §4.2's
wire-format parsers: it normalizes a service's JSON into
`Vec<{title, url, snippet}>`. `json` is a generic JSON-path-configurable
parser for services not otherwise covered; a genuinely new shape is a new
parser, everything else is a profile. `key` is a reference resolved from env
/ `keys.toml` (§8), never inline.
- **Network egress is from the harness process, not a sandboxed child.** The
call is first-party HTTP like a provider call (§2), so §6.2/§6.3 child
confinement does not apply; the only network reached is the configured
`endpoint`. This is why search is governed by the *permission layer* (below),
which is the right tool for a harness-initiated egress with cost and an
injection surface.
- **Permission-gated** (Requirements T-14): a built-in rule `web_search → ask`
(§6.1), allowlistable to `allow` per-session or per-project like a bash
command. When `search.enabled = false` the tool is not registered at all.
- **Results are untrusted content.** The tool result is tagged so the TUI
labels it as fetched web content (Design §4.10); the engine never interprets
a result as an instruction. Result count is capped by `max_results` and long
snippets pass through the §5.3 size backstop, so search cannot flood context.

### 5.6 MCP client (FR-11)

**No vendor SDK crate (P-4's own rule, extended here).** MCP's stdio
transport is newline-delimited JSON-RPC 2.0 over a child process's
stdin/stdout — exactly the shape `tokio::process::Command` (already a direct
dependency, §2) plus `tokio::io::AsyncBufReadExt::lines()` and `serde_json`
(already locked) already handle with no framing complexity beyond
`Content` \n `Content` \n. A first-party `McpClient` (`emberly-core`,
alongside `engine/subagents.rs`) is therefore the default and, per §12,
needs **no new dependency**; a remote transport (SSE/HTTP), if ever added,
is a second `McpTransport` impl behind the same trait, not a rewrite.

```rust
#[async_trait]
trait McpTransport: Send + Sync {
    async fn call(&self, method: &str, params: Value) -> Result<Value, McpError>;
}
```

- **Connection lifecycle.** At session start (and on `/reload`, C-5), the
engine spawns one `McpTransport` per enabled `[mcp.servers.<name>]` profile
(§8.5) whose trust gate (below) passes, sends the MCP `initialize` +
`tools/list` handshake, and builds one `Tool` impl per discovered tool by
implementing §5.1's trait as a thin proxy that serializes `args` into an
MCP `tools/call` request and maps the JSON-RPC response/error into a
`ToolOutcome` (HC-6 — a transport error or a tool-side error both become a
structured failure, never a panic or a bare harness error). This is T-7's
door, opened for real: the tool trait's transport-agnostic contract means
no engine change was needed, only this one new `Tool` implementation.
- **Namespacing (Requirements §13 resolved here: initial convention).** Each
discovered tool registers as `mcp__<server>__<tool>`, so the server that
owns a tool is legible directly from its name — the same fact the
permission-prompt provenance line (Design §5.3) and tool-activity line
(Design §4.15) read to name the server. A name collision within one
server's own tool list is that server's own bug, surfaced as a structured
connection-time failure (HC-6) naming both tools; collision with a
built-in tool name cannot occur by construction, since the namespace always
carries the `mcp__` prefix no built-in uses.
- **Registration through the existing `ToolRegistry`, not a parallel one.**
Discovered tools are inserted into the same registry every built-in tool
lives in (§5.1) — a subagent's filtered registry (§8.4) simply may or may
not include them like any other name; no separate "external tools" registry
or dispatch path exists.
- **Permission gating is the ordinary rule engine, no proxy needed at this
layer.** Unlike a subagent's calls (§8.4), an MCP tool call is made by the
*root* engine itself (or a subagent's, proxied exactly as any of its other
tool calls already are, §8.4) — so it flows through `ToolCtx`'s existing
permission gate with no new gate type. The rendering (Design §5.3) reads the
server name from the tool's own namespaced identifier, needing no new
payload field.
- **Untrusted-content tagging.** An MCP tool result is tagged exactly as
`web_search`'s is (§5.5) so the TUI labels it as external, untrusted
content (Design §4.15) — one shared tagging mechanism, two tools using it.
- **Workspace-trust gate on project-declared servers (extends §6.7).** A
server declared in project-tier config (§8.5) is checked against the
session's trust decision *before* the engine spawns its transport or dials
its endpoint — mirroring the project-skill check (§8.2) exactly, including
its "neither cataloged nor invocable" outcome on an untrusted root. A
user-global server is never trust-gated (it is not project-resident).

## 6. Sandbox & Permissions (`emberly-sandbox`)

Two layers with different jobs (Requirements §6.1). The rule layer
decides *when to ask*; the sandbox layer decides *what is possible*.

### 6.1 Rule engine

- Rules are (tool, matcher) → allow | ask | deny, evaluated most-specific
first. Bash matchers are prefix matchers on the command string —
explicitly convenience-tier (Requirements §6.5); no shell parsing.
- Sources, in precedence order: built-in defaults → global config →
project `.agents/permissions.toml` → session grants (memory only).
- Session grants come from "allow for this session" at the prompt;
"always allow in this project" writes to `permissions.toml` (with the
written line shown to the user).
- Every request/decision/execution is a transcript event (HC-7).
- Default bash allowlist (initial; user-extensible): `ls`, `cat`,
`head`, `tail`, `wc`, `grep`, `rg`, `find`, `pwd`, `echo`, `which`,
`git status`, `git diff`, `git log`, `git show`, `git branch`,
`cargo check`, `cargo tree`, `cargo metadata`. (Resolves the
Requirements §13 open item; final list is a living config default.)
Note: `sed` is intentionally absent — a prefix rule would match the
destructive `sed -i`/`sed >` forms too, and §6.5 forbids flag-parsing
as a matching mechanism. Use `read_file` with `start_line`/`end_line`
for a prompt-free ranged read instead.
- Non-bash tool defaults: `read`/`glob`/`grep`/`read_image`/`read_document`
inside the root → `allow`; `write`/`edit` inside the root → `ask` (unchanged);
`web_search` →
`ask`, allowlistable (§5.5, T-14). The engine-only tools (`todo`, `recall`,
`ask_user`, `skill`) take no rule — they touch no filesystem/network and are
not permission-gated (§5.2).

### 6.2 OS confinement — Linux (Landlock)

- Via the `landlock` crate (pure-Rust syscall wrapper; no libc/C
dependency for the sandbox path).
- At startup, probe the kernel's Landlock ABI version (cheap syscall).
Result is engine state, emitted as `SandboxStatus` and written into
`session_start` (visible per Design §3.1/§8.2).
- Ruleset applied to every spawned child between fork and exec
(best-effort ABI: request the highest supported feature set, record
what was actually granted):
  - Project root: read + write.
  - `.git/` under the root: **no write** — except when the invoked
  command resolves to the genuine git binary (§6.4), which gets a
  ruleset including `.git/` write (HC-5).
  - System paths (`/usr`, `/lib`, `/etc`, toolchain dirs): read + exec.
  - Everything else: no access. A confined child gets **no outside-root
  write grant** — there is no per-invocation escape hatch, and nothing
  could populate one: `bash` cannot know which paths a command will
  touch (§6.5 rules out parsing commands for paths), so it declares
  none. The tools that may step outside the root on an explicit,
  once-only approval (HC-4 ask) — `read_file`, `write_file`,
  `edit_file`, `read_document`, `read_image` — run in-process and never
  enter a sandbox profile. This is stricter than HC-4 requires and
  compatible with it: HC-4 forbids acting without permission, not being
  unable to act with it. A per-invocation grant was specified through
  v0.11 and is withdrawn here unbuilt; widening confined children to
  outside-root writes is an owner decision, held.
- The harness process itself is never confined — only children. A weird
host configuration therefore cannot destabilize Emberly itself; it
can only weaken the fence around spawned commands.

### 6.3 OS confinement — macOS (Seatbelt)

- `sandbox-exec`-style profile applied to children via the system's
sandbox APIs; same policy shape as §6.2. This is the one place HC-2's
OS-interface exemption applies: Apple's system libraries are the only
road to the kernel on macOS. No third-party C is introduced.

### 6.4 Genuine-git resolution (HC-5)

A command qualifies for the `.git/`-writable profile only if: the first
token is `git` (no alias — checked against a plain token, not shell
alias expansion, which never reaches us anyway since we exec directly),
AND resolving it through PATH yields a canonical binary path that is
outside the project root and matches the git binary recorded at session
start (`which git`, canonicalized). A `./git` in the repo, a PATH
shadowing inside the project, or an `sh -c "git ..."` wrapper does NOT
qualify (the last runs confined; plain `git` subprocesses it spawns are
unconfined-by-inheritance only if git itself is the exec target —
wrapped invocations simply run without `.git/` write access, which is
safe-closed).

### 6.5 Degradation policy (sandbox unavailable)

Design principle, per owner decision: **make the risk clear, tighten
the convenience away, stay usable.** Never refuse to run by default,
never crash, never pretend.

When the Landlock probe fails (kernel without
`CONFIG_SECURITY_LANDLOCK`, LSM not enabled at boot, pre-5.13 kernel)
or macOS profile application fails:

1. **Notify explicitly.** Session-start header and sidebar show
  `sandbox: unavailable (<reason>)` in warning styling; a one-time
   plain-language notice explains what this means. Status is also a
   transcript event.
2. **Tighten convenience.** Auto-accept-edits and auto mode become
  unavailable (the mode selector shows why). The default bash
   allowlist is suspended — every bash command asks. Per-action
   prompting works exactly as normal; the app remains fully usable.
3. **Enforce the hard lines at the rule layer, honestly labeled.**
  Tool-level path checks (root confinement, `.git/` refusal) continue —
   they live in `emberly-tools` and never depended on the kernel — and
   the UI labels protection as "policy-level" rather than "kernel-level".
4. **Opt-in strictness.** `sandbox.require = true` in config makes
  Emberly refuse to start a session without kernel confinement, for
   environments (e.g. sensitive legal data) where degraded operation is
   unacceptable. Default is `false`.

Partial degradation (old Landlock ABI missing some capability, e.g.
truncate control pre-ABI-v3) is reported the same way, with the
granted-vs-requested delta in the status detail; convenience modes stay
available if the core write/`.git/` confinement was granted.

### 6.6 Modes (Requirements §6.4)

Mode is engine state, changed by `Command::SetMode`, transcript-logged.
Auto modes are constructible only when `SandboxStatus` is fully or
acceptably-partially confined (§6.5); the type system enforces this
(mode transitions take the sandbox status as a parameter).

### 6.7 Workspace trust (FR-1)

Model mirrors the established harnesses (Claude Code's user-global
`~/.claude.json` keyed by canonical path; VS Code Workspace Trust's
subtree trust and explicit manage surface): **user-global, keyed by
canonical path, subtree-trusted**, with an optional pre-trust allowlist.

- **Store:** `~/.config/emberly/trust.toml` (XDG), `0600`, **global only** —
never a project key, so a repository cannot pre-declare itself trusted
(Requirements FR-1). Entries are canonical paths with an accepted flag and
timestamp.
- **Membership (subtree trust):** a project root is trusted if it *or any
ancestor directory* is in the store — trusting a folder trusts its
subtree, as VS Code and Claude Code do, so a trusted repo does not
re-prompt per subdirectory.
- **Pre-trust allowlist:** `trust.trusted_dirs = [..]` in **global** config
auto-trusts matching roots at startup without a prompt (adopts the
`trustedDirectories` pattern requested for Claude Code). Project config
cannot contribute here (FR-1).
- **Checked in the binary at startup** (§10), before the engine begins the
loop *and before any project file is read into a prompt*: canonicalize the
root, test membership (store ∪ allowlist). A miss raises the trust gate — a
plain pre-engine prompt printed before either frontend takes the terminal, so
it reads the same in rich and plain mode and needs no engine event (Design
§8.4); a non-interactive launch declines cleanly. On decline the
process exits cleanly with no session started; on accept the canonical
path is written to the store and the session proceeds. The decision is a
transcript event (`trust_decision`, §3.2).
- **Revocation:** `emberly trust list` / `emberly trust revoke <path>` —
an explicit surface, so revoking never means hand-editing a file (the
documented pain point in the harnesses we are following).
- This is a **session-start gate, not containment** (Requirements FR-1
honesty clause): it lives outside `emberly-sandbox`, changes no ruleset,
and never widens HC-4/HC-5 or relaxes a prompt. It decides *whether* the
agent runs here, never *what* it may do.
- **Trust gates project memory and project skills (0.4).** Because trust is
checked before any project text reaches the model, project-scope memory
(FR-6, §8.1) and project-scope skills (FR-7, §8.2) are loaded and
surfaced **only** after the root is trusted; an untrusted (e.g.
non-interactive-declined) root loads neither. User-global memory and skills
are unaffected — they are the user's own, not the project's. This realizes
Design §4.9/§8.4: untrusted-folder memory/skills are silently absent, not
half-loaded.

## 7. Context Management (`emberly-core`)

- Budget: `window − reserved_output` (default reserve 8k tokens or the
model's max-output, whichever is smaller). `ContextUsage` emitted on
every accounting change (Design status line + sidebar).
- **Pinned, never compacted:** system prompt, project instructions
(AGENTS.md/CLAUDE.md per C-1), original task statement (first user
message of the session, tagged in the transcript). The 0.4 always-in-context
material joins the pinned set: the **memory index** (FR-6, §8.1), the
**skill catalog** (name+description per available skill, FR-7, §8.2),
and the **current task list** (T-11) — each is small, load-bearing for the
next step, and must survive compaction and windowing. This resolves the
Requirements §13 open item on task-list pinning: pinned by default
(`context.pin_task_list = true`, §8). Memory entry *bodies* and skill
*bodies* are NOT pinned — they load on demand via the `todo`/`skill`/memory
tools and ride the normal window (progressive disclosure keeps the standing
cost to the index/catalog only, in the spirit of the §8 context economy).
- `**/compact`** (manual): valid only at clean boundaries (every
`tool_use` has its `tool_result`; if invoked mid-run, queued until
the boundary). Summarization request uses the *current provider* with
a purpose-built prompt (from the prompts directory, overridable per
C-1): original task, decisions, files modified & how, current state,
next steps. View rebuilt as [system][pinned][summary-as-user-msg]
[last N turns verbatim] (N default 6, config
`context.keep_recent_turns`). Compaction is a transcript event; the
JSONL log is untouched (Requirements §8.3).
- **Failure fallback:** if the summarization call fails, hard-truncate
oldest non-pinned turns to 50% budget with a visible warning — a full
context never produces a stuck session.
- **Automatic compaction (FR-4).** One threshold check in the accounting
path: when `ContextUsage.pct` crosses `context.auto_compact_threshold`
(default `0.85`) the engine schedules a compaction at the next clean
boundary (same queueing as manual, same pinned/boundary/fallback rules) —
the *only* difference from `/compact` is the trigger and the
`compaction.trigger = auto` transcript field (§3.2). On by default
(`context.auto_compact = true`); set `false` to compact manually only.
After an auto-compaction fires, it will not re-fire until usage has fallen
and re-crossed the threshold (the compaction itself drops usage well below
it, so no thrash). Surfaced in the harness voice per Design §8.6. This
enables what v0.5 §7 left designed-for; the manual path is unchanged.
- **Adaptive context window (FR-3).** Independently of compaction, the engine
sends only a working window of recent turns to the provider. The window is
the last `context.window_turns` (default `40`) non-pinned turns; pinned
content (system prompt, project instructions, original task) and any active
compaction summary are always sent and never counted against the window.
Turns older than the window are replaced *in the sent context only* by a
single synthetic marker — `[N earlier turns elided from context — still in the session transcript]` — while remaining verbatim in the transcript and,
for the user, in TUI scrollback (Design §8.6).
  - **Model-driven re-read via `recall` (Requirements T-10/FR-3).** The
  window never guesses; when the model needs a dropped turn it calls the
  `recall` built-in (§5.2). The elision marker carries the transcript range
  it stands for; `recall` takes that range and returns those turns in the
  engine's normalized, reduced form (§5.3), served from the transcript
  records the engine already holds — a pure engine round-trip, no
  filesystem/network, not permission-gated (§6), like `ask_user`. It is
  deliberately **not** `read_file` over the raw JSONL: that would re-inflate
  the elided noise in verbose transcript form and defeat the window's token
  economy (Requirements T-10). A generous `window_turns` default keeps the
  drop — and thus the recall — rare.
  - Windowing composes with compaction: compaction summarizes the middle and
  is pinned into the sent context; windowing bounds how many *post-summary*
  turns ride verbatim. Both are cost economies; neither rewrites the log
  (HC-7). The window bound is part of the derived cache (§3.2a).
- **Loop-breaking guardrail (S-5).** The engine keeps a rolling signature
of recent steps: for each turn, the multiset of `(tool_name, normalized-args)` tuples plus a hash of the resulting `tool_result`
content and the set of files modified. No-progress heuristic (initial;
tune with use): trip when the last `loop.repeat_window` turns (default 3)
repeat tool-call signatures **and** produce no new modified files and no
new distinct tool-result hashes — i.e. the loop is re-treading, not
advancing. On trip: stop issuing provider calls, emit `LoopHalted{reason}`
(UiEvent + transcript event, §3), and await a user `Command` (resume /
stop / steer, Design §8.5). Config `[loop] enabled, repeat_window, max_no_progress_turns`. The guardrail never trips while files change or
tool results differ (genuine progress); it is a heuristic (Requirements
S-5), and the guarantee is termination-into-a-decision, not perfect
classification.
- **Completion gate (S-6).** Registered completion checks gate the loop's claim
of *done* — the model-driven analog to the guardrail above: S-5 stops a loop
that re-treads, S-6 stops one that lands early. Checks are registered from config
(the shipped path) and, by the same internal registration hook, by a frontend or
a tool (Requirements S-6); the engine holds a `Vec<CompletionCheck>` populated at
startup. A config check is a command with an expected exit status:

  ```toml
  [[completion.check]]
  name        = "tests"
  command     = "cargo test"
  expect_exit = 0        # pass iff the command exits with this status
  ```

  - **Evaluation timing.** The gate evaluates on a *completion attempt* — the
  agent loop reaching a natural stop (an assistant turn with no tool calls). With
  no checks registered the gate is inert and the loop ends exactly as today; with
  checks, each runs and any failure re-opens the loop.
  - **Failure re-opens the loop (HC-6).** A failing check's name and structured
  reason (exit status + the §5.3-reduced tail of its output) are appended as a
  tool-result-shaped message the model reads and reacts to (agent-world, Design
  §8.7), and the loop continues; a passing gate lets the loop terminate.
  - **Checks run under §6, un-prompted.** A command check executes through the
  sandbox exactly as `bash` (§6.2/§6.3 confinement, the `S-4` timeout) — it has
  **no privileged path around the safety model** (Requirements S-6 honesty
  clause). It is *not* re-prompted per evaluation: being registered in
  trust-gated config is the authorization, exactly as a project-config allowlist
  entry is (§6.1). Containment still applies; only the ask is waived, and only
  because the user authored the check.
  - **Bounded attempts → halt.** After `completion.max_attempts` failed
  completion attempts (default `3`; initial, tune with use) the engine stops
  issuing provider calls, emits `CompletionGateHalted{failing, attempts}` (UiEvent
  + `completion_gate_halt` transcript event, §3), and awaits a
  `Command::ResolveCompletionGate` — `resume` (try again), `steer(text)` (hand
  guidance to the model), `stop`, or `finish` (**override**: end the task as done
  over a still-failing gate, recorded with `override: true` — the gate binds the
  model's claim of done, never the user's authority, Requirements S-6). This is
  the same termination-into-a-decision guarantee as S-5 and prevents an S-6/S-5
  standoff: a model that cannot satisfy a check cannot spin forever.
  - Every evaluation is a `completion_check` transcript event (name, pass/fail,
  reason — HC-7). Config `[completion] enabled` (default `true`, but inert
  without registered checks), `max_attempts`.

## 8. Configuration & Prompts

- Settings: TOML. Global `~/.config/emberly/config.toml` (XDG), project
`.agents/config.toml`; project wins per key. Env `EMBERLY_`* overrides
for CI/scripting.
- Prompts: markdown files, per-file override in `.agents/prompts/`
(C-1), with per-model-family variants resolved as
`<name>.<family>.md` falling back to `<name>.md` (P-7).
- `emberly init` materializes active defaults into `.agents/`
(C-2; UX per Design §8.1). `emberly config show` prints every active
piece with its provenance tier (C-3).
- Project instructions: `AGENTS.md` native; `CLAUDE.md` read when
`AGENTS.md` absent; both present → `AGENTS.md` wins with a notice
(C-1).
- Secrets (API keys): env vars first (`ANTHROPIC_API_KEY`, etc.), or
keys file `~/.config/emberly/keys.toml` with `0600` perms enforced
(warn+refuse on group/world-readable). Never in project config, never
in transcripts (requests are logged with auth headers redacted).
- **In-app editing (C-5).** The TUI edits `.agents/config.toml` and prompt
files via the editable overlay or `$EDITOR` handoff (Design §4.6). Writes
target the project tier (Requirements C-1); the value's provenance
(`config show`, C-3) is shown before the edit. Prompts and most config
live-reload on save via a per-field comparison in the engine's
`reload_config` (system/compact prompt, provider profiles and their
content, loop/completion/truncate/context tunables, tool explanations,
image/document limits, memory/skills config, the tool registry, and
permission rules, preserving in-session grants); `sandbox.require` is the
one setting that cannot be re-applied to an already-running process and
is named as restart-only at reload time rather than silently dropped.
- **In-app model/provider/effort switching (C-6).** `Command::SwitchModel {profile}` and `Command::SetEffort{level}` swap the active `Provider` /
effort for subsequent turns; both are transcript events and never rewrite
prior turns. The picker (Design §3.1) lists the `[providers.*]` profiles
(§4.5) and the active model's effort levels (§4.6).
- **Guided provider/model setup (C-7).** The wizard's four answers
(adapter, endpoint, model id, key) build a `ProfileFile` through the same
construction `build_profile`/`resolve_auth` (§4.5) already validate — a
wizard-created profile cannot reach a state raw editing wouldn't also
allow. Writing `.agents/config.toml` is parse → insert/replace the
`[providers.<name>]` table → reserialize with the existing `toml` crate
(no new dependency, §12); comments/formatting elsewhere in the file are
not guaranteed to survive that reserialization. The key is
read-merge-written into `~/.config/emberly/keys.toml` (creating it at
`0600` if absent, same enforcement as above) — one entry added or
overwritten, the rest of the table untouched. On success, the same
`Command::ReloadConfig` (C-5) fires — the wizard has no bespoke
apply/reload path of its own, and does not auto-switch the active session
(consistent with C-6's "reload never auto-switches" policy); the user
picks the new profile from the same picker afterward. No new transcript
event — matches the existing (untranscripted) `/config` reload behavior.
- **New config keys** (initial; tune with use): `[providers.<name>]`
(§4.5); per-model effort default (§4.6); `reasoning = collapsed | expanded | hidden` view default, **default `collapsed`** (Design §4.4);
`ui.tool_explanations = bool`, **default `true`** (§5.4); `[loop] enabled, repeat_window, max_no_progress_turns` (§7).
- **New config keys, 0.3 context economy** (initial; tune with use):
`truncate.reduce = bool` (**default `true`**) toggles salient tool-result
reduction (§5.3, FR-2); `context.window_turns` (**default `40`**) bounds the
adaptive window (§7, FR-3); `context.auto_compact = bool` (**default
`true`**) and `context.auto_compact_threshold` (**default `0.85`**) drive
automatic compaction (§7, FR-4). The existing `context.keep_recent_turns`
(default `6`) still sets the verbatim tail kept by a compaction. The derived
conversation-state cache (§3.2a, FR-5) has no key — it is always written and
always guarded by the staleness check, so it needs no opt-in.
- **New config keys, 0.4 feature set** (initial; tune with use):
`context.pin_task_list = bool` (**default `true`**) pins the task list across
compaction/windowing (§7, T-11); `image.max_bytes` (**default `5 MiB`**) caps
a `read_image` (§5.2, T-12) and the optional per-model `vision = bool`
(**default `false`**) gates image sends (§4.2, P-11); `[search]` — `enabled`
(**default `true`**), `adapter`, `endpoint`, `auth`, `max_results` (**default
`5`**) — configures the web-search backend (§5.5, T-14); `memory.enabled`
(**default `true`**) and `memory.max_index_entries` (§8.1, FR-6);
`skills.enabled` (**default `true`**) (§8.2, FR-7); `ui.mouse = bool`
(**default `true`**) enables pointer capture in the rich TUI (§9, Design
§3.4). Memory and skill *store locations* are fixed (§8.1/§8.2), not config.
- **New config keys, 0.4.1 feature set** (initial; tune with use):
`[[completion.check]]` entries (`name`, `command`, `expect_exit`) register gate
checks, and `[completion]` — `enabled` (**default `true`**, inert without
registered checks) and `max_attempts` (**default `3`**) — drive the completion
gate (§7, S-6); `document.max_bytes` (**default `32 MiB`**) caps a
`read_document` (§5.2, T-16) and the optional per-model `documents = bool`
(**default `false`**) gates document sends (§4.2, P-12).
- **Trust:** store at `~/.config/emberly/trust.toml`, `0600`, global only;
optional `trust.trusted_dirs` pre-trust allowlist in global config
(§6.7) — neither is ever a project key (Requirements FR-1).
- **New config keys, 0.5.1 feature set** (initial; tune with use):
`[mcp] enabled` (**default `true`**) and `[mcp.servers.<name>]` — `transport`
(**initial: `"stdio"` only**), `command`/`args` (stdio) — (§5.6/§8.5, FR-11,
C-8); `image.max_attachments` (**default `4`**) bounds attachments on one
prompt (§8's image caps, extended to FR-10) — reuses the existing
`image.max_bytes`/format checks (§5.2) per-attachment, no separate cap
needed there. Session export (FR-12, §8.6) introduces no config key — its
output path is a command argument, not a setting.

### 8.1 Persistent memory (FR-6)

- **Layout.** Two scopes, each a directory of one-fact-per-file entries plus an
index:
  - **User-global:** `~/.config/emberly/memory/` (XDG) — `MEMORY.md` index +
  `<slug>.md` entry files.
  - **Project:** `.agents/memory/` — same shape, inside the project root.
  Each entry file is TOML frontmatter (`name`, `description`, `type`) + a
  markdown body (the fact). TOML frontmatter reuses the existing `toml` crate —
  no YAML dependency (deliberate: keeps the dependency tree pure-Rust and small,
  HC-2). The `MEMORY.md` index is one line per entry (`name` — `description`),
  the always-loaded catalog (§7 pinned).
- **The memory tool is schema-constrained (Requirements FR-6 / HC-4).** `memory`
takes `{op: write|update|remove|recall, scope: user|project, name, description?, type?, body?}` — **content fields, never a path.** The engine
derives the filename from `name` (slugified) within the fixed scope directory;
the model cannot escape it (`..`, absolute paths, and separators in `name` are
rejected). The engine performs the write. This is why it is harness-managed
persistence, not an agent filesystem write, and does not widen HC-4 (FR-6
honesty clause): the model chooses *what to remember*, never *where a file
lands*. `recall` returns a specific entry body; `write`/`update`/`remove`
mutate the store and refresh the index event (`MemoryStatus`, §3.1).
- **Loading.** At session start the engine loads both indexes into pinned
context (§7). **Project memory loads only if the root is trusted** (§6.7);
user-global memory always loads. Entry bodies are fetched on demand by
`recall` (progressive disclosure), so a large memory costs only its index
until consulted.
- **User-inspectable (FR-6).** Memory files are plain text the user reads,
edits, and deletes directly, and the sidebar Memory inspector (Design §4.9)
edits them via the Design §4.6 in-app path. Nothing about memory is hidden
state.
- **Config:** `memory.enabled` (default `true`); `memory.max_index_entries`
(soft cap that warns when the index itself grows large enough to want the §7
economy — Requirements §13 open item).

### 8.2 Skills (FR-7)

- **Layout.** A skill is a folder `<name>/` containing `SKILL.md` (TOML
frontmatter `name`, `description` + markdown instruction body) and optional
bundled resources/scripts. Two scopes resolved at startup:
  - **User-global:** `~/.config/emberly/skills/<name>/`.
  - **Project:** `.agents/skills/<name>/` (loaded only if the root is trusted,
  §6.7).
- **Discovery & precedence.** The engine scans both locations and builds the
**skill catalog** — `SkillMeta{name, description, origin}` per skill — loaded
into pinned context (§7) so the model always knows what it can invoke. On a
name collision **project overrides user-global** (a project ships a tuned
variant), mirroring config precedence (project wins per key); the shadow is
reported via `emberly config show` (Requirements §13 open item on precedence
resolved here — project wins, surfaced).
- **Invocation.** The `skill` tool (§5.2) reads the named skill's `SKILL.md`
body into the tool result — progressive disclosure: only metadata is standing
context, the body loads on invoke. Bundled resources are listed with their
paths so the model can `read_file`/run them.
- **Scripts run under the normal safety model (FR-7 honesty clause).** A skill's
script is executed only via an ordinary `bash` tool call, fully permission-
and sandbox-gated (§6) — the `skill` tool never executes anything, it only
surfaces instructions. A project skill from an untrusted root is neither
cataloged nor invocable (§6.7).
- **Config:** `skills.enabled` (default `true`).

### 8.3 Session scratch space (FR-8)

- **Layout.** `.agents/scratch/<session-id>/` — created lazily on the
session's first `scratch_write` call, not eagerly at session start, so a
session that never uses it leaves no directory behind. Added to `.gitignore`
alongside `.agents/sessions/` — never committed, and this is also why
`glob`/`grep` (§5.2) do not surface scratch content by default: both honor
`.gitignore`.
- **The tool is schema-constrained (Requirements FR-8/T-17), the T-13
pattern.** `scratch_write` takes `{name, content}` — content fields, never a
path. The engine derives the real path from `name` within the session's
fixed scratch directory and rejects `..`, absolute paths, and separators in
`name`, exactly as `memory` (§8.1). Creates or replaces the file. Because the
model never supplies a destination, the write does not widen HC-4 and is not
sandboxed or permission-gated (§6.1) — the call and its result are still
ordinary `tool_call`/`tool_result` transcript events (HC-7), like any other
tool.
- **Lifecycle.** No automatic cleanup — a scratch directory persists exactly
like its session's transcript, so a resumed session finds its own files in
place. `emberly clean [<session-id>]` (§10, FR-8) is the reclaim path: with
no argument, it reports the total size across every session's scratch
directory, asks a plain `y`/`n` confirmation (Design §8.8), then deletes all
of them; given a session id, it scopes to that one session only. A target
with nothing to clean says so and exits zero.
- **Reading scratch content back** uses the existing `read_file` (T-1) once
the model has the name it used; there is no dedicated read/list tool
(Requirements §5, T-17) — scope kept to the write path that motivated this.

### 8.4 Multi-agent subsystem (FR-9)

The central design decision: **a subagent is a nested `Engine`, not a second
implementation.** §2's "Engine as single owner" is preserved, not broken, by
making the *root* engine of a session the sole owner of everything that must
stay singular (the rule engine and its session grants, the frontend's one
screen), while a subagent gets its own instance of everything that is
naturally per-loop (conversation, context window, compaction, the guardrail,
its own task list) — reusing `turn.rs`, `context.rs`, `guardrail.rs`, and the
completion gate wholesale. This is the same payoff A-2 (a fake frontend can
drive the engine in tests) already banked: a subagent's "frontend" is simply
a second, headless driver — `SubagentManager` (`engine/subagents.rs`, new,
alongside the existing `guardrail.rs`/`skills.rs`/`memory.rs` per-concern
modules) — instead of the TUI.

- **Construction.** `spawn_agents` (T-18) builds one `EngineConfig` per
requested subagent, each a **derivation** of the parent's own, never a fresh
default:
  - `provider` — resolved via the existing `ProviderFactory` (§4.5's
  injected seam, unchanged) from an optional `profile`/`model` in the spawn
  args, defaulting to the parent's own active profile/model. Selecting a
  different already-configured profile is the only "new provider" surface
  this feature needs (P-8 pays rent again).
  - `tools` — a `ToolRegistry` filtered from the parent's own: **never** the
  four multi-agent tools themselves (this is where the depth bound lives —
  a subagent's registry structurally cannot contain `spawn_agents`, so
  recursive spawning is not a runtime check to get right, it is a
  registration that never happens), and, when the spawn args name a subset,
  further filtered to only those names — validated against the parent's
  *own* registry (`ToolRegistry::get` returning `None` for a name the
  parent itself lacks is a spawn-time structured failure, HC-6, never a
  silent grant). `max_depth` is a compile-time constant (`1`), not a config
  key — exposing a dial that cannot honestly be turned in this version
  would be worse than not exposing one (Requirements §2.2).
  - `system` — the subagent's spawn-time system prompt is the harness's own
  baked-in tool-use scaffold (unchanged, C-1) with the spawn call's
  model-authored persona/task text inserted into a dedicated section, never
  a bare replacement of the scaffold — a subagent still knows the tool-call
  conventions and safety framing every agent loop assumes.
  - `project_root`, `tool_explanations`, `image_max_bytes`/
  `document_max_bytes`, and the `LoopConfig`/`CompletionConfig`/
  `ContextConfig` in force are the parent's current values, copied at spawn
  time — one runtime configuration per session, not a second tier to keep
  in sync (Requirements §13 leaves independent subagent tuning an open,
  tune-with-use question).
  - **Permission gate is proxied, not duplicated.** A subagent's `ToolCtx`
  gets a `ProxyPermissionGate` (`emberly-tools`, implementing the existing
  `PermissionGate` trait) that forwards `authorize()` over an internal
  channel to the **root** engine, tagged with the subagent's id/name. The
  root engine remains the sole owner of `RuleEngine` and session grants
  (§2's invariant extended, not relaxed): a subagent never gets its own copy
  of the rules that could drift from the session's. The root engine's
  existing permission round trip (`PermissionRequest`/`PermissionAnswer`,
  §3.1) already serializes to one prompt on screen; a subagent's request
  queues behind it exactly like a second concurrent request would, carrying
  the tag Design §4.13/§5 renders as the provenance line. `ask_user` (T-8)
  proxies the same way, for the same reason — one user, one place they are
  ever asked anything. The tag rides as a new optional `on_behalf_of:
  Option<String>` field on `PermissionRendering` (`crate::types`) — additive,
  `None` for the primary agent's own requests, unchanged for every consumer
  that does not read it.
  - **Memory, skills, and scratch are shared, not forked.** A subagent's
  `MemoryGate`/`SkillGate`/`ScratchGate` point at the parent session's own
  `MemoryStore`/`SkillCatalog`/`ScratchStore` (already `Arc`-held, §8.1/§8.2/
  §8.3) — a subagent reads and writes the *same* durable memory, invokes the
  *same* skill catalog, and drops scratch files into the *same* session
  scratch directory as the parent. The **task list is not shared** — each
  subagent gets its own private `TaskListGate` state, since it is planning
  its own delegated work, not the parent's (T-11 is per-loop by nature).
  - **Transcript.** A fresh `FileTranscript` at
  `.agents/sessions/<parent-session-id>/subagents/<subagent-id>.jsonl`
  (new nested directory; `SessionId` reused as-is). The subagent's own
  `Engine` records `session_start`/`user_message`/`assistant_message`/
  `tool_call`/`tool_result`/`permission_request`/`permission_decision` exactly
  as any engine does (HC-7 unmodified) — a nested session in the audit-trail
  sense, discoverable from the parent's `tool_call` args (which record the
  subagent's id and, via it, its transcript path) exactly as an image/
  document read records a project-relative path (§4.1) rather than
  duplicating bytes.
- **Driving a turn.** `SubagentManager` sends a `Command::UserInput` into the
subagent engine's own command channel (the spawn task/persona text, or a
`message_agent` call's follow-up text) and awaits its `UiEvent::TurnEnded`,
collecting the final assistant text as the tool result. It does **not**
forward the subagent's `AssistantDelta`/`ToolStarted`/`ToolFinished` events
onto the parent session's own `UiEvent` stream (Design §4.13); it instead
emits the three coarse events of §3.1 (`SubagentSpawned`/`SubagentStatus`/
`SubagentEnded`) to the parent frontend and writes the fine-grained ones only
to the subagent's own transcript, served to the per-agent inspector via a new
`Command::InspectAgent{id}` → `UiEvent::AgentActivity{id, turns: Vec<..>}`
pair (mirroring `InspectSkill`/`SkillBody`, §3.1) that reads the subagent's
live conversation state directly (no polling the transcript file — the
subagent `Engine`'s in-memory history is queryable the same way
`recall_turns` already exposes the parent's, §6).
- **Concurrency.** `spawn_agents` builds its N `EngineConfig`s, spawns N
`tokio::spawn`ned subagent engine tasks, and drives all N "run this turn"
futures with `futures::future::join_all` (already a workspace dependency,
§12) — the actual fan-out primitive. Each is individually wrapped in
`tokio::time::timeout(agents.spawn_timeout_secs)`; a timeout leaves that
subagent's engine task alive and addressable via `message_agent`/
`list_agents` and reports `still running` in the batch result (Design
§4.13), rather than canceling it.
- **Resource bounds (config, `[agents]`, initial; tune with use).**
`enabled = true`; `max_concurrent = 3` — the ceiling on subagents alive at
once per session; exceeding it from `spawn_agents` is a structured failure
(HC-6) for the excess names, not a partial silent spawn; `spawn_timeout_secs
= 600` (§8.4 above); `idle_timeout_secs = 1800` — a subagent with no
`message_agent` traffic for this long is reaped (ended, §3.1
`SubagentEnded{reason: "idle timeout"}`) as a resource safety valve distinct
from the spawn timeout, which bounds only the *first* call.
- **Lifecycle and crash honesty (extends HC-3).** `end_agent` (T-21) drops
the subagent engine task and its channels; every subagent still alive when
the owning session ends is ended with it (its transcript's `session_end` is
written exactly as the parent's is). On process crash or restart, subagent
engine tasks are gone with the process (they hold no cross-process state);
the parent session's own resume (§3.3) does **not** attempt to reconstruct
or reconnect them — a `message_agent`/`list_agents` call against a
pre-crash id returns the structured "no such agent" failure of Requirements
FR-9, and the model may re-spawn. Reconnecting a live subagent across a
resume is deferred (Requirements §2.2).
- **Cost accounting.** Each subagent engine tracks its own `TokenUsage`/cost
exactly as any engine does (§4.4); `SubagentManager` adds each subagent's
usage into the **parent session's** running totals as it accrues (the same
`SessionUsage`/`CostEstimate` events, §3.1) so a delegated task's spend is
never a side channel (Requirements FR-9). Per-subagent detail remains
available via the inspector; whether it also shows inline in the sidebar
entry is a Design choice (Design Guideline §13, open).

### 8.5 MCP servers (FR-11, C-8)

- **Layout.** Server profiles resolve through the ordinary two-tier-plus-
project config (C-1): `[mcp.servers.<name>]` in global or project
`config.toml`, project winning per name like any other config table.

```toml
[mcp.servers.<name>]
transport = "stdio"          # initial supported transport (Requirements §13)
command   = "npx"
args      = ["-y", "@some/mcp-server"]
enabled   = true              # per-server off switch, no code change (Requirements FR-11)
```

- **Connection is engine startup logic** (§5.6), driven off this table
exactly as `[providers.*]` drives provider profiles (§4.5) — a genuinely new
transport is new code, a new server is configuration alone.
- **Workspace-trust gate.** A project-tier server entry is checked against
the session's trust decision before the engine ever spawns its transport —
mirroring the skill-catalog check (§8.2) line for line: an untrusted root
means the server is neither connected, cataloged, nor offered to the model
(Requirements FR-1/FR-11). A user-global server entry is never trust-gated.
- **Live reload.** `[mcp.servers.*]` participates in `reload_config` (§8) —
adding, removing, or disabling a server on save reconnects/disconnects it
without a restart, the same guarantee every other 0.2+ config surface
already gives.
- **Config:** `mcp.enabled` (default `true`, a global kill switch alongside
per-server `enabled`).

### 8.6 Session export (FR-12)

- **A read-only renderer over existing state, not a new store.** The
exporter reads the derived conversation-state view (§3.2a) — already the
product of truncation/reduction/windowing/compaction — plus the raw
transcript (§3.2) for anything the view has dropped but the export should
still surface in full (full tool-result bodies via their `full_output_ref`
sidecar, §5.3), and, when present, each subagent's own nested transcript
(§8.4) under `.agents/sessions/<id>/subagents/*.jsonl`. It performs no write
back into either (FR-12's "never mutates the transcript" honesty clause) —
`emberly export`/the in-session command is the only new code path, and it
opens files strictly read-only.
- **Format: static, self-contained HTML (Requirements §13 resolved:
initial format).** First-party string building over the already-normalized
message/tool-event types (A-3's `serde`-derived structs are the same data
this renders from) — **no templating-engine dependency** (§12): a
`fn render_session_html(&DerivedView, &[SubagentTranscript]) -> String`
producing one `.html` file with inlined CSS, no external assets, matching
this project's own "self-contained artifact" bar. A raw-JSONL export needs
no code — the transcript file is already a plain file the user can copy.
- **Surface.** `emberly export [--session <id>] <output-path>` (CLI,
binary composition root, §10) and an in-session `/export <path>` command
(TUI, §9) sharing the same renderer function — no divergent logic between
the two entry points. Output path is a plain filesystem path argument, may
be outside the project root (Requirements FR-12 — user-initiated, HC-4 does
not apply); the renderer refuses only on an ordinary I/O error (unwritable
path, disk full), returned as a plain CLI/command error, never a panic
(HC-3).
- **No redaction (Requirements FR-12 honesty clause).** The renderer copies
content verbatim, including anything a truncation/reduction marker already
names as present in the sidecar; it performs no scanning for secrets or
sensitive text. Design §8.11's one-time disclosure line is the sole
mitigation this feature ships with.
- **No new `TranscriptEvent` or `SCHEMA_VERSION` bump.** Export reads
existing event types only; the in-session command emits `SessionExported`
(§3.1) as a `UiEvent`, not a transcript event — the export itself is not
part of what the session did, only an action taken on its record afterward.

## 9. TUI (`emberly-tui`)

- `ratatui` + `crossterm`. Layout per Design §3: main pane, collapsible
right sidebar, one-line status bar; sidebar auto-collapses below 100
columns with critical info migrating to the status line.
- **Rendering:** first-party minimal markdown pass (fenced code, bold,
inline code, lists, headings-as-bold) per Design §4.1 — full
markdown parsers are deliberately not used for chat flow. Syntax
highlighting via `syntect` with the `fancy-regex` backend (pure Rust;
the default Oniguruma backend is C and excluded by HC-2). Diffs
rendered first-party from the edit tool's before/after (we own both
sides; no external diff binary), overlay view per Design §4.2.
- **Thai text (Requirements §2.1):** all width/wrap/cursor math uses
`unicode-segmentation` (grapheme cluster boundaries) +
`unicode-width` (column width per cluster) — never `str::len`, never
chars-as-columns. Applies to the input editor, wrapping, and overlay
scrolling alike. Thai combining marks (zero-width) and kerned
clusters are covered by cluster-wise processing; test fixtures include
Thai strings with stacked vowel/tone marks (§14).
- **Input:** first-party line editor (grapheme-aware): history,
Emacs-style basics, multi-line via Shift+Enter, bracketed paste.
- **Command palette:** Ctrl+P, fuzzy match over the command registry
(single source of truth also serving `/commands` and help).
- **Reasoning trail (Design §4.4):** collapsed dim line with expand
affordance, driven by `ReasoningDelta`; streams in place while thinking,
settles to the collapsed line on answer; `reasoning` view key sets the
default; `hidden` still records to the transcript.
- **Tool-call explanation (Design §4.5):** a single dim caption line under
the call from `ToolStarted.explanation`; absent when the model gave none,
never a placeholder.
- **In-app editor & pickers (Design §3.1, §4.6):** editable overlay reusing
the §4.2 overlay machinery, plus `$EDITOR` handoff; model/provider and
effort pickers as overlays fed by `[providers.*]` and `ModelInfo`.
- **Guided setup wizard (Design §4.6, C-7):** a new `Overlay` variant built
from the same `Choices` (adapter step) and `LineEditor` (endpoint/model/key
steps) primitives already used elsewhere in this section — no new
input-handling code, only new screens. Entered via a trailing row in the
existing model/provider picker. Writing to disk is behind a new
frontend-side injected trait (mirroring `ConfigReloader`'s seam, §8)
implemented by the binary composition root, so `emberly-tui` gains a
wizard without owning `config.toml`/`keys.toml` schema knowledge itself
(A-1). The key-entry step renders `•` per keystroke; the real text lives
only in that step's `LineEditor` buffer until the write, never elsewhere.
- **Question prompt (Design §5.1):** the `ask_user` surface — neutral
styling, never the reserved safety band, selectable options + free-text,
no unsafe default; Esc returns a structured decline.
- **Trust gate (Design §8.4)** at startup and **loop-break surface
(Design §8.5)** in harness voice — both keep full degraded-mode
guarantees (ASCII, capitalized choices, deliberate key).
- **Motion (Design §6.4):** ember-pulse spinner, streaming accent
glow, overlay ease-in, sidebar settle — all driven by a single
animation ticker (~12fps) that is *skipped entirely* when
`motion=false`, degraded mode, or a permission prompt is open.
Animation state never carries information.
- **Degraded mode (Design §7):** `NO_COLOR`/`TERM=dumb`/`--plain` →
line-oriented append-only output, ASCII markers, no cursor
repositioning, full permission-prompt guarantees in capitals. This
code path is a tested, supported configuration (§14) and is the
contract for the future headless frontend.
- **Task list (Design §4.7):** an inline checklist block where the model
updates it, plus a sidebar Tasks section, both driven by `TaskListUpdated`
(§3.1). Status glyphs `○`/`◐`/`✓` with ASCII `[ ]`/`[~]`/`[x]` in degraded
mode; the single in-progress item ember-accented. Render-only — the TUI never
mutates the list.
- **Images (Design §4.8):** a `read_image` result renders as a labeled
reference line (`name · WxH · format`), never in-terminal pixels (no
sixel/kitty/iTerm in scope); an unsupported-vision result (§5.2) renders as a
calm tool-result note, not a harness error.
- **Memory & skills (Design §3.1/§4.9):** sidebar Memory section (count from
`MemoryStatus`, opens an editable entry inspector via the Design §4.6 in-app path)
and Skills section (list from `SkillsAvailable`, opens a read-only `SKILL.md`
body view). Memory/skill/recall tool lines show `user`/`project` origin from
the tool-result payload; untrusted-folder memory/skills are simply absent.
- **Web results (Design §4.10):** the `web_search` tool result renders as a
list of `{title, url, snippet}` explicitly styled as untrusted fetched web
content with visible source URLs — never harness or assistant voice.
- **Agents (Design §3.1/§4.13):** sidebar Agents section lists currently
alive subagents from `SubagentSpawned`/`SubagentStatus`/`SubagentEnded`
(§3.1, §8.4), present only while at least one is alive; selecting one sends
`Command::InspectAgent` and renders the returned `AgentActivity` as a
read-only, live-updating overlay (§4.2 pattern) — the subagent's own
assistant text and tool activity, never streamed into the main pane.
`spawn_agents`/`message_agent`/`end_agent` tool lines carry the subagent
name (Design §4.13); a permission prompt raised by a subagent's own tool
call carries its `PermissionRendering`'s new optional `on_behalf_of: Option<String>` field (§8.4) as one dimmed line, with no other change to the
prompt (Design §5).
- **Image attach (Design §4.14):** `/attach <path>` (always available) and a
file-picker overlay reusing the existing `Overlay`/`Choices` machinery
(§4.6/§4.6-wizard pattern above), filtered to image extensions and rooted at
the project. A bracketed-paste event (already handled for text input, §9
top) whose payload resolves to an existing image file path is intercepted
before insertion into the line editor and offered as an attach instead of
literal text. On send, the resolved file is read exactly as `read_image`
(§5.2) validates and encodes it, and the resulting `ContentBlock::Image` is
attached to the outgoing `user_message` (§4.1) — rendered as the Design
§4.14 chip on that message once sent, never as a tool-activity line.
`image.max_attachments` (§8) caps how many ride one message; over the cap is
a plain input-time error, not a truncated silent send.
- **MCP (Design §3.1/§4.15/§5.3):** sidebar MCP section lists connected
servers from `McpServerConnected`/`McpServerFailed` (§3.1/§5.6), present
only while at least one server is connected or has a failure to show;
selecting a server opens a read-only overlay (§4.2 pattern) listing its
discovered tools. An `mcp__<server>__<tool>` call's tool-activity line and
permission prompt both parse the server name back out of the tool's own
namespaced identifier (§5.6) — no extra payload field to thread through.
- **Session export (Design §8.11):** `/export <path>` calls the same
`render_session_html` (§8.6) the `emberly export` CLI command uses; shows
the existing animation-ticker spinner (Design §6.3/§6.4, above) with an
"exporting" verb phrase only while the write is visibly in flight, then the
Design §8.11 finished line and sensitive-content disclosure. No new
overlay — this is a command, not a picker.
- **Mouse (Design §3.4):** `crossterm` `EnableMouseCapture` gated on
`ui.mouse` and rich mode — a single control point (like the §6.4 animation
ticker) so capture is off whenever `ui.mouse = false`, degraded mode, or
`--plain`. Wheel events scroll the focused pane/overlay; click events map to
focus+select on interactive rows (sidebar entries, palette, pickers,
collapsed reasoning/task blocks) and never synthesize an approval on a
permission prompt (Design §5, §3.4). The terminal's own Shift-modified
selection passes through unintercepted, so native copy still works; users who
want the terminal to own the mouse entirely set `ui.mouse = false`.
- **Strings:** all interface strings in one module/table
(Design §6.2 discipline) — not a localization framework, just no
scattered literals.

## 10. Binary & Supervisor (`emberly`)

- CLI: `emberly` (start/attach in cwd project), `emberly init`,
`emberly resume [id]`, `emberly config show`, `emberly trust [list|revoke <path>]` (FR-1, §6.7), `emberly clean [<session-id>]` (FR-8, §8.3),
`--plain`, `--model`, `--provider`,
`--effort`, `--version`.
- Startup runs the workspace-trust check (§6.7) before the engine starts
the loop: an untrusted root raises the trust gate and, on decline, exits
cleanly with no session created.
- Supervisor (HC-3, S-2): the binary installs a top-level catch
(panic hook + supervising the engine/TUI tasks). On any abnormal
path: restore the terminal (always — a corrupted terminal is a
failure), flush and fsync the transcript, append `abnormal_exit`,
print the resume hint, exit non-zero. The transcript writer flushes
per event during normal operation precisely so there is almost
nothing to lose.

## 11. Error Handling Policy

- Library crates: `thiserror` enums, no `unwrap`/`expect` (CI-enforced,
§1). Binary: `anyhow` at the edge.
- Two registers surfaced distinctly (Design §6.1): `ToolOutcome`
failures flow to the model as data (HC-6); `HarnessError` events flow
to the user in harness voice with what/why/next.
- Every harness error message is written in the string table with its
what/why/next fields explicit — the structure is in the type, so an
error without a "next step" does not compile.

## 12. Dependencies (initial locked set)

Core: `tokio`, `serde`, `serde_json`, `thiserror`, `anyhow` (bin only),
`toml`, `time`, `uuid`.
Providers: `reqwest` (no default features; `rustls-tls`, `json`,
`stream`), `futures`, `async-trait`, `eventsource-stream` or first-party
SSE (decide at implementation; first-party preferred).
Tools: `ignore`, `grep-searcher`, `globset`, `similar` (fuzzy hint for
edit no-match).
Sandbox: `landlock` (Linux). macOS confinement via std process hooks +
system API (no third-party crate if avoidable).
TUI: `ratatui`, `crossterm`, `unicode-segmentation`, `unicode-width`,
`syntect` (fancy-regex backend), `nucleo-matcher` (palette fuzzy match).

The 0.2 feature set adds **no new dependencies**: endpoint-configurable
provider profiles, reasoning effort/trace, the ask-user tool, tool-call
explanation, workspace trust, in-app editing/pickers, and the loop
guardrail are all engine, config, and TUI logic over the existing crate
set. `emberly-sandbox`'s frozen dependency list is untouched (the trust
gate lives outside it, §6.7).

The 0.3 feature set also adds **no new dependencies**: tool-result reduction
(FR-2), the adaptive window (FR-3) and its `recall` tool (T-10), automatic
compaction (FR-4), and the derived conversation-state cache (FR-5) are engine
and config logic over the existing set — the cache serializes the
already-`serde`-derived normalized message types (A-3) to JSON via
`serde_json`. `emberly-sandbox` is untouched.

The 0.4 feature set is the **first to add dependencies since v0.1**, all
pure-Rust (HC-2) and all in `emberly-tools`/`emberly-core`, never
`emberly-sandbox`:

- `base64` and `imagesize` (Tools) — base64-encode image bytes for the provider
request and read image dimensions from the header without a full decode
(T-12/P-11). `imagesize` is chosen over the full `image` crate deliberately:
we only need format + dimensions, not pixel decoding, so we avoid pulling a
codec tree.
- `reqwest` is added to `**emberly-tools*`* for `web_search` (T-14, §5.5) — not
a new external crate (Providers already locks it), but a new crate→crate edge;
same feature set (no default features; `rustls-tls`, `json`, `stream`).

Everything else in the 0.4 set is engine/config logic over the existing crates:
the task list (T-11), memory (FR-6) and skills (FR-7) use `serde`/`toml` and
first-party frontmatter splitting (no YAML crate — deliberate, HC-2); mouse
handling (Design §3.4) is `crossterm`, already locked. `emberly-sandbox`'s
frozen dependency list is untouched.

The 0.4.1 feature set adds **no new dependencies**: document input (P-12/T-16)
base64-encodes with the already-locked `base64` and sniffs PDF by a first-party
`%PDF-` magic-byte check (no PDF parser — HC-2, the passthrough promise of §4.1),
and the completion gate (S-6) is engine/config logic running check commands
through the existing `bash`/sandbox path. `emberly-sandbox` is untouched.

The 0.4.2 feature set adds **no new dependencies**: guided provider setup
(C-7) reuses the existing `toml` crate for the config.toml write (accepting
the reformatting tradeoff, §8) and the existing `LineEditor`/`Choices` TUI
primitives (§9) for the wizard's screens. `emberly-sandbox` is untouched.

The 0.5 feature set adds **no new dependencies**: the multi-agent subsystem
(FR-9, §8.4) is `emberly-core`/`emberly-tools` composition over already-locked
crates — `tokio::spawn` and `tokio::time::timeout` (already in use for the
existing single engine task and the bash tool's timeout, §2/§5.2) drive
concurrent subagent engines, and `futures::future::join_all` is a new call
against `futures`, already a direct `emberly-core` dependency (used by
`turn.rs`'s stream draining, §3). `emberly-sandbox` is untouched — a
subagent's tool calls run under the same confinement as the primary agent's,
proxied through the same root-owned rule engine (§8.4), never a second
sandbox configuration.

The 0.5.1 feature set adds **no new dependencies**: MCP client support
(FR-11, §5.6) is a first-party newline-delimited JSON-RPC client over
`tokio::process`/`tokio::io` and `serde_json`, all already locked — the
same "thin first-party client, no vendor SDK" rule P-4 states for providers,
extended here; user-attached images (FR-10, §4.1/§9) reuse the existing
`base64`/`imagesize` (§12, 0.4) encode/decode path with no new crate; and
session export (FR-12, §8.6) is first-party string building over already-
`serde`-derived types, deliberately avoiding a templating-engine dependency.
`emberly-sandbox` is untouched — an MCP tool call runs under the exact same
rule engine and sandbox confinement as any other tool call (§5.6/§6), never
a second permission mechanism.

Policy (Requirements §10): additions require `cargo vet` acceptance;
`cargo deny` (licenses, duplicates, advisories) + `cargo geiger` report
in CI; `emberly-sandbox` additions require explicit owner sign-off.

## 13. Build & Release

- **Release targets:** `x86_64-unknown-linux-musl`,
`aarch64-unknown-linux-musl` (fully static, HC-2),
`aarch64-apple-darwin`. Best-effort: `x86_64-apple-darwin`.
**Windows: deferred** — no Landlock/Seatbelt equivalent; shipping a
weaker safety tier is declined (current scope). (Resolves Requirements §13.)
- CI gates per PR: fmt, clippy (with the lint policy of §1 as errors),
test suite incl. degraded-mode and Thai-fixture tests, `cargo deny`,
`cargo vet`, musl static build check.
- Linux CI runs sandbox escape tests on a Landlock-enabled kernel
(§14.3). Release checklist includes the name-collision check
(Design §1.1) and a `--plain` smoke run.
- **License: AGPL-3.0-or-later, dual-licensed.** The full workspace
(`license.workspace = true`) carries AGPLv3's network-copyleft terms —
chosen over the prior Apache-2.0 metadata (no `LICENSE` file had ever
existed for it) specifically because plain GPL's copyleft triggers only on
distribution, not on running a modified version as a network service;
AGPL closes that gap. The repository owner is sole copyright holder and
separately offers a negotiated commercial license to parties that want an
exception to the copyleft terms. A CLA granting relicensing rights is a
prerequisite before any external contribution is merged — without one, a
merged contribution is AGPL-only from that contributor and the owner can
no longer grant a clean commercial exception covering it.
- **crates.io: real publish, not a name-only reservation.** All six
workspace crates publish under their real names with their real
functionality. `emberly-providers`, `emberly-sandbox`, and `emberly-tools`
have no internal path dependency and publish first (any order);
`emberly-core` depends on all three; `emberly-tui` depends on
`emberly-core`; `emberly` (the binary) depends on all five — this order is
required, since crates.io refuses a `path`-only dependency and each
workspace-internal dependency needs a matching registry `version` once its
target is published. `cargo add`-ing a library crate binds the importer to
AGPL-3.0-or-later exactly as cloning the repository would; that friction is
the deliberate point of the license choice above, not an oversight.
- **Homebrew: prebuilt binary only, personal tap.** No build-from-source
formula, and no submission to `homebrew-core`. A personal tap formula
fetches a prebuilt binary — built for the release targets above — from the
matching GitHub Release. `cargo-dist` owns both halves from one config:
cross-compiling the release targets into GitHub Release artifacts, and
keeping the tap formula's `url`/`sha256` in sync with each tagged release.

## 14. Testing Strategy

1. **Unit + scripted fake provider.** `FakeProvider` implements
  `Provider` from a script of canned `StreamEvent`s (text, tool calls,
   errors, mid-stream drops). The entire agent loop, permission flow,
   truncation, and compaction are testable offline and deterministic.
2. **Transcript replay.** Recorded JSONL sessions as fixtures; replayed
  through the engine with assertions on resulting state/events (A-2,
   A-3 paying rent). Schema-version fixtures guard forward
   compatibility (§3.3).
3. **Sandbox escape tests** (regression teeth for HC-4/HC-5), run on
  real Landlock in CI: write outside root → must fail; write to
   `.git/` via file tool and via bash → must fail; fake/aliased/
   in-repo `git` → must NOT get `.git/` write; genuine git commit →
   must succeed; degraded environment (Landlock disabled container) →
   must produce the §6.5 behavior exactly (status shown, auto modes
   locked, allowlist suspended).
4. **Live smoke suite.** Small real-provider script (one tool-use
  round trip per provider), manual/nightly only, never in the merge
   path.
5. **0.2 feature coverage** (offline via `FakeProvider` where possible):
  `FakeProvider` scripts emitting `ReasoningDelta` (P-10) and an effort
   parameter round-trip (P-9); an `ask_user` round trip asserting the loop
   blocks then resumes with the answer (T-8); a scripted re-treading loop
   that must trip `LoopHalted` and a progressing loop that must not (S-5);
   a provider profile pointed at a fake endpoint proving a new provider is
   config-only (P-8); trust-gate tests — untrusted root prompts and decline
   exits with no session, project-local trust key is ignored (FR-1).
6. **0.3 context-economy coverage** (offline, deterministic): per-tool
  reducer unit tests — a noisy `bash`/`grep`/`glob` output reduces to its
   salient content and the full output is recoverable from the sidecar, and
   `truncate.reduce = false` passes output through (FR-2); an adaptive-window
   test asserting old turns are dropped from the sent context but the
   transcript and elision marker are intact, plus a `recall` round-trip via
   `FakeProvider` asserting the tool returns the dropped turns in normalized,
   reduced form (not raw JSONL) and never raises a permission prompt (FR-3,
   T-10); an automatic-compaction
   test driving `ContextUsage` past the threshold via `FakeProvider` usage
   numbers and asserting a `compaction` event with `trigger = auto` fires at
   the next clean boundary and does not thrash (FR-4); a resume test asserting
   the cache fast-path restores the same view the live session held, and that
   a deliberately-staled/corrupted/deleted cache falls back to transcript
   replay producing the identical view (FR-5, the HC-7 subordination made a
   test).
7. **0.4 feature coverage** (offline via `FakeProvider` where possible):
  a `todo` round-trip asserting a full-list replace emits `TaskListUpdated`
   and a `task_list` transcript event and stays pinned across a compaction
   (T-11); an image round-trip asserting `read_image` root-confines its path,
   rejects an oversize/`.git/` path, appends a `ContentBlock::Image`, and — on
   a `vision:false` model — returns the structured unsupported-capability
   result instead of sending (T-12/P-11); memory tests — a write/recall
   round-trip, a path-escape attempt in `name` (`..`, absolute, separators)
   that must be rejected (the HC-4 boundary made a test), the index loading as
   pinned context, and project-scope memory NOT loading under an untrusted root
   (FR-6, §6.7); skill tests — discovery builds the catalog, `skill` loads only
   the body on invoke (metadata standing), project-over-user precedence with a
   shadow notice, an untrusted-root project skill absent from the catalog, and a
   bundled script running only through the ordinary permission-gated `bash` path
   (FR-7, §6.7); web-search tests — a profile pointed at a fake search endpoint
   proving search is config-only and provider-agnostic, the `web_search → ask`
   gate firing (and an allowlist grant suppressing it), results tagged untrusted
   and capped at `max_results` (T-14); and a mouse unit test asserting capture
   is disabled under `ui.mouse=false` and in degraded mode and that a click
   never synthesizes a permission approval (Design §3.4/§5).
8. **0.4.1 feature coverage** (offline via `FakeProvider` where possible): a
  document round-trip asserting `read_document` root-confines its path, rejects
   an oversize / `.git/` / non-PDF (magic-byte) file, appends a
   `ContentBlock::Document`, and — on a `documents:false` model — returns the
   structured unsupported-capability result instead of sending (T-16/P-12); and
   completion-gate tests — a `FakeProvider` script attempting completion with a
   registered check failing, asserting the failure re-opens the loop as a
   tool-result while a passing check lets it terminate; a check command proven to
   run through the sandboxed `bash` path (contained, not re-prompted); the
   bounded-attempt halt firing `CompletionGateHalted` after `max_attempts` with
   each resolution (`resume`/`steer`/`stop`/`finish`) behaving correctly and
   `finish` recording `override: true`; and an inert gate (no registered checks)
   leaving loop termination unchanged (S-6).
9. **0.5 feature coverage** (offline via `FakeProvider` where possible): a
  `spawn_agents` concurrency test asserting N subagents given in one call
   genuinely overlap in wall-clock time (not serialized) and the call returns
   once every one reaches its first stop, each tagged with its id and answer
   (T-18, FR-9); a `message_agent` round trip against a still-alive subagent
   id, and a structured failure for an unknown/ended id (T-19); a tool-ceiling
   test asserting a subagent's registry never contains the four multi-agent
   tools themselves (the depth bound, FR-9) and never a name absent from the
   parent's own registry even when explicitly requested; a permission-proxy
   test asserting a subagent's `authorize()` call reaches the **root**
   engine's `RuleEngine` (an existing session grant already covers a
   subagent's action without a second prompt, and a genuinely new one queues
   behind whatever prompt is currently showing, tagged with the subagent's
   name) rather than consulting an independent rule state; a `max_concurrent`
   ceiling test asserting a spawn attempt over the limit returns a structured
   failure for the excess names only (HC-6); a spawn-timeout test asserting a
   slow subagent is reported `still running` rather than canceled and remains
   addressable afterward; a crash/resume test asserting a `message_agent`/
   `list_agents` call against a pre-restart subagent id returns the structured
   "no such agent" failure rather than hanging or panicking (the HC-3
   honesty clause made a test); and a cost-rollup test asserting a
   subagent's token usage lands in the owning session's `SessionUsage`/
   `CostEstimate` totals (P-6, FR-9).

Agent *quality* evaluation (does it code well) is explicitly out of
scope for this spec — post-release discipline with separate tooling.

## 15. Milestones

M1 — engine loop + fake provider + read/bash/edit + per-action prompts
(no sandbox yet), line-mode output. *Proves the core.*
M2 — Landlock confinement + rule engine + degradation policy + escape
tests. *Proves the safety story.*
M3 — Anthropic + OpenAI-compat live providers, streaming, retries,
token/cost accounting. *Proves P-1..P-6.*
M4 — full TUI: panes, sidebar, palette, diff overlay, Thai input
fixtures, degraded mode parity. *Proves the design guideline.*
M5 — sessions: transcript, resume, `/compact`, `init`/config
provenance, macOS Seatbelt, release pipeline. *Proves the initial release.*
M6 — 0.2 feature set: endpoint-configurable provider profiles (P-8) incl.
the Z.ai profile, reasoning effort (P-9) and reasoning trail (P-10),
ask-user tool (T-8), tool-call explanation (T-9), workspace trust (FR-1),
in-app config/prompt editor and model/effort pickers (C-5, C-6), and the
loop-breaking guardrail (S-5). *Proves the 0.2 scope.*
M7 — 0.3 context-economy feature set: wire the `/compact` command surface
into the registry (the existing implementation gap — Requirements §8.3 +
Design §3.3), tool-result reduction (FR-2), the adaptive context window with
its `recall` re-read tool (FR-3, T-10), automatic compaction (FR-4), and the
derived conversation-state resume cache (FR-5). *Proves the 0.3 scope: a long session
costs proportionally less in tokens, dollars, and resume time, with no fact
lost from the transcript.*
M8 — 0.4 capability-parity feature set: the task-list tool (T-11), multimodal
image input (P-11) and the read-image tool (T-12), persistent memory (FR-6,
T-13), the skill system (FR-7, T-15), the harness-owned web-search backend and
tool (T-14), and TUI mouse support (Design §3.4). *Proves the 0.4 scope: the
harness reaches capability parity with mature coding agents — planning, sight,
durable memory, extensible skills, live web reach, and pointer interaction —
without conceding provider-agnosticism (§1), the pure-Rust build (HC-2), or the
safety model (§6).*
M9 — 0.4.1 feature set: the completion gate (S-6) and document (PDF) input
(P-12) with the read-document tool (T-16). *Proves the 0.4.1 scope: the loop can
be held to registered checks before it declares done, and the model can read the
document formats real-world source material arrives in — both with no new
dependencies (§12), and the gate with no privileged path around the safety model
(§6).*
M10 — 0.4.2 feature set: guided provider/model setup (C-7) — a step-by-step
wizard entered from the model/provider picker that creates a new
`[providers.<name>]` profile and its `keys.toml` entry without hand-editing
either file, reusing the existing config-reload path (C-5) to apply live.
*Proves the 0.4.2 scope: going from zero to a working provider is an in-app
guided flow, not a raw-TOML editing exercise, with no compromise to the C-1
tier model, C-3 provenance, or the never-in-transcript secret-handling
guarantee.*
M11 — 0.4.3 feature set: session scratch space (FR-8) with the
`scratch_write` tool (T-17) and the `emberly clean` reclaim command. *Proves
the 0.4.3 scope: the model has disposable, harness-owned working space that
costs no permission prompt and never lands in the user's tracked project,
and the user can reclaim it on demand — with no widening of HC-4 and no new
dependency.*
M12 — 0.5 feature set: the multi-agent subsystem (FR-9) — spawning one or
more subagents concurrently (T-18), conversing with a specific one across
further calls (T-19), enumerating (T-20) and ending (T-21) them — built as
nested `Engine` instances driven by a headless `SubagentManager` frontend,
with permission/sandbox/workspace-trust enforcement proxied through the
root engine so no subagent runs under a weaker safety posture than the
session itself. *Proves the 0.5 scope: the primary agent can delegate
bounded, independent, fully-audited work to subagents — in parallel where
it fans out, addressable across turns where it doesn't — without a second
implementation of the engine loop, a second safety model, or a single new
dependency.*
M13 — 0.5.1 feature set: MCP client support (FR-11) — a first-party
JSON-RPC-over-stdio client discovering and registering external tools
through the existing transport-agnostic `Tool` trait (T-7), namespaced,
trust-gated per server (C-8), and permission-gated identically to any
built-in tool; user-attached image input (FR-10) — an attach surface over
the existing multimodal content path (P-11/T-12), rendered as message
content rather than tool activity; and session export (FR-12) — a
read-only, self-contained HTML rendering of a complete session, including
its subagents, over the existing transcript and derived-view state. *Proves
the 0.5.1 scope: three usability gaps close without a privileged path
around the safety model, a second persistence mechanism, or a single new
dependency (§12).*

## 16. Open Items

**v0.15 (2026-09-02, distribution & licensing).** §13 Build & Release
expanded with the project's first public distribution channels and a
license change — the prior Apache-2.0 metadata (no `LICENSE` file had ever
existed for it) replaced with AGPL-3.0-or-later, dual-licensed by the sole
copyright holder. No product behavior changes: no new FR-n/HC-n/P-n/T-n
IDs, no new `UiEvent`/`TranscriptEvent` variant, no `SCHEMA_VERSION` bump.
Requirements and Design are untouched and remain pinned at v0.12 — this
bump is Spec-only, since nothing about WHAT the harness does or how it
looks/feels/speaks changed. Approved by the owner (2026-09-02) with no
requested changes.

Open items introduced by this scope:

- **Sub-crate name reservation on crates.io.** This scope reserves
`emberly` by publishing it for real; whether the five internal crate names
are also worth reserving (low-cost insurance against squatting, independent
of whether they see real external use) is left to the owner, not decided
here.
- **CLA mechanics.** §13 makes a CLA a prerequisite for merging any
external contribution, but the concrete tooling (e.g. a `cla-assistant`
gate) is not built as part of this scope — there are no external
contributors yet, so nothing blocks on it today.

**v0.14 (2026-08-16, 0.5.1 feature set).** Three capabilities graduated
together from Requirements §2.2/new scope land as `emberly-core`/
`emberly-tools`/`emberly-tui` composition with **no new dependencies**
(§12). MCP client support (FR-11) is a first-party newline-delimited
JSON-RPC-over-stdio client (`McpClient`, §5.6) discovering external tools
and registering them into the existing `ToolRegistry` through T-7's
already-transport-agnostic `Tool` trait, namespaced `mcp__<server>__<tool>`,
gated by the same workspace-trust check project skills already use (§8.2)
and the same permission gate every tool already flows through — no proxy
gate was needed here, unlike a subagent's (§8.4), because these calls are
made by the engine that already owns the rule engine. User-attached image
input (FR-10) needed no new `ContentBlock` variant: a user attachment and a
model-read image are the identical `ContentBlock::Image` (P-11), told apart
structurally by which transcript event carries it (`user_message` vs.
`tool_result`, §4.1) rather than a flag. Session export (FR-12) is a
read-only renderer (`render_session_html`, §8.6) over already-existing
state — the derived view (§3.2a), the raw transcript (§3.2), and subagent
transcripts (§8.4) — producing self-contained HTML with no templating
dependency and no new persistence mechanism. New IDs realized: FR-10 (§4.1,
§9), FR-11 + C-8 (§5.6, §8.5), FR-12 (§8.6). New `UiEvent` variants
`McpServerConnected`/`McpServerFailed`/`SessionExported` (§3.1) are additive;
no new `TranscriptEvent` variant and no `SCHEMA_VERSION` bump — an MCP tool
call is an ordinary `tool_call`/`tool_result` pair, an attached image is an
ordinary `user_message` content block, and export reads without writing.
Minor, additive bump; Requirements bumped to v0.12 and Design to v0.12 in
lockstep (pins refreshed) — all three still `draft`, pending owner review
(§0 of the 0.5.1 Implementation Plan gates Phase 1 on their approval).

Open items introduced by the 0.5.1 scope:

- **MCP transport scope (FR-11, §5.6, Requirements §13 open question).**
Stdio-launched local servers only, this version; a remote transport
(SSE/HTTP) is a second `McpTransport` impl behind the same trait when a
real need appears, not a redesign.
- **MCP per-server/per-tool permission defaults (FR-11, §5.6, Requirements
§13 open question).** Every MCP-sourced tool call defaults to the ordinary
per-tool `ask` posture (§6.1) this version; whether a server-level
allow-listing convenience (rather than per-tool-name rules) is worth adding
is a tune-with-use question once real servers are in daily use.
- **Image attach caps (FR-10, §8, Requirements §13 open question).**
`image.max_attachments = 4` is a placeholder; tune with real use once a
file-picker and drag-drop are exercised against real terminals.
- **True clipboard-image-byte paste (FR-10, Requirements §2.2 new
deferral).** No cross-terminal API delivers clipboard image bytes (only
pasted text); revisit only if a specific terminal's extended protocol
becomes worth special-casing.
- **Session export format scope (FR-12, §8.6, Requirements §13 open
question).** Self-contained HTML is the only format this version; whether
a second format (e.g. Markdown) is worth the added renderer is deferred
until real export use appears. No size/streaming handling exists yet for a
very large session's HTML — confirmed acceptable or revisited once a
real oversized session is exported.

**v0.13 (2026-08-15, 0.5 feature set).** The multi-agent subsystem, absorbed
as FR-9/T-18–T-21, lands as `emberly-core`/`emberly-tools` composition with
**no new dependencies** (§12) — a subagent is a nested `Engine` instance
driven by a new headless `SubagentManager` frontend rather than a second
engine implementation, with its permission gate proxied through the root
engine so `emberly-sandbox` needs no change and no subagent runs under
weaker enforcement than the session itself (§8.4). New IDs realized: FR-9
(multi-agent subsystem, §8.4), T-18 (`spawn_agents`, §8.4), T-19
(`message_agent`, §8.4), T-20 (`list_agents`, §8.4), T-21 (`end_agent`,
§8.4). Three new `UiEvent` variants (`SubagentSpawned`/`SubagentStatus`/
`SubagentEnded`, §3.1) and one new `PermissionRendering` field
(`on_behalf_of`, §8.4) are additive, and no new `TranscriptEvent` variant is
needed at the primary session's level (spawn/message/list/end are ordinary
`tool_call`/`tool_result` pairs, §3.2) — no `SCHEMA_VERSION` bump. Minor,
additive bump; Requirements bumped to v0.11 and Design to v0.11 in lockstep
(pins refreshed).

Open items introduced by the 0.5 scope:

- **Resource-bound defaults (FR-9, §8.4, Requirements §13 open question).**
`max_concurrent = 3`, `spawn_timeout_secs = 600`, and `idle_timeout_secs =
1800` are placeholders; tune with real multi-agent sessions so the ceiling
stops a runaway fan-out without cutting off genuinely long-running
delegation.
- **Independent per-subagent tuning (FR-9, §8.4).** v0.5 copies the parent's
`LoopConfig`/`CompletionConfig`/`ContextConfig` onto every subagent at spawn
time; whether a subagent should ever get its own independently-tuned
guardrail/completion-gate/window settings (e.g. a stricter loop guardrail
for an untrusted delegated task) is deferred to a tune-with-use pass.
- **Per-subagent cost display (FR-9, §8.4, Design Guideline §10 open
question).** Cost rolls into the session total unconditionally (built); the
sidebar Agents entry showing per-subagent cost inline versus only on
inspection is a Design decision still open.
- **The Hugging Face / local-inference-server request is not absorbed this
version.** See Requirements §2.2/§13 and Design (unchanged) — the harness's
existing endpoint-configurable provider profiles (P-8) already reach such a
server at zero Tech Spec cost once one exists; whether Emberly Code
additionally gains a thin, provider-agnostic CLI process-management
convenience is an owner decision to make before any Tech Spec content is
drafted for it.

**v0.11 (2026-07-30, 0.4.3 feature set).** Session scratch space, absorbed as
FR-8/T-17, lands as tool-layer + binary logic with **no new dependencies**
(§12) — the scratch directory reuses the existing `.agents/` layout
conventions and the `scratch_write` tool reuses the `memory` tool's
path-derivation logic (T-13, §8.1). New IDs realized: FR-8 (scratch-space
lifecycle, §8.3), T-17 (`scratch_write` tool, §5.2). No new transcript event
type — the tool's call/result are ordinary `tool_call`/`tool_result` events,
so no `SCHEMA_VERSION` bump. Minor, additive bump; Requirements bumped to
v0.10 and Design to v0.10 in lockstep (pins refreshed).

Open items introduced by the 0.4.3 scope:

- **`emberly clean` confirmation and non-interactive use (FR-8, §8.3,
Requirements §13 open question).** Initial behavior is an interactive `y`/`n`
confirmation only; a non-interactive flag (e.g. `--yes`) for scripted/CI use
is deferred to a tune-with-use pass if it turns out to be needed.
- **Scratch space has no size cap or retention policy (FR-8, §8.3).** Unlike
`truncate.max_bytes` or the memory index cap, nothing currently bounds how
much a session can accumulate between `emberly clean` runs; confirm in
practice whether an unbounded scratch directory is an actual problem before
adding one.

**v0.10 (2026-07-21, 0.4.2 feature set).** Guided provider/model setup,
absorbed as C-7, lands as TUI + binary logic with **no new dependencies**
(§12) — the existing `toml` crate handles the config.toml write
(parse-mutate-reserialize; comments/formatting elsewhere in the file are not
guaranteed to survive, §8) and the existing `LineEditor`/`Choices` TUI
primitives cover the multi-step flow (§9). New ID realized: C-7 (guided
setup, §8/§9). No new transcript event type — the wizard's completion reuses
the existing `Command::ReloadConfig` path (C-5), so no `SCHEMA_VERSION`
bump. Minor, additive bump; Requirements bumped to v0.9 and Design to v0.9
in lockstep (pins refreshed).

Open items introduced by the 0.4.2 scope:

- **Config.toml write fidelity (C-7, §8).** Reserializing the whole file on
a wizard write does not preserve user comments/formatting outside the
touched `[providers.<name>]` table (decided against adding `toml_edit`,
§12); confirm in M10 this is an acceptable tradeoff in practice, and
revisit if it draws complaints.
- **Wizard scope: create-only vs. edit-existing (C-7, Requirements §13 open
question).** Initial scope is creating a new profile only; editing an
existing profile's fields through the same wizard is deferred to a later
tune-with-use pass.
- **Adapter/auth-scheme coverage in the wizard (C-7, §8).** Initial wizard
supports the two adapters (`anthropic`/`openai`) and the auth schemes
`build_profile`/`resolve_auth` already validate (bearer/x-api-key/header/
none); a non-standard scheme still requires raw editing (C-5) as the
stated fallback.

**v0.9 (2026-07-15, 0.4.1 cross-project feature set).** Two capabilities
requested by TREEGAL Yggdrasil (a consumer of the engine) and absorbed into
Requirements v0.8 land as engine/config/TUI logic with **no new dependencies**
(§12). New IDs realized: S-6 (completion gate, §7/§3/§8), P-12 + T-16 (document
input, §4.1/§4.2/§5.2). The `completion_check` and `completion_gate_halt`
transcript events and the `ContentBlock::Document` block are additive (no
`SCHEMA_VERSION` bump). Minor, additive bump; Requirements bumped to v0.8 and
Design to v0.8 in lockstep (pins refreshed). A third request (Windows supervised
posture) was **held, not absorbed** — the requester targets macOS only for its
prototyping stage — so §13's Windows-deferred posture is unchanged.

Open items introduced by the 0.4.1 scope:

- **Document formats and caps (P-12/T-16, §5.2).** PDF only (Requirements §2.3
declines docx and other word-processor formats); the 32 MiB byte cap and the
`%PDF-` magic-byte sniff are the initial set. Confirm the `document` block maps
cleanly to the Anthropic native document block and to each OpenAI-compatible
endpoint's document input (or is correctly declared `documents:false`) against
live endpoints in M9, and confirm provider page/token limits surface as clean
unsupported/oversize results, not crashes.
- **Completion-gate defaults and registration (S-6, §7).** `max_attempts = 3`
is a placeholder; tune so the gate stops a premature landing without recreating
an S-5 spin. Confirm the config command-check shape (`expect_exit`) covers the
common coding checks (test/lint/build), and validate the frontend/tool
registration hook against a real non-config registrant when one exists.

**v0.8 (2026-07-12, 0.4 capability-parity scope).** The 0.4 feature set lands
across `emberly-core`/`-tools`/`-tui` and is the first version since v0.1 to add
dependencies (§12: `base64`, `imagesize`, and `reqwest` into `emberly-tools`) —
all pure-Rust (HC-2), `emberly-sandbox` untouched. New IDs realized: T-11 (task
list, §5.2/§7), P-11 + T-12 (multimodal image input, §4.1/§4.2/§5.2), FR-6 +
T-13 (persistent memory, §8.1), FR-7 + T-15 (skills, §8.2), T-14 (web search,
§5.5), and mouse support (Design §3.4, §9). The `task_list` transcript event and
the image content block are additive (no `SCHEMA_VERSION` bump). Minor, additive
bump; Requirements bumped to v0.7 and Design to v0.7 in lockstep (pins
refreshed).

Open items introduced by the 0.4 scope:

- **Search adapter coverage (T-14, §5.5).** `brave`/`tavily`/`searxng`/`json`
are the initial response-shape parsers; validate against the real services in
M8 and add parsers only for genuinely new shapes. Confirm the bearer / header
/ query auth set suffices.
- **Image formats and caps (T-12/P-11, §5.2).** PNG/JPEG/GIF/WebP and a 5 MiB
cap are the initial set; confirm each maps cleanly to the Anthropic `image`
block and the OpenAI `image_url` `data:` URI against live endpoints in M8, and
confirm `imagesize` covers every accepted format's header.
- **Memory index growth (FR-6, §8.1).** `memory.max_index_entries` is a soft
warn threshold; determine in M8 the point at which a large always-pinned index
itself wants the §7 economy (e.g. description-only truncation or an on-demand
index tier).
- **Skill precedence surfacing (FR-7, §8.2).** Project-over-user precedence is
resolved; confirm the shadow notice in `config show` is clear enough, and
decide whether a user skill should ever be invocable by a qualified name when
shadowed.
- **Mouse capture / native-selection passthrough (Design §3.4, §9).** Shift-
passthrough for native selection is terminal-dependent; verify behavior across
the target terminals in M8 and document the ones where `ui.mouse = false` is
the only way to get native selection.

**v0.7 (2026-07-11, 0.3 context-economy scope).** The 0.3 feature set lands as
engine/config additions over the existing crates (§12): tool-result reduction
(FR-2, §5.3), the adaptive window (FR-3, §7), automatic compaction on by
default at `0.85` (FR-4, §7), and the derived resume cache (FR-5, §3.2a/§3.3).
The `compaction` transcript event gains an additive `trigger` field (§3.2, no
schema bump). The missing `/compact` command surface is folded into M7 as an
implementation gap against the existing §8.3 requirement, not new scope.
Minor, additive bump; Requirements bumped to v0.6 and Design to v0.6 in
lockstep (pins refreshed).

Open items introduced by the 0.3 scope:

- **Adaptive-window re-read affordance (FR-3, §7) — RESOLVED (owner,
2026-07-11).** Recovered via a dedicated `recall` built-in (Requirements
T-10, §5.2/§7), not `read_file` over raw JSONL — the raw path would
re-inflate elided noise and defeat the window. `recall` is engine-internal
(no filesystem/network) and not permission-gated. Remaining M7 detail: the
marker↔range identifier scheme (turn indices vs. an opaque marker id).
- **Per-tool reducer rules (FR-2, §5.3).** The initial `bash`/`grep`/`glob`
reducers are a starting set; validate against real tool output in M7 so
reduction never hides what the model needs, and extend the reducer registry
as new salient shapes appear.
- `**context.window_turns` and `context.auto_compact_threshold` defaults
(§7, §8).** `40` and `0.85` are placeholders; tune with real long sessions
so windowing/compaction fire before overflow without cutting genuine
working context.
- **Cache staleness guard (FR-5, §3.2a).** Byte-length + offset is the initial
guard; confirm it is sufficient in M7 (a content hash is the fallback if
same-length divergence is ever observed).

**Resolved in v0.6 (2026-07-11, M6 close — Phase 5).** Workspace trust (FR-1) is
realized as a **pre-engine binary gate** (§6.7), not an engine event: the v0.5
`TrustRequest{path}` UiEvent (§3.1) is **withdrawn** — trust is decided before
the engine loop and before project files are read, so it never crosses the
engine↔frontend channel. `trust_decision` (§3.2) is written on accept only (a
decline starts no session). The loop-breaking guardrail (S-5) landed as
specified: `LoopHalted`/`loop_halt` events plus a `ResolveLoop{resume|stop| steer}` command. Minor, additive bump; Requirements/Design unchanged (pins
refreshed to Spec v0.6).

- First-party SSE vs `eventsource-stream` (decide in M3 by reading the
crate; bias first-party).
- `truncate.`* and `context.keep_recent_turns` defaults — placeholders
above; tune with real use.
- Session-title generation: heuristic in M5 (first user message,
clipped); model-generated title as post-release nicety.
- macOS Seatbelt profile details (M5 spike; public API is old and
thinly documented — budget investigation time).
- `[loop]` default thresholds (§7) — placeholders; tune with real use so
the guardrail catches runaways without cutting off genuine progress.
- Provider auth-scheme coverage (§4.5): confirm bearer / x-api-key /
custom-header suffices for target endpoints; extend the set only if a
real profile needs it (M6).
- Effort enum granularity (§4.6): the four-level → thinking-budget ladder is
implemented and owner-approved (2026-07-11: Low 2k / Medium 8k / High 16k /
Max 32k, clamped to the model's output allowance; openai maps to
`reasoning_effort` with Max→high). Still to do: validate against each live
provider's native control in M6; collapse or extend if the mapping is lossy.
- Reasoning-signature replay (§4.7): the mechanism ships (a `ReasoningSignature`
stream event + `ContentBlock::Reasoning` carrier); confirm per-provider echo
requirements against live endpoints in M6 so multi-turn thinking is correct.
Redacted-thinking replay is implemented but untested against a live endpoint.

