# Emberly Code — Implementation Plan (0.4.3 feature set)

**Status:** ✅ **done** — Phase 1 (three externally-reported bug fixes) and
Phase 2 (session scratch space, FR-8/T-17) are both **done**. Written
as-built, after the fact — see the note under "How this plan was written"
below.
**Date:** 2026-07-30
**Owner:** Wattanit
**Source documents** (G-14 as-built pin for the 0.4.3 release):
- Requirements Document v0.10 (`docs/emberly-code-requirements.md`) — WHAT/WHY
- Design Guideline v0.10 (`docs/emberly-code-design-guideline.md`) — UX/voice
- Technical Specification v0.11 (`docs/emberly-code-tech-spec.md`) — HOW

These three versions are the as-built truth for the 0.4.3 release, all
`Status: approved` (owner). A stale pin is a defect (G-16); when a foundation
document bumps, update this pin. Discoveries that change HOW flow back into
the Technical Specification as version bumps (G-24/G-25) — this plan never
becomes a shadow spec.

This plan expands Tech Spec §15 milestone **M11** into workable phases. It
builds on the shipped 0.4.2 product (M10 — guided provider/model setup, C-7)
and everything beneath it. The prior plans are preserved in `docs/version-0-1/`
… `docs/version-0-4-2/` as the earlier as-built records.

### How this plan was written

Like the 0.4.2 plan, this one was **not** written before implementation —
both phases below were already built and shipped when this document was
drafted. It records what was actually built and why, in the same structure
the prospective plans use, rather than a task list checked off after the
fact.

Phase 1 originated from three issues filed against the public repository by
the Yggdrasil Project team (a separate product consuming Emberly as its
engine, reporting bugs on their own domain-agnostic merits — the same
relationship as the 0.4.1 feature requests, not an upstream/downstream
obligation), not a pre-planned scope — each item below cites the
Requirements/HC id(s) its fix restores, since none of them are new
capability. Phase 2 originated as an owner-proposed idea (a Claude-Code-style
scratchpad) that was scoped, drafted upstream through the SFD process, and
approved before any code was written — the normal prospective order, just
compressed into the same session as Phase 1's fixes.

---

## Phase 1 — Three externally-reported bug fixes — ✅ DONE (2026-07-30)

**Goal:** Fix three concrete bugs reported against the public repository by
the Yggdrasil Project team, each restoring an existing guarantee rather than
adding scope.

**Scope**
- **`sse_completion_stream` had no independent stall-detection timeout
  (issue #11, commit `c0727ae`).** The stream loop relied solely on
  `reqwest`'s overall request timeout to end a dead connection — and this
  codebase's own `reqwest::Client` never actually sets one, so a stalled
  connection could hang a session forever with no error and no recovery.
  Fixed by wrapping each `body.next()` in `tokio::time::timeout` with a 90s
  idle window (the issue's suggested 60–120s range); a stall now ends the
  stream with a new, retryable `ProviderError::Timeout` instead of hanging.
  Verified with a paused-tokio-clock unit test (deterministic, no real
  sleeping) confirming both the timeout firing and an active stream being
  unaffected.
- **An interrupted turn could commit a message no provider adapter accepts
  (issue #13, commit `cf404c4`).** `push_assistant_message` committed a turn
  to conversation history whenever it had reasoning, even with no text and
  no tool calls — the exact shape produced by canceling a turn mid-thought,
  before any visible output. Every adapter maps that to a contentless
  assistant message (`{"role": "assistant", "content": null}` on the
  OpenAI-compatible wire), which several backends reject outright, and once
  committed the rejection repeated on every subsequent request — a
  permanently wedged session with no in-app recovery. Fixed by never
  committing a turn with no text and no tool calls to conversation history,
  regardless of reasoning; the reasoning is still written to the transcript
  for display. A doc comment on `TurnOutput` now flags this guard for
  whoever adds a new kind of displayable-but-unserializable content next.
- **`count_tokens`'s flat chars/4 ratio badly underestimated non-Latin
  scripts (issue #12, commit `bda515d`).** Thai (and other scripts with no
  space-delimited words and denser BPE packing — CJK, Hangul, etc.) were
  estimated at the same 4-chars-per-token ratio as English, undercounting by
  roughly 2x. This is not merely a display bug: `context_tokens()` feeds the
  FR-4 auto-compaction latch directly, so the undercount meant auto-compact
  armed too late (or never) for sessions in those scripts, risking a real
  context-window overflow with no proactive warning — reported by the
  Yggdrasil team from a Thai legal-text workflow against a local
  OpenAI-compatible server with no authoritative usage. Fixed with a new
  shared `estimate_tokens` (dense-script characters at ~2 chars/token,
  everything else at ~4), weighted per character so mixed-script text is
  counted correctly on both halves — not classified as entirely one script
  or the other.

**Satisfies:** no new IDs (all three are corrections to existing
guarantees — provider error handling, HC-7/wire correctness, and the FR-4
auto-compaction signal).

**Done when:** each fix above shipped with a regression test that fails
against the pre-fix code and passes after (verified for all three, not just
asserted), plus `cargo test --workspace` green and `cargo clippy --workspace
--all-targets` (`RUSTFLAGS="-D warnings"`) / `cargo fmt --check` clean after
every commit.

---

## Phase 2 — Session scratch space (FR-8, T-17) — ✅ DONE (2026-07-30)

**Goal:** Give the model a disposable, harness-owned working directory for
temporary files — scripts, intermediate output, working notes — that costs no
permission prompt and never lands in the user's tracked project, plus a CLI
path for the user to reclaim its disk space. Proposed as "would a
Claude-Code-style scratchpad help this project," scoped through the SFD
process before any code (Requirements §8.9/§5, Design §4.12/§8.8, Tech Spec
§5.2/§8.3/§10), matching how every other capability in this suite is
absorbed.

**Scope**
- **`scratch_write` tool (`emberly-tools`).** Schema-constrained exactly like
  the `memory` tool (T-13): the model supplies `{name, content}`, never a
  path; the engine validates the name (rejecting `..`, absolute paths, and
  separators — but, unlike memory's `slug`, *not* transforming it, since a
  scratch file's extension and case are meaningful) and resolves the real
  file within the session's fixed scratch directory. Because the model never
  supplies a destination, it is not permission-gated — the same rationale as
  T-13, verified against the actual precedent rather than assumed: the
  existing no-ask tools either touch no filesystem at all (`recall`, `todo`,
  `ask_user`) or, like memory, let the harness alone pick the destination
  from schema-constrained content. A raw `write_file`/`bash` pointed at the
  scratch directory would not have qualified for the same exemption.
- **`ScratchStore` (`emberly-core`).** Unlike `MemoryStore`, this needed no
  channel/oneshot round trip to the engine loop: a scratch write has no side
  effect on any other engine-owned state (no cached index, no sidebar status
  to refresh), so the gate acts directly. Directory is
  `.agents/scratch/<session-id>/`, created lazily on first write, derived
  straight from `project_root` + `session_id` (unlike memory, there is only
  ever one location, so this needed no new `EngineConfig` field). Rebuilt on
  every `/new` and `/resume` (`adopt_session`) so scratch space is never
  carried across a session switch.
- **`.gitignore`:** `.agents/scratch/` added, mirroring `.agents/sessions/`.
  This is also why `glob`/`grep` don't surface scratch content by default —
  verified against their actual `WalkBuilder` config (`.hidden(false)`, so
  they *do* search dotfiles; it's `.gitignore` they honor) rather than
  assumed, and corrected in the Requirements/Tech Spec drafts before they
  were approved.
- **`emberly clean [<session-id>]` (`emberly` binary).** No id reports total
  scratch disk usage across every session and, on confirmation, deletes all
  of it; an id scopes to one session. Confirmation is a plain `y`/`n`
  question with the same safe-default-decline-when-non-interactive behavior
  as the trust prompt — verified live against a real temp project (empty,
  populated, and scoped-by-id cases, plus the non-interactive decline path).
- **TUI:** no new rendering code — a scratch-write's `ToolOutcome.summary`
  (`"scratched · {name} · {bytes}B written"`) flows through the same generic
  tool-activity line every other tool already uses.

**Satisfies:** FR-8, T-17; Tech Spec §5.2, §8.3, §10; Design §4.12, §8.8.

**Done when:** the scratch-write round trip (file lands under the right
session directory, never permission-gated, a path-escape rejected as
structured data not a crash) and the `emberly clean` CLI paths are covered by
tests, verified live against a real temp project for the CLI, plus a full
`cargo build/test/clippy(-D warnings)/fmt` pass — all confirmed 2026-07-30.

---

## Phase dependency summary

```
0.4.2 product (M10) on 0.4.1 (M9) on 0.4 (M8) on 0.3 (M7) on 0.2 (M6) on 0.1 (M1-M5)
   +-> Phase 1  Three bug fixes (#11, #13, #12)     -- provider/engine corrections
   +-> Phase 2  Session scratch space (FR-8, T-17)  -- new tool + engine store + CLI verb
```

Phase 2 does not depend on Phase 1's fixes — it touches an unrelated part of
the tool/engine surface (a new gate, a new store, a new CLI command), none of
which the SSE/reasoning/token-estimate fixes introduced or altered. Both
phases shipped in the same session because that is the order the work
arrived in: three bug reports, then a feature idea.

---

## Not in this plan

- **A `scratch_read`/`scratch_list` tool** — considered and declined: reading
  a known scratch file already works through the existing `read_file` (T-1),
  and the model always knows the name it used within the same session, so a
  dedicated read path would duplicate an existing tool for no new capability.
- **A non-interactive `--yes` flag for `emberly clean`** — deferred
  (Tech Spec §16 open item); add if scripted/CI use of the command turns out
  to need it.
- **A size cap or retention policy on scratch space** — deferred
  (Tech Spec §16 open item); confirm in practice whether an unbounded
  scratch directory between `emberly clean` runs is an actual problem before
  adding one.
- **Extending the dense-script list beyond Thai/Lao/Myanmar/Khmer/CJK/Hangul**
  (issue #12) — Arabic, Devanagari, and others still fall back to the 4
  chars/token default; extend if a report surfaces for one of them.
- **A client-level `reqwest` request timeout** (issue #11's underlying gap —
  `provider_setup.rs`'s `build_https_client()` sets none at all) — the new
  per-chunk idle timeout mitigates the specific stall case this release was
  about; a connection dribbling one byte every 89 seconds would still run
  unbounded overall. Left as a known gap, not silently declared fixed.

---

## Open items

Tech Spec §16 v0.11's two new open items (the `emberly clean` non-interactive
flag; the scratch-space size/retention policy) are carried forward unchanged
by this plan — see "Not in this plan" above. No other new open items were
introduced.
