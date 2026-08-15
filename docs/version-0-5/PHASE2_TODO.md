# Phase 2 — Engine machinery (`emberly-core`) — TODO & Progress

**Milestone:** M12 phase 2 (Tech Spec §15) — the 0.5 feature set.
**Satisfies:** FR-9, T-18–T-21 functionally complete; Tech Spec §8.4, §3.1,
§7 (config). Pinned to **Req v0.11 / Design v0.11 / Spec v0.13** (all
`approved`). Depends on Phase 1 (the tools this phase makes real).

## Group 1 — Derived `EngineConfig` construction

- [ ] Provider resolution via the existing `ProviderFactory` from an
      optional `profile`/`model` in the spawn spec, defaulting to the
      parent's own active profile/model (P-8 — zero new provider code).
- [ ] Filtered `ToolRegistry` builder: never the four multi-agent tools
      themselves (the structural depth bound); further filtered to a
      requested subset when the spawn spec names one, validated against
      the *parent's own* registry (a name absent there is a spawn-time
      structured failure, HC-6 — never a silent grant of a capability the
      session itself lacks).
- [ ] System-prompt composition: the harness's baked-in tool-use scaffold
      (unchanged) with the spawn spec's `system_prompt` text inserted into
      a dedicated section — never a bare replacement.
- [ ] Copy the parent's current `LoopConfig`/`CompletionConfig`/
      `ContextConfig`, `tool_explanations`, `image_max_bytes`/
      `document_max_bytes` onto the derived config.
- [ ] `max_depth` is a Rust constant (`1`), not a config key — confirm no
      code path exposes it as tunable.

## Group 2 — Permission/ask proxying (the load-bearing piece)

- [ ] `ProxyPermissionGate` (`emberly-tools`, alongside `permission.rs`):
      implements `PermissionGate`, forwards `authorize()` over an internal
      channel to the root engine, tagged with the subagent's id/name.
- [ ] A matching proxy for `AskUserGate` (T-8) — same channel pattern, same
      tagging, same "one user, one place they're ever asked" principle.
- [ ] Root engine: receives proxied requests on the internal channel and
      folds them into its **existing** `PermissionRequest`/
      `PermissionAnswer` round trip to the frontend — no second `RuleEngine`
      instance is ever constructed; an existing session grant already
      covers a subagent's action without a new prompt.
- [ ] New optional `on_behalf_of: Option<String>` field on
      `PermissionRendering` (additive) carrying the subagent's name for the
      Design §4.13/§5 provenance line — `None` for the primary agent's own
      requests.
- [ ] **Explicit test** (this is the single easiest thing to get subtly
      wrong): construct a session with an existing "always allow" grant,
      spawn a subagent whose task triggers the same rule, and assert **no**
      new `PermissionRequest` fires — proving the subagent consulted the
      *parent's* rule state, not a fresh copy that happens to look similar.

## Group 3 — Shared vs. per-subagent state

- [ ] Memory/skill/scratch gates: point the subagent's `ToolCtx` at the
      parent's own `Arc<MemoryStore>`/`Arc<SkillCatalog>`/`Arc<ScratchStore>`
      — shared, not forked.
- [ ] Task-list gate: a **fresh, private** task-list state per subagent
      (not shared with the parent) — confirm `TaskListUpdated` from a
      subagent never leaks into the parent's own sidebar Tasks section.

## Group 4 — Transcript

- [ ] Nested path: `.agents/sessions/<parent-session-id>/subagents/
      <subagent-id>.jsonl`; `FileTranscript`/`TranscriptEvent` reused
      unmodified (no new transcript schema).
- [ ] The parent's `tool_call` args for `spawn_agents`/`message_agent`
      record the subagent id (and thus, via it, its transcript path) —
      exactly the image/document-read precedent (a project-relative
      reference, not duplicated bytes/content).

## Group 5 — `SubagentManager` (`engine/subagents.rs`, new)

- [ ] Owns the map of alive subagent engine tasks: id → (join handle,
      command-channel sender, name, spawn time, last-activity time).
- [ ] Implements `SubagentGate` for the root `Engine`.
- [ ] `spawn_agents`: builds N derived `EngineConfig`s, `tokio::spawn`s N
      subagent `Engine::run` tasks, drives "send initial `Command::UserInput`,
      await `UiEvent::TurnEnded`" for each via `futures::future::join_all`,
      each wrapped in `tokio::time::timeout(agents.spawn_timeout_secs)`. A
      timeout leaves the task alive (`StillRunning`), never cancels it.
- [ ] `message_agent`: sends `Command::UserInput` into the named subagent's
      existing channel, awaits its next `TurnEnded`; an unknown/ended id is
      `NotFound` without touching any channel.
- [ ] `list_agents` / `end_agent`: straightforward map reads/removals;
      `end_agent` drops the join handle and channel (task is dropped, its
      transcript's `session_end` written first).
- [ ] Session-end lifecycle: every subagent still alive when the owning
      session ends is ended with it.
- [ ] Idle reap: a background check (or a check-on-access) ending any
      subagent with no `message_agent` traffic for `idle_timeout_secs`,
      emitting `SubagentEnded{reason: "idle timeout"}`.
- [ ] Crash/resume honesty: confirm (with a test that actually restarts the
      process/engine, not just reasons about it) that a subagent id from
      before a crash returns the structured "no such agent" failure on the
      next `message_agent`/`list_agents` call, never a hang or panic.

## Group 6 — Events, cost, config

- [ ] `UiEvent::SubagentSpawned{id, name, profile, model}`,
      `SubagentStatus{id, name, status}`, `SubagentEnded{id, reason}` — all
      `#[non_exhaustive]`-friendly, additive.
- [ ] `Command::InspectAgent{id}` / `UiEvent::AgentActivity{id, turns}` —
      mirrors `InspectSkill`/`SkillBody`; reads the subagent `Engine`'s
      in-memory history directly (no polling its transcript file).
- [ ] Cost rollup: each subagent's `TokenUsage`/cost accrues into the
      **parent session's** `SessionUsage`/`CostEstimate` as it happens.
- [ ] `[agents]` config: `enabled` (default `true`), `max_concurrent`
      (default `3`), `spawn_timeout_secs` (default `600`),
      `idle_timeout_secs` (default `1800`) — wired into `ReloadedConfig`/
      `reload_config` like every other tunable.
- [ ] `max_concurrent` enforcement: a `spawn_agents` call that would exceed
      the ceiling returns a structured failure for the excess names only
      (not a silent partial spawn, not a crash).

**Done when:** `cargo test -p emberly-core -p emberly-tools` is green,
including: a genuine wall-clock-overlap concurrency test for a batch spawn;
the permission-proxy test from Group 2; tool-ceiling/depth-bound tests
against the actual filtered registry; `max_concurrent`/spawn-timeout/
idle-reap tests; the crash/resume stale-id test; a cost-rollup test — plus
`clippy`/`fmt` clean. No TUI changes in this phase.
