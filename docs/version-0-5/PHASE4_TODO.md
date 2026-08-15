# Phase 4 — Hardening, docs, and full-stack verification — TODO & Progress

**Milestone:** M12 phase 4 (Tech Spec §15) — the 0.5 feature set, final
phase. **Satisfies:** M12 in full. Pinned to **Req v0.11 / Design v0.11 /
Spec v0.13** (all `approved`). Depends on Phases 1–3.

## Group 1 — End-to-end tests (Tech Spec §14 item 9)

- [ ] Concurrency: N subagents spawned in one `spawn_agents` call genuinely
      overlap in wall-clock time against a real (not mocked) multi-task
      runtime.
- [ ] `message_agent` round trip driven through a live TUI session, not
      just the engine's own unit tests.
- [ ] Tool-ceiling and depth-bound tests against the *actual* filtered
      registry a spawned subagent receives (not a hypothetical one).
- [ ] Permission-proxy test: an existing session grant already covers a
      subagent's action with no new prompt; a genuinely new one queues
      correctly and carries the right subagent tag.
- [ ] `max_concurrent` ceiling test: the excess names in an over-limit
      `spawn_agents` call fail structured, the rest succeed.
- [ ] Spawn-timeout test: a slow subagent is reported `still running`
      rather than canceled, and remains addressable afterward.
- [ ] Crash/resume test: kill and restart the process; a pre-restart
      subagent id fails structured ("no such agent") on the next
      `message_agent`/`list_agents` call.
- [ ] Cost-rollup test: a subagent's token usage is reflected in the
      sidebar's own displayed session total, not just in an internal
      counter.

## Group 2 — Docs

- [ ] Confirm against the current README structure whether user-facing
      tool/config documentation is maintained there (as it was for prior
      features); if so, add the four tools and `[agents]` config keys in
      the same style.

## Group 3 — Full workspace pass

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --all-targets` (`RUSTFLAGS="-D warnings"`)
- [ ] `cargo fmt --check`
- [ ] `cargo deny check`
- [ ] `cargo vet` — confirming no new dependency slipped in anywhere (Tech
      Spec §12's explicit "no new dependencies" claim for the 0.5 set).

**Done when:** every item above passes. Fill in the completion date here
once confirmed, per this project's as-built convention (e.g. "confirmed
2026-08-XX").
