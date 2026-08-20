# Emberly Code — Implementation Plan (0.5.1 feature set)

**Status:** ✅ **All six phases code-complete (2026-08-19).** All three
foundation documents below are `approved` (owner, 2026-08-16); this plan's
G-14 pin is no longer provisional. Release not yet tagged.
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

## Phase 3 — MCP tool-layer contract (`emberly-tools`) — ✅ code-complete 2026-08-16

**Goal:** Define the trait boundary for MCP-sourced tools — compiling,
tested, and inert (no registry wiring) until Phase 4 wires a real
connection in.

**Architectural refinement from the original plan (disclosed, not silent):**
the plan as written called for a `McpGate`/`DropMcpGate` pair mirroring
`SubagentGate`/`MemoryGate`. Building it out, that pattern turned out not to
fit: a `*Gate` exists because a *fixed, statically-registered* built-in tool
(`memory`, `spawn_agents`) needs `ToolCtx` to carry engine-owned state that
doesn't exist until runtime. An MCP-sourced tool is the opposite shape — an
unbounded, dynamically-discovered set, and each one only exists once a real
connection has already named it (Phase 4). There is nothing to gate a *call*
through in the meantime, so a `Drop`-style fail-closed default would be
inert by definition, not a meaningful contract. What Phase 3 actually needed
to fix — and does — is the shape Phase 4 builds against: a `McpTransport`
trait, the namespacing/collision rule, and a generic `Tool` proxy
constructed directly over a transport handle, fully testable now against a
fake transport with no gate indirection at all. This is a plan-level
correction, not a Tech Spec conflict — §5.6 only ever committed to "a thin
proxy implementing the `Tool` trait," which this satisfies exactly.

**Landed as built (`emberly-tools/src/mcp.rs`):**
- `McpTransport` trait — `async fn call(&self, method, params) -> Result<Value, McpError>` — exactly the Tech Spec §5.6 shape, so Phase 4's stdio
  client is a drop-in implementer.
- `McpError` (`thiserror`, `Connect`/`Protocol` variants) — mapped to a
  structured `ToolOutcome::failure` by `McpTool::execute`, never a panic
  (HC-6).
- `namespaced_tool_name(server, tool) -> "mcp__<server>__<tool>"` (Requirements
  §13 resolved) and `build_mcp_tools(server, specs, transport)`, which
  detects a same-server name collision as a structured `McpError` naming
  both tools before any `McpTool` is even constructed.
- `McpTool: Tool` — a thin proxy: `execute` calls `transport.call("tools/call", …)` and maps the result to `ToolOutcome::success(..).with_untrusted()`
  (mirroring `web_search`'s untrusted-content tagging, T-14/Design §4.15) or
  a structured failure. `describe()` names the original tool and its server.

**Done when:** `cargo build/test/clippy(-D warnings)/fmt --check -p
emberly-tools` clean — ✅ all green. 6 new unit tests against a fake
`McpTransport`: namespacing, collision detection, successful proxy + untrusted
tagging + correct `tools/call` params, transport-failure-as-structured-
failure, and `describe()`. No engine wiring yet — proven correct in isolation,
exactly as planned.

---

## Phase 4 — MCP engine machinery — ✅ code-complete 2026-08-16

**Goal:** Make Phase 3's contract real — connect to configured servers,
discover their tools, register them into the tool registry, and route calls
through the existing permission/sandbox/trust layers with no privileged
path.

**Architectural refinement from the original plan (disclosed, not silent):**
the plan named this `emberly-core` alone. Following the exact precedent
`web_search` already set (Tech Spec §5.5) — the *binary* conditionally builds
the tool registry, not the engine — connecting/discovering/registering MCP
servers landed the same way, in `emberly` (`provider_setup::build_tool_registry`/
`connect_mcp_servers`), not inside `Engine`. `emberly-core` supplies the
reusable pieces: `McpClient` (the transport) and the `mcp_connections`
reporting path. This keeps one construction pattern for "a tool that needs
external setup before it can be registered," rather than two.

**Landed as built:**
- **`emberly-core::McpClient`** (`src/mcp_client.rs`) — a first-party,
  newline-delimited JSON-RPC-over-stdio client (`tokio::process` +
  `tokio::io` + `serde_json`, **no new dependency** beyond enabling the
  already-present `tokio` crate's `io-util` feature). `spawn()` performs the
  `initialize` handshake; `list_tools()` wraps `tools/list`; calls are
  serialized behind one lock (correctness over throughput this version,
  Requirements §13) with a 30s response timeout (S-4's "never hang"
  principle, extended to an external process).
- **`C-8` config** (`emberly/src/config.rs`): `[mcp]` + `[mcp.servers.<name>]`,
  merged per-server-name exactly like `[providers.<name>]`; each resolved
  server carries a `project_scoped` flag — `true` only when *that name*
  appears in the project tier's own parsed file (global-tier servers are
  never trust-gated), mirroring the project-skill trust distinction (FR-7).
- **Workspace-trust gating** (`provider_setup::connect_mcp_servers`): a
  project-scoped server is filtered out *before* any spawn is attempted when
  the workspace isn't trusted — silent, correct absence, matching skills.
- **Tool registration**: `emberly_tools::build_mcp_tools` results register
  into the same `ToolRegistry` `default_registry()`/`web_search` already use
  — no parallel registry, no new permission mechanism (an MCP tool call
  flows through the ordinary `ToolCtx` gate, ordinary rule engine).
- **Audit trail** (`emberly-core`): a new `TranscriptEvent::McpConnection`
  (additive) plus `UiEvent::McpServerConnected`/`McpServerFailed`, emitted
  once at session start (`Engine::report_mcp_connections`, reading
  `EngineConfig.mcp_connections` — populated by connecting *before* the
  engine exists, same timing as `web_search`) and again on every `/reload`
  (`connect_mcp_servers` reconnects fresh, matching `web_search`'s own
  fresh-client rebuild on reload — `ReloadedConfig.mcp_connections`).
  MCP tool calls themselves need no special-casing: they're ordinary
  `tool_call`/`tool_result` transcript events like any tool (HC-7, no
  exemption needed because none was ever a gap).
- **A real bug found and fixed while wiring this up**: `main.rs`'s local
  `trust_granted` variable is actually `newly_trusted` (true only on a
  *fresh* grant this run, used correctly elsewhere to gate the one-time
  `TrustDecision` transcript write) — not "is this workspace trusted." Since
  `build_tool_registry` only ever runs after the pre-engine trust gate
  already returned `Proceed`, trust holds unconditionally by the time it's
  called, whether newly granted or already on record — the same fact
  project skills/memory already load under without re-checking. Passing
  `newly_trusted` there silently skipped every project-scoped MCP server on
  any session that didn't *just* trust the folder this run (caught by a
  manual end-to-end smoke test, not by unit tests, since the unit tests
  correctly exercised `connect_mcp_servers`'s own trust parameter in
  isolation — the bug was one level up, at the call site). Fixed with an
  explicit `workspace_trusted = true` binding at the call site, commented
  with why reaching that line already proves it.

**Done when:** `cargo build/test/clippy(-D warnings)/fmt --check` clean for
the whole workspace — ✅ all green. New tests: 2 `McpClient` tests in
`emberly-core` (one spawns a real local Python script speaking MCP JSON-RPC
end to end: initialize → tools/list → tools/call; one confirms a
nonexistent command is a structured connect error). 4 integration tests in
`emberly`'s `provider_setup` (connect + register + **invoke the registered
tool under a genuinely granted permission rule**; a project-scoped server
refused with zero connection attempts when untrusted; the same server
connecting once trust is granted; an unsupported transport as a structured
failure). Beyond the automated suite: a full manual end-to-end smoke test
of the real `emberly` binary — a temp project with `[mcp.servers.echo]`
pointing at a real local Python MCP server, pre-trusted via
`trust.trusted_dirs`, run non-interactively — confirmed the
`mcp_connection` transcript record with the correctly namespaced
`mcp__echo__echo` tool name appears in the real session transcript. This
smoke test is what caught the `trust_granted` bug above.

---

## Phase 5 — MCP TUI/config surface

✅ code-complete 2026-08-19

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

**Landed as built**
- `emberly init`'s `CONFIG_TEMPLATE` gained a commented `[mcp.servers.myserver]`
  example block, matching the existing `[agents.*]` pattern (C-2).
- Rich TUI: `App.mcp_servers: Vec<McpServerSummary>` mirrors the existing
  `agents` catalog exactly — populated by `UiEvent::McpServerConnected`
  (upsert by name, so a `/reload` reconnect refreshes rather than
  duplicates) and `UiEvent::McpServerFailed` (retain-filter, so a dropped
  reconnect never leaves a stale "connected" row). A no-empty-stub "MCP"
  sidebar section (mirroring "Agents") and an `OverlayContent::McpServerList`
  inspector (`/mcp`, ↑/↓, Enter, Esc/q) round out the surface —
  `crates/emberly-tui/src/app/mcp.rs`.
- Plain mode: `/mcp` and `/mcp <name>` in `line.rs` mirror `/agents`/
  `/agents <name>` exactly, plus rendering lines for both `UiEvent`s.
- **Architectural refinement vs. the plan**: unlike `/agents <name>` and
  `/skills <name>`, selecting a server (Enter, or `/mcp <name>`) needs **no
  engine round-trip at all** — a server's discovered tool list is already
  fully known from the connect-time event, so the inspector opens a
  read-only text overlay directly from cached state. No `Command::InspectMcp`
  exists; this is simpler than the plan implied, not a scope cut.
- Permission-prompt and tool-activity-line provenance turned out to need
  **no new rendering code at all**: both already render generically from
  `Tool::describe()` and `PermissionRequest.summary`/`.detail`, which
  `McpTool` already populates with the originating server's name (Phase 3).
  Confirmed by reading `render.rs` rather than by writing anything new.
- **A real bug found and fixed before building on top of it**: `McpTool::execute`
  (`crates/emberly-tools/src/mcp.rs`, from Phase 3) never called
  `ctx.authorize()` at all — its own doc comment and Requirements FR-11/Tech
  Spec §5.6 both claim MCP tool calls are permission-gated "exactly like a
  built-in tool's," but the `ctx` parameter was unused. Found by re-reading
  the module while verifying that claim, not by a failing test. Fixed by
  adding the `PermissionRequest`/`ctx.authorize()` call (denied → a
  structured `ToolOutcome`, transport never invoked); added
  `execute_is_permission_gated_a_denial_is_structured_never_a_call` plus an
  `AllowGate` alongside the existing `DenyGate` to keep the success-path
  tests passing.

**Done when:** `cargo build/test/clippy(-D warnings)/fmt --check` clean for
the whole workspace — ✅ all green (269 `emberly-tui` tests, +16 from this
phase: 8 in `app/tests.rs` mirroring the Agents inspector suite, 8 in
`line.rs` mirroring the Agents plain-mode suite). A manual end-to-end smoke
test of the real compiled `emberly --plain` binary — a temp project with
`[mcp.servers.echo]` pointing at a real local Python MCP server, pre-trusted
via `trust.trusted_dirs` — confirmed: the `mcp_connection` transcript record
appears with the namespaced `mcp__echo__echo` tool name; `/mcp` lists
"echo (1 tool)"; `/mcp echo` shows the tool read-only with no engine round
trip. "Tool is invocable" and "permission prompt attributes the originating
server" were verified through the existing automated suite rather than
re-run live here: `provider_setup`'s Phase-4 integration test already
invokes a registered MCP tool (spawned from a real local server) under a
genuinely granted permission rule through the same `build_tool_registry`
path the binary uses, and `mcp.rs`'s new denial test confirms the transport
is never called when `ctx.authorize()` refuses — triggering that same call
through the compiled binary would need a live model/provider, out of scope
for this smoke test. One test-harness observation, not a product bug: piping
`/mcp` into stdin with zero delay can race ahead of the async connect
notice (the command sees an empty catalog and says so correctly) — a
timing artifact of unbuffered piped input, not reachable by an actual user
who can't type before the first prompt renders.

---

## Phase 6 — Hardening, docs, and verification

✅ code-complete 2026-08-19

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

**Landed as built**
- **Three of the four end-to-end tests already existed** from earlier
  phases and were re-verified rather than duplicated: `provider_setup.rs`'s
  `mcp_server_connects_and_registers_its_tool` (Phase 4 — a real spawned
  MCP server, its tool invoked through the ordinary `ToolCtx::authorize`
  permission gate) and its project-scoped-trust siblings (Phase 4) cover
  the MCP/permission and untrusted-folder cases; `engine_loop.rs`'s
  `attach_then_send_appends_image_block_matching_read_image` (Phase 1)
  already round-trips a real attached image through a `FakeProvider` and
  asserts the outgoing request carries the image block. Only the fourth was
  a genuine gap: existing export tests checked subagent inclusion and
  non-mutation as two *separate*, in-memory (`Vec`-equality) checks against
  `render_session_html` directly. Added
  `export_includes_a_real_subagent_and_never_mutates_either_source_file`
  (`crates/emberly/src/export.rs`) — real parent + subagent `.jsonl` files
  on disk, run through the actual `emberly export` CLI path, asserting the
  output HTML contains both and that each source file's raw bytes are
  byte-for-byte unchanged afterward.
- **No sandbox-specific MCP test was added.** Re-reading Tech Spec §5.6's
  own claim ("no tool call here supplies a filesystem path or bypasses the
  permission/sandbox model") clarified this is a *structural* fact, not a
  behavior to exercise: `McpTool::execute` never touches a path or the
  `Sandbox` trait at all — it only calls `ctx.authorize()` then the
  transport — so there is nothing sandbox-specific for an MCP test to
  cover beyond what Phase 4/5's permission tests already do. §12's phrase
  "the exact same rule engine and sandbox confinement as any other tool
  call" holds vacuously, by construction, not by a new integration path.
- README: version banner and Project status bumped to v0.5.1; a new
  Release history row (M13, all three FRs); an **MCP servers** config
  subsection (`[mcp.servers.*]`) and a companion **Using connected MCP
  servers** features subsection; `/attach`/`/export`/`/mcp` added to the
  Commands table; an **Exporting a session** paragraph and `emberly export`
  in the command-line reference; image-input split into **model-initiated**
  (`read_image`) vs **you-initiated** (`/attach`) to keep the existing
  paragraph honest about which is which; Documents section version pins
  corrected from stale v0.10 (Requirements/Design) to the current v0.12/
  v0.12/v0.14, and its phased-build-plan link repointed from a stale
  `version-0-4-3` reference to this release's own plan.
- Full workspace pass: `cargo build`, `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`, and
  `cargo test --workspace --all-features` — all green (`emberly` binary
  tests 62→63; `emberly-tui` unchanged at 269, no TUI changes this phase).
  `cargo vet` was not run: Phase 4 added no new dependency (confirmed by
  diffing `Cargo.lock` against the pre-M13 commit — zero added/removed
  package entries; the one Phase 4 change was enabling tokio's existing
  `io-util` feature), so there is nothing new to vet. `cargo deny check`
  was run as an extra check beyond scope and does fail on two pre-existing
  transitive advisories (a `rustls-webpki` CRL-parsing panic via
  `rustls-rustcrypto`, and unmaintained `yaml-rust` via `syntect`) — both
  confirmed present identically at the pre-M13 commit (`a87f728`), so
  neither is a regression from this feature set; left unresolved as
  out-of-scope for a release that added zero dependencies.
- Requirements v0.12 / Design v0.12 / Tech Spec v0.14 remain the pins this
  plan's header names (Phase 0) — reconfirmed current, no further bump
  needed for M13's as-built record (G-14).

**Done when:** all of the above are green — ✅. The release is **not yet
tagged**; tagging is a release action reserved for explicit instruction.
