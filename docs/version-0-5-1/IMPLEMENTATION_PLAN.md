# Emberly Code — Implementation Plan (0.5.1 feature set)

**Status:** 📝 **Phase 0 closed (2026-08-16) — Phase 1 may begin.** All three
foundation documents below are `approved` (owner, 2026-08-16); this plan's
G-14 pin is no longer provisional.
**Date:** 2026-08-16
**Owner:** Wattanit
**Source documents** (G-14 as-built pin):
- Requirements Document v0.12 (`docs/emberly-code-requirements.md`) — WHAT/WHY
  — **Status: approved**.
- Design Guideline v0.12 (`docs/emberly-code-design-guideline.md`) — UX/voice
  — **Status: approved**; covers FR-10/FR-11/FR-12/C-8 (§4.14, §4.15, §5.3,
  §8.10, §8.11).
- Technical Specification v0.14 (`docs/emberly-code-tech-spec.md`) — HOW
  — **Status: approved**; covers FR-10/FR-11/FR-12/C-8 (§4.1, §5.6, §8.5,
  §8.6, §9, M13).

This plan expands Technical Specification §15 with a new milestone, **M13**,
covering three capabilities graduated together in Requirements v0.12: MCP
client support (FR-11), user-attached image input (FR-10), and session export
(FR-12). It builds on the shipped 0.5 product (M12 — the multi-agent
subsystem) and everything beneath it. The prior plans are preserved in
`docs/version-0-1/` … `docs/version-0-5/` as the earlier as-built records.

Unlike the 0.5 set (one integrated capability, phased by dependency), this
set is **three independent slices**, the same shape as the 0.4 feature set:
each capability ships and tests on its own, and two of the three (image
attach, session export) have no dependency on each other or on MCP. Only MCP
client support is large enough to need its own internal dependency-ordered
sub-phases (tool-layer contract → engine machinery → surface), mirroring how
0.5 phased the multi-agent subsystem. Phases are ordered smallest-and-most-
contained first, so the release has shippable slices early rather than one
large capability landing last.

---

## Guiding principles (apply to every phase)

- **Requirements → Design → Spec, in that order (SFD write order).** No
  phase below drafts implementation against a foundation document that is
  still `draft` for the behavior it needs. Phase 0 exists precisely to close
  this gap before any engineering phase starts.
- **Reuse over reinvention (mirrors the 0.5 plan's own first principle).**
  User-attached images are a frontend affordance over the multimodal content
  path that already exists (P-11) and the same content block the read-image
  tool (T-12) already produces — no provider or engine change. Session export
  is a read-only renderer over the transcript and derived conversation-state
  cache that already exist (§8.2, §8.8) — it is not a new persistence
  mechanism and must never become one. MCP-sourced tools ride the existing
  `Tool` trait (T-7) and the existing `ToolRegistry`/permission-gate
  machinery — a subagent-style parallel tool system for MCP would repeat the
  mistake the 0.5 plan explicitly warned against for subagents.
- **No privileged path around the safety model (FR-11 honesty clauses).**
  An MCP-sourced tool call is permission-gated, sandbox-confined, and
  workspace-trust-governed exactly like a built-in tool call. A
  project-declared MCP server is trust-gated exactly like a project-resident
  skill (FR-7) before the harness will even connect to it. `emberly-sandbox`
  is untouched by this entire plan — nothing here is security-critical
  containment; it adds surface *through* the existing rule/permission/trust
  layers, the same posture the 0.4 and 0.5 plans held.
- **No vendor SDK crates (P-4's own rule, extended to MCP).** The harness's
  own provider clients are thin first-party HTTP clients specifically to
  avoid vendor-SDK churn and dependency risk (P-4); the same reasoning
  applies to the MCP client itself. The default plan is a **first-party,
  minimal JSON-RPC-over-stdio client** built on dependencies already in the
  tree (`tokio::process`, `serde_json`) rather than pulling in a third-party
  MCP SDK crate — Phase 3 confirms this is feasible for the transport(s)
  Requirements scopes in (§13 open question) before committing to it; if it
  is not, pulling in a vetted MCP crate is an explicit, named dependency
  addition subject to Dependency Policy (§10) sign-off, not a default.
- **Every phase leaves a shippable, tested slice**, green on
  `cargo build/test/clippy(-D warnings)/fmt` at each phase boundary.

---

## Phase 0 — Foundation document updates (prerequisite, not engineering) — ✅ closed 2026-08-16

**Goal:** Bring Design Guideline and Technical Specification up to date with
Requirements v0.12, and get all three documents to `approved`, before any
Phase 1–6 work starts. This phase is SFD drafting work, not code.

**Closed.** Owner approved all three documents on 2026-08-16 with no
requested changes. Phase 1 may begin.

**Scope (as landed)**
- **Requirements v0.12** — approved (FR-10, FR-11, C-8, FR-12, and the
  §2.1/§2.2/§13 updates).
- **Design Guideline v0.12** — approved. The attach gesture (`/attach`,
  file-picker, recognized drag-drop paste) and its degradation, with true
  clipboard-image-byte paste explicitly declined (§4.14, FR-10); MCP tools
  rendered as quiet tool activity with server provenance and a sidebar
  inspector (§4.15), the MCP tool-call permission prompt (§5.3), and the
  connection trust-gate/failure voice (§8.10, FR-11); the export command's
  voice and its one-time, non-blocking sensitive-content disclosure line
  (§8.11, FR-12).
- **Technical Specification v0.14** — approved. New milestone **M13**;
  MCP as a first-party JSON-RPC-over-stdio client (§5.6) — stdio only this
  version (§13 open question resolved for now) — with the
  `mcp__<server>__<tool>` namespacing convention and server config (§8.5,
  C-8); user-attached images needing no new `ContentBlock` variant, told
  apart from a model-read image structurally by transcript event (§4.1);
  session export as a read-only, self-contained HTML renderer over existing
  transcript/derived-view state, subagent transcripts included (§8.6); all
  three confirmed to add **no new dependency** (§12).

**Done when:** all three foundation documents are `approved` at their
current versions (Requirements v0.12, Design v0.12, Tech Spec v0.14) — ✅
done, this header's pin above is confirmed, no caveat remains.

---

## Phase 1 — User-attached image input (FR-10) — ✅ code-complete 2026-08-16

**Goal:** Let the user attach an image to a prompt through the TUI, entering
context over the existing multimodal content path (P-11) with no provider or
engine change.

**Landed as built:**
- `emberly-tools`: shared `image::encode_image_bytes` extracted from
  `read_image` (T-12) so both paths enforce identical format/size rules —
  `read_image.rs` refactored to call it, no behavior change (existing tests
  unchanged and still passing).
- `emberly-core`: `Command::AttachImage{path}` validates and stages an image
  into new engine state, `Engine::pending_attachments` — deliberately *not*
  a new field on `Command::UserInput`, to avoid a ~130-callsite mechanical
  change across `engine_loop.rs`'s test suite; `UserInput` drains the
  staged list into the outgoing message unchanged in shape. On a vision
  model the image becomes the identical `ContentBlock::Image` T-12 already
  produces; on a non-vision model, a plain text note takes its place — never
  a silent drop (P-11/HC-6 honesty clause). New `UiEvent::ImageAttached` /
  `AttachFailed`. `TranscriptEvent::UserMessage` gained an additive `images`
  field (metadata only — name/dimensions/format, never the base64 bytes,
  matching T-12's own `ToolResult` precedent) so a resumed/exported session
  still shows what was attached. `image.max_attachments` (default 4)
  threaded through config exactly like `image.max_bytes`.
- `emberly-tui`: `/attach <path>` (both frontends — rich TUI and plain/line
  mode); a new `ConvItem::Attachment` chip renders right after the user's
  message in the timeline (`📎 name · WxH · FORMAT attached`), distinct from
  tool-activity styling; `seed_history` replays it from the transcript's
  `images` field on resume; a compose-area `pending_attachments` list
  confirms staged images before send and is cleared on send or session
  switch.

**Scope cut from the original plan (disclosed, not silent):** no
paste-from-clipboard or drag-and-drop recognition, and no file-picker
overlay, in this pass — `/attach <path>` is the only surface. Both are
still open per Design §4.14/§10 and Requirements §13; adding them later is
a frontend-only addition, no engine change (the same reuse property this
phase already banked for `read_image`).

**Done when:** `cargo build/test/clippy(-D warnings)/fmt --check` clean for
the whole workspace — ✅ all green (`emberly-core`: 5 new engine-level
tests incl. the vision/non-vision/cap/oversize/transcript-metadata cases;
`emberly-tui`: 4 new tests for `/attach` parsing in both frontends and the
stage-then-send chip). **Not done:** a real vision-capable provider smoke
test — this environment has no interactive TUI session or live provider
credentials to run one; flagged here rather than assumed passing.

---

## Phase 2 — Session export (FR-12) — ✅ code-complete 2026-08-16

**Goal:** Let the user export a complete session — including any subagents it
spawned (FR-9) — to a portable, human-readable file, without adding any new
persistence mechanism.

**Landed as built:**
- `emberly-core`: new `export` module — `render_session_html` (first-party
  HTML string building, no templating dependency) walks a session's
  `TranscriptRecord`s directly (not the model-facing derived `Message` view,
  which drops too much — tool calls/results, permission decisions, mode
  changes) and renders each event in its existing agent-world/harness-world
  register. `collect_subagent_transcripts` reads every `*.jsonl` under
  `<sessions_dir>/<session_id>/subagents/`, best-effort-labeled by name via
  a small parser over the parent's own `spawn_agents` tool-result text
  (falls back to the bare id — never load-bearing). Usage/cost summary reads
  the existing derived-view cache (`try_load_view_cache`, §3.2a) when fresh;
  when it's missing or stale, the export says so honestly rather than
  fabricating a number — a real, disclosed limitation of the pre-existing
  cache design (usage/cost was never persisted to the transcript itself),
  not something this phase could fix without expanding scope.
- `Command::ExportSession{path}` (engine-issued at idle, mirroring
  `AttachImage`): reads the session's own live transcript path
  (`active_path`) and subagents directory read-only, writes the rendered
  HTML, and emits `UiEvent::SessionExported` on success or a plain `Notice`
  on an ordinary I/O failure (HC-3 — never a crash). `path` is user-named
  directly, so — like `AttachImage` — no project-root confinement or
  permission prompt applies.
- `emberly-tui`: `/export <path>` in both frontends, sharing one
  `strings::export::SENSITIVE_CONTENT_NOTE` constant for the Design §8.11
  disclosure line so both frontends print it verbatim.
- `emberly` (binary): `emberly export [--session <id>] <output-path>` CLI
  command, sharing `emberly_core::render_session_html`/
  `collect_subagent_transcripts` with the in-session path — no divergent
  logic between the two entry points, as the Tech Spec required. Manually
  smoke-tested end to end against a synthetic transcript (unlike Phase 1,
  this needed no live provider — output verified: correct HTML escaping,
  tool call/result rendering, and the honest "usage unavailable" fallback).

**Scope cut from the original plan (disclosed, not silent):** no
output-location picker in either frontend — `/export <path>` is the only
in-session surface, the same cut Phase 1 made for `/attach`. Still open per
Design §4.14/§10 pattern; adding one later is frontend-only.

**Done when:** `cargo build/test/clippy(-D warnings)/fmt --check` clean for
the whole workspace — ✅ all green. 3 new engine-level tests (writes HTML +
never mutates the source transcript, checksummed before/after; preserves a
truncation marker rather than claiming false completeness; an unwritable
output path is a plain notice, never a crash), 5 new `export` module unit
tests (rendering, HTML-escaping, subagent-section labeling, missing-subagents-
dir honesty), 3 new CLI-layer tests, 4 new TUI tests (both frontends'
`/export` parsing and the `SessionExported` disclosure line) — 15 new tests
total. CLI command manually smoke-tested end to end.

---

## Phase 3 — MCP tool-layer contract (`emberly-tools`)

**Goal:** Define the trait boundary for MCP-sourced tools, following the
exact `SubagentGate`/`MemoryGate` pattern already established in this crate
— compiling, tested, and inert (stub gate, every call fails closed) until
Phase 4 wires a real connection in.

**Scope**
- Request/outcome types for connecting to a configured server, discovering
  its tools, and invoking one; a fail-closed `McpError`; an `McpGate` trait.
- A `DropMcpGate` default, exactly like `DropSubagentGate`/`DropMemoryGate`.
- The namespacing convention Phase 0 named, enforced at this layer so a
  collision is a structured construction-time failure, never a silent
  overwrite of a built-in tool's registry entry.
- Unit tests: fail-closed behavior surfaces as `ToolOutcome::failure` (HC-6),
  never a panic; namespacing collision cases.

**Done when:** `cargo build/test/clippy/fmt -p emberly-tools` clean; no
engine wiring yet, proving the contract is correct in isolation.

---

## Phase 4 — MCP engine machinery (`emberly-core`)

**Goal:** Make Phase 3's contract real — connect to configured servers,
discover their tools, register them into the tool registry, and route calls
through the existing permission/sandbox/trust layers with no privileged
path.

**Scope**
- `C-8` config: server profiles (name, transport, command/args or endpoint +
  auth), two-tier-plus-project resolution (C-1), provenance (C-3).
- **Workspace-trust gating.** A project-tier server profile is neither
  connected to nor surfaced from an untrusted folder (FR-1), checked before
  the harness spawns/dials anything — mirroring the skill-loading trust
  check (FR-7) exactly.
- **Connection + transport.** Per Phase 0's chosen transport(s): a
  first-party client per this plan's "no vendor SDK" principle, or a named,
  vetted dependency if Phase 3/4 discovers the first-party path is not
  viable (flagged to the owner before adding it, per Dependency Policy §10).
- **Tool registration.** Discovered tools enter the same `ToolRegistry` every
  built-in and subagent-filtered registry already uses, namespaced per
  Phase 3's convention, each still subject to the ordinary permission
  gate (§6) — no new permission mechanism.
- **Untrusted-content labeling.** A tool result from an MCP server is
  labeled and treated as untrusted content, mirroring T-14's web-search
  result handling exactly.
- **Audit trail (extends HC-7).** Connection lifecycle, tool discovery, and
  every call/result are transcript events, no exemption.

**Done when:** `cargo build/test/clippy/fmt -p emberly-core` clean; an
integration test connects a fake local MCP server (stdio), discovers a tool,
invokes it under a granted permission rule, and confirms an untrusted-folder
project profile is refused pre-connection.

---

## Phase 5 — MCP TUI/config surface

**Goal:** Make MCP connections and their tools visible and configurable from
inside a running session — no silent background capability.

**Scope**
- `emberly init` scaffolds a commented `[mcp.servers.*]` block (C-2 pattern).
- A sidebar/inspector surface (Design Guideline's Phase-0 decision) showing
  connected servers, their discovered tools, and connection status.
- Permission-prompt provenance line naming the originating server for an
  MCP-sourced tool call, mirroring the subagent provenance line the 0.5 TUI
  phase already added.
- Live config reload for `[mcp.servers.*]`, matching the `[agents]` reload
  behavior from 0.5 Phase 2.

**Done when:** `cargo build/test/clippy/fmt -p emberly-tui` (and `emberly`)
clean; a manual smoke test connects a real local MCP server end to end and
confirms the tool is invocable, visible, and its permission prompt correctly
attributes the originating server.

---

## Phase 6 — Hardening, docs, and verification

**Goal:** Close the Tech Spec M13 checklist, document all three capabilities,
and leave the workspace green end to end — the same closing shape as 0.5's
own Phase 4.

**Scope**
- End-to-end tests: a real MCP tool call through the full permission/sandbox
  stack; a project-declared server refused from an untrusted folder; an
  image attached and round-tripped through a `FakeProvider`; a session
  export including a subagent, checked byte-for-byte against the source
  transcript for non-mutation.
- README documentation for image attach, MCP server configuration and the
  `/mcp`-style surface, and the export command.
- A full workspace pass: `cargo build/test/clippy --all-targets -D
  warnings/fmt --check`, plus `cargo vet` if Phase 4 added any new
  dependency.
- Requirements/Design/Spec version pins closed out in this plan's header
  (already current from Phase 0, reconfirmed here as the release's as-built
  record, per G-14).

**Done when:** all of the above are green and the release is tagged.
