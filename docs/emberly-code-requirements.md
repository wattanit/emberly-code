# Emberly Code AI Coding Harness — Requirements Document

**Version:** 0.4 
**Status:** approved 
**Date:** 2026-07-09
**Owner:** Wattanit
**Companion documents:** Design Guideline v0.4 (downstream), Technical
Specification v0.2 (downstream)

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

### 2.1 In scope for v1

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
- Context management: tool-result truncation at ingestion, manual
  compaction, visible context-usage indicator.
- Engine/frontend separation via an event model (TUI is the only v1
  frontend, but the boundary must exist).
- Thai text support as content: Thai input and display everywhere
  content appears (input, conversation, file content, diffs, titles),
  with grapheme-cluster-aware text layout and cursor handling (Thai
  combining marks are zero-width; layout must not assume one character
  equals one column). Interface text itself is English in v1.

### 2.2 Explicitly deferred (designed-for, not built in v1)

- **MCP client support.** The internal tool abstraction must permit a future
  MCP adapter, but no MCP implementation ships in v1.
- **Automatic compaction.** v1 ships truncation-at-ingestion and manual
  `/compact` only. Auto-compaction is a fast-follow once the summarization
  prompt is validated through real use.
- Headless / server / IDE frontends (enabled by the event-model boundary,
  not built).
- **User theming.** v1 ships a single built-in theme; all colors live in
  one centralized theme definition so a theme system later is a data
  change, not a refactor.
- **Windows support.** No Landlock/Seatbelt equivalent exists; shipping
  a platform in a weaker safety tier is declined for v1.

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
- **P-2.** v1 must support at minimum: (a) the Anthropic Messages API,
  (b) one OpenAI-compatible endpoint. Requirement (b) transitively covers
  Ollama, vLLM, OpenRouter, and most local/private deployments, which is a
  deliberate requirement given private-deployment use cases.
- **P-3.** The abstraction must be proven by at least two live backends
  before v1 ships (an abstraction with one implementation is untested).
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

## 5. Tool Suite (v1)

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
  commands (`ls`, `cat`, `rg`, `git status`, `git diff`, `cargo check`,
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

## 9. Architecture Requirements

(Behavioral requirements only; structure belongs to the Technical Spec.)

- **A-1 — Engine/frontend separation.** A core engine emits typed events
  (assistant text delta, tool call requested, tool result, permission
  needed, context/compaction status, errors) and accepts commands. Frontends
  consume events and issue commands. The v1 TUI is one such frontend; the
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
- Auto-compaction trigger threshold and rollout criteria (fast-follow after
  v1; requires validated summarization prompt).

Resolved since v0.1: product/command name (Emberly Code / `emberly`,
Design Guideline §1.1); default bash allowlist initial contents (Tech
Spec §6.1); release target list (Tech Spec §13); Windows posture
(deferred, §2.2).
