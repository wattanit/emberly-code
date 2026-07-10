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
| 2. Adapter effort mapping + no-op | [ ] | anthropic→thinking-budget; openai→`reasoning_effort`; drop when unsupported |
| 3. `ReasoningDelta` stream event + trace translation | [ ] | adapter reasoning parts → normalized delta; signature preserve/replay |
| 4. Effort as engine state | [ ] | `Command::SetEffort`, `EffortChanged`, `effort_change`; threaded into requests; per-model config default |
| 5. Reasoning trace through engine + transcript | [ ] | `ReasoningDelta` UiEvent relay; distinct `reasoning` field; `hidden` still records |
| 6. UI — thinking trail + effort picker | [ ] | collapsed/expand/stream/settle; `reasoning` view key; picker + sidebar line; `/effort`; degraded block; line mode |
| 7. Tests, docs, exit criterion | [ ] | fake-provider round-trips; README; exit met |

**Overall Phase 3: NOT STARTED.**

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

- [ ] `anthropic` adapter: `effort` → a thinking-budget token count on the
      wire request (the mapping table lives in the adapter; document the
      four-level → budget mapping as "initial; tune with use"). `None` or an
      unsupported model ⇒ omit the thinking block entirely.
- [ ] `openai` adapter: `effort` → the `reasoning_effort` wire field
      (map `Max` to the provider's highest supported value; note the mapping
      inline). `None` ⇒ omit the field.
- [ ] No-op path: when a request carries an `Effort` the active model does
      not support, the adapter silently omits it — asserted by a test, since
      "setting it is never an error" is the P-9 contract.
- [ ] `fake` provider: echoes the received `effort` (e.g. via a captured
      last-request handle or a `ReasoningDelta`/text marker) so the engine
      round-trip test in group 4/7 can assert what the engine sent.
- [ ] Tests: anthropic maps each level to the expected budget and omits when
      `None`; openai maps each level to `reasoning_effort` and omits when
      `None`; unsupported-model no-op.

## 3. Reasoning trace — stream event + translation  *(Tech Spec §4.7, P-10)*

- [ ] `StreamEvent::ReasoningDelta { text: String }` — a new arm on the
      `#[non_exhaustive]` enum (additive; consumers already handle unknown
      variants). Distinct from `TextDelta`; never concatenated with it.
- [ ] `anthropic` adapter: translate `thinking`/`redacted_thinking` content
      blocks into `ReasoningDelta`; keep the block's opaque **signature**
      alongside the normalized reasoning so it can be echoed back on the next
      tool-use turn (multi-turn thinking). The signature stays **inside the
      adapter** — no wire detail leaks past the boundary (P-1). A provider
      without this requirement ignores it.
- [ ] `openai` adapter: translate the provider's reasoning-summary parts (if
      any) into `ReasoningDelta`; no signature echo required unless the wire
      demands it.
- [ ] `fake` provider: a scripted stream that emits `ReasoningDelta` frames
      interleaved before `TextDelta`, so the engine + UI trail can be tested
      headlessly.
- [ ] Tests: an SSE fixture with thinking blocks decodes to `ReasoningDelta`
      then `TextDelta` in order; the signature is preserved and replayed on a
      simulated follow-up tool-use turn; a stream with no thinking emits no
      `ReasoningDelta`.

## 4. Effort as engine state  *(C-6 effort half, Tech Spec §3.2, §6.6, §8)*

Mirrors the mode/`SwitchModel` machinery from earlier phases: per-session
state, switchable in-session, transcript-logged, announced never silent.

- [ ] `Command::SetEffort { effort: Effort }` (core) — issued at idle; the
      frontend gates it while a turn runs; applies to subsequent turns and
      never rewrites prior turns.
- [ ] `UiEvent::EffortChanged { effort: Effort }` (core) — the sidebar
      updates; the change is also announced via a `Notice` (never silent,
      Design §3.1), consistent with `ModelChanged`.
- [ ] `TranscriptEvent::EffortChange { effort: Effort }` — additive, no
      `SCHEMA_VERSION` bump (warn-skipped by older readers, like `ModelSwitch`
      was). An audit record (HC-7); the conversation view is unaffected.
- [ ] Engine holds the active `effort` in state, seeds it from the model's
      `default_effort` (and the per-model config default, §8), threads it
      into every `CompletionRequest`, and resets/re-seeds on `SwitchModel`
      to the new model's default. `SetEffort` updates it, logs the transcript
      event, emits `EffortChanged` + `Notice`.
- [ ] Config: a per-model `effort` default key (Tech Spec §8) resolved at
      construction and on reload, feeding `default_effort`. Absent ⇒ the
      model's built-in default.
- [ ] Tests: an engine test with the `fake` provider asserts the effort it
      sends changes after `SetEffort`, that `EffortChanged` + the transcript
      event are emitted, and that `SwitchModel` re-seeds the default.

## 5. Reasoning trace through the engine + transcript  *(Tech Spec §4.7, P-10)*

- [ ] Engine relays `StreamEvent::ReasoningDelta` as `UiEvent::ReasoningDelta
      { text }` (a new UiEvent arm) — the frontend streams it into the trail.
- [ ] The engine accumulates reasoning for the turn and records it as a
      **distinct field** on the assistant transcript event:
      `AssistantMessage { text, reasoning: Option<String> }` (additive field,
      `#[serde(default, skip_serializing_if = "Option::is_none")]`, no schema
      bump). Never concatenated into `text` (P-10).
- [ ] `hidden` view still writes the trace: the transcript record is written
      regardless of the view key — `hidden` is a view choice, never a discard
      (Design §4.4). Verified by a test.
- [ ] Tests: a `fake`-provider turn emitting reasoning produces a
      `ReasoningDelta` UiEvent and an `AssistantMessage` whose `reasoning` is
      populated and whose `text` excludes the reasoning; a turn with no
      reasoning leaves `reasoning` `None`.

## 6. UI — the thinking trail + effort picker  *(Design §4.4, §3.1)*

### Thinking trail (Design §4.4)
- [ ] Collapsed by default: a single dimmed line — `reasoning (N lines)` —
      with an expand affordance. Expanded, renders in secondary/chrome color,
      one visual step below assistant text so reasoning is always
      distinguishable from the answer.
- [ ] Live while streaming: the dimmed reasoning may stream in place; when the
      answer begins (`AssistantDelta`), the trail settles to its collapsed
      line unless the user pinned it open.
- [ ] `reasoning = collapsed | expanded | hidden` view key (Design §4.4),
      **default `collapsed`**. `hidden` suppresses the trail in the view only;
      the trace is still recorded (group 5). Threaded from config →
      `frontend::run` → the TUI (same pattern as `config_template`).
- [ ] Degraded mode (§7): the trail is a plain labeled block
      (`--- reasoning ---`), never color-only; defaults to collapsed via a
      one-line marker (`/view`-able). Line mode prints reasoning under the
      marker or omits it per the view key.

### Effort picker + sidebar (Design §3.1)
- [ ] `ChoiceKind::Effort` on the existing `Choices` overlay, rows fed by the
      active model's `ModelInfo.effort_levels`; selecting sends
      `Command::SetEffort`. When `effort_levels` is empty, the picker reports
      "this model has no effort control" rather than showing an empty list.
- [ ] Sidebar effort line (Design §3.1 model block): shows the current level;
      hidden/greyed when the model exposes none. Updated on `EffortChanged`.
- [ ] `/effort [level]` command + palette entry (`AppCommand::Effort`): no
      arg opens the picker; an arg sets it directly (validated against the
      model's levels). Line mode: `/effort <level>` sets it and prints the
      new level (no overlay), consistent with line-mode `/model`.

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

- **Effort-level → thinking-budget token mapping** (anthropic, §4.6): the
  four-level → budget table is "initial; tune with use". Values proposed in
  group 2, confirmed with the owner before commit. (Tech Spec Open Question:
  effort enum granularity, validate the four-level mapping against live
  endpoints in M6.)
- **`reasoning` view key default = `collapsed`** — already resolved in
  Design v0.4 (§4.4). No re-decision needed.
- **No `SCHEMA_VERSION` bump** for `EffortChange` / the `reasoning` field on
  `AssistantMessage` — both additive, warn-skipped by older readers, matching
  the `ModelSwitch` precedent from Phase 1.
