# Phase 4 — Hardening, docs, and full-stack verification — TODO & Progress

**Milestone:** M12 phase 4 (Tech Spec §15) — the 0.5 feature set, final
phase. **Satisfies:** M12 in full. Pinned to **Req v0.11 / Design v0.11 /
Spec v0.13** (all `approved`). Depends on Phases 1–3.

## Group 1 — End-to-end tests (Tech Spec §14 item 9)

All in `crates/emberly-core/tests/engine_loop.rs`, under the "Phase 4 Group 1"
heading unless noted otherwise.

- [x] Concurrency: `spawn_agents_batch_runs_subagents_concurrently_not_serially`
      — three subagents in one batch each run a real, unmocked 1s `bash
      sleep`, timed on the real clock (no `tokio::time::pause`, which would
      not exercise genuine OS-level concurrency). Asserts total wall time
      stays well under what three *serialized* sleeps would take (>=3s),
      proving the `tokio::spawn` + `join_all` design actually overlaps them.
- [x] `message_agent` round trip: already covered pre-Phase-4 by
      `spawn_then_message_round_trip`, driven over the real
      `Command`/`UiEvent` channel pair — the exact same boundary any
      frontend, including the rich TUI, talks to the engine through (A-1).
      Judgment call: a *second*, TUI-crate-level reproduction of this round
      trip was considered and skipped — it would need a new
      `emberly-providers` dev-dependency on `emberly-tui` to get a
      `FakeProvider` in there, purely to re-prove what the engine-level test
      plus `emberly-tui`'s own `apply_event`-driven unit tests (which prove
      the TUI reacts correctly to `SubagentSpawned`/`AgentActivity`, etc.)
      already establish between them. Flagged here rather than silently
      assumed equivalent.
- [x] Tool-ceiling/depth-bound against the *actual* registry:
      `a_subagent_cannot_call_spawn_agents_against_its_own_actual_registry`
      — distinct from the pre-existing
      `spawn_agents_rejects_a_requested_multi_agent_tool_by_name` (which
      only proves an *explicit request* at spawn time is rejected). This one
      never requests `spawn_agents` for the subagent at all (the silent
      default exclusion) and has the subagent's own model try to call it
      mid-conversation anyway — proven via its own transcript
      (`Command::InspectAgent`) to fail as an ordinary "unknown tool"
      result, never a crash and never an actual nested spawn.
- [x] Permission-proxy, the genuinely-new-ask half:
      `a_new_subagent_permission_ask_queues_and_carries_the_subagent_tag` —
      complements the pre-existing
      `subagent_permission_ask_is_covered_by_an_existing_session_grant`
      (existing-grant half) by asserting directly on
      `PermissionRendering.on_behalf_of` for a first-time ask.
- [x] `max_concurrent` ceiling: already covered pre-Phase-4 by
      `spawn_agents_over_max_concurrent_fails_only_the_excess`.
- [x] Spawn-timeout / addressable-afterward:
      `spawn_timeout_reports_still_running_and_the_subagent_remains_listed`
      — a subagent whose first turn (a real 2s bash sleep) outlives a 1s
      `spawn_timeout_secs` is reported "still running", and a follow-up
      `list_agents` still lists it.
- [x] Crash/resume: `a_subagent_id_from_before_a_restart_is_unknown_to_the_new_process`
      — two genuinely separate `Engine` instances (not a literal process
      kill) stand in for a restart; the second instance's own `next_seq`
      counter also starts fresh, so its first spawn would mint the very
      same id "agent-1" — proving the "no such agent" result on the old id
      isn't a coincidental non-collision but the real, structural absence
      of any surviving `AgentState`.
- [x] Cost-rollup: already covered pre-Phase-4 by
      `subagent_cost_and_usage_roll_up_into_the_session_total`.

`cargo test -p emberly-core`: 137 passed, 0 failed (up from 132 pre-Phase-4).

## Group 2 — Docs

- [x] README.md updated in the same style as prior features (memory/skills):
  - "A complete agentic harness" feature list now names multi-agent
    delegation.
  - New "Multi-agent delegation" subsection (alongside "Persistent memory &
    skills" and "Web search & image/document input") explaining the four
    tools, the structural depth bound, the permission-proxy/cost-rollup
    honesty guarantees, and `/agents`.
  - "Config keys at a glance" table gained `[agents] enabled` (+
    `max_concurrent`/`spawn_timeout_secs`/`idle_timeout_secs` defaults).
  - The sidebar description and the commands table (`/agents`) updated to
    match.
  - Deliberately **not** touched: "Project status" (still says "v0.4.5")
    and "Release history" (no v0.5 row yet) — those are release-time
    actions (a version bump commit, per this project's established
    pattern, e.g. `89e5372`), not a Phase 4 docs task, and v0.5 has not
    been cut as a release yet.

## Group 3 — Full workspace pass

- [x] `cargo build --workspace` — clean.
- [x] `cargo test --workspace` — 245 (emberly-tui) + 137 (emberly-core) +
      all other crates' suites pass, 0 failed.
- [x] `cargo clippy --workspace --all-targets` (`RUSTFLAGS="-D warnings"`) —
      clean.
- [x] `cargo fmt --check` — clean. Along the way, fixed pre-existing
      formatting drift in two untouched files (`crates/emberly/build.rs`,
      `crates/emberly/src/provider_setup.rs`) that had been carried since
      before this session — whitespace/wrapping only, no logic change —
      since Group 3 is the first point in this feature's work where a
      workspace-wide `fmt --check` is actually a gate rather than a
      scoped-diff nicety.
- [~] `cargo deny check` — neither this nor `cargo vet` were installed
      locally; installed `cargo-deny` to run it, and discovered
      **`deny.toml` failed to parse at all**: `[bans] wildcard-dependencies`
      is not a key current `cargo-deny` recognizes (renamed to `wildcards`
      at some past release). Fixed that one-line, schema-only rename (no
      policy change) so the check can run — and it turns out this gate has
      likely never actually executed successfully, because once it
      parses, it reports a substantial, entirely pre-existing set of
      findings unrelated to the M12 multi-agent feature:
      - 4 RUSTSEC vulnerability advisories on `rustls-webpki` (transitive,
        via the `rustls-rustcrypto` TLS backend) plus a Marvin Attack
        timing-sidechannel advisory.
      - 3 unmaintained-crate advisories: `bincode`, `paste`, `yaml-rust`
        (all transitive, via `syntect`).
      - 1 license rejection: `webpki-roots` ships `CDLA-Permissive-2.0`,
        not in `deny.toml`'s allow list.
      - 10 "wildcard dependency" flags — this workspace's *own* internal
        crates (`emberly-core`, `emberly-tui`, etc.) are declared as
        `path = "../x"` with no version, which `cargo-deny`'s wildcard
        check treats as unpinned.
      **Per the owner's direction, this is recorded as a follow-up, not
      triaged in this phase** — it long predates and is fully independent
      of the multi-agent work; resolving it means real judgment calls
      (upgrade transitive deps? add justified `ignore`/`exceptions`
      entries? exempt workspace-internal path deps from the wildcard
      check? replace `syntect`?) that deserve their own dedicated pass. CI's
      `cargo-deny-action` is not version-pinned either, so this was likely
      already silently failing there too, unnoticed until now.
- [~] `cargo vet` — not run. CI itself marks this job
      `continue-on-error: true` and documents it as "non-blocking until
      the audit set is seeded" (`.github/workflows/ci.yml`), and no
      `supply-chain/` directory exists in this repo yet — there is nothing
      seeded to meaningfully check locally. Seeding it (via `cargo vet
      init` + importing an audit set) is its own infrastructure task, out
      of scope here and folded into the same follow-up as the `cargo deny`
      findings above.

**Follow-up (tracked, not part of M12):** triage the `cargo deny`
findings above (RUSTSEC advisories, license rejection, unmaintained
crates, internal-workspace wildcard-dependency flags) and seed `cargo
vet`'s audit set. Independent of the multi-agent feature; needs its own
prioritization pass.

**Done when:** every Group 1–3 item above passes or is explicitly
recorded as a tracked follow-up. **Confirmed 2026-08-16** (all Group 1–3
items either pass or are the two `~` follow-ups noted above, deliberately
deferred per owner direction).
