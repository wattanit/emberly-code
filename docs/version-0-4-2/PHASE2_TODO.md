# Phase 2 — Post-ship hardening — TODO & Progress

**Milestone:** M10 phase 2 (Tech Spec §15) — the 0.4.2 feature set.
**Satisfies:** FR-3, FR-4, HC-5, HC-6, C-6 — corrections to existing
guarantees, no new IDs. Pinned to **Req v0.9 / Design v0.9 / Spec v0.10**
(all `approved`).

Written as-built (see `IMPLEMENTATION_PLAN.md`'s "How this plan was written")
— this phase originated as a live bug-report list while dogfooding the 0.4.2
branch, not a pre-planned scope. Each item is one bug, found and fixed in
this order, each with its own regression test.

- [x] **Compaction split `(tool_use, tool_result)` pairs (FR-3, FR-4).**
      `Engine::compact`'s cut point is now turn-boundary-safe
      (`group_turn_starts`, matching `windowed_messages`). Regression test:
      `compact_never_splits_tool_use_from_result`.
      Same commit: plain/line mode gained `/compact` and `CompactionStatus`
      rendering (both previously missing/dropped there).
      (commit `e7ad2bd`)
- [x] **Command palette order was accumulation-order, not deliberate.**
      Regrouped by frequency of use (owner's call): session start/switch →
      runtime switches → inspection/control → setup/tuning → help/quit.
      README command table updated to match. (commit `ee87feb`)
- [x] **`Esc`/`Ctrl+C` didn't cancel an in-flight turn in the rich TUI.**
      Wired both to `Command::Cancel` while busy, matching the doc comment
      and README's own claim; idle behavior unchanged (Esc no-ops, Ctrl+C
      quits/clears). Regression tests: `esc_cancels_an_in_flight_turn`,
      `ctrl_c_cancels_an_in_flight_turn_but_quits_when_idle`.
      (commit `84ccd82`)
- [x] **`write_file`'s permission overlay didn't show new-file content
      (HC-6).** Now always builds a full unified diff (diffing against `""`
      for a new file), matching `edit_file`. Regression test:
      `write_permission_detail_is_a_full_diff_for_a_new_file`.
      (commit `6df1af0`)
- [x] **Parallel tool calls could corrupt each other's content on
      OpenAI-compatible backends (HC-6).** `OpenAiMapper` now falls back to
      the wire's own `index` (unique per parallel call) instead of a shared
      empty string when `id` is missing. Regression test:
      `parallel_tool_calls_get_distinct_ids_when_backend_omits_id`.
      (commit `68af736`)
- [x] **No provider configured replayed a stale Phase 1 placeholder
      reply.** `Engine` now refuses `Command::UserInput` up front (checking
      `provider.id() == "placeholder"`) with a `Notice` pointing at
      `/model`, never recording/running it as a real turn. Regression test:
      `no_provider_configured_refuses_before_a_turn_runs`.
      (commit `cecdbe2`)
- [x] **The genuine-git shell-metacharacter check was quote-blind (HC-5).**
      `is_genuine_git` now uses a quote-aware scanner (single-quoted text
      fully literal; double-quoted text neutralizes chaining/redirection but
      still flags command/parameter substitution). Regression tests:
      `quoted_metacharacters_in_a_commit_message_still_qualify`,
      `unterminated_quote_stays_safe_closed`.
      (commit `61ea285`)
- [x] **Chained git commands (confirmed intended, not a bug) — but the
      model had no way to know.** Owner decision: leave the sandbox
      behavior as-is (each git command runs as its own `bash` call);
      instead told the model why via a new system-prompt boundary.
      `prompts::VERSION` bumped 4 → 5, logged in `prompts/CHANGELOG.md`.
      (commit `e6adee6`)
- [x] **Stale "Phase N" doc-comment relics across the workspace.** Module
      and item doc comments describing shipped features as future Phase
      2/3/5 work, reworded to describe current reality; removed one dead
      constant (`SESSION_ENDED`). Worst instance was user-facing:
      `emberly init`'s generated `permissions.toml` template.
      (commit `82e1181`)
- [x] **Design Guideline / historical-doc cleanup (this documentation
      pass, no version bump — owner's call).**
  - [x] §4.6: removed the "Quick edit — TUI overlay" bullet (never built;
        only the `$EDITOR` handoff exists) and corrected two claims that
        plain mode falls back to `$EDITOR` for config/prompt/wizard editing
        (it never does — it prints the path and relies on `/reload`).
  - [x] `docs/version-0-4/IMPLEMENTATION_PLAN.md`: fixed a stale
        "planned — not yet started" status on the completed 0.4 plan.
  - [x] `docs/version-0-2/IMPLEMENTATION_PLAN.md`: repaired two broken
        `docs/phase1/` cross-references (renamed to `docs/version-0-1/`).
  - [x] README: fixed the same `$EDITOR`-in-plain-mode contradiction, the
        stale foundation-doc version pins, and added the 0.4.2 release-
        history row.

**Done when:** ✅ every fix above shipped with its named regression test;
`cargo test --workspace` green and `cargo clippy --workspace --all-targets
--all-features` (`RUSTFLAGS="-D warnings"`) / `cargo fmt --check` clean after
each commit (verified 2026-07-22).
