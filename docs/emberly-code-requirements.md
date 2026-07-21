# Emberly Code AI Coding Harness — Requirements Document

**Version:** 0.9    
**Status:** approved
**Date:** 2026-07-21    
**Owner:** Wattanit    
**Companion documents:** Design Guideline v0.9 (downstream), Technical  
Specification v0.10 (downstream)

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

Added in the 0.4 feature set (capability parity with mature coding harnesses;
each item carries an ID and full statement in the section cited; this list is
the scope overview, not the requirement):

- A model-maintained, user-visible task list ("todo") for planning and
tracking multi-step work within a session (§5, T-11).
- Multimodal image input: the provider abstraction carries image content
(§4, P-11), and a tool lets the model read an image file within the project
into context (§5, T-12).
- A persistent memory system: durable facts, held in a harness-owned store,
that auto-load into context each session and are written by the model
through a memory tool (§7.1, FR-6; §5, T-13).
- A skill system: named, progressively-disclosed capability folders the model
discovers and invokes on demand (§7.2, FR-7; §5, T-15).
- A web-search tool reaching a harness-owned, configurable search backend —
provider-agnostic, permission-gated, its results treated as untrusted
content (§5, T-14).
- Pointer (mouse) interaction in the TUI — scroll and selection where the
terminal provides it, additive to keyboard control and never the sole path
to any function. Its patterns and degradation are an interaction decision
and belong to the Design Guideline; Requirements holds no separate ID for
it (routing: interaction patterns are Design's, not Requirements').

Added in the 0.4.1 feature set (capabilities requested by TREEGAL Yggdrasil,
a separate product that consumes Emberly as its engine, and absorbed on their
own domain-agnostic merits — not by any upstream/downstream obligation; each
item carries an ID and full statement in the section cited; this list is the
scope overview, not the requirement):

- A completion gate: named pass/fail checks a frontend, tool, or config
registers for a session, which block the loop's own claim of "done" while any
check fails and halt to the user after bounded failed attempts — the
loop-control sibling of S-5 (§11, S-6).
- Document (PDF) input: the provider abstraction carries document content
blocks (§4, P-12), and a tool lets the model read a project document into
context (§5, T-16) — the P-11/T-12 image pattern applied to documents.

Added in the 0.4.2 feature set (each item carries an ID and full statement in
the section cited; this list is the scope overview, not the requirement):

- Guided provider/model setup: a step-by-step in-app flow to add a new
provider profile — including its API key — without hand-editing
`config.toml` or `keys.toml` (§7, C-7).

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
- **User-attached images.** Current scope ingests images the model reads
from the project (T-12); a surface for the user to attach, paste, or drag
an image into a prompt is deferred. The multimodal content path (P-11) is
built so adding an attach surface later is a frontend affordance, needing
no provider or engine change.
- **Provider-native / server-side tools.** Web search is harness-owned and
provider-agnostic by decision (§1, T-14); a provider's own server-side
search (or other server-side tools) is not used, because a capability that
works only on the vendors that offer it is not provider-agnostic. The tool
abstraction (T-7) does not preclude wrapping such a capability later, but it
must never become the only way a capability works.

### 2.3 Out of scope

- Model hosting or fine-tuning.
- Guaranteeing git history immortality (see §6.3 — the harness protects
`.git/` from non-git writes; it does not prevent destructive but
legitimate git operations).
- Deep shell-semantics parsing as a security mechanism (see §6.5).
- Non-PDF document formats (docx and other word-processor formats) as harness
input. PDF is accepted as a provider-native passthrough block (P-12, T-16);
other formats require harness-side conversion or extraction — a dependency and
a domain concern a coding harness has no reason to carry, better handled
outside the harness or as a skill (FR-7). This is a firmer line than a deferred
door: the harness reads the document formats providers accept natively and no
others.
- Document creation and editing. Reading documents into context is a tool
(T-16); producing or modifying documents is a domain capability that ships as
a skill (FR-7), never harness core.

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
- **P-11 — Multimodal (image) input.** The harness's normalized message type
carries image content blocks alongside text; each adapter maps them to its
provider's native image representation. A provider or model without vision
support returns a structured, informative unsupported-capability result
(HC-6) — never a silent drop, never a crash — so the model learns the image
could not be used rather than proceeding as if it had seen it. Where the
provider reports image token usage it feeds token and cost accounting (P-6).
Image support lives behind the Provider abstraction (P-1), not in the tool:
this is what makes the read-image tool (T-12) portable across providers
rather than tied to one vendor.
- **P-12 — Document input.** The harness's normalized message type carries
document content blocks (initially PDF; format list in the Technical
Specification) alongside text and image blocks; each adapter maps them to its
provider's native document representation. The harness does not parse or render
the document — it passes the document bytes through to the provider exactly as
it passes image bytes (P-11), so no document-parsing dependency enters the tree
and HC-2 is untouched. A provider or model without document support returns a
structured, informative unsupported-capability result (HC-6) — never a silent
drop, never a crash. Where the provider reports document token usage it feeds
token and cost accounting (P-6). Document support lives behind the Provider
abstraction (P-1), not in the tool, so the read-document tool (T-16) is
portable across providers rather than tied to one vendor — the same reasoning
as P-11.

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
- **T-11 — Task list ("todo") tool.** A built-in tool the model calls to
maintain an explicit, ordered list of the steps a multi-step task requires,
each with a status (e.g. pending / in-progress / done), updating items as
work proceeds. The list is model-authored planning made visible to the user
(its display is a Design decision); the harness never writes or reorders it.
Like ask-user (T-8) and recall (T-10) it is engine state — the model
organizing its own plan — touching no filesystem or network, so it is **not
permission-gated** (§6). Its purpose is legibility and self-direction on long
tasks: without it a plan lives only in prose the user must reconstruct, and
the model has no durable within-session scratchpad for what remains. The
current task list is session state recorded in the transcript (HC-7); it is
not cross-session memory (that is FR-6).
- **T-12 — Read-image tool.** A built-in tool the model calls to read an image
file within the project root — a screenshot, diagram, or mockup — bringing it
into context as an image content block (P-11). The path is normalized and
root-confined exactly as read_file (T-1), and it is governed by the same
permission rules as any project read (§6.2). Supported formats are named in
the Technical Specification. On a provider without vision it returns the
structured unsupported-capability result of P-11 (HC-6), never a crash. A
coding agent often needs to *see*, not just be told — a UI defect is faster
shown than described. User-supplied images are deferred (§2.2); this tool
covers images already present in the project.
- **T-13 — Memory tool.** A built-in tool the model calls to write, update, and
remove durable memory entries and to recall them on demand. It is the write
path of the persistent memory system (FR-6, §7.1): entries persist across
sessions and the memory index auto-loads at session start. The tool writes
into a **harness-owned, schema-constrained memory store** — the model supplies
fact content, never a filesystem destination path — so it is harness-managed
persistence (the same category as the transcript and the trust store), not an
arbitrary agent write against the user's filesystem; §7.1 states how this
relates to HC-4. Without a memory channel every session starts from zero;
with it, durable facts (user preferences, project conventions, prior
decisions) survive across sessions.
- **T-14 — Web-search tool.** A built-in tool the model calls to search the web
through a **harness-owned, configurable search backend**: a thin first-party
HTTP client (P-4) reaching a configured search endpoint declared as a named
profile (endpoint, auth scheme, API-key reference), mirroring the
endpoint-configurable pattern of P-8 so search stays provider-agnostic (§1)
and is never coupled to a model vendor's server-side capability. It is
**permission-gated** (§6): a model-initiated network egress carrying real cost
and a prompt-injection surface is an action the user governs — default ask,
rule-allowlistable like bash (§6.2). Its results are **untrusted content**:
text fetched from the web may carry instructions hostile to the user, and the
harness treats and labels it as data the model reads, never as instructions to
the harness. Only a genuinely new search wire format requires new code; a new
search service is a new profile. Without web search the agent cannot reach
information beyond its training cut-off; harness-owned search buys that reach
without surrendering provider-agnosticism.
- **T-15 — Skill-invocation tool.** A built-in tool the model calls to invoke a
skill by name (FR-7, §7.2), loading that skill's full instructions (and any
bundled resources) into context on demand. Each skill's name and one-line
description are always in context so the model knows what it can invoke; the
body loads only when invoked — progressive disclosure that keeps the context
cost of carrying many skills near zero until one is used (a context economy in
the spirit of §8). Invoking a skill loads instructions and may make the
skill's bundled scripts runnable; because a skill can carry executable code and
instructions, skill provenance and trust are governed per FR-7 and workspace
trust (FR-1, §6) — a skill has no privileged path around the safety model.
Skills give the harness extensible, shareable capabilities without hard-coding
each one.
- **T-16 — Read-document tool.** A built-in tool the model calls to read a
document file within the project root into context as a document content block
(P-12). The path is normalized and root-confined exactly as read_file (T-1) and
read-image (T-12), governed by the same permission rules as any project read
(§6.2). On a provider without document support it returns the structured
unsupported-capability result of P-12 (HC-6), never a crash. Supported formats
are PDF (size/page/token caps named in the Technical Specification); non-PDF
word-processor formats such as docx are out of scope (§2.3), and document
creation and editing are not built-in tools — those are skills (FR-7), never
core. An agent working against real-world source material must read the formats
that material actually arrives in; a harness that can see images but not
documents draws the line at the wrong place.

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
- **C-7 — Guided provider/model setup.** The user must be able to add a new
provider profile — adapter, endpoint, and the API key it needs — from within
a running session through a guided, step-by-step flow, without hand-editing
`config.toml` or `keys.toml`. The result must be indistinguishable from a
profile configured by hand: it lands in the project config tier (C-1), is
provenance-visible (C-3), live-reloads (C-5), and is immediately selectable
through existing profile switching (C-6). A key entered through the flow
must never be written to project-tier config, never printed to the screen or
a log, and never recorded in the session transcript — the same handling a
hand-placed `keys.toml` entry already gets, with no exemption for the guided
path. Guided setup is an additional path onto the same configuration, not a
parallel system: raw editing (C-5) remains available as the fallback for any
profile shape (per-model metadata, a non-standard auth scheme) the guided
flow's fixed field set does not cover.

### 7.1 Persistent memory

- **FR-6 — Persistent memory.** The harness maintains a durable memory of facts
that survive across sessions: at session start the memory **index** loads into
context automatically, and during a session the model writes, updates, and
removes entries through the memory tool (T-13). Memory is organized as small,
individually-addressable entries — one fact per entry — with an always-loaded
index and on-demand entry bodies, so a large memory costs little context until
a specific entry is consulted (the same progressive-disclosure economy as
skills, §7.2, and in the spirit of §8). Two scopes: **user-global** memory
(facts about the user and how they work, spanning projects) and **project**
memory (facts about this project); both auto-load, and the model and user can
see which scope a fact lives in.
  - **Relation to HC-4 and the permission model (honesty clause).** The memory
  store is **harness-owned and schema-constrained**: the model supplies fact
  content, never a filesystem destination, and the harness performs the write
  into its own store — the same category of harness-managed persistence as the
  transcript (HC-7) and the trust store (FR-1), not an agent-chosen write
  against the user's filesystem. HC-4 governs the agent affecting arbitrary
  files outside the project root; memory never widens it, because memory can
  never target an arbitrary path. Project-scope memory additionally lives
  inside the project root and follows the normal write rules (§6.2). The user
  can inspect, edit, and delete memory directly — memory is never a place the
  harness hides state the user cannot see.
  - **Relation to project instructions (C-1).** Memory is distinct from
  AGENTS.md/CLAUDE.md: those are user-authored instructions the harness reads;
  memory is agent-authored knowledge the harness accumulates. Both load into
  context; neither replaces the other.
  - **Injection honesty clause.** Auto-loading project memory is, like
  auto-loading AGENTS.md (FR-1), a path by which project-resident text enters
  the model's context. Project memory is therefore governed by workspace trust
  (FR-1): memory from an untrusted folder is not loaded. Loaded memory is
  pinned context (§8.3) and what is loaded is always visible to the user.

### 7.2 Skills

- **FR-7 — Skill system.** The harness supports **skills**: named, self-contained
capability folders the model discovers and invokes on demand. A skill is a
folder with a manifest — a name, a one-line description, and an instruction
body — and may bundle supporting resources and executable scripts. Discovery is
progressive: every available skill's **name and description are always in
context** so the model knows what it can reach, while a skill's full
instruction body loads only when the model invokes it (T-15). This keeps the
context cost of carrying many skills near zero until one is used (the same
economy as memory, §7.1, and §8). Skills resolve from a **user-global**
location and a **project** location, so a user carries personal skills across
projects and a project can ship its own.
  - **Trust and safety (honesty clause).** A skill carries **instructions and,
  potentially, executable code** — both a prompt-injection and a
  code-execution surface. Project-resident skills are governed by workspace
  trust (FR-1): skills from an untrusted folder are neither surfaced nor
  invocable. Invoking a skill only loads instructions into context; running a
  skill's bundled script is a command execution governed by the full
  permission and sandbox model (§6), exactly as any other command — a skill
  has no privileged path around the safety model. A skill's origin
  (user-global vs project) is visible so the user can weigh how far to trust
  what it asks the model to do.

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
- **S-6 — Completion gate.** The harness supports registered completion
checks: named checks a frontend, tool, or configuration registers for a
session, each returning pass or fail with a structured reason. While any
registered check fails, the agent loop may not terminate as "done": on a
completion attempt the failing checks return to the model as structured tool
results (HC-6) and the loop continues, or — after a bounded number of failed
completion attempts (threshold tunable; default set in the Technical
Specification) — the harness halts and returns control to the user, in the
harness's own voice, exactly as S-5 does for a non-progressing loop. S-4 stops
a hung child; S-5 stops a spinning loop; S-6 stops a premature landing. A
check that executes a command (e.g. a test suite) runs under the full
permission and sandbox model of §6 like any other command — a completion check
has no privileged path around the safety model, the same principle that governs
a skill's bundled scripts (FR-7). The gate binds only the *model's* claim of
completion: it never blocks the user from ending a session, and a user may
always stop over a failing gate. Every gate evaluation, its result, and its
reasons are transcript events (HC-7). A session with no registered checks
behaves exactly as today — the gate is inert until something registers into it.
Honesty clause: the gate governs the loop's claim of completion, not the truth
of the checks; a check is only as good as what it verifies, and a passing gate
is never presented to the user as a guarantee beyond what the registered checks
actually tested.

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
- Web-search backend default, auth-scheme coverage, and result count/snippet
limits (T-14). Tech Spec sets the initial profile shape and caps; tune with
use.
- Supported image formats and per-image size/token caps (T-12, P-11). Tech
Spec sets the initial set; tune with use.
- Memory entry format, index size cap, and the point at which the auto-loaded
memory itself needs the §8 context economy (FR-6). Tech Spec sets initial
values; tune with use.
- Skill manifest format and the resolution/precedence rule between user-global
and project skills (FR-7). Tech Spec sets the initial scheme; tune with use.
- Whether the task list is pinned across compaction (§8.3) or may be dropped
and re-read like other turns (T-11). Tech Spec sets the initial policy; tune
with use.
- Completion-gate check registration mechanics, gate-evaluation timing, and the
default number of failed completion attempts before the harness halts to the
user (S-6). Tech Spec sets initial values; tune with use so the gate stops a
premature landing without recreating an S-5 spin.
- Supported document formats beyond PDF and per-document size/page/token caps
(P-12, T-16). Tech Spec sets the initial set; tune with use.
- Guided provider setup's exact wizard-supported field/auth-scheme coverage,
and whether it can edit an existing profile or only create new ones (C-7).
Tech Spec sets the initial scope; tune with use.

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

Resolved since v0.6 (0.4 feature set): the six 0.4 capabilities were routed
and scoped as follows. Web search resolved as a **harness-owned, configurable
backend** (provider-agnostic, permission-gated, results treated as untrusted),
not provider-native server-side search — chosen to keep §1
provider-agnosticism real rather than nominal (T-14). Image ingestion resolved
as **model-reads-a-project-file** (T-12) over user-attach; user-attached images
are deferred with a designed-for door on the P-11 content path (§2.2).
Multimodal support placed behind the Provider abstraction (P-11), not the tool,
so it is portable across vendors. Memory resolved as an **auto-loaded index plus
model-written entries** in a harness-owned, schema-constrained store, user-global
and project scope; its HC-4 relationship is resolved as harness-managed
persistence (the model supplies content, never a path), not an agent filesystem
write (FR-6, T-13). Skills resolved as **progressive-disclosure folders**
(name/description always in context, body on invoke) that may bundle scripts,
governed by workspace trust (FR-1) and the full permission/sandbox model
(FR-7, T-15). The task list resolved as non-permission-gated engine state, like
recall and ask-user (T-11). Mouse interaction resolved as a **Design-owned
interaction capability** — additive to keyboard control, never the sole path to
a function — carrying no separate Requirements ID (routing: interaction is
Design's, §2.1).

Resolved since v0.7 (0.4.1 feature set): three capabilities requested by
**TREEGAL Yggdrasil** — a separate product that consumes Emberly as its engine,
not an SFD-downstream document — were assessed on their own domain-agnostic
merits (Yggdrasil is a consumer, so absorbing these is a choice to serve the
engine's users, never an upstream obligation) and routed as follows. The
**completion gate** is absorbed as **S-6** (§11), sibling to S-5: registered
pass/fail checks gate the loop's claim of "done," with a bounded-attempts halt
to the user so an unsatisfiable check cannot recreate the S-5 spin it exists to
stop. Two honesty clauses were added on absorption — a check that runs a command
has no privileged path around §6, and the gate binds only the model's
done-claim, never the user's ability to stop. **Document input** is absorbed as
**P-12** (§4) and **T-16** (§5), the P-11/T-12 image pattern applied to
documents: PDF only, passed through to the provider unparsed so HC-2 is
untouched. Non-PDF formats (docx) are declined to §2.3 as harness-side
conversion a coding harness need not carry, and document creation/editing stay
skills (FR-7), never core. The **Windows supervised-posture** request is
**held, not absorbed**: the requesting consumer targets macOS only for its
prototyping stage, so the scope reversal — and the §3 honesty work it would
require, since HC-4/HC-5 become structurally policy-level-only on a platform
with no confinement implementation — is deferred until a real platform need
exists. A fourth request (compile-time tool profiles) was withdrawn by the
requester before absorption. S-6, P-12, and T-16 are the IDs the Yggdrasil
foundation suite will cite as their origin when drafted (G-24/G-25).
