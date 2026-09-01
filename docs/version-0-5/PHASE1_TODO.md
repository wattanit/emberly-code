# Phase 1 — Tool-layer contract (`emberly-tools`) — TODO & Progress

**Milestone:** M12 phase 1 (Tech Spec §15) — the 0.5 feature set.
**Satisfies:** the tool-layer half of T-18–T-21 (Tech Spec §8.4); no engine
behavior yet — every tool is inert (fails closed) until Phase 2. Pinned to
**Req v0.11 / Design v0.11 / Spec v0.13** (all `approved`).

**Status:** ✅ **done** (2026-08-15), on branch `v0.5-phase1`.

## Group 1 — The gate contract (`subagent.rs`)

- [x] `SubagentSpawnSpec { name, system_prompt, profile: Option<String>,
      model: Option<String>, tools: Option<Vec<String>> }` — one spawn
      request within a batch.
- [x] `SubagentSpawnBatch { agents: Vec<SubagentSpawnSpec> }` — the
      `spawn_agents` request shape.
- [x] `SubagentSpawnOutcome` per agent: `Answered(String)` (final text),
      `StillRunning` (past the per-call timeout, remains addressable),
      `Failed(String)` (reason — e.g. over `max_concurrent`, an internal
      provider error) — one subagent's failure never blocks the others'
      results (HC-6, Requirements T-18).
- [x] `SubagentSpawnResult { id: String, name: String, outcome:
      SubagentSpawnOutcome }`.
- [x] `SubagentMessageRequest { id: String, message: String }` and
      `SubagentMessageOutcome`: `Replied(String)`, `NotFound`,
      `Failed(String)`.
- [x] `SubagentListEntry { id: String, name: String, status:
      SubagentStatus }` where `SubagentStatus` is `Running |
      AwaitingPermission | Done | TimedOut | Error` (matches the `UiEvent`
      status values Tech Spec §3.1 names).
- [x] `SubagentEndOutcome`: `Ended`, `NotFound`.
- [x] `SubagentError` — the gate-unreachable fail-closed error (HC-6),
      mirroring `MemoryError`/`ScratchError`.
- [x] `SubagentGate` trait (`async_trait`): `spawn_agents`,
      `message_agent`, `list_agents`, `end_agent` — each returning
      `Result<_, SubagentError>`.
- [x] `DropSubagentGate` — the default installed by `ToolCtx::new`; every
      method returns `Err(SubagentError)`, mirroring `DropMemoryGate`/
      `DropScratchGate` exactly. Unit test
      `drop_gate_fails_every_call_closed` confirms all four methods.

## Group 2 — The four tools (`builtin/*.rs`)

- [x] `builtin/spawn_agents.rs` — `SpawnAgentsTool` (T-18). Args:
      `{agents: [{name, system_prompt, profile?, model?, tools?}]}` (at
      least one entry required — an empty list is rejected before the gate
      is ever called). `describe()`: `spawn_agents · <names,
      comma-joined>`. `execute()`: builds a `SubagentSpawnBatch`, calls
      `ctx.spawn_agents`, renders each result as `<id> (<name>): <answer |
      still running | failed: reason>` joined by newlines; `ok` is `true`
      unless the *whole* call failed (gate unreachable) — individual
      per-agent failures live in the content, never abort the batch.
- [x] `builtin/message_agent.rs` — `MessageAgentTool` (T-19). Args:
      `{id, message}`. `describe()`: `message_agent · <id>`. `execute()`:
      maps `Replied`/`NotFound`/`Failed` to success/structured-failure
      `ToolOutcome`s (HC-6) — an unknown/ended id names the reason, never a
      crash or silent no-op.
- [x] `builtin/list_agents.rs` — `ListAgentsTool` (T-20). No required args
      (empty object schema, matching `recall`'s pattern for optional
      fields). `describe()`: `list_agents`. `execute()`: renders each alive
      subagent's id/name/status as one line.
- [x] `builtin/end_agent.rs` — `EndAgentTool` (T-21). Args: `{id}`.
      `describe()`: `end_agent · <id>`. `execute()`: maps `Ended`/`NotFound`
      to success/structured-failure.

## Group 3 — Wiring

- [x] `ctx.rs`: `subagent: Arc<dyn SubagentGate>` field (default
      `Arc::new(DropSubagentGate)`), `with_subagent_gate` builder, and four
      passthrough methods (`spawn_agents`, `message_agent`, `list_agents`,
      `end_agent`) — mirroring every existing gate exactly.
- [x] `lib.rs`: `pub mod subagent;` plus re-exports of the gate trait and
      all request/outcome/error types; the four new tools re-exported
      alongside the existing builtins.
- [x] `builtin/mod.rs`: the four tools added to `default_registry()`
      (temporary — Phase 2 introduces the *filtered* registry a spawned
      subagent actually receives, which excludes these four by
      construction).

## Group 4 — Tests

- [x] Schema round-trip tests for all four tools (valid args parse; missing
      required fields produce `invalid_args`, not a panic — see
      `spawn_agents_rejects_missing_required_fields_as_invalid_args`).
- [x] Fail-closed tests: with the default `DropSubagentGate` installed,
      each of the four tools returns a structured `ToolOutcome::failure`
      (HC-6) — never a panic, never a silent success (`*_default_gate_fails_closed`
      for all four tools, in `tests/builtin_tools.rs`).
- [x] `describe()` output tests for all four tools.
- [x] An additional test not originally listed: `spawn_agents` rejects an
      *empty* `agents` list as a structured failure before ever reaching
      the gate (`spawn_agents_rejects_an_empty_agent_list`) — worth calling
      out since it's a schema-adjacent validation the JSON Schema
      `minItems: 1` alone doesn't enforce at the Rust deserialization layer.

**Done when:** ✅ `cargo build -p emberly-tools`, `cargo test -p emberly-tools`
(118 tests, all passing), `cargo clippy -p emberly-tools --all-targets`
(`RUSTFLAGS="-D warnings"`), and `cargo fmt -p emberly-tools --check` are all
clean (confirmed 2026-08-15). Also confirmed: `cargo build --workspace` and
`cargo test --workspace` remain green with the four new tools flowing through
`default_registry()` into `emberly-core`/`emberly`/`emberly-tui`, and
`Cargo.lock`/`Cargo.toml` show zero diff — no new dependency (Tech Spec §12).

Note: workspace-wide `cargo fmt --check` reports pre-existing formatting
drift in a handful of files this phase never touched (`emberly/build.rs`,
`emberly/src/provider_setup.rs`, `emberly-core/src/engine/mod.rs`) — unrelated
to this phase's scope and left as-is rather than reformatted alongside it.
