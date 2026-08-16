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

## Phase 1 — User-attached image input (FR-10)

**Goal:** Let the user attach an image to a prompt through the TUI, entering
context over the existing multimodal content path (P-11) with no provider or
engine change.

**Scope**
- Paste-from-clipboard and an explicit file-picker/path affordance in the
  TUI input, per Design Guideline's Phase-0 decision; drag-and-drop where
  the terminal protocol supports it, degrading cleanly where it does not.
- Attached images are validated against the same format/size caps as T-12
  (Tech Spec, Phase 0) and converted to the same `ContentBlock::Image` T-12
  already produces — reusing that construction path, not duplicating it.
- A provider/model without vision support returns P-11's existing structured
  unsupported-capability result; attaching never silently drops the image.
- Transcript event for an attached image (extends HC-7) so it is
  indistinguishable, in the audit trail, from any other content block.

**Done when:** `cargo build/test/clippy/fmt` clean for `emberly-tui` and
`emberly-core`; an offline `FakeProvider`-driven test attaches an image and
asserts the resulting content block matches T-12's own; a real vision-capable
provider smoke-tested manually per this repo's existing practice.

---

## Phase 2 — Session export (FR-12)

**Goal:** Let the user export a complete session — including any subagents it
spawned (FR-9) — to a portable, human-readable file, without adding any new
persistence mechanism.

**Scope**
- An exporter reading the existing derived conversation-state / transcript
  (§8.2, §8.8) read-only; no write path back into transcript or cache.
- Renders to the format(s) Phase 0 named (e.g. static, dependency-free HTML)
  using first-party string building — no templating-engine dependency
  unless Phase 0 explicitly named one.
- Includes: conversation, tool calls/results (respecting existing
  truncation/reduction markers, §8.1/§8.5), permission decisions, mode
  changes, and a cost/usage summary (P-6); subagent activity included per
  FR-12's extension of FR-9.
- Surface: a CLI command (`emberly export` or Tech Spec's chosen verb) and,
  per Design Guideline's Phase-0 decision, an in-session command. Output path
  is user-chosen and may fall outside the project root (FR-12 — user-
  initiated, HC-4 does not apply).
- No content redaction, per FR-12's honesty clause; Design's export-time
  warning (Phase 0) is the mitigation, not a filter this phase builds.

**Done when:** `cargo build/test/clippy/fmt` clean; a test exports a session
with a nested subagent transcript and asserts the output contains both,
contains no transcript mutation (checksum the source transcript before/
after), and round-trips a session already reduced by truncation/compaction
without losing the "content elided, N available on demand" markers into
false completeness.

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
