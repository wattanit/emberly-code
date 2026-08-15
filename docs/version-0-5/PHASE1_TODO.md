# Phase 1 — Tool-layer contract (`emberly-tools`) — TODO & Progress

**Milestone:** M12 phase 1 (Tech Spec §15) — the 0.5 feature set.
**Satisfies:** the tool-layer half of T-18–T-21 (Tech Spec §8.4); no engine
behavior yet — every tool is inert (fails closed) until Phase 2. Pinned to
**Req v0.11 / Design v0.11 / Spec v0.13** (all `approved`).

## Group 1 — The gate contract (`subagent.rs`)

- [ ] `SubagentSpawnSpec { name, system_prompt, profile: Option<String>,
      model: Option<String>, tools: Option<Vec<String>> }` — one spawn
      request within a batch.
- [ ] `SubagentSpawnBatch { agents: Vec<SubagentSpawnSpec> }` — the
      `spawn_agents` request shape.
- [ ] `SubagentSpawnOutcome` per agent: `Answered(String)` (final text),
      `StillRunning` (past the per-call timeout, remains addressable),
      `Failed(String)` (reason — e.g. over `max_concurrent`, an internal
      provider error) — one subagent's failure never blocks the others'
      results (HC-6, Requirements T-18).
- [ ] `SubagentSpawnResult { id: String, name: String, outcome:
      SubagentSpawnOutcome }`.
- [ ] `SubagentMessageRequest { id: String, message: String }` and
      `SubagentMessageOutcome`: `Replied(String)`, `NotFound`,
      `Failed(String)`.
- [ ] `SubagentListEntry { id: String, name: String, status:
      SubagentStatus }` where `SubagentStatus` is `Running |
      AwaitingPermission | Done | TimedOut | Error` (matches the `UiEvent`
      status values Tech Spec §3.1 names).
- [ ] `SubagentEndOutcome`: `Ended`, `NotFound`.
- [ ] `SubagentError` — the gate-unreachable fail-closed error (HC-6),
      mirroring `MemoryError`/`ScratchError`.
- [ ] `SubagentGate` trait (`async_trait`): `spawn_agents`,
      `message_agent`, `list_agents`, `end_agent` — each returning
      `Result<_, SubagentError>`.
- [ ] `DropSubagentGate` — the default installed by `ToolCtx::new`; every
      method returns `Err(SubagentError)`, mirroring `DropMemoryGate`/
      `DropScratchGate` exactly.

## Group 2 — The four tools (`builtin/*.rs`)

- [ ] `builtin/spawn_agents.rs` — `SpawnAgentsTool` (T-18). Args:
      `{agents: [{name, system_prompt, profile?, model?, tools?}]}` (at
      least one entry required). `describe()`: `spawn_agents · <names,
      comma-joined>`. `execute()`: builds a `SubagentSpawnBatch`, calls
      `ctx.spawn_agents`, renders each result as `<id> (<name>): <answer |
      still running | failed: reason>` joined by newlines; `ok` is `true`
      unless the *whole* call failed (gate unreachable) — individual
      per-agent failures live in the content, never abort the batch.
- [ ] `builtin/message_agent.rs` — `MessageAgentTool` (T-19). Args:
      `{id, message}`. `describe()`: `message_agent · <id>`. `execute()`:
      maps `Replied`/`NotFound`/`Failed` to success/structured-failure
      `ToolOutcome`s (HC-6) — an unknown/ended id names the reason, never a
      crash or silent no-op.
- [ ] `builtin/list_agents.rs` — `ListAgentsTool` (T-20). No required args
      (empty object schema, matching `recall`'s pattern for optional
      fields). `describe()`: `list_agents`. `execute()`: renders each alive
      subagent's id/name/status as one line.
- [ ] `builtin/end_agent.rs` — `EndAgentTool` (T-21). Args: `{id}`.
      `describe()`: `end_agent · <id>`. `execute()`: maps `Ended`/`NotFound`
      to success/structured-failure.

## Group 3 — Wiring

- [ ] `ctx.rs`: `subagent: Arc<dyn SubagentGate>` field (default
      `Arc::new(DropSubagentGate)`), `with_subagent_gate` builder, and four
      passthrough methods (`spawn_agents`, `message_agent`, `list_agents`,
      `end_agent`) — mirroring every existing gate exactly.
- [ ] `lib.rs`: `pub mod subagent;` plus re-exports of the gate trait and
      all request/outcome/error types; the four new tools re-exported
      alongside the existing builtins.
- [ ] `builtin/mod.rs`: the four tools added to `default_registry()`
      (temporary — Phase 2 introduces the *filtered* registry a spawned
      subagent actually receives, which excludes these four by
      construction).

## Group 4 — Tests

- [ ] Schema round-trip tests for all four tools (valid args parse; missing
      required fields produce `invalid_args`, not a panic).
- [ ] Fail-closed tests: with the default `DropSubagentGate` installed,
      each of the four tools returns a structured `ToolOutcome::failure`
      (HC-6) — never a panic, never a silent success.
- [ ] `describe()` output tests for all four tools.

**Done when:** `cargo build -p emberly-tools`, `cargo test -p emberly-tools`,
`cargo clippy -p emberly-tools --all-targets` (`RUSTFLAGS="-D warnings"`),
and `cargo fmt --check` are all clean.
