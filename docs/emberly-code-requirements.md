# Emberly Code AI Coding Harness — Requirements Document

**Version:** 0.6 
**Status:** approved
**Date:** 2026-07-11
**Owner:** Wattanit
**Companion documents:** Design Guideline v0.6 (downstream), Technical
Specification v0.7 (downstream)

This document defines WHAT the harness must do and WHY. HOW it is built is
deferred to the Technical Specification. UX, visual, and voice decisions are
deferred to the Design Guideline. Where this document constrains
implementation (e.g. dependency policy), it does so because the constraint is
itself a requirement, not an implementation preference.

---

## 1. Purpose and Motivation

An interactive AI coding agent for the terminal, comparable in function to
Claude Code: the user states tasks in natural language; the agent reads,
edits, and creates files, runs commands, and iterates until the task is done
or the user intervenes.

The product exists for three reasons, in priority order:

1. **Full source control.** The entire harness is owned, auditable, and
   modifiable in-house. No dependence on the continued existence, licensing,
   or governance of third-party open-source harnesses.
2. **Safety and transparency.** Every agent action is bounded by an
   enforceable permission and sandbox model, and every action is recorded in
   a complete audit trail. The agent's operating instructions are inspectable.
3. **Stability.** The harness must run reliably on standard and non-standard
   stacks (including musl/static deployments) without crashes originating in
   the harness itself.

Provider-agnosticism is a first-class requirement: the harness must not be
structurally coupled to any single model vendor.

## 2. Scope

### 2.1 In scope

- Interactive terminal session (single project directory per session).
- Multiple model provider backends behind a common internal abstraction.
- Built-in tool suite: file read, file write, file edit, bash, glob, grep.
- Permission system with hard safety boundaries, rule-based convenience
  tiers, and escalating auto-accept modes.
- OS-level sandboxing of command execution (Linux: Landlock; macOS:
  Seatbelt).
- Two-tier agent configuration (baked-in defaults, project-level overrides)
  plus project-instruction file compatibility (AGENTS.md native; CLAUDE.md
  read for cross-harness compatibility).
- Session persistence as append-only JSONL transcripts; session resume.
- Context management: a layered token-economy strategy — tool-result
  truncation and salient reduction at ingestion, an adaptive context window,
  manual and automatic compaction, and a visible context-usage indicator
  (§8).
- Engine/frontend separation via an event model (the TUI is the only
  frontend in current scope, but the boundary must exist).
- Thai text support as content: Thai input and display everywhere
  content appears (input, conversation, file content, diffs, titles),
  with grapheme-cluster-aware text layout and cursor handling (Thai
  combining marks are zero-width; layout must not assume one character
  equals one column). Interface text itself is English in current scope.

Added in the 0.2 feature set (each carries an ID and full statement in the
section cited; this list is the scope overview, not the requirement):

- In-app configuration and prompt editing, and in-app model / provider /
  reasoning-effort selection, without leaving the session (§7, C-5, C-6).
- A workspace-trust consent gate before the agent operates in a folder the
  user has not previously trusted (§6.8, FR-1).
- Additional model providers reachable purely by configuration through the
  wire-format adapters that already exist; per-session reasoning-effort
  selection; and capture and display of a model's reasoning trace where the
  provider exposes one (§4, P-8, P-9, P-10).
- A model-initiated ask-user tool for decision points the model cannot
  resolve itself, and a model-authored per-call explanation line for
  non-obvious tool calls (§5, T-8, T-9).
- A loop-breaking guardrail that halts a non-progressing agent loop and
  returns control to the user (§11, S-5).

Added in the 0.3 feature set (efficiency and cost optimization: the token
and dollar cost of a long session is itself a product concern, addressed as
a layered context-management strategy over §8; each item carries an ID and
full statement in the section cited; this list is the scope overview, not the
requirement):

- Deterministic tool-result token reduction beyond size-based truncation —
  keep the salient content in context, the full result in the transcript
  (§8.5, FR-2).
- An adaptive context window that carries recent turns and keeps older ones
  retrievable rather than always sending the whole conversation (§8.6, FR-3).
- Automatic compaction at a high context-usage threshold, graduating the
  automatic-compaction door kept open in v0.1's §2.2 (§8.7, FR-4).
- Token-efficient session resume from a derived conversation-state cache
  rather than re-ingesting the full audit transcript (§8.8, FR-5).

### 2.2 Explicitly deferred (designed-for, not yet built)

- **MCP client support.** The internal tool abstraction must permit a future
  MCP adapter, but no MCP implementation ships yet.
- Headless / server / IDE frontends (enabled by the event-model boundary,
  not built).
- **User theming.** Current scope ships a single built-in theme; all colors live in
  one centralized theme definition so a theme system later is a data
  change, not a refactor.
- **Windows support.** No Landlock/Seatbelt equivalent exists; shipping
  a platform in a weaker safety tier is declined (current scope).

### 2.3 Out of scope

- Model hosting or fine-tuning.
- Guaranteeing git history immortality (see §6.3 — the harness protects
  `.git/` from non-git writes; it does not prevent destructive but
  legitimate git operations).
- Deep shell-semantics parsing as a security mechanism (see §6.5).

## 3. Hard Constraints

These are non-negotiable and testable. A release that violates any of them
is defective by definition.

- **HC-1 — Safe Rust in first-party code.** All first-party crates carry
  `#![forbid(unsafe_code)]`. Dependencies may contain `unsafe` internally;
  they are accepted on the basis of ecosystem standing and formal audit
  records (see §10).
- **HC-2 — No C dependencies in the default build.** Pure-Rust dependency
  tree: `rustls` (not OpenSSL bindings), pure-Rust crypto, no `libgit2`
  bindings (shell out to the git CLI instead). The default build must
  produce fully static musl-targeted binaries with no libc surprises on
  every supported architecture (e.g. `x86_64-unknown-linux-musl`,
  `aarch64-unknown-linux-musl`). Scope clarification: this constraint
  bans third-party C libraries linked or bundled; OS-provided system
  interfaces are exempt (on macOS there is no path to the kernel except
  Apple's system libraries).
- **HC-3 — Panic-free core.** Core crates enforce
  `#![deny(clippy::unwrap_used, clippy::expect_used)]`; all fallible paths
  return `Result`. A top-level supervisor catches anything truly
  unexpected, persists the session transcript, and exits cleanly. The
  harness must never lose a session to a crash.
- **HC-4 — Project-root confinement.** No agent-initiated action may affect
  the filesystem outside the project root without explicit per-action user
  permission. No convenience rule, session grant, or auto-accept mode can
  waive this. Enforced at the sandbox layer, not (only) the rule layer.
- **HC-5 — `.git/` protection.** Nothing spawned by the agent may write to
  `.git/` except the genuine git binary. File tools hard-refuse `.git/`
  paths with no ask option. No mode can waive this. Enforced at the sandbox
  layer.
- **HC-6 — Tool failures are data, not crashes.** Every tool failure (file
  not found, non-unique edit match, command timeout, permission denial)
  returns to the model as a structured, informative tool result. The
  harness never surfaces a tool failure as a harness error.
- **HC-7 — Complete audit trail.** Every session is recorded as an
  append-only JSONL transcript containing all messages, tool calls, tool
  results, permission requests and decisions, mode changes, and compaction
  events. The transcript is never rewritten or pruned; the in-context
  conversation is a derived view over it.

## 4. Provider Support

- **P-1.** All providers are accessed through a single internal abstraction
  (a `Provider` trait) with the harness's own normalized message, tool-call,
  and streaming-event types. No provider-specific type leaks past the
  provider layer.
- **P-2.** The harness must support at minimum: (a) the Anthropic Messages API,
  (b) one OpenAI-compatible endpoint. Requirement (b) transitively covers
  Ollama, vLLM, OpenRouter, and most local/private deployments, which is a
  deliberate requirement given private-deployment use cases.
- **P-3.** The abstraction must be proven by at least two live backends
  before release (an abstraction with one implementation is untested).
- **P-4.** Provider clients are thin, first-party HTTP clients (`reqwest` +
  `serde`). Vendor SDK crates are not used — they churn and reintroduce the
  third-party-dependence risk this project exists to avoid.
- **P-5.** Streaming responses are required for all supported providers.
- **P-6.** Per-provider token accounting sufficient to drive the context
  usage indicator and compaction threshold. Approximate counting (e.g.
  chars/4) is acceptable where exact tokenizers are impractical; the
  requirement is a reliable trigger, not exact counts. Token accounting
  also drives a per-session cost estimate from configurable per-model
  pricing; cost figures are always labeled as estimates.
- **P-7.** Per-model prompt variants must be supported by the configuration
  system (prompts tuned for one model family underperform on others).
- **P-8 — Endpoint-configurable adapters.** Every provider adapter accepts a
  configurable endpoint — base URL plus authentication scheme — so any
  service that speaks a wire format an adapter already parses is reachable
  by configuration alone. Adding such a provider is a named config profile
  (which adapter, base URL, auth, model list, pricing) and requires no code;
  only a genuinely new wire format requires a new adapter. This is what
  makes provider-agnosticism (§1) real rather than nominal: P-1's
  normalization only pays off if reaching a new endpoint costs configuration,
  not engineering. The immediate driver is the Z.ai coding plan, reached
  through whichever wire format it speaks; the harness holds no
  vendor-specific knowledge beyond that profile.
- **P-9 — Reasoning effort.** For models exposing a reasoning-effort or
  thinking-budget control, the user must be able to select the effort level,
  per session and switchable within a session; available levels and the
  default are determined by the active model. A model without such a control
  ignores the setting — setting it is never an error. Reasoning effort trades
  latency and cost against answer quality, and the user, not a compiled-in
  default, owns that trade for their task.
- **P-10 — Reasoning trace.** Where a provider streams the model's
  reasoning/thinking content distinctly from its answer, the harness must
  capture it, surface it as distinct from the answer, and record it in the
  transcript (HC-7). Reasoning content is never silently discarded and never
  rendered as if it were the answer. Its display treatment is a Design
  decision. Transparency (§1.2) covers how the model reached an action, not
  only the action.

## 5. Tool Suite

- **T-1 — Read file.** Bounded output (see §8.1 truncation).
- **T-2 — Write file.** Creates or replaces a file.
- **T-3 — Edit file.** Exact string match-and-replace (not line-number
  based). Failure messages must state precisely why an edit failed (no
  match / multiple matches) — model recovery quality depends on this.
- **T-4 — Bash.** Executes shell commands under the sandbox and permission
  model of §6, with timeouts.
- **T-5 — Glob.** File pattern matching within the project root.
- **T-6 — Grep.** Content search (ripgrep-class) within the project root.
- **T-7.** The tool abstraction must be defined such that a future MCP
  adapter can present external tools through the same interface without
  engine changes.
- **T-8 — Ask-user tool.** A built-in tool the model may call to ask the user
  a question and receive a typed answer before continuing, for genuine
  decision points the model cannot resolve itself (ambiguous requirements, a
  choice the user owns). It presents the question and, where the model
  supplies them, discrete options; the user's answer returns to the model as
  a structured tool result. It is a model-initiated pause, distinct from a
  permission prompt (which guards an action) and from the loop guardrail
  (S-5, harness-initiated). Without it the model either guesses or stalls; an
  explicit "I need your input" channel is safer and clearer than either.
- **T-9 — Tool-call explanation.** Each tool call may carry a short,
  model-authored, human-readable line stating what the call does and/or why,
  surfaced alongside the call. It is filled only for calls whose intent is
  not self-evident — the semantic intent of an opaque command cannot be
  derived by the harness and must come from the model. This serves
  transparency and in-situ learning (§1.2) at a cost of a few output tokens
  per non-obvious call; it must be defeatable by configuration for users who
  do not want the tokens spent.
- **T-10 — Recall tool.** A built-in tool the model may call to bring earlier
  conversation turns — those dropped from the working window by the adaptive
  context window (FR-3, §8.6) — back into context on demand. It is the
  explicit, model-driven counterpart to windowing: the harness never
  auto-expands the window or guesses when old history matters; when the model
  needs a dropped turn, it asks. Recall returns the requested turns in the
  harness's normalized, reduced form (the same economy as §8.1/§8.5), never
  the raw transcript, so recovering history costs tokens proportional to what
  is recalled, not to the transcript's raw size — a recall path that re-inflated
  the elided noise would defeat the window it serves. It reads only the current
  session's own history and touches no filesystem or network, so it is **not
  permission-gated** (§6): re-reading conversation the model itself produced
  and already saw is not an action against the project, and the permission
  model guards actions against the project, not the model reading its own
  context. Without a recall channel the adaptive window would be lossy
  amnesia; with it, the window is a token economy the model can reverse when a
  task genuinely needs the older context.

## 6. Permission and Safety Model

### 6.1 Two-layer design

Safety is enforced in two distinct layers with different jobs:

- **Sandbox layer (hard containment).** OS-level filesystem and network
  confinement (Landlock on Linux, Seatbelt on macOS) enforcing HC-4 and
  HC-5. This layer is not user-configurable per rule and is not waivable by
  any mode.
- **Rule layer (convenience).** User-facing allow/ask/deny rules over
  (tool, argument-pattern) pairs that decide when to prompt. This layer
  improves ergonomics; it is not the security boundary and must never be
  described as one.

### 6.2 Default rule behavior

- Reads inside the project root: **allow**.
- Writes and edits inside the project root: **ask**, with an
  "allow for this session" option offered at the prompt.
- Bash: prefix-match against a default allowlist of harmless read-only
  commands (`ls`, `cat`, `grep`, `rg`, `git status`, `git diff`, `cargo check`,
  and similar), user-extensible in project config. Everything else: **ask**.
- Any action affecting paths outside the project root: **ask**, always,
  per-action, regardless of rules or mode (HC-4).

### 6.3 `.git/` special-casing

- File tools (write/edit): hard **deny** for any path under `.git/`, with
  explanation, no ask option.
- Bash: only invocations resolved to the genuine git binary (prefix-matched
  AND binary-resolved, so aliases and lookalikes do not qualify) receive a
  sandbox profile permitting writes under `.git/`; all other commands run
  under a profile that denies it.
- Honest limitation, to be stated in user docs: this guarantees "only git
  touches git's data," not immortal history. `git reset --hard` and
  `git push --force` remain possible via legitimate git. Remotes and reflog
  are the user's lifeline; out of harness scope.

### 6.4 Modes

Escalating auto-accept tiers, all bounded by HC-4 and HC-5 (which live at
the sandbox layer and are not part of the rule system any mode can relax):

- **Normal:** prompts per §6.2 rules.
- **Auto-accept edits:** file writes/edits inside the project root
  auto-allow; bash still follows the rules.
- **Auto mode:** allowlisted and session-granted bash also auto-runs;
  anything outside the project root or off-list still asks.

Mode changes are transcript events (HC-7). Auto-accept modes are
available only while OS-level confinement is active (see §6.7).

### 6.5 Rule-layer honesty

Bash prefix rules are convenience-tier. The harness must not attempt deep
shell-semantics parsing (subshells, command chaining, expansions) as a
security mechanism — it cannot win and breeds false confidence. Real
containment is the sandbox layer.

### 6.6 Denials and persistence

- A user denial returns to the model as a structured tool result ("user
  denied this command"), never a silent drop, so the agent can route around
  it.
- Per-project rule grants persist in project config (e.g.
  `.agents/permissions.toml`); session-only grants are held in memory.
- Every permission request, the user's answer, and what actually executed
  are transcript events (HC-7).

### 6.7 Sandbox degradation

OS-level confinement may be unavailable on some hosts (e.g. kernels
without Landlock support or with the LSM disabled at boot). The required
behavior — principle: **make the risk clear, tighten the convenience
away, stay usable**:

- Sandbox status is probed at startup and is always-visible state
  (session header, sidebar, transcript). Absence or partial availability
  is explicitly notified to the user in plain language; the harness never
  silently pretends confinement exists.
- Without confinement: auto-accept modes (§6.4) become unavailable and
  the default bash allowlist is suspended (every command asks).
  Per-action prompting continues normally — the harness remains fully
  usable.
- The hard lines (HC-4, HC-5) continue to be enforced at the rule/tool
  layer, honestly labeled as policy-level rather than kernel-level
  protection.
- An opt-in config (`sandbox.require = true`) refuses to start sessions
  without kernel confinement, for environments where degraded operation
  is unacceptable. The default is permissive-but-honest.
- Sandbox unavailability must never crash the harness or degrade the
  harness process itself; confinement applies to spawned children only.

### 6.8 Workspace trust

- **FR-1 — Workspace trust gate.** Before the agent may read, edit, or
  execute anything in a project directory the user has not previously
  marked trusted, the harness asks the user, in plain language, to confirm
  they trust the folder, and does not start the agent loop until they
  consent. Declining does not start a session. The trust decision is
  remembered per canonical folder path, extends to that folder's subtree
  (a trusted folder does not re-prompt per subdirectory), and is revocable.
  The user may also pre-declare trusted directories in configuration to
  skip the prompt. The trust record and any pre-declaration live outside
  the project (so a repository cannot pre-declare itself trusted). Opening a coding agent on unfamiliar code is itself a risk —
  malicious instructions can live in project files (`AGENTS.md`/`CLAUDE.md`,
  source comments, build scripts); a one-time explicit trust decision makes
  taking that risk a conscious act rather than a silent default.
- Honesty clause: trust is a **consent gate, not containment**. A trusted
  folder still operates under the full permission and sandbox model
  (§6.1–§6.7); trusting a folder never widens HC-4 or HC-5, never suspends
  a permission prompt, and never enables an auto-accept mode. Trust governs
  *whether* the agent runs here at all; it never governs *what* it may do
  once it does.

## 7. Configuration ("brain" configuration)

- **C-1 — Two tiers plus project instructions.** Resolution order:
  1. Baked-in defaults compiled into the binary (usable out of the box).
  2. Per-file overrides in `.agents/prompts/` (overriding the system prompt
     does not force the user to also maintain tool descriptions).
  3. Project instructions appended on top — always additive, never
     replacing lower tiers. `AGENTS.md` is the harness's native
     instruction file; `CLAUDE.md` is also read for cross-harness
     compatibility. If both exist, `AGENTS.md` takes precedence and
     `CLAUDE.md` is ignored (with a notice), to avoid appending duplicated
     or conflicting instructions.
- **C-2 — Init materialization.** Defaults are hidden until the user runs an
  explicit `init` command (or flag), which writes the active defaults into
  `.agents/prompts/` for inspection and tuning. The harness never
  auto-creates files in a repository on first run.
- **C-3 — Provenance visibility.** The harness must be able to report which
  tier every active configuration piece came from (at session start and/or
  via a `--show-config` command). "What is my agent actually running on"
  must always be answerable.
- **C-4.** Prompts and tool descriptions are treated as behavior-critical
  configuration, versionable per model family (P-7).
- **C-5 — In-app configuration and prompt editing.** The user must be able
  to view and edit the active configuration and prompt files from within a
  running session, without dropping to a separate shell. Edits take effect
  on the running session (with any restart-only changes named as such at the
  point of editing). Editing continues to honor the tier model (C-1) and
  provenance (C-3): the user always sees which tier a value comes from
  before changing it, and edits land in the project tier, never silently
  mutating baked-in defaults. Configuration a user cannot find is
  configuration a user cannot trust; in-app editing closes the gap between
  "the harness is configurable" and "I can actually change it."
- **C-6 — In-app model, provider, and effort selection.** The user must be
  able to switch the active provider/model and the reasoning-effort level
  (P-9) from within a running session, choosing among configured provider
  profiles (P-8). A switch applies to subsequent turns, is a transcript
  event (HC-7), and never rewrites prior turns. This makes model choice a
  per-task decision the user owns mid-session, not a launch-time
  commitment.

## 8. Session Persistence and Context Management

### 8.1 Truncation at ingestion

Oversized tool results are truncated at the moment they are appended to the
conversation — before they enter context. Keep head and tail, elide the
middle with an explicit marker (e.g. `[... 4,200 lines elided ...]`), and
reference where the full output lives so the model can re-read a specific
portion on demand. Deterministic, per-event, no model call. This is the
first line of context defense and applies to bash output and file reads
alike.

### 8.2 Transcript as ground truth

- One append-only JSONL file per session. Event types include: user
  message, assistant text, tool call, tool result (full, pre-truncation
  where feasible, or with truncation noted), permission request/decision,
  mode change, compaction event, session lifecycle events.
- The in-context conversation is a derived view over the transcript.
  Compaction and truncation modify the view, never the log.
- Sessions are resumable from their transcript, including after a crash
  (HC-3 guarantees the transcript is persisted on abnormal exit).
- Session metadata includes an auto-generated, user-renamable session
  title.

### 8.3 Manual compaction (`/compact`)

- User-invoked. The model summarizes everything except the last N turns
  using a purpose-built prompt (original task, decisions made, files
  modified and how, current state, next steps); the conversation view is
  rebuilt as [system prompt] + [summary] + [recent turns verbatim].
- **Pinned content never compacted:** system prompt, project instructions
  (AGENTS.md or CLAUDE.md, whichever is active per C-1), the
  original task statement.
- Compaction may occur only at clean message boundaries (every tool_use has
  its tool_result).
- If the summarization call fails, fall back to hard truncation of oldest
  turns with a visible warning — a full context must never produce a stuck
  session.
- The compaction event (summary text + range of turns replaced in the view)
  is recorded in the transcript.

### 8.4 Context usage indicator

A context-usage percentage is always visible to the user (status line),
driven by P-6 token accounting against the active model's window minus a
reserved output budget. Invisible context exhaustion is a defect.

The 0.3 requirements below (§8.5–§8.8) are a layered context-economy
strategy, cheapest and most conservative first: size truncation (§8.1) →
salient tool-result reduction (§8.5) → an adaptive window over turns (§8.6) →
compaction, manual (§8.3) and automatic (§8.7). Each layer only changes what
is *sent to the model*; none rewrites or prunes the transcript (HC-7). §8.8
carries the resulting economy across a resume.

### 8.5 Tool-result token reduction

- **FR-2 — Tool-result token reduction.** Beyond the size-based truncation
  of §8.1, a tool result may be reduced to its salient content before it
  enters the model context, keeping the parts that inform the model's next
  step and eliding the noise, while the complete unreduced result is written
  to the transcript (HC-7) and remains retrievable on demand (via the §8.1
  reference marker). Reduction is deterministic and adds no model call to the
  agent loop — a context defense that itself costs a model round-trip or
  per-tool-call latency defeats its own purpose. What counts as salient is
  per tool (e.g. a build's diagnostics and its summary line, not its progress
  chatter; a matcher's hits, not the directories it walked). The reduction is
  marked in context so the model knows content was withheld and can re-read
  the full result. A reduction that discards information the model then cannot
  recover is defective — the full result is always one reference away. This
  refines §8.1: §8.1 bounds size blindly (head/tail); FR-2 reduces by meaning
  where a tool's output has a known salient shape.

### 8.6 Adaptive context window

- **FR-3 — Adaptive context window.** The model context need not carry every
  past turn verbatim: the harness maintains a working window of the most
  recent turns, and older turns may be dropped from the context sent to the
  model while remaining in the transcript and retrievable. The harness does
  not guess when old history matters — dropping is reversible by the model:
  when the model needs older context it recalls it through the **recall tool
  (T-10)**, which returns the dropped turns in normalized, reduced form. Recall
  reads only the session's own history, so it is not permission-gated (T-10,
  §6). Pinned content (§8.3 — system prompt, project instructions,
  original task) is never dropped from the window. The window is a token and
  cost economy for long sessions, distinct from compaction (§8.3, §8.7):
  windowing drops-but-keeps-retrievable, compaction summarizes. The window
  size is tunable (Technical Specification sets an initial default; §13).
  Honesty clause: a windowed session is not a lossy session — every dropped
  turn is in the transcript and one re-read away; the window changes what is
  sent, never what happened (HC-7).

### 8.7 Automatic compaction

- **FR-4 — Automatic compaction.** The harness automatically compacts (§8.3)
  when context usage crosses a high threshold of the budget (§8.4), without
  waiting for the user to invoke compaction manually — a long-running session
  must not stall, overflow, or silently degrade because the user did not act
  on the indicator. Automatic compaction uses the same mechanism, the same
  pinned-content rules, the same clean-boundary rule, and the same
  summarization-failure fallback as manual compaction (§8.3); it differs only
  in its trigger. It is **on by default**; the threshold is tunable and
  automatic compaction is disableable in configuration for users who prefer to
  compact manually only. Every automatic compaction is a transcript event
  (HC-7) and is surfaced to the user (Design Guideline) — the context changing
  under the model is never silent. This graduates the automatic-compaction
  item formerly deferred in §2.2 through its named door (see §13, "Resolved
  since v0.5").

### 8.8 Efficient session resume

- **FR-5 — Efficient session resume.** Resuming a session must not cost the
  tokens of re-ingesting the full audit transcript. The harness persists the
  session's derived conversation state — the in-context view, already the
  product of truncation (§8.1), reduction (§8.5), windowing (§8.6), and
  compaction (§8.3, §8.7) — so a resume restores that state directly rather
  than replaying and re-tokenizing the entire append-only log. HC-7 honesty
  clause: the transcript remains the **sole** ground truth; the persisted
  conversation state is a derived cache, always reconstructable from the
  transcript, and resume falls back to full transcript replay (see the
  Technical Specification's resume section) whenever the cache is absent,
  stale, or unreadable — the optimization never becomes a second source of
  truth, and losing the cache never loses a session. The economy is
  observable: resuming a long session consumes context proportional to its
  working view, not to its full history.

## 9. Architecture Requirements

(Behavioral requirements only; structure belongs to the Technical Spec.)

- **A-1 — Engine/frontend separation.** A core engine emits typed events
  (assistant text delta, tool call requested, tool result, permission
  needed, context/compaction status, errors) and accepts commands. Frontends
  consume events and issue commands. The TUI is one such frontend; the
  boundary must make headless/IDE/server frontends possible without engine
  changes.
- **A-2 — Testability.** The engine must be drivable by a fake frontend and
  a fake provider in tests. Recorded JSONL transcripts should be replayable
  as test fixtures for harness behavior.
- **A-3 — Serializable event model.** All engine events are serializable
  from day one (this is what makes HC-7 and A-2 cheap rather than bolted
  on).

## 10. Dependency Policy

- Acceptance criteria for third-party crates: widely depended upon
  ("famous crates" standard), preferably covered by importable `cargo vet`
  audit records (Google, Mozilla, Rust project audits), pure Rust per HC-2.
- `cargo vet` (and/or `cargo-geiger` reporting) runs in CI; new dependencies
  require explicit acceptance.
- Honest external claim, verbatim policy: "first-party code is 100% safe
  Rust; dependencies are vetted" — never "no unsafe anywhere in the
  binary."

## 11. Stability and Quality Requirements

- **S-1.** No segfaults or panics attributable to the harness on supported
  targets, explicitly including non-glibc stacks (musl/static builds are a
  supported first-class target, per HC-2).
- **S-2.** Abnormal termination of any kind must persist the session
  transcript before exit (HC-3).
- **S-3.** Provider/network failures degrade gracefully: surfaced to the
  user, recorded in the transcript, session remains resumable.
- **S-4.** Command execution always runs under timeouts; a hung child
  process must not hang the harness.
- **S-5 — Loop-breaking guardrail.** The harness must detect when the agent
  loop is no longer making progress — repeating substantially the same
  tool calls, or iterating without changing state — and, on detection, halt
  the loop and return control to the user rather than continue indefinitely.
  This is the model-driven analog of S-4: S-4 stops a single hung child;
  S-5 stops a spinning agent from burning cost, tokens, and time with
  nothing to show. The halt is harness-initiated and surfaced in the
  harness's own voice, distinct from the model-initiated ask-user tool
  (T-8). Detection is a heuristic and cannot be perfect; the requirement is
  that a runaway loop always ends in a user decision, never in silent
  unbounded spend. The detection thresholds are tunable configuration
  (defaults set in the Technical Specification); the guardrail must never
  interrupt a loop that is genuinely progressing.

## 12. Deliverables and Document Plan

1. **Requirements Document** — this document.
2. **Design Guideline** — next: name/branding, `init` experience, prompt
   and status-line aesthetics, error voice, help text tone, color/unicode
   policy for constrained terminals, streaming/markdown rendering policy.
   Written before the Technical Spec because several of these decisions
   constrain the event model and TUI crate.
3. **Technical Specification** — last: crate/workspace layout (core,
   providers, tools, tui, thin cli binary), event model definition,
   sandbox implementation per OS, dependency list, build/release pipeline.

## 13. Open Questions (carried into later docs)

- N (verbatim recent turns kept by compaction) and truncation head/tail
  sizes (Tech Spec sets initial defaults; tune with use).
- Loop-guardrail detection heuristic — what precisely counts as "no
  progress," and the default thresholds (S-5). Tech Spec sets initial
  values; tune with use.
- Adaptive-window default size (FR-3). Tech Spec sets the initial default;
  tune with use.
- Per-tool "salient content" rules and defaults for tool-result reduction
  (FR-2). Tech Spec sets initial rules; tune with use so reduction never
  hides what the model needs.
- Automatic-compaction default threshold (FR-4). Tech Spec sets the initial
  value; tune with use so it fires before overflow without compacting too
  eagerly.

Resolved since v0.1: product/command name (Emberly Code / `emberly`,
Design Guideline §1.1); default bash allowlist initial contents (Tech
Spec §6.1); release target list (Tech Spec §13); Windows posture
(deferred, §2.2).

Resolved since v0.4: model-authored tool-call explanation, considered and
deferred during v0.1, now graduated to in scope (T-9); "more model
providers" resolved as endpoint-configurable adapters + config profiles
rather than per-vendor code (P-8), with the Z.ai coding plan as the first
profile; workspace trust scoped as a consent gate, not multi-root access
or per-folder rules (FR-1); in-app editing to cover both a TUI overlay and
`$EDITOR` handoff (C-5); ask-user tool (T-8) and loop guardrail (S-5)
confirmed as separate mechanisms; workspace-trust storage follows the
established harnesses — user-global, keyed by canonical path,
subtree-trusted, with a pre-trust allowlist and an explicit
`emberly trust revoke` (FR-1, Tech Spec §6.7); tool-call explanation on by
default and config-defeatable (T-9); reasoning trail defaults to collapsed
(P-10, Design §4.4).

Resolved since v0.5 (0.3 feature set): automatic compaction graduated from
the §2.2 deferred tier to in scope through its named door — on by default at
a high threshold, threshold tunable, disableable (FR-4, §8.7), which also
retires the v0.1 "auto-compaction trigger threshold and rollout criteria"
open question (threshold is now a tunable default set in the Tech Spec).
Compaction was found to be an existing requirement (§8.3 user-invoked
compaction, plus Design §3.3's three-way reachability), so the missing
trigger is an implementation gap to close in v0.3, not a new requirement.
Session-resume efficiency scoped as a derived, rebuildable conversation-state
cache subordinate to the transcript, never a second ground truth (FR-5, §8.8,
HC-7 preserved). Tool-result token reduction scoped as deterministic, with no
model call added to the agent loop (FR-2, §8.5). Adaptive context window
scoped as deterministic windowing plus model-driven re-read of dropped turns,
distinct from compaction (FR-3, §8.6). The model's re-read affordance is
resolved as a dedicated **recall tool (T-10)** returning normalized, reduced
turns — chosen over reusing `read_file` on the raw transcript, which would
re-inflate the elided noise in verbose JSONL and so defeat the window's token
economy; recall is engine-internal (no filesystem/network) and not
permission-gated.
