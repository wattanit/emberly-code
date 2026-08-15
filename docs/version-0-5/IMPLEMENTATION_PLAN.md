# Emberly Code — Implementation Plan (0.5 feature set)

**Status:** 🚧 **in progress** — Phase 1 (tool-layer contract) is **done**
(2026-08-15, branch `v0.5-phase1`, merged to `v0.5`). Phase 2 (engine
machinery, branch `v0.5-phase2`) has its **core done and tested**
(2026-08-15): the four multi-agent tools are fully functional end to end,
including the load-bearing permission-proxy correctness property, proven by
five real integration tests — see `docs/version-0-5/PHASE2_TODO.md` for the
precise done/not-done split (TUI event plumbing, cost rollup, `[agents]`
config-file reading, and idle reap remain). Phases 3–4 not started. This
plan was written prospectively, before any code, the normal SFD order
(unlike the 0.4.2/0.4.3 plans, which were written as-built after the fact).
**Date:** 2026-08-15
**Owner:** Wattanit
**Source documents** (G-14 as-built pin for the 0.5 release):
- Requirements Document v0.11 (`docs/emberly-code-requirements.md`) — WHAT/WHY
- Design Guideline v0.11 (`docs/emberly-code-design-guideline.md`) — UX/voice
- Technical Specification v0.13 (`docs/emberly-code-tech-spec.md`) — HOW

These three versions are the as-built truth for the 0.5 release, all
`Status: approved` (owner, 2026-08-15). A stale pin is a defect (G-16); when a
foundation document bumps, update this pin. Discoveries that change HOW flow
back into the Technical Specification as version bumps (G-24/G-25) — this plan
never becomes a shadow spec.

This plan expands Tech Spec §15 milestone **M12** — the multi-agent
subsystem (FR-9, T-18–T-21) — into workable phases. It builds on the shipped
0.4.3 product (M11 — session scratch space) and everything beneath it. The
prior plans are preserved in `docs/version-0-1/` … `docs/version-0-4-3/` as
the earlier as-built records.

Unlike every prior version, the 0.5 set is **one integrated capability**, not
several independently-shippable ones — you cannot ship `spawn_agents` without
the engine machinery that actually runs a subagent, and the TUI surface has
nothing to render until that machinery exists. The phases below are
therefore layered by **dependency**, not sliced by user-facing feature: each
phase is a precondition for the next, and only the last phase produces a
fully working, user-visible capability. This mirrors how the 0.4.3 scratch
space phase was broken into groups (tool → store/engine → CLI → TUI) — scaled
up because this capability is substantially larger.

---

## Guiding principles (apply to every phase)

- **A subagent is a nested `Engine`, not a second implementation (Tech Spec
  §8.4).** The whole plan is organized around reuse: `turn.rs`, `context.rs`,
  `guardrail.rs`, and the completion gate are not reimplemented for
  subagents — a subagent *is* another `Engine` instance, driven by a headless
  `SubagentManager` "frontend" instead of the TUI. Any temptation to special-case
  subagent turn-handling instead of constructing a real nested `Engine` is a
  signal to stop and re-read Tech Spec §8.4.
- **The root engine stays the sole owner of what must stay singular (Tech
  Spec §2, extended).** `RuleEngine`/session grants and the one frontend
  screen are never duplicated per subagent. A subagent's `authorize()` and
  `ask_user()` calls proxy through the root engine's own existing channels,
  tagged with the subagent's id/name — this is the piece most likely to be
  gotten wrong (a subagent with its own independent rule state would be a
  safety regression, not a convenience), so Phase 2 treats it as the load-bearing
  design, not an afterthought.
- **No privileged path around the safety model (Requirements FR-9 honesty
  clause).** Every subagent tool call — bash, write, edit, web search — runs
  under the exact permission/sandbox/workspace-trust posture the session
  already has. `emberly-sandbox` is untouched by this entire plan (Tech Spec
  §12): nothing here is security-critical containment; it adds surface
  *through* the existing rule/permission/trust layers.
- **Depth is structural, not a runtime check.** A subagent's `ToolRegistry`
  never contains `spawn_agents`/`message_agent`/`list_agents`/`end_agent` —
  the no-recursion guarantee (Requirements §2.2) comes from what is
  registered, not from a depth counter that could be gotten wrong at one call
  site and right at another.
- **Transparency, never a black box (Requirements §1, FR-9).** Every
  subagent's activity is inspectable; nothing about it is a silent background
  process. Phase 4's per-agent inspector is not a nicety layered on top — it
  is how FR-9's transparency clause is actually satisfied.
- **Pure-Rust, no new dependencies (HC-2, Tech Spec §12).** `tokio::spawn`,
  `tokio::time::timeout`, and `futures::future::join_all` are already in use
  or already a direct dependency (Tech Spec §12). Any new external crate
  showing up during implementation is a signal to stop and re-check the
  design against the Tech Spec before adding it.
- **Every phase leaves a shippable, tested slice**, green on
  `cargo build/test/clippy(-D warnings)/fmt` at each phase boundary, even
  though only Phase 4 is end-to-end user-visible.

---

## Phase 1 — Tool-layer contract (`emberly-tools`)

**Goal:** Define the `SubagentGate` trait and the four built-in tools against
it, following the exact `MemoryGate`/`ScratchGate` pattern already
established in this crate — schema-constrained, content-fields-never-a-path
where applicable, fail-closed default gate. This phase produces compiling,
tested tool code with a *stub* gate; the tools are inert (every call fails
closed) until Phase 2 wires a real engine-backed gate in.

**Scope**
- **`subagent.rs` (`emberly-tools`).** Request/outcome types for all four
  operations (`SubagentSpawnSpec`, batch spawn request/result, message
  request/outcome, list entry, end outcome), a fail-closed `SubagentError`,
  and the `SubagentGate` trait (`spawn_agents`, `message_agent`,
  `list_agents`, `end_agent`). `DropSubagentGate` — the default installed by
  `ToolCtx::new` — fails every call, exactly like `DropMemoryGate`/
  `DropScratchGate`.
- **Four `builtin/*.rs` tool files**, one per tool (T-18–T-21), each a `Tool`
  impl with its JSON schema and a `describe()` line, calling through
  `ToolCtx` to the gate. `spawn_agents` takes a batch (`agents: [...]`, each
  with `name`, `system_prompt`, optional `profile`/`model`/`tools`) and
  returns per-agent results so one subagent's failure never aborts the
  others (HC-6, Requirements T-18). `message_agent` takes `{id, message}`.
  `list_agents` takes no arguments. `end_agent` takes `{id}`.
- **`ctx.rs`:** a `subagent: Arc<dyn SubagentGate>` field defaulting to
  `DropSubagentGate`, a `with_subagent_gate` builder, and four passthrough
  methods — mirroring every existing gate exactly.
- **Registration:** the four tools added to `default_registry()` and
  exported from `lib.rs`. Registering them in the *default* registry is
  temporary convenience for this phase's own tests; Phase 2 introduces the
  *filtered* registry a subagent itself receives (which excludes these four
  tools by construction).
- **Unit tests:** JSON schema round-trips for all four tools; the default
  (`DropSubagentGate`) fail-closed behavior surfaces as a structured
  `ToolOutcome::failure` (HC-6), never a panic, for every tool.

**Satisfies:** the tool-layer half of T-18–T-21 (Tech Spec §8.4); no engine
behavior yet.

**Done when:** `cargo build -p emberly-tools`, `cargo test -p emberly-tools`,
`cargo clippy -p emberly-tools --all-targets` (`-D warnings`), and
`cargo fmt --check` are all clean, and the four tools are inert (return a
structured "not available" failure) with no engine wiring — proving the tool
layer is correct in isolation before anything depends on it.

---

## Phase 2 — Engine machinery (`emberly-core`)

**Goal:** Make the tools from Phase 1 actually do something: construct a
real subagent as a nested `Engine`, drive it, and route its permission
requests through the root engine. This is the phase that makes multi-agent
delegation real; everything before it is scaffolding and everything after it
is surfacing what this phase built.

**Scope**
- **`engine/subagents.rs` (new, alongside `guardrail.rs`/`skills.rs`/
  `memory.rs`).** `SubagentManager`: owns the map of currently alive
  subagent engine tasks (id → join handle + command channel + metadata),
  implements `SubagentGate` for the root `Engine`.
- **Derived `EngineConfig` construction.** Given a `SubagentSpawnSpec`:
  resolve the provider via the existing `ProviderFactory` (default: the
  parent's own active profile/model); build a filtered `ToolRegistry` (never
  the four multi-agent tools; further filtered to a requested subset,
  validated against the *parent's own* registry — a name absent there is a
  spawn-time structured failure, never a silent grant); compose the system
  prompt as the harness's baked-in scaffold plus the spawn call's persona/task
  text; copy the parent's current `LoopConfig`/`CompletionConfig`/
  `ContextConfig`, `tool_explanations`, and image/document byte caps.
- **`ProxyPermissionGate` (`emberly-tools`, alongside `permission.rs`) +
  wiring.** Implements the existing `PermissionGate`/`AskUserGate` traits by
  forwarding to the root engine over an internal channel, tagged with the
  subagent's id/name; the root engine's existing `PermissionRequest`/
  `PermissionAnswer` round trip (already single-flight to the frontend)
  queues a tagged request exactly like a second concurrent one. **No new
  `RuleEngine` instance is ever constructed for a subagent** — this is the
  single most important thing this phase gets right.
- **Memory/skill/scratch gates are shared, not forked**: a subagent's gates
  point at the parent's own `Arc<MemoryStore>`/`Arc<SkillCatalog>`/
  `Arc<ScratchStore>`. The **task list is not shared** — each subagent gets
  its own private task-list state.
- **Nested transcript.** `.agents/sessions/<parent-session-id>/subagents/
  <subagent-id>.jsonl`, reusing `FileTranscript`/`TranscriptEvent` unmodified.
- **Driving a turn.** `SubagentManager` sends `Command::UserInput` into the
  subagent's own command channel and awaits `UiEvent::TurnEnded`, wrapped in
  `tokio::time::timeout(agents.spawn_timeout_secs)`; a timeout leaves the
  engine task alive (reported `still running`, never canceled).
  `spawn_agents` drives N such futures with `futures::future::join_all` — the
  actual concurrency.
- **New `UiEvent` variants** (`SubagentSpawned`/`SubagentStatus`/
  `SubagentEnded`, Tech Spec §3.1) and **`Command::InspectAgent` /
  `UiEvent::AgentActivity`** (mirroring `InspectSkill`/`SkillBody`) for
  Phase 4's inspector — plumbed here even though nothing renders them yet.
- **Config:** `[agents]` — `enabled`, `max_concurrent`, `spawn_timeout_secs`,
  `idle_timeout_secs` (Tech Spec §8.4 defaults); `max_depth` is a compile-time
  constant, not a config key. Wired into `ReloadedConfig`/`reload_config`
  like every other tunable.
- **Cost rollup, idle reap, lifecycle.** Subagent token usage folds into the
  parent session's `SessionUsage`/`CostEstimate` as it accrues; an idle
  subagent past `idle_timeout_secs` is ended automatically
  (`SubagentEnded{reason: "idle timeout"}`); every subagent still alive at
  session end is ended with it; a stale id after a crash/restart returns the
  structured "no such agent" failure (HC-3 honesty clause).

**Satisfies:** FR-9, T-18–T-21 (functionally complete); Tech Spec §8.4, §3.1,
§7 (config).

**Done when:** the full `cargo test -p emberly-core -p emberly-tools` suite
is green, including the Tech Spec §14 item-9 coverage that needs the real
engine (concurrency genuinely overlaps in wall-clock time, permission
requests provably reach the root `RuleEngine` rather than an independent
copy, the tool ceiling and depth bound hold, `max_concurrent`/timeout/idle-reap
behave, a stale post-restart id fails structured rather than panicking, cost
rolls up) — plus `clippy`/`fmt` clean. No TUI changes yet; this phase is
usable only through tests and a headless driver.

---

## Phase 3 — TUI (`emberly-tui`)

**Goal:** Surface what Phase 2 built: the sidebar Agents section, the
per-agent inspector overlay, quiet tool-activity lines, and the
permission-prompt provenance line — the Design Guideline §4.13/§8.9/§5
surfaces.

**Scope**
- **Sidebar Agents section (Design §3.1/§4.13).** Renders from
  `SubagentSpawned`/`SubagentStatus`/`SubagentEnded`; present only while at
  least one subagent is alive, following the established no-empty-stub rule
  (Tasks/Memory/Skills/completion gate).
- **Per-agent inspector overlay (Design §4.13).** Selecting a sidebar entry
  sends `Command::InspectAgent`; renders the returned `AgentActivity` as a
  read-only, live-updating overlay reusing the existing `§4.2` overlay
  machinery — a subagent's own assistant text and tool activity, never
  streamed into the main pane.
- **Tool-activity lines.** `spawn_agents`/`message_agent`/`end_agent` render
  as quiet one-line events carrying the subagent name(s) (Design §4.13),
  through the existing generic `ToolOutcome.summary` rendering path — no new
  rendering code needed for the primary agent's own call/result, mirroring
  how scratch-write needed none (0.4.3 precedent).
- **Permission-prompt provenance line (Design §5).** The prompt renderer
  reads the new `on_behalf_of` field on `PermissionRendering` and adds one
  dimmed line naming the subagent; every other guarantee (Deny default, full
  content, forbidden patterns) is unchanged.
- **Degraded mode (Design §7).** All of the above in plain ASCII, no color/
  motion/mouse reliance — the same parity every prior version's surfaces get.

**Satisfies:** the Design Guideline §4.13/§8.9/§5/§3.1 surfaces over FR-9.

**Done when:** live-verified in a real terminal session (per this project's
own UI-testing practice): spawn a batch of subagents, watch the sidebar and
inspector update, converse with one via `message_agent`, trigger a permission
prompt from a subagent's own tool call and confirm the provenance line
appears, then verify degraded mode (`--plain`) keeps full parity — plus
`clippy`/`fmt` clean.

---

## Phase 4 — Hardening, docs, and full-stack verification

**Goal:** Close the gap between "each phase's own tests pass" and "the
capability works end to end, under load, exactly as specified" — the
cross-cutting tests that only make sense once tool layer, engine, and TUI
are all wired together.

**Scope**
- **Full Tech Spec §14 item-9 coverage**, end to end: a `spawn_agents`
  concurrency test against a real (not mocked) multi-task `tokio` runtime;
  a `message_agent` round trip through a live TUI session; the tool-ceiling
  and depth-bound tests against the actual filtered registry a spawned
  subagent receives; a permission-proxy test confirming an existing session
  grant already covers a subagent's action with no new prompt, and that a
  genuinely new one queues correctly with the right subagent tag; a
  `max_concurrent` test; a spawn-timeout test; a crash/resume test (kill and
  restart the process, confirm a stale id fails structured); a cost-rollup
  test against the sidebar's own displayed total.
- **README / user-facing docs** (if this project maintains user docs for
  each shipped capability — confirm against the current README structure)
  describing the four tools and the `[agents]` config keys.
- **Full workspace pass:** `cargo build --workspace`,
  `cargo test --workspace`, `cargo clippy --workspace --all-targets`
  (`-D warnings`), `cargo fmt --check`, `cargo deny check`, `cargo vet`
  (Requirements §10) — confirming no new dependency slipped in anywhere
  (Tech Spec §12's explicit claim).

**Satisfies:** M12 in full — the milestone's own "done when" (Tech Spec
§15): delegation works, in parallel where it fans out, addressable across
turns where it doesn't, with no second engine implementation, no second
safety model, and no new dependency.

**Done when:** every item above passes, confirmed 2026-08-XX (date filled in
on completion, per this project's as-built convention).

---

## Phase dependency summary

```
0.4.3 product (M11) on 0.4.2 (M10) on 0.4.1 (M9) on 0.4 (M8) on 0.3 (M7) on 0.2 (M6) on 0.1 (M1-M5)
   +-> Phase 1  Tool-layer contract (emberly-tools)      -- SubagentGate + 4 tools, stub gate
   +-> Phase 2  Engine machinery (emberly-core)          -- nested Engine, proxy gate, real behavior
   +-> Phase 3  TUI (emberly-tui)                        -- Agents section, inspector, provenance line
   +-> Phase 4  Hardening & full-stack verification      -- end-to-end tests, docs, workspace pass
```

Strictly sequential — unlike the 0.4.3 plan's two independent phases, each
phase here is a precondition for the next: Phase 2 cannot be tested without
Phase 1's tools to drive, Phase 3 has nothing to render without Phase 2's
events, and Phase 4's end-to-end tests need all three built.

---

## Not in this plan

- **Recursive subagent spawning** (a subagent spawning its own subagent) —
  out of scope by design (Requirements §2.2); the depth bound is structural
  (a subagent's registry never contains the four multi-agent tools), not a
  future toggle this plan prepares for.
- **Cross-session subagent reconnection after a crash/restart** — explicitly
  deferred (Requirements §2.2); Phase 2's crash/resume behavior is "fail
  structured, let the model re-spawn," not reconstruction.
- **A concurrent multi-pane live view of several subagents at once** —
  deferred (Requirements §2.2); Phase 3 builds the single per-agent inspector
  only.
- **Independent per-subagent `LoopConfig`/`CompletionConfig`/`ContextConfig`
  tuning** — v0.5 copies the parent's current values at spawn time (Tech
  Spec §16 open item); revisit if a real need for divergent tuning appears.
- **The Hugging Face / local-inference-server capability** — not part of
  M12 at all (Requirements §2.2/§13); a separate, owner-level scope decision,
  untouched by this plan.

---

## Open items

Tech Spec §16 v0.13's open items (resource-bound defaults, independent
per-subagent tuning, per-subagent cost display granularity, the Hugging
Face/local-inference-server scope question) are carried forward unchanged by
this plan — see "Not in this plan" above and the Requirements §13 / Design
Guideline §10 entries they cite. No other new open items are introduced.
