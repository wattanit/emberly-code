# Phase 2 — Adaptive context window & the `recall` tool (FR-3, T-10) — TODO & Progress

**Milestone:** M7 group 2 (Tech Spec §15) — the 0.3 context-economy set, **layer
3** of Requirements §8.4 (an adaptive window over turns, sitting on top of the
§8.1 size truncation and the §8.5/FR-2 salient reduction that now ship).
**Satisfies:** FR-3, T-10; Tech Spec §7, §5.2, §3.1/§3.2, §8; Design §8.6.
Pinned to Req v0.6 / Design v0.6 / Spec v0.7 (all `approved`).
**Goal:** Send only a working window of recent turns to the provider — drop
older turns from the *sent context* (not the transcript, not the user's
scrollback) behind one synthetic elision marker — and let the model pull dropped
turns back on demand via a new `recall` built-in that returns them in
normalized, reduced form. Deterministic, no model call; not permission-gated.

**Depends on:** the shipped product (M1–M6) and Phase 1 (FR-2). This phase reuses
Phase 1's `reduce_output` (`crates/emberly-tools/src/reduce.rs:64`) for the turns
`recall` returns, and mirrors the `ask_user` engine-state-tool pattern
(`AskUserGate`/`AskGate`, `crates/emberly-tools/src/ask_user.rs`,
`crates/emberly-core/src/gate.rs:63`) for `recall`.

**Legend:** `[ ]` todo · `[~]` in progress · `[x]` done · `[!]` blocked

---

## Progress summary

| Group | Status | Notes |
|---|---|---|
| 1. `[context]` config: `window_turns` (default 40) + `keep_recent_turns` | [ ] | |
| 2. Adaptive windowing at send time (`build_request`) | [ ] | |
| 3. Marker↔range identifier scheme (resolve the §16 open item) | [ ] | |
| 4. `recall` built-in tool + engine gate (not permission-gated) | [ ] | |
| 5. Context-usage reflects the working window | [ ] | |
| 6. UI: `recall` as ordinary quiet tool activity + degraded parity | [ ] | |
| 7. Tests (offline, deterministic — §14.6) + exit criterion | [ ] | |

**Overall Phase 2: NOT STARTED.**

---

## 1. `[context]` config section  *(FR-3; Tech Spec §7, §8)*

No `[context]` section is wired yet — only the `KEEP_RECENT = 6` constant
(`engine.rs:43–45`, whose comment already anticipates `context.keep_recent_turns`).
Follow the existing `[loop]` → `LoopConfig` precedent (`config.rs:71`,
`engine.rs:54`).

- [ ] Add `ContextConfig { window_turns, keep_recent_turns }` to `ConfigFile`
      (`crates/emberly/src/config.rs:20`) with `#[serde(default)]`; defaults
      `window_turns = 40` (Tech Spec §8), `keep_recent_turns = 6` (the current
      `KEEP_RECENT`). Resolve in `Resolved`/`load()` (`config.rs:296`, `:506+`).
- [ ] Thread to the engine: new field(s) on `EngineConfig` (`engine.rs:75`) and
      `Engine` (`engine.rs:275`); **replace the hardcoded `KEEP_RECENT`**
      (`engine.rs:45`, used by `compact()`) with the configured value so manual
      and automatic compaction (Phase 3) both read one source.
- [ ] `config show` reports both keys with their provenance tier (C-3), like any
      config key. Defaults are placeholders — "initial; tune with use" (Tech Spec
      §16, Requirements §13).

## 2. Adaptive windowing at send time  *(FR-3; Tech Spec §7, §3.2; HC-7)*

Window is a **send-time view**, not a mutation. `build_request()`
(`engine.rs:1715`) currently does `messages: self.conversation.clone()`
(`:1742`); insert windowing there so `self.conversation` stays **complete** in
memory (needed for `recall`, group 4) and the transcript/UI are untouched.

- [ ] Keep only the last `window_turns` non-pinned turns in the sent messages;
      elide the earlier ones. **Pinned, never dropped and never counted against
      the window** (Tech Spec §7): the system prompt (already separate, in
      `CompletionRequest.system` via `effective_system()`, `engine.rs:1741`,
      `:1755`), the **original task** (`conversation[0]`, positionally pinned as
      compaction already assumes, `engine.rs:652`), and any **active compaction
      summary** message.
- [ ] **Clean turn boundaries only.** Never cut between a `ContentBlock::ToolUse`
      and its matching `Message::tool_result` — a windowed message list must stay
      provider-valid (every tool_use has its tool_result), the same invariant
      compaction respects. Define a "turn" grouping over `Vec<Message>`
      (`message.rs:66`) and window whole turns from the tail.
- [ ] Replace the dropped span with a **single synthetic marker message** —
      `[N earlier turns elided from context — still in the session transcript]`
      (Tech Spec §7) — carrying the transcript range it stands for (group 3).
      Inserted only into the sent view, never pushed to `self.conversation` and
      never written to the transcript (HC-7: the log and the user's scrollback
      stay whole; the window changes only what is *sent*).
- [ ] Compose with compaction (Tech Spec §7): windowing bounds how many
      *post-summary* turns ride verbatim; the compaction summary is pinned into
      the sent context and never windowed away. Verify order: compaction rebuilds
      `self.conversation` (`compact()`, `engine.rs:649`), windowing views it at
      send time — one view, not a second mutation.

## 3. Marker↔range identifier scheme  *(FR-3; Tech Spec §7, §16)*

Resolves the plan/Spec §16 open item: "turn indices vs. an opaque marker id."
The elision marker (group 2) must name a range that `recall` (group 4) can take.

- [ ] **Decided (owner, 2026-07-11): stable monotonic turn numbers.** Each turn
      gets a chronological number assigned at creation and never renumbered; the
      marker reads e.g. `turns 5–16 elided…` and `recall` takes that range (or a
      sub-range). An opaque id buys nothing here — the dropped span is always a
      contiguous chronological block, so a plain number is legible and directly
      quotable. Compaction dropping turns 3–14 simply retires those numbers; later
      turns keep theirs (no positional renumbering). This refines Tech Spec §7/§16
      — file design feedback (G-24) so the Spec absorbs it at the next bump; do not
      edit the Spec from here.
- [ ] No stale-range handling needed under compaction (owner decision — see
      notes): the marker is **regenerated from the current conversation on every
      send**, so indices always reflect the present turn list. Compaction-discarded
      turns are represented by the summary (a normal pinned turn), which `recall`
      never needs to address.

## 4. `recall` built-in tool + engine gate  *(T-10, FR-3; Tech Spec §5.2, §7, §6)*

A new built-in that reads the session's own history — no filesystem, no network —
so, like `ask_user`, it bypasses the sandbox and is **not permission-gated**
(§6). Mirror the `ask_user` wiring.

- [ ] `RecallTool` in `crates/emberly-tools/src/builtin/recall.rs`; `ToolSpec`
      name `recall`, args = a turn range (or the marker id from group 3). Register
      in `default_registry()` (`builtin/mod.rs:26`).
- [ ] Engine-state access via a gate on `ToolCtx` mirroring `AskUserGate`:
      `RecallGate` trait + a `DeclineRecall`/default, a channel-backed engine impl
      alongside `AskGate` (`gate.rs:63`), installed in `make_ctx().with_*_gate`
      (`engine.rs:1766`), and serviced in the engine like `on_user_ask`
      (`engine.rs:1225`). The tool calls `ctx.recall(range)` and returns the
      result as a normal `ToolOutcome` (HC-6).
- [ ] **Data source: the engine's in-memory `self.conversation`** (the full,
      un-windowed vector — windowing never removes turns from it, group 2), sliced
      by the range and **reduced via `reduce_output`** (`reduce.rs:64`) so recall
      costs tokens proportional to what is recalled, **never raw JSONL** (T-10 —
      the raw path would re-inflate the elided noise and defeat the window).
      `recall` only ever addresses **window-dropped** turns, which are always in
      `self.conversation` (windowing is send-time only) — so there is **no
      compacted-range special case**: the compaction summary is a normal pinned
      turn, and turns compaction discarded are represented by it, not recalled
      (owner decision — see notes).
- [ ] Not permission-gated and no sandbox involvement (T-10, §6): re-reading the
      model's own context is not an action against the project. Assert this in a
      test (group 7).

## 5. Context-usage reflects the working window  *(FR-3; Design §8.6; Tech Spec §7)*

Design §8.6: the context-usage indicator must reflect the **working window**, so
"why did usage drop" is always answerable.

- [ ] Token accounting / `emit_context_usage` (`engine.rs`, emitted at
      `ingest_tool_result:1616` and elsewhere) counts the **windowed sent view**
      (pinned + last `window_turns` + marker), not the full `self.conversation`.
      `ContextUsage` (`event.rs`) then tracks what is actually sent (P-6 remains
      trigger-grade, not exact).
- [ ] A `recall` inflates the next request (the recalled turns ride in the tool
      result), so usage rising after a recall is expected and correct — verify it
      is reflected, not hidden.

## 6. UI: `recall` as ordinary quiet tool activity  *(Design §8.6, §4.5, §7)*

- [ ] `recall` renders as **ordinary, quiet tool activity** — one dim line via
      the existing `ToolStarted`/`ToolFinished` (`event.rs:39`, `:54`; TUI
      `app.rs:529`), optionally with a §4.5 explanation line — **no new `UiEvent`
      or `TranscriptEvent` variant** (unlike `ask_user`, which needs a blocking
      surface). It is the model reading its own context; keep it visibly distinct
      from the user-facing `/view` (§4.3) — the two never share a surface.
- [ ] The user's scrollback stays whole: windowing governs what is *sent*, so a
      window-dropped turn is **not** erased from the conversation the user reads
      (Design §8.6). Confirm the TUI conversation model is unaffected by send-time
      windowing.
- [ ] Degraded mode (§7): `recall` activity and any marker render in ASCII with
      no color-only meaning.

## 7. Tests + exit criterion  *(Tech Spec §14.6 offline, deterministic)*

- [ ] **Windowing:** with `window_turns` small, assert old turns are dropped from
      the **sent** request (inspect `build_request`/what `FakeProvider` receives)
      while `self.conversation`, the transcript, and the elision marker are
      intact; assert pinned content (system, `conversation[0]`, compaction
      summary) is always sent.
- [ ] **Clean boundary:** a window cut never splits a `tool_use`/`tool_result`
      pair — the sent message list is provider-valid.
- [ ] **`recall` round-trip via `FakeProvider`:** the model calls `recall` for a
      dropped range; assert it returns those turns in **normalized, reduced** form
      (not raw JSONL) and that it **never raises a permission prompt** (T-10, §6).
- [ ] **Composition with compaction:** windowing over a post-compaction
      conversation keeps the summary pinned (a normal turn) and windows only the
      turns around it; `recall` still targets only window-dropped turns, all
      present in `self.conversation` (no compacted-range branch).
- [ ] **Usage tracks the window:** `ContextUsage` reflects the sent window, not
      the full conversation (group 5).
- [ ] **Exit criterion (Phase 2 done when):** old turns are dropped from the sent
      context while the transcript and marker are intact; a `recall` round-trip
      returns dropped turns in normalized, reduced form and raises no permission
      prompt (Tech Spec §14.6, FR-3, T-10). Workspace clippy-clean under the §1
      lint policy; offline suite green.

---

## Decisions & notes log

> Fill in as the phase proceeds (mirrors the v0.1/v0.2 phase docs' logs).

- **Marker↔range identifier (owner decision, 2026-07-11): stable monotonic turn
  numbers.** The elision marker names the dropped span by chronological turn
  number (`turns 5–16 elided…`) and `recall` takes that range or a sub-range.
  Opaque ids rejected — the span is always a contiguous chronological block, so a
  plain number is legible and directly quotable, and it buys nothing over a
  sequential number. Numbers are assigned at turn creation and never renumbered
  (compaction retires numbers, never shifts them), so "turn 12" means the same
  thing all session. Refines Tech Spec §7/§16 → design feedback (G-24) at the next
  Spec bump.
- **Compaction × recall (owner decision, 2026-07-11):** once turns are compacted,
  treat the compaction summary as a **normal (pinned) turn**. This removes the
  special case: `recall` only ever brings back **window-dropped** turns, which —
  because windowing is a send-time view and never mutates `self.conversation` —
  are always present in memory. Turns compaction genuinely discarded are
  represented by the summary and are not something `recall` reaches for. The
  marker is regenerated per send, so turn indices stay consistent across
  compaction with no stale-range logic.
- **`recall` data source** — the engine's in-memory `self.conversation` (full,
  because windowing is send-time only), not a JSONL re-read; reduced via Phase 1's
  `reduce_output`.
- **No new event variants** — `recall` rides the existing `ToolStarted`/
  `ToolFinished`; windowing writes nothing to the transcript (send-time view
  only, HC-7). Revisit only if a dedicated recall surface earns its keep.
- Windowing is a **view over** `self.conversation`, never a mutation — the one
  invariant that keeps `recall`, the transcript, and the user's scrollback whole.
