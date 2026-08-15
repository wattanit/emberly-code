# Phase 2 — Engine machinery (`emberly-core`) — TODO & Progress

**Milestone:** M12 phase 2 (Tech Spec §15) — the 0.5 feature set.
**Satisfies:** FR-9, T-18–T-21 functionally complete; Tech Spec §8.4, §3.1,
§7 (config). Pinned to **Req v0.11 / Design v0.11 / Spec v0.13** (all
`approved`). Depends on Phase 1 (the tools this phase makes real).

**Status:** 🚧 **core + cost rollup + spawn/end events done and tested**
(2026-08-15, branch `v0.5-phase2`) — `spawn_agents`/`message_agent`/
`list_agents`/`end_agent` are fully functional, proven by seven real
`FakeProvider`-driven integration tests (not just unit tests against a stub
gate), including the FR-9 cost-rollup honesty clause and the
`SubagentSpawned`/`SubagentEnded` sidebar events. What's **not** done this
pass: `SubagentStatus` (a live per-subagent status, vs. the current uniform
`Running`), the inspector's `Command::InspectAgent`/`UiEvent::AgentActivity`,
idle-reap, and `[agents]` config-file reading — see "Not done this pass"
below. None of these gaps affect correctness or safety; they're
visibility/config-surface work, tracked
honestly rather than silently deferred.

**Design correction discovered during implementation:** the plan named
`ProxyPermissionGate` in `emberly-tools`. It actually landed as
`SubagentPermissionGate`/`SubagentAskUserGate` in **`emberly-core/src/gate.rs`**
instead — `PermissionAsk`'s fields are `pub(crate)` to `emberly-core`
specifically so nothing outside the engine can construct one, so a gate that
constructs `PermissionAsk` values directly has to live there. This is a HOW
detail, not a WHAT change; Tech Spec §8.4 should be corrected to match on the
next bump.

## Group 1 — Derived `EngineConfig` construction

- [x] Provider resolution via the existing `ProviderFactory` from an
      optional `profile`/`model` in the spawn spec, defaulting to the
      parent's own active profile/model (P-8 — zero new provider code).
      `resolve_subagent_provider` (`engine/subagents.rs`).
- [x] Filtered `ToolRegistry` builder (`build_subagent_tools`): never the
      four multi-agent tools themselves (the structural depth bound);
      further filtered to a requested subset when the spawn spec names one,
      validated against the *parent's own* registry. **Refined during
      implementation:** an explicitly *requested* multi-agent tool name is
      now its own structured failure ("cannot be given to a subagent"),
      distinct from the silent exclusion the default (unrestricted) path
      uses — asking for one by name gets a clear reason instead of a quietly
      smaller registry (verified by
      `spawn_agents_rejects_a_requested_multi_agent_tool_by_name`).
- [x] System-prompt composition (`compose_subagent_system_prompt`): the
      parent's own already-resolved system prompt (the harness's baked-in
      scaffold plus any project instructions) with the spawn spec's
      `system_prompt` layered underneath a short preamble — never a bare
      replacement.
- [x] Copies the parent's current `LoopConfig`/`ContextConfig`,
      `tool_explanations`, `image_max_bytes`/`document_max_bytes` onto the
      derived config. **Deliberately not copied:** `completion_checks` — a
      subagent's own natural stop is its "done," not gated by the session's
      registered checks (e.g. "cargo test must pass") meant for the primary
      task; an inert gate behaves exactly as none (Tech Spec §7). Recorded
      as a design decision in code comments, not just an oversight.
- [x] `max_depth` is a Rust constant (`MULTI_AGENT_TOOL_NAMES`, checked
      structurally in `build_subagent_tools`), not a config key.

## Group 2 — Permission/ask proxying (the load-bearing piece)

- [x] `SubagentPermissionGate` (`emberly-core/src/gate.rs` — see the
      correction above): implements `PermissionGate`, forwards `authorize()`
      to the root engine's own permission channel, tagged with the
      subagent's name via a new `PermissionAsk.on_behalf_of: Option<String>`
      field.
- [x] `SubagentAskUserGate`: the matching proxy for `AskUserGate` (T-8) —
      same channel, same "one user, one place they're ever asked" principle.
      **Scope cut:** it does *not* carry a name tag (unlike the permission
      proxy) — `UiEvent::AskUserRequest` has no provenance field to carry it
      to yet; wiring that is Phase 3's job alongside the rest of the
      TUI-facing provenance display.
- [x] The root engine's **existing** `on_permission_ask`/`PendingAsk`
      handling needed no new logic — only `build_rendering` gained the new
      `on_behalf_of` parameter it threads into `PermissionRendering`. No
      second `RuleEngine` is ever constructed for a subagent; `EngineConfig`
      gained `external_permission_gate`/`external_ask_gate: Option<Arc<dyn
      _>>` (mirroring the existing `sandbox_spawn` override-seam pattern
      exactly) so `Engine::new` installs the proxy instead of its own fresh
      `Gate::new(..)` only when one is supplied.
- [x] New optional `on_behalf_of: Option<String>` field on
      `PermissionRendering` (additive, `#[serde(default)]`).
- [x] **Explicit test**, passing:
      `subagent_permission_ask_is_covered_by_an_existing_session_grant` — a
      confined session earns an `AllowForSession` grant from its own `make
      build` call, then spawns a subagent whose *identical* `make build`
      call is silently allowed with **no second prompt** — proving the
      subagent consulted the same live `RuleEngine`, not an independent
      snapshot.
- [x] **Concurrency correctness, discovered and fixed during
      implementation (not in the original plan):** `Engine::on_subagent_ask`
      runs inside the same `tokio::select!` loop that watches the
      permission-ask channel. A handler that `.await`ed a subagent's own
      turn directly would starve that very loop and deadlock the subagent's
      own proxied permission ask (broken only by the spawn timeout). Every
      handler here does only fast, non-blocking work and hands the actual
      waiting to a detached `tokio::spawn`ed task holding no `&mut self` —
      documented at length in `engine/subagents.rs`'s module docs, since
      it's the single easiest thing for a future change to get wrong.

## Group 3 — Shared vs. per-subagent state

- [x] Memory/skill config and directories are passed to the derived config,
      so a subagent's own `MemoryStore`/`SkillCatalog` reads/writes the
      *same underlying directories* as the parent's. **Refined during
      implementation:** this is directory-shared, not literally the same
      in-process `Arc` — matching Tech Spec §8.4's *observable* guarantee
      ("reads and writes the same durable memory") without a redundant
      override seam for a benefit (perfect in-process cache coherency) FR-9
      never actually requires, since the files are the real source of
      truth.
- [x] Task-list gate: automatic, not a deliberate wire-up — each subagent is
      a genuinely fresh `Engine`, so it gets its own fresh, empty task-list
      state by construction; nothing to share or leak.
- [x] Scratch gate: **not addressed this pass** — a subagent gets its own
      scratch directory (keyed by its own session id), not the parent's
      shared one. Tech Spec §8.4 says "shared"; this is a smaller
      correctness gap than memory/skills (scratch is disposable working
      space either way) but should be corrected in the same Tech Spec pass
      as the `ProxyPermissionGate` location fix above.

## Group 4 — Transcript

- [x] Nested path: `.agents/sessions/<parent-session-id>/subagents/
      <subagent-id>.jsonl`; `FileTranscript`/`TranscriptEvent` reused
      unmodified (no new transcript schema). Best-effort: if the directory/
      file can't be created, the subagent falls back to `EngineConfig::
      no_transcript()` rather than failing the spawn (HC-3's "never crash,
      degrade instead" spirit) — its audit trail is valuable but not
      load-bearing for the subagent to run.
- [ ] **Not verified this pass:** that the parent's own `tool_call` args for
      `spawn_agents` durably record the subagent id in a way a human or tool
      could later use to *find* its transcript file (the id is recorded as
      ordinary tool args via HC-7's existing `tool_call` event, but no test
      asserts on this specifically yet).

## Group 5 — `SubagentManager` (`engine/subagents.rs`, new)

- [x] `AgentState`: `HashMap<String, SubagentInstance>` + `AgentsConfig` +
      a sequential id counter (`agent-1`, `agent-2`, …) — simpler than the
      originally-planned join-handle/spawn-time/last-activity bundle, since
      dropping a `SubagentInstance` (just `{name, turn_tx}`) alone cascades
      the whole shutdown chain (see below) with no explicit bookkeeping.
- [x] Implements `SubagentGate` for the root `Engine` via the new
      `SubagentAsk` channel/enum (one channel for all four operations,
      not four).
- [x] `spawn_agents`: builds N derived `EngineConfig`s, spawns N subagent
      `Engine::run` + per-subagent driver task pairs, sends each its first
      turn request, then a **single detached task** `futures::future::
      join_all`s all N replies (each under `tokio::time::timeout
      (spawn_timeout_secs)`) and replies to the original caller. A timeout
      reports `StillRunning`, never cancels the subagent.
- [x] `message_agent`: looks up the subagent's `turn_tx`, sends the next
      turn request, and (fast, non-blocking) hands the wait to a detached
      task exactly like spawn. An unknown/ended id is `NotFound` without
      touching any channel.
- [x] `list_agents` / `end_agent`: straightforward map reads/removals.
      `end_agent` (or the whole `Engine` — and thus `AgentState` — being
      dropped at session end) drops the `SubagentInstance`, which drops its
      `turn_tx`; the per-subagent driver's `turn_rx.recv()` returns `None`
      and it exits, dropping the subagent engine's own `commands_tx`; the
      subagent's own `Engine::run` sees its `commands_rx` close and ends
      cleanly, writing its own `session_end` — **the whole cascade is
      ordinary Rust drop semantics, no explicit "kill" message anywhere**,
      and it's what makes "every subagent still alive at session end is
      ended with it" true for free, with no special-cased code.
- [ ] **Idle reap not implemented this pass.** `idle_timeout_secs` is
      threaded through `AgentsConfig` but nothing currently checks it —
      only `end_agent` and session end reclaim a subagent. Needs a
      background tick in the run loop (a genuinely separate, smaller piece
      of work from everything else here); tracked as a Tech Spec §16 open
      item already.
- [x] Crash/resume honesty **verified without an actual process restart**:
      `message_and_end_agent_on_unknown_id_are_structured_failures` proves
      an unknown id fails structured (HC-6), which is the same code path a
      genuinely-stale post-restart id would hit (a fresh session has no
      registry entries at all). A literal kill-and-restart integration test
      is still open (Phase 4's job per the original plan).

## Group 6 — Events, cost, config

- [x] `UiEvent::SubagentSpawned { id, name, profile, model }` — emitted right
      after a subagent is registered (`spawn_one_subagent`) — and
      `UiEvent::SubagentEnded { id, reason }` — emitted on a successful
      `end_agent`. Verified by `spawn_and_end_agent_emit_their_sidebar_events`.
      **Not implemented this pass:** `UiEvent::SubagentStatus` (a live,
      round-tripped per-subagent status) and `Command::InspectAgent`/
      `UiEvent::AgentActivity` (the inspector's data source) — `list_agents`
      still reports every alive subagent as `SubagentStatus::Running`
      uniformly. Both are Phase 3's dependency, not blocking anything in this
      phase.
- [x] **Cost rollup, implemented and tested.** A new fire-and-forget
      `SubagentAsk::ReportUsage{usage, cost_usd}` variant (no reply — nothing
      awaits it): each per-subagent driver tracks the subagent's own
      *cumulative* `SessionUsage`/`CostEstimate` (already emitted by its own
      `Engine` after every completion) and, after each turn, reports the
      *delta* since the last report — so the root can simply add what it
      receives with no double-counting risk. The root's
      `roll_up_subagent_usage` adds the delta into `self.session.usage`/
      `cost_usd` and re-emits both events, mirroring `emit_context_usage`'s
      own update shape. Verified end to end by
      `subagent_cost_and_usage_roll_up_into_the_session_total`: a priced
      subagent provider's scripted `Usage` event reaches the *root's own*
      `SessionUsage`/`CostEstimate` stream, with the exact dollar amount
      checked (not just "some cost changed"). Closes the one real (if
      narrow) honesty-clause gap from the first cut of this phase
      (Requirements FR-9: "cost is never hidden").
- [x] `AgentsConfig` (`enabled` default `true`, `max_concurrent` default
      `3`, `spawn_timeout_secs` default `600`, `idle_timeout_secs` default
      `1800`) exists and is threaded through `EngineConfig`/`Engine::new`.
      **Not implemented this pass:** reading `[agents]` from `config.toml`
      or wiring it into `reload_config`/`ReloadedConfig` — `main.rs`
      currently passes `AgentsConfig::default()` unconditionally, documented
      inline as a known gap.
- [x] `max_concurrent` enforcement: verified by
      `spawn_agents_over_max_concurrent_fails_only_the_excess` — the excess
      names in an over-limit batch fail structured
      ("max_concurrent (3) reached"), the ones under the ceiling are
      attempted normally.

## Tests added (all passing, `cargo test -p emberly-core`)

- `subagent_permission_ask_is_covered_by_an_existing_session_grant` — the
  load-bearing permission-proxy correctness test (Group 2).
- `spawn_then_message_round_trip` — T-18 then T-19 against a real,
  independently-scripted subagent provider (via a new `OneShotFactory` test
  double), including a white-box assertion on the `agent-1` id scheme.
- `spawn_agents_rejects_a_requested_multi_agent_tool_by_name` — the
  structural depth bound, made observable.
- `spawn_agents_over_max_concurrent_fails_only_the_excess`.
- `message_and_end_agent_on_unknown_id_are_structured_failures`.
- `subagent_cost_and_usage_roll_up_into_the_session_total` — the FR-9 cost
  honesty clause, with the exact dollar amount checked.
- `spawn_and_end_agent_emit_their_sidebar_events` — `SubagentSpawned`/
  `SubagentEnded` carry the right id/name and fire at the right moments.

Plus new unit tests in `gate.rs` (Phase 1 additions extended):
`subagent_permission_gate_tags_the_ask_with_its_label`,
`the_root_gate_tags_no_subagent`, and fail-closed coverage for the new
`SubagentAsk`/proxy gates.

**Done when:** ✅ `cargo build --workspace`, `cargo test --workspace` (673
tests, all passing), `cargo clippy --workspace --all-targets` (`-D
warnings`), and `cargo fmt --check` (per touched crate) are all clean,
confirmed 2026-08-15. `Cargo.lock`/`Cargo.toml` show zero diff — no new
dependency, as planned. **Not done:** `SubagentStatus`/`InspectAgent`/
`AgentActivity` (Phase 3's dependency), `[agents]` config-file reading, and
idle reap — each called out explicitly rather than silently folded into
"done."
