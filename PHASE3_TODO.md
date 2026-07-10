# Phase 3 — Reasoning effort & the thinking trail — TODO & Progress

**Milestone:** M6 group 3 (Tech Spec §15). Let the user own the
latency/cost/quality trade per task (P-9), and make the model's reasoning
visible without imposing it (P-10). Completes the effort half of C-6 that
Phase 1 left open.
**Satisfies:** P-9, P-10; C-6 (effort half); Tech Spec §4.6, §4.7,
§3.1/§3.2 (events), §9; Design §3.1, §4.4. Pinned to Req v0.5 / Design v0.5
/ Spec v0.4.
**Goal:** an effort round-trip is observable per live provider that
supports one and a no-op on one that doesn't; a `FakeProvider` emitting
`ReasoningDelta` renders as a collapsed, expandable trail and is recorded
distinctly in the transcript; the `hidden` view still writes the trace.

**Depends on:** Phase 1 (merged) — the picker surface (`OverlayContent::
Choices` / `ChoiceKind`), the injected-capability + `SwitchModel` pattern,
and the `ModelInfo` plumbing are the templates this phase reuses. The
effort picker is the same overlay as the model picker.

**Legend:** `[ ]` todo · `[~]` in progress · `[x]` done · `[!]` blocked

---

## Progress summary

| Group | Status | Notes |
|---|---|---|
| 1. Effort model (data types) | [x] | Done 2026-07-10; `Effort` enum + `CompletionRequest.effort` + `ModelInfo` levels/default; 7 tests |
| 2. Adapter effort mapping + no-op | [x] | Done 2026-07-10; anthropic→thinking-budget (2k/8k/16k/32k, clamped); openai→`reasoning_effort` (Max→high); no-op when unsupported; 10 tests |
| 3. `ReasoningDelta` stream event + trace translation | [x] | Done 2026-07-11; `ReasoningDelta`+`ReasoningSignature` events, `ContentBlock::Reasoning`; anthropic thinking/redacted capture + replay; openai `reasoning_content`; 8 tests |
| 4. Effort as engine state | [x] | Done 2026-07-11; `SetEffort`/`EffortChanged`/`EffortChange`; seeded+threaded+re-seeded on switch; config `effort`/`effort_levels`; 6 tests |
| 5. Reasoning trace through engine + transcript | [x] | Done 2026-07-11; `UiEvent::ReasoningDelta` relay; distinct `reasoning` field on `AssistantMessage`; `Reasoning` block prepended; 2 tests |
| 6. UI — thinking trail + effort picker | [x] | Done 2026-07-11; trail (collapse/expand/settle, Ctrl-R); `reasoning` view key; effort picker + sidebar + `/effort`; line-mode block; README; 9 tests |
| 7. Tests, docs, exit criterion | [ ] | fake-provider round-trips; README; exit met |

**Overall Phase 3: groups 1–6 complete; group 7 (final tests/docs/exit)
remaining.** Effort is a config-declared, in-session-switchable control mapped
per adapter (no-op when unsupported); the reasoning trace is captured,
replayed, streamed to a collapsed/expandable trail, and recorded distinctly.
Workspace clippy + fmt clean; 21 test binaries green (TUI 111).

---

## 1. Effort model — data types  *(Tech Spec §4.6, P-9)*

Pure data in `emberly-providers`; no wire mapping yet. Establishes the
vocabulary the adapters and engine share.

- [x] `Effort` enum (`Low | Medium | High | Max`) in `model.rs`.
      `Serialize`/`Deserialize` (snake_case), `Copy`, `Ord` (low→max),
      `as_str`/`Display`/`parse` + `ALL` for logs, transcript, and CLI parsing.
- [x] `CompletionRequest.effort: Option<Effort>` — `#[serde(default,
      skip_serializing_if = "Option::is_none")]`; `CompletionRequest::new`
      leaves it `None`. The engine's two request builders set `effort: None`
      for now (summarize permanently; the turn builder until group 4).
- [x] `ModelInfo` gains `effort_levels: Vec<Effort>` (empty ⇒ no control; UI
      hides the picker) and `default_effort: Option<Effort>`, both
      `#[serde(default)]`. The `fake` provider declares the full ladder
      (default `Medium`) for headless round-trips; live/config providers stay
      empty until adapters/config populate them (groups 2/4).
- [x] Unit tests (7): enum serde round-trip + case-insensitive parse +
      ordering; `CompletionRequest` without `effort` deserializes to `None`
      and `new` omits it; `ModelInfo` with no effort fields deserializes to
      empty/`None` and with them round-trips. (In-lib tests use the
      `match/panic` helper, per the crate's `expect_used` deny.)

## 2. Adapter effort mapping + no-op  *(Tech Spec §4.6, P-9)*

Each adapter maps the normalized `Effort` to its provider's native control,
or drops it — a no-op is never an error (P-9).

- [x] `anthropic` adapter: `effort` → `thinking.budget_tokens`. Owner-approved
      ladder (2026-07-10): Low 2k / Medium 8k / High 16k / Max 32k, **clamped**
      to `max_output − 1024` (Anthropic requires `1024 ≤ budget < max_tokens`;
      a 1024-token reserve leaves room for the answer). Too-small `max_output`
      (< 2048) ⇒ omit the block. Temperature override dropped when thinking is
      on (the API forbids it).
- [x] `openai` adapter: `effort` → `reasoning_effort` (`low`/`medium`/`high`);
      `Max`→`high` since the field has no higher value (noted inline). `None`
      ⇒ omit.
- [x] No-op path: an adapter maps effort only when `model_info.effort_levels`
      is non-empty (the model declares reasoning control). Empty ⇒ the field is
      silently omitted even if the request carries an effort — asserted by a
      test in each adapter (P-9: "never an error").
- [x] `fake` provider: captures the last request (`last_effort()` accessor) so
      the engine round-trip test in group 4 can assert the effort sent.
- [x] Tests (10): anthropic per-level budget + clamp + tiny-allowance omit +
      no-effort omit + unsupported no-op + temperature-drop; openai per-level
      (incl. Max→high) + no-effort omit + unsupported no-op.

## 3. Reasoning trace — stream event + translation  *(Tech Spec §4.7, P-10)*

- [x] `StreamEvent::ReasoningDelta { text }` **and** `ReasoningSignature
      { signature, redacted }` — two additive arms on the `#[non_exhaustive]`
      enum. Delta is the streamed reasoning text (UI/transcript); Signature
      carries the opaque replay token once the block closes. Distinct from
      `TextDelta`; never concatenated.
- [x] `ContentBlock::Reasoning { text, signature, redacted }` — the normalized
      captured block, carried in the conversation so an adapter that requires
      reasoning echoed back can replay it. `signature` is an opaque `Option
      <String>`, never interpreted by the engine (P-1); `redacted` marks an
      encrypted block whose text was withheld.
- [x] `anthropic` adapter: mapper translates `thinking_delta`→`ReasoningDelta`,
      accumulates `signature_delta` and flushes it as `ReasoningSignature` at
      block stop, and surfaces `redacted_thinking` as a placeholder delta +
      preserved `data`. `block_to_anthropic` replays a `Reasoning` block as a
      `thinking` (or `redacted_thinking`) wire block — the only place the wire
      shape/signature is known (P-1).
- [x] `openai` adapter: maps a `reasoning_content` / `reasoning` delta →
      `ReasoningDelta`; no signature echo on this wire. `Reasoning` blocks in
      the conversation are naturally skipped by `message_to_openai`.
- [x] `fake` provider: no change needed — `ScriptedResponse` already carries
      arbitrary `StreamEvent`s, so tests script `ReasoningDelta` frames directly.
- [x] Tests (8): anthropic thinking stream → delta/delta/signature/text in
      order; redacted → placeholder + preserved data; plain text emits no
      reasoning; `Reasoning` replays as `thinking` w/ signature and as
      `redacted_thinking`; openai `reasoning_content` → delta and plain content
      emits none. Engine context-count includes replayed reasoning; summary
      skips it.

## 4. Effort as engine state  *(C-6 effort half, Tech Spec §3.2, §6.6, §8)*

Mirrors the mode/`SwitchModel` machinery from earlier phases: per-session
state, switchable in-session, transcript-logged, announced never silent.

- [x] `Command::SetEffort { effort }`, `UiEvent::EffortChanged { effort }`,
      `TranscriptEvent::EffortChange { effort }` — `Effort` re-exported through
      `core::types` so all three share one type. Transcript event is additive,
      no `SCHEMA_VERSION` bump (warn-skipped, like `ModelSwitch`).
- [x] Engine holds `effort: Option<Effort>`, seeds it from the model's
      `default_effort` at construction, threads it into the turn request
      builder (summarize stays `None`), and re-seeds on `SwitchModel` (emitting
      `EffortChanged` when the new default differs). `set_effort` is a no-op
      when unchanged, else logs the transcript event and emits `EffortChanged`
      + a `Notice`. Mid-turn `SetEffort` is ignored (applies next turn).
- [x] Config: per-model `effort` (default level; presence enables the control)
      + optional `effort_levels` subset (defaults to the full ladder), resolved
      in `provider_setup` into `ModelInfo.default_effort`/`effort_levels`.
      `ModelFile` lost `Copy` (now `Clone`) to hold the `String`/`Vec`.
- [x] Tests (6): engine — effort seeded/threaded/re-seeded, `EffortChanged` +
      `Notice` + `EffortChange` transcript, switch re-seed (via `last_effort()`
      + a re-seeding `ProviderFactory`); config — default+full-ladder, no-effort
      ⇒ no control, explicit level subset.

## 5. Reasoning trace through the engine + transcript  *(Tech Spec §4.7, P-10)*

- [x] Engine relays `StreamEvent::ReasoningDelta` → `UiEvent::ReasoningDelta
      { text }` and stashes `ReasoningSignature` into a per-turn `TurnOutput`
      (text + reasoning + opaque signature), replacing the old `(StreamEnd,
      String)` return.
- [x] The engine records reasoning as a **distinct field** —
      `AssistantMessage { text, reasoning: Option<String> }` (additive, serde
      default/skip, no schema bump) — and prepends a `ContentBlock::Reasoning`
      to the assistant message (before text/tool_use, Anthropic ordering) so
      the signature replays. Never concatenated into `text` (P-10).
- [x] `hidden` writes the trace: the engine always records reasoning; hiding is
      the frontend's view choice (group 6). Verified by a test. `resume` does
      **not** reconstruct reasoning blocks (signature not persisted; historical
      thinking may be stripped — only the live turn needs replay).
- [x] Tests (2): a `fake`-provider turn emitting reasoning yields a
      `ReasoningDelta` UiEvent + an `AssistantMessage` whose `reasoning` is set
      and `text` excludes it; a turn with no reasoning leaves `reasoning` `None`.

## 6. UI — the thinking trail + effort picker  *(Design §4.4, §3.1)*

### Thinking trail (Design §4.4)
- [x] `ConvItem::Reasoning { text, expanded }`: collapsed default renders a dim
      `▸ reasoning (N lines)`; expanded renders `▾ reasoning` + the text in
      chrome colour, one step below the answer. **Ctrl-R** toggles the latest
      trail (the expand affordance).
- [x] Live/settle: the trail streams expanded while thinking; on the first
      `AssistantDelta` it settles to collapsed unless the view pins it
      (`Expanded`).
- [x] `ReasoningView` (collapsed|expanded|hidden), **default collapsed**, from
      the `reasoning` config key. Threaded config → `Resolved.reasoning` →
      `frontend::run` (parses) → `tui::run`/`line::run`. `hidden` skips the
      view item (engine still records — group 5).
- [x] Degraded mode (§7): `LineRenderer` prints a plain `--- reasoning ---`
      block then `--- answer ---`; `hidden` suppresses it. (Stateful renderer:
      one toggle flag for the block label.)

### Effort picker + sidebar (Design §3.1)
- [x] `ChoiceKind::Effort` on the `Choices` overlay, rows from the model's
      levels (carried on `EffortChanged { effort, available }`, emitted at
      startup/set/switch); Enter → `Command::SetEffort`. Empty levels ⇒ a calm
      "no reasoning-effort control" notice, no overlay.
- [x] Sidebar effort line: shows the current level; hidden when the model
      exposes none. Updated on `EffortChanged`.
- [x] `/effort [level]` + palette entry (`AppCommand::Effort`): no arg opens
      the picker; an arg sets it, validated against the model's levels (a
      not-offered level is a notice, not a command). Line mode: `/effort
      <level>` sends `SetEffort`; no arg prints usage.

## 7. Tests, degraded mode, docs & exit criterion

- [ ] Unit + integration: the group 1–6 tests above, plus a full
      `fake`-provider engine turn asserting effort round-trip **and** a
      reasoning trail end-to-end (delta → UiEvent → distinct transcript
      field).
- [ ] Degraded-mode parity: line mode `/effort <level>` and the reasoning
      block are smoked; the rich-TUI trail collapse/expand and picker are
      covered by render/app unit tests (the interactive glow is the
      `TerminalGuard` path, as in Phase 2).
- [ ] README: a "Reasoning effort and the thinking trail" section — the
      `/effort` command + picker, the per-model config default, the
      `reasoning` view key and its `collapsed`/`expanded`/`hidden` meaning
      (and that `hidden` still records), and the plain-mode behavior.
- [ ] **Exit criterion met:** effort is observable in the request per live
      provider that supports one and a no-op on one that doesn't; a
      `FakeProvider` emitting `ReasoningDelta` renders as a collapsed,
      expandable trail and is recorded in a distinct transcript field; the
      `hidden` view still writes the trace; `/effort` switches per session and
      is announced. Lint gates + suite green.

---

## Decisions & notes log

Open decisions to confirm with the owner as they arise (do not guess — SFD
G-22 routes these to the owner):

- **Effort-level → thinking-budget token mapping** (anthropic, §4.6):
  **owner-approved 2026-07-10** — Low 2,048 / Medium 8,192 / High 16,384 /
  Max 32,768, clamped to `max_output − 1024`, floored at 1,024, omitted when
  `max_output < 2048`. "Initial; tune with use." Still the Tech Spec Open
  Question to validate against live endpoints in M6.
- **`reasoning` view key default = `collapsed`** — already resolved in
  Design v0.4 (§4.4). No re-decision needed.
- **No `SCHEMA_VERSION` bump** for `EffortChange` / the `reasoning` field on
  `AssistantMessage` — both additive, warn-skipped by older readers, matching
  the `ModelSwitch` precedent from Phase 1.
- **`EffortChanged` reshaped in group 5/6:** from group 4's `{ effort: Effort }`
  to `{ effort: Option<Effort>, available: Vec<Effort> }`, emitted at
  startup/set/switch, so one event drives both the sidebar (current level) and
  the picker (offered levels) across the process boundary. Pre-release, so no
  compat concern; the transcript `EffortChange` stays `{ effort: Effort }`.
- **Spec feedback to raise in group 7 (docs):** the Spec §4.1 event list names
  only `ReasoningDelta`; implementation added a companion `ReasoningSignature`
  event and a normalized `ContentBlock::Reasoning { text, signature, redacted }`
  so the opaque provider signature survives across turns for replay (§4.7's
  "echoed back on subsequent tool-use turns"). The signature is opaque — no
  wire *type* crosses the boundary, satisfying P-1 in substance. Record as a
  downstream feedback entry (Spec §4.1/§4.7) at the next Spec bump (SFD G-24).
- **Resume does not restore the last in-session effort:** a resumed session
  re-seeds from the model's `default_effort` rather than replaying the last
  `EffortChange` from the transcript. Effort is per-session state; restoring it
  on resume is a possible later refinement, not in this phase's scope.
- **Redacted-thinking replay is best-effort, untested live:** captured as a
  placeholder delta + preserved `data` and replayed as `redacted_thinking`, but
  never exercised against a live endpoint (redacted blocks are rare). Noted so
  it is not mistaken for verified.
