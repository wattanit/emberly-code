# Phase 1 — Three externally-reported bug fixes — TODO & Progress

**Milestone:** M11 phase 1 (Tech Spec §15) — the 0.4.3 feature set.
**Satisfies:** no new IDs — corrections to existing guarantees. Pinned to
**Req v0.10 / Design v0.10 / Spec v0.11** (all `approved`).

Written as-built (see `IMPLEMENTATION_PLAN.md`'s "How this plan was written")
— each item below is one GitHub issue filed by the Yggdrasil Project team,
fixed independently, with its own regression test.

- [x] **Issue #11 — `sse_completion_stream` had no independent stall
      detection.** Wrapped each `body.next()` in `tokio::time::timeout`
      (90s default, `DEFAULT_STREAM_IDLE_TIMEOUT`); a stall now ends the
      stream with a new, retryable `ProviderError::Timeout` instead of
      hanging forever (the client's own `reqwest::Client` sets no overall
      timeout at all, confirmed by reading `provider_setup.rs`, so this was
      worse than "a long timeout" — it was unbounded). Refactored
      `sse_completion_stream` to take the raw byte stream
      (`sse_stream_from_body`) so the timeout is unit-testable with a
      paused tokio clock. Regression tests:
      `idle_stream_times_out_instead_of_hanging_forever`,
      `active_stream_is_unaffected_by_the_idle_timeout`. (commit `c0727ae`)
- [x] **Issue #13 — an interrupted turn could commit a permanently-broken
      assistant message.** `push_assistant_message` now never commits a
      turn with no text and no tool calls to conversation history,
      regardless of reasoning — closing the gap where a turn canceled
      mid-thought (reasoning only, no visible text yet) serialized to a
      contentless message several OpenAI-compatible backends reject, with
      the rejection then repeating on every retry. The reasoning is still
      written to the transcript for display. Added a doc comment on
      `TurnOutput` flagging this guard for whoever adds a new
      displayable-but-unserializable content kind next. Regression test:
      `reasoning_only_turn_is_not_committed_to_conversation` (confirmed to
      fail against the pre-fix code, reproducing the exact malformed
      message from the report, before confirming it passes with the fix).
      (commit `cf404c4`)
- [x] **Issue #12 — `count_tokens`'s flat chars/4 ratio badly underestimated
      non-Latin scripts.** Added `estimate_tokens` (`emberly-providers`),
      shared by both live providers: dense-script characters (Thai, Lao,
      Myanmar, Khmer, CJK ideographs, Hiragana/Katakana, Hangul) at
      ~2 chars/token, everything else at ~4 — weighted per character so
      mixed-script text isn't classified as entirely one script or the
      other. Confirmed this isn't merely cosmetic: `context_tokens()` feeds
      the FR-4 auto-compaction latch directly, so the undercount risked a
      real context-window overflow for non-Latin-script sessions on
      providers with no authoritative usage (the reported case: Thai legal
      text against a local OpenAI-compatible server). Regression tests:
      ASCII baseline, pure Thai, CJK/Hangul, mixed-script. (commit
      `bda515d`)

**Done when:** ✅ all three fixes shipped with their named regression test
(each verified to fail pre-fix and pass post-fix); `cargo test --workspace`
green and `cargo clippy --workspace --all-targets` (`RUSTFLAGS="-D warnings"`)
/ `cargo fmt --check` clean after each commit (verified 2026-07-30).
