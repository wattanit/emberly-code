# Emberly Code — Implementation Plan (0.3 feature set)

**Status:** ✅ **complete (2026-07-12)** — all four phases implemented and
merged to `version0.3`. The whole 0.3 context-economy feature set (M7) ships:
FR-2 salient tool-result reduction, FR-3 adaptive context window + T-10 `recall`
tool, FR-4 automatic compaction + the `/compact` command surface, and FR-5 the
derived resume cache.
**Date:** 2026-07-11 (planned); completed 2026-07-12
**Owner:** Wattanit
**Source documents** (G-14 as-built pin for the 0.3 release):
- Requirements Document v0.6 (`docs/emberly-code-requirements.md`) — WHAT/WHY
- Design Guideline v0.6 (`docs/emberly-code-design-guideline.md`) — UX/voice
- Technical Specification v0.7 (`docs/emberly-code-tech-spec.md`) — HOW

These three versions are the as-built truth for the 0.3 release, all
`Status: approved` (owner, 2026-07-11); the Technical Specification publishes at
v0.7 alongside the 0.3 release (owner decision, 2026-07-12 — not bumped). One
post-merge fix shipped under the *same* versions after an alignment audit: the
`compaction.trigger` transcript field now serializes as `manual`/`auto` to match
Tech Spec §3.2 (it was emitting PascalCase). A stale pin is a defect (G-16).

This plan expands Tech Spec §15 milestone **M7** (the 0.3 context-economy
feature set) into workable phases. It builds on the shipped 0.2 product
(M6 — endpoint-configurable provider profiles + Z.ai, in-app editing,
reasoning effort/trail, ask-user + tool-call explanation, workspace trust,
loop-breaking guardrail) and the 0.1 product beneath it (M1–M5 — the six
crates, the agent loop, both live providers, the full TUI, sessions /
resume / `/compact` engine, the permission rule engine, and OS confinement
on Linux + macOS). The prior plans are preserved in `docs/version-0-1/` and
`docs/version-0-2/` as the earlier as-built records.

The 0.3 set is not new user-facing surface area so much as a **token- and
dollar-economy layer** over the context management that already exists
(Tech Spec §7): the cost of a long session is itself a product concern
(Requirements §8, intro to §8.5–§8.8). The phases are therefore sliced by
the economy layer they add, in the exact order Requirements §8.4 mandates —
cheapest and most conservative first: salient tool-result reduction (§8.5)
→ an adaptive window over turns (§8.6) → compaction, manual (§8.3) and
automatic (§8.7) → efficient resume (§8.8) carrying the resulting economy
across a restart. Each layer only changes what is *sent to the model*; none
rewrites the transcript, and each phase ships on its own.

---

## Guiding principles (apply to every phase)

- **The transcript is the spine; every layer is a view over it (HC-7).**
  Reduction (FR-2), windowing (FR-3), compaction (FR-3/FR-4), and the resume
  cache (FR-5) all change only what is *sent to the provider* or *how the
  view is restored* — never the append-only JSONL. Every phase must leave the
  transcript complete and unrewritten; a full result, a dropped turn, and a
  compacted range are all still one re-read away (Requirements §8.4, Tech
  Spec §3.2). This is the honesty clause the whole 0.3 set turns on.
- **The seams already exist — use them, don't reshape them.** Context
  management lives in `emberly-core` (Tech Spec §7); the tool suite and its
  ingestion path live in `emberly-tools` (§5.3); the transcript, its sidecar
  outputs, and the derived cache live in the session layer (§3.2/§3.2a). Every
  0.3 feature is added *through* those boundaries — a new per-tool reducer, a
  window bound in the sent context, a `recall` `Tool`, an accounting-path
  threshold check, a derived-state sidecar — never by leaking a new concept
  past them (A-1, A-3, P-1, T-7).
- **No new dependencies (Tech Spec §12).** The whole 0.3 set is engine and
  config logic over the existing crate set; the resume cache serializes the
  already-`serde`-derived normalized message types (A-3) to JSON via
  `serde_json`. Anything that appears to need a new crate is a signal to
  re-check the design first. `emberly-sandbox` is untouched — none of the
  0.3 layers is security-critical (they change context economy, not what an
  action may do).
- **Deterministic, no added model call — except compaction, by design.** The
  context-defense layers must add no model round-trip and no per-tool-call
  latency to the agent loop: FR-2 reduction and FR-3 windowing are pure,
  deterministic transforms (Requirements §8.5, §8.6 — "a context defense that
  itself costs a model round-trip defeats its own purpose"). The *only* layer
  that calls the model is compaction (FR-4/§8.3), which is a summarization
  call fired at a clean boundary, exactly as it already is for manual
  `/compact`.
- **Every phase leaves a shippable, tested slice.** Each ends green with its
  offline, deterministic `FakeProvider`-driven coverage from Tech Spec §14.6,
  and with degraded-mode parity where it adds or changes a UI surface
  (Design §7).

---

## Phase 1 — Tool-result salient reduction (FR-2)

**Goal:** Reduce a tool result to its salient content *by meaning* before it
enters the model context — keeping the parts that inform the next step,
eliding the noise — while the complete result stays in the transcript and
one `/view` away. Deterministic, no model call.

**Scope**
- **Per-tool reducer registry (Tech Spec §5.3).** Pure functions
  `(&ToolSpec, &raw_output) -> reduced_output`, registered per tool; a tool
  with no registered reducer falls straight through to the size backstop.
- **Initial reducer set (initial; tune with use — Requirements §13).**
  `bash`: keep exit status, all of stderr, and head+tail of stdout, collapsing
  runs of near-identical progress/percentage lines. `grep`: keep every hit and
  its `file:line` header, drop nothing that is a match (hits are the point).
  `read_file`: no semantic reducer — already bounded by optional
  `start_line`/`end_line` and the size backstop; reducing by meaning would
  risk hiding code the model asked for. `glob`: keep the path list, head+tail
  with the elision marker if it exceeds the count backstop.
- **Ordering: salient reduction first, then the §8.1 size backstop.** The
  blind head/tail defense (`truncate.max_lines` 400, `truncate.max_bytes`
  64 KiB, head 150 / tail 100) always applies after reduction (Tech Spec
  §5.3). The full untruncated output is always written to the sidecar
  `.agents/sessions/<id>-outputs/` and referenced by `full_output_ref` on the
  `tool_result` transcript event (§3.2).
- **Never a dead end (Design §4.3, §8.6).** The reduction/elision marker names
  what was withheld and offers `/view` to the complete output; recoverability
  is guaranteed by construction because the sidecar holds the full result.
- **Config** `truncate.reduce = bool` (**default `true`**, Tech Spec §8)
  disables the reduction layer for users who want raw results.

**Satisfies:** FR-2; Tech Spec §5.3, §3.2 (`full_output_ref`), §8
(`truncate.reduce`); Design §4.3, §8.6; HC-7 (transcript untouched).

**Done when:** per-tool reducer unit tests show a noisy `bash`/`grep`/`glob`
output reducing to its salient content with the full output recoverable from
the sidecar, and `truncate.reduce = false` passing output through; the
`tool_result` transcript line records the full (pre-reduction) result, proven
untouched (Tech Spec §14.6, HC-7).

---

## Phase 2 — Adaptive context window & the `recall` tool (FR-3, T-10)

**Goal:** Stop carrying every past turn verbatim — send a working window of
recent turns, drop older ones from the *sent context* while they stay in the
transcript, and make the drop reversible by the model on demand rather than
by the harness guessing when old history matters.

**Scope**
- **Working window (Tech Spec §7).** The last `context.window_turns`
  (**default `40`**) non-pinned turns are sent. Pinned content (system prompt,
  project instructions, original task — §8.3) and any active compaction
  summary are always sent and never counted against the window.
- **Elision marker, not deletion.** Turns older than the window are replaced
  *in the sent context only* by one synthetic marker — `[N earlier turns
  elided from context — still in the session transcript]` — carrying the
  transcript range it stands for. They remain verbatim in the transcript and,
  for the user, in TUI scrollback (Design §8.6). The window changes what is
  sent, never what happened (HC-7 honesty clause, Requirements §8.6).
- **`recall` built-in tool (Tech Spec §5.2, Requirements T-10).** Args: a turn
  range (or the id referenced by the elision marker). Returns the engine's
  **normalized, reduced** messages for that range (reusing Phase 1's §5.3
  reduction), served from the transcript records the engine already holds —
  never raw JSONL, so recall costs tokens proportional to what is recalled,
  not the transcript's raw size (Requirements T-10). A pure engine
  round-trip: no filesystem, no network, so like `ask_user` it bypasses the
  sandbox and is **not permission-gated** (§6) while still flowing through the
  `Tool` trait. Deliberately not `read_file` over raw JSONL, which would
  re-inflate the elided noise and defeat the window's economy.
- **Usage tracks the working window (Design §8.6).** `ContextUsage` reflects
  the sent window so "why did usage drop" is always answerable, never
  mysterious. A `recall` call renders as ordinary, quiet tool activity in the
  flow (one dim line, optional §4.5 explanation) — the model visibly choosing
  to reload history — kept distinct from the user-facing `/view` (§4.3).
- **Config** `context.window_turns` (**default `40`**, Tech Spec §8); the
  window bound is part of the derived cache (§3.2a), consumed in Phase 4.

**Satisfies:** FR-3, T-10; Tech Spec §7, §5.2, §3.1/§3.2, §8; Design §8.6.

**Done when:** a window test asserts old turns are dropped from the sent
context while the transcript and the elision marker are intact; a `recall`
round-trip via `FakeProvider` asserts the tool returns the dropped turns in
normalized, reduced form (not raw JSONL) and never raises a permission prompt
(Tech Spec §14.6, FR-3/T-10).

---

## Phase 3 — Compaction completion: `/compact` surface + automatic compaction (FR-4)

**Goal:** Finish compaction on both ends — wire the missing `/compact`
command surface into the registry (an existing implementation gap against the
already-built §8.3 mechanism), and add the automatic trigger so a long
session compacts before it overflows without waiting on the user to act on
the indicator.

**Scope**
- **Close the `/compact` surface gap (Requirements §8.3, Design §3.3).** The
  summarization mechanism — purpose-built prompt, pinned-content rules,
  clean-boundary rule, and the summarization-failure hard-truncate fallback —
  already exists in `emberly-core` (Tech Spec §7, shipped in M5). What is
  missing is the command surface: wire `/compact` into the command registry
  so it is reachable all three ways — palette, `/command`, keybinding
  (Design §3.3). This closes the gap named in Requirements §13 "Resolved
  since v0.5" and Tech Spec §16 (an implementation gap, not new scope).
- **Automatic compaction (FR-4, Tech Spec §7).** One threshold check in the
  accounting path: when `ContextUsage.pct` crosses
  `context.auto_compact_threshold` (**default `0.85`**), schedule a compaction
  at the next clean boundary — the same queueing, pinned-content,
  clean-boundary, and failure-fallback rules as manual; the *only* difference
  is the trigger and the `compaction.trigger = auto` transcript field (§3.2 —
  additive, older readers warn-skip it, no `SCHEMA_VERSION` bump; absent reads
  as `manual`). After an auto-compaction fires it will not re-fire until usage
  has fallen and re-crossed the threshold (the compaction drops usage well
  below it, so no thrash).
- **On by default, disableable (Requirements §8.7, Tech Spec §8).**
  `context.auto_compact = bool` (**default `true`**); set `false` to compact
  manually only. `context.keep_recent_turns` (default `6`) still sets the
  verbatim tail a compaction keeps.
- **Surfaced in the harness voice, never silent (Design §8.6, §6.1).**
  Compaction is a harness-world moment, not model output: manual `/compact`
  shows the spinner then one calm line ("Compacted 24 turns into a summary.");
  automatic is the same line with its reason ("Context was near full —
  compacted 24 turns to keep going."). No alarm styling — staying under budget
  is routine housekeeping. The summary is ordinary conversation content,
  inspectable in full via `/view`.

**Satisfies:** FR-4 and the §8.3 command-surface gap; Tech Spec §7, §3.2
(`trigger`), §8, §15 (M7); Design §3.3, §8.6, §6.1; HC-7 (log untouched).

**Done when:** `/compact` is reachable via palette, `/command`, and
keybinding and compacts at a clean boundary; an automatic-compaction test
drives `ContextUsage` past the threshold via `FakeProvider` usage numbers and
asserts a `compaction` event with `trigger = auto` fires at the next clean
boundary and does not thrash (Tech Spec §14.6, FR-4).

---

## Phase 4 — Efficient session resume from a derived cache (FR-5)

**Goal:** Make resume cost the tokens of the *working view*, not of
re-ingesting the full audit transcript — restoring the derived state directly
— while the transcript remains the sole ground truth and resume always falls
back to a correct full replay.

**Scope**
- **Derived conversation-state sidecar (Tech Spec §3.2a).**
  `.agents/sessions/<id>-view.json` holds the current in-context view
  (normalized messages), token-accounting totals, the compaction
  summary/range history, and the current window bound (§7 — the state Phases 2
  and 3 shape). It serializes the already-`serde`-derived normalized message
  types (A-3) via `serde_json` — no new dependency (§12).
- **Derived, never authoritative (Requirements FR-5, HC-7 subordination).**
  The cache is a pure function of the transcript, rewritten in place each time
  the view changes; HC-7 does not apply to it (it holds no fact the transcript
  lacks). Losing, corrupting, or deleting it loses nothing. A best-effort
  write after each turn suffices, because the replay fallback is always
  correct.
- **Staleness guard (Tech Spec §3.2a).** The cache records the transcript's
  byte-length and the byte offset it was built through; on resume, if the
  transcript has grown past that offset, is shorter, or the file is
  unreadable/parse-fails, the cache is discarded and resume falls back to full
  replay. The cache is trusted only when it provably matches the log.
- **Cache-first resume, honest about the slow path (Tech Spec §3.3, Design
  §8.6).** Fast path: load the view, accounting, and window state from the
  cache directly — no per-line re-tokenization; silent (silence about the fast
  path). Fallback: replay the transcript (apply `compaction` events as view
  transformations, re-derive the window bound; unknown newer `v` events warn,
  not crash), announced in one dimmed harness-voice line (speech about the
  slow path). No config key — the cache is always written and always guarded,
  so it needs no opt-in (§8).

**Satisfies:** FR-5; Tech Spec §3.2a, §3.3, §8, §12; Design §8.6; HC-7
(transcript remains sole ground truth).

**Done when:** a resume test asserts the cache fast-path restores the same
view the live session held, and that a deliberately staled / corrupted /
deleted cache falls back to transcript replay producing the *identical* view
(Tech Spec §14.6 — the HC-7 subordination made a test).

---

## Phase dependency summary

```
0.2 product (M6, shipped) on 0.1 (M1–M5, shipped)
   ├─> Phase 1  Tool-result reduction (FR-2)
   │      └─> Phase 2  Adaptive window & recall (FR-3, T-10)  ── recall returns FR-2-reduced turns
   ├─> Phase 3  Compaction: /compact surface + auto (FR-4)     ── reuses the M5 compaction engine
   └─> Phase 4  Resume cache (FR-5)                            ── caches the view Phases 2–3 shape
```

The order follows Requirements §8.4's cheapest-first layering: salient
reduction → adaptive window → compaction (manual surface + automatic) →
efficient resume carrying the economy across a restart. Two soft dependencies
set the single track:

- **Phase 2 wants Phase 1 first.** `recall` returns dropped turns in Phase 1's
  §5.3 *reduced* form, so the reducer set should exist before the window that
  relies on it (Requirements T-10).
- **Phase 4 comes last.** The derived cache serializes the window bound
  (Phase 2) and the compaction summary/range history (Phase 3); building it
  last lets it capture the full derived state (Tech Spec §3.2a).

**Phase 3 is independent of 1 and 2 in mechanism** — it reuses the already
built M5 compaction engine and could be reordered or parallelized. The
default single-track order 1→2→3→4 is the layered strategy read top to
bottom; each phase ships on its own.

---

## Not in this plan

- **Auto-compaction is now *in* scope** (Phase 3) — it graduated from the
  Requirements §2.2 deferred tier through its named door (FR-4, §8.7); this
  plan builds it, not defers it. **MCP, headless/IDE/server frontends, user
  theming, and Windows** remain deferred per Requirements §2.2 — their seams
  stay unused, not reshaped (A-1, T-7).
- **Release-pipeline / prebuilt binaries** — 0.1 shipped compile-from-source
  with the pipeline deferred by owner; 0.2 did not revisit it and neither does
  0.3. Install-from-source continues unless the owner reopens it.
- **Agent-quality evaluation** (does it code well) — out of scope per Tech
  Spec §14; post-release discipline with separate tooling.

---

## Open items (carried into the phases per G-11; resolvers named)

These are the Tech Spec §16 open items the 0.3 phases resolve. Discoveries
that change HOW flow back into the Technical Specification as version bumps
(G-24/G-25) — this plan never becomes a shadow spec.

- **Marker↔range identifier scheme (FR-3, Phase 2).** Turn indices vs. an
  opaque marker id for the window-elision marker that `recall` consumes.
  Resolve: in Phase 2 (Tech Spec §16).
- **Per-tool reducer rules (FR-2, Phase 1).** The initial `bash`/`grep`/`glob`
  reducers are a starting set. Resolve: validate against real tool output in
  Phase 1 so reduction never hides what the model needs, and extend the
  registry as new salient shapes appear (Tech Spec §16, Requirements §13).
- **`context.window_turns` and `context.auto_compact_threshold` defaults
  (Phases 2, 3).** `40` and `0.85` are placeholders. Resolve: tune with real
  long sessions so windowing/compaction fire before overflow without cutting
  genuine working context (Tech Spec §16, Requirements §13).
- **Cache staleness guard sufficiency (FR-5, Phase 4).** Byte-length + offset
  is the initial guard. Resolve: confirm sufficiency in Phase 4; a content
  hash is the fallback if same-length divergence is ever observed (Tech Spec
  §16).
