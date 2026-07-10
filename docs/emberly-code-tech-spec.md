# Emberly Code — Technical Specification

**Version:** 0.4 
**Status:** approved 
**Date:** 2026-07-10
**Owner:** Wattanit
**Companion documents:** Requirements Document v0.5 (upstream contract),
Design Guideline v0.5 (upstream for all UI/UX decisions)

This document defines HOW Emberly Code is built. Requirements-level
identifiers (HC-n, P-n, T-n, C-n, S-n, A-n) refer to the Requirements
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
│   ├── emberly-tools/     # Tool trait + six built-in tools
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
`TrustRequest{path}` (FR-1), `ContextUsage{pct, tokens}`, `CostEstimate{..}`,
`SandboxStatus(..)`, `ModeChanged(..)`, `ModelChanged{provider, model}` and
`EffortChanged(level)` (C-6/P-9), `HarnessError{..}`, `SessionMeta{..}`,
`FileModified{path, adds, dels}`, `CompactionStatus(..)`.

### 3.2 `TranscriptEvent` (durable, append-only JSONL)

One JSON object per line in
`.agents/sessions/<session-id>.jsonl`. Every event carries:

```json
{"v": 1, "ts": "2026-07-06T09:14:02.113Z", "type": "...", ...}
```

- `v` — schema version, present from day one (HC-7 longevity).
- Event types: `session_start` (model, provider, config provenance,
  sandbox status), `trust_decision` (path + trusted/declined — FR-1,
  §6.7), `user_message`, `assistant_message` (complete, not deltas;
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
  (summary text + replaced range), `session_title`, `session_end`,
  `abnormal_exit` (written by the supervisor when possible).
- The transcript is ground truth; the in-context conversation is rebuilt
  from it (resume) or maintained in parallel with it (live session).
  Nothing ever rewrites a transcript line (HC-7, Requirements §8.2).

### 3.3 Resume

`emberly resume` (and the offer-on-next-launch flow, Design §8.3)
replays the transcript: conversation view is reconstructed by applying
`compaction` events as view transformations. Unknown event types (newer
`v`) are surfaced as a warning, not a crash.

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

`CompletionStream` yields normalized `StreamEvent`s: `TextDelta`,
`ReasoningDelta` (model thinking, distinct from the answer — P-10),
`ToolCallStart/Delta/End`, `Usage`, `Done`, `Err`. No provider wire
type crosses this boundary (P-1). A provider that does not stream
reasoning simply never emits `ReasoningDelta`.

### 4.2 Implementations

- **Anthropic Messages API** — content blocks, `tool_use`/`tool_result`
  mapping, SSE streaming.
- **OpenAI-compatible** — `tool_calls` mapping, SSE streaming; base URL
  configurable, which transitively covers Ollama, vLLM, OpenRouter,
  private deployments (P-2).

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
  work, the adapter preserves the signature alongside the normalized
  reasoning and replays it per that provider's rule — kept inside the
  adapter so no wire detail leaks past the boundary (P-1). A provider
  without this requirement ignores it.

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

| Tool | Notes |
|---|---|
| `read_file` | Path-normalized (symlinks resolved, `..` collapsed) then root-checked. Output truncation per §7. |
| `write_file` | Refuses `.git/` (HC-5). Creates parent dirs inside root only. |
| `edit_file` | Exact string match-and-replace. Failure messages distinguish *no match* vs *N matches found* and, for no-match, include the closest fuzzy region as a hint (T-3 — model recovery quality depends on this). |
| `bash` | See §6. Timeout default 120s, configurable per-call by the model up to a config ceiling. Env is a scrubbed allowlist (PATH, HOME, LANG, TERM + config additions) — secrets in the user's env are not inherited by default. |
| `glob` | Root-confined; ignores `.git/` and honors `.gitignore` by default. |
| `grep` | First-party wrapper over the `grep-searcher`/`ignore` crates (the ripgrep libraries — pure Rust, same author). Root-confined. |
| `ask_user` | Presents a question and optional discrete options to the user and blocks the agent loop until answered (T-8). Returns the typed answer, or a structured `{declined: true}` if dismissed, so the model can proceed or stop. Touches no filesystem or network — a pure engine↔frontend round-trip — so it bypasses the sandbox but still flows through the `Tool` trait; it is a `Command`/`UiEvent` pair under the hood (§3). |

### 5.3 Truncation at ingestion (Requirements §8.1)

Applied when a tool result is appended: if output exceeds
`truncate.max_lines` (default 400) or `truncate.max_bytes` (default
64 KiB), keep head (default 150 lines) + tail (default 100 lines),
insert `[... N lines elided — /view to open full output ...]`, write the
full output to the sidecar file, and record `full_output_ref` in the
transcript event. Deterministic, no model call. The `/view` handoff
(Design §4.3) opens the sidecar read-only in `$VISUAL`/`$EDITOR`.

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
  `head`, `tail`, `wc`, `rg`, `find`, `pwd`, `echo`, `which`,
  `git status`, `git diff`, `git log`, `git show`, `git branch`,
  `cargo check`, `cargo tree`, `cargo metadata`. (Resolves the
  Requirements §13 open item; final list is a living config default.)

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
  - Everything else: no access. Paths outside root that the user
    explicitly approved per-action (HC-4 ask) are added to that single
    invocation's ruleset only.
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
  loop: canonicalize the root, test membership (store ∪ allowlist). A miss
  raises the trust gate (frontend prompt, Design §8.4); on decline the
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

## 7. Context Management (`emberly-core`)

- Budget: `window − reserved_output` (default reserve 8k tokens or the
  model's max-output, whichever is smaller). `ContextUsage` emitted on
  every accounting change (Design status line + sidebar).
- **Pinned, never compacted:** system prompt, project instructions
  (AGENTS.md/CLAUDE.md per C-1), original task statement (first user
  message of the session, tagged in the transcript).
- **`/compact`** (manual): valid only at clean boundaries (every
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
- Auto-compaction: designed-for (the trigger is one threshold check in
  the accounting path) but not enabled in current scope (Requirements §2.2).
- **Loop-breaking guardrail (S-5).** The engine keeps a rolling signature
  of recent steps: for each turn, the multiset of `(tool_name,
  normalized-args)` tuples plus a hash of the resulting `tool_result`
  content and the set of files modified. No-progress heuristic (initial;
  tune with use): trip when the last `loop.repeat_window` turns (default 3)
  repeat tool-call signatures **and** produce no new modified files and no
  new distinct tool-result hashes — i.e. the loop is re-treading, not
  advancing. On trip: stop issuing provider calls, emit `LoopHalted{reason}`
  (UiEvent + transcript event, §3), and await a user `Command` (resume /
  stop / steer, Design §8.5). Config `[loop] enabled, repeat_window,
  max_no_progress_turns`. The guardrail never trips while files change or
  tool results differ (genuine progress); it is a heuristic (Requirements
  S-5), and the guarantee is termination-into-a-decision, not perfect
  classification.

## 8. Configuration & Prompts

- Settings: TOML. Global `~/.config/emberly/config.toml` (XDG), project
  `.agents/config.toml`; project wins per key. Env `EMBERLY_*` overrides
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
  live-reload on save; keys that require a restart are a static list in the
  config module, and the editor names them at save time — the type carries
  a `reload: Live | RestartRequired` flag per key so "does this need a
  restart" is not a guess.
- **In-app model/provider/effort switching (C-6).** `Command::SwitchModel
  {profile}` and `Command::SetEffort{level}` swap the active `Provider` /
  effort for subsequent turns; both are transcript events and never rewrite
  prior turns. The picker (Design §3.1) lists the `[providers.*]` profiles
  (§4.5) and the active model's effort levels (§4.6).
- **New config keys** (initial; tune with use): `[providers.<name>]`
  (§4.5); per-model effort default (§4.6); `reasoning = collapsed |
  expanded | hidden` view default, **default `collapsed`** (Design §4.4);
  `ui.tool_explanations = bool`, **default `true`** (§5.4); `[loop] enabled,
  repeat_window, max_no_progress_turns` (§7).
- **Trust:** store at `~/.config/emberly/trust.toml`, `0600`, global only;
  optional `trust.trusted_dirs` pre-trust allowlist in global config
  (§6.7) — neither is ever a project key (Requirements FR-1).

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
- **Strings:** all interface strings in one module/table
  (Design §6.2 discipline) — not a localization framework, just no
  scattered literals.

## 10. Binary & Supervisor (`emberly`)

- CLI: `emberly` (start/attach in cwd project), `emberly init`,
  `emberly resume [id]`, `emberly config show`, `emberly trust
  [list|revoke <path>]` (FR-1, §6.7), `--plain`, `--model`, `--provider`,
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

## 16. Open Items

- First-party SSE vs `eventsource-stream` (decide in M3 by reading the
  crate; bias first-party).
- `truncate.*` and `context.keep_recent_turns` defaults — placeholders
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
- Effort enum granularity (§4.6): validate the four-level mapping against
  each live provider's native control in M6; collapse or extend if the
  mapping is lossy.
- Reasoning-signature replay (§4.7): confirm per-provider echo requirements
  against live endpoints in M6 so multi-turn thinking is correct.
