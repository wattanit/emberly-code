# Phase 4 — Interaction: ask-user & tool-call explanation — TODO & Progress

**Milestone:** M6 (Tech Spec §15) — the *interaction* slice of the 0.2 feature
set. Give the model an explicit "I need your input" channel (T-8) and a quiet,
honest caption for what a non-obvious tool call is doing (T-9).
**Satisfies:** T-8, T-9; Tech Spec §5.2, §5.4, §3.1/§3.2 (events), §14.5;
Design §4.5, §5.1. Pinned to **Req v0.5 / Design v0.5 / Spec v0.5**.
**Goal:** an `ask_user` round trip blocks the loop and resumes with the answer
(and a dismiss returns a structured decline), offline via `FakeProvider`; a
scripted non-obvious tool call renders a dim explanation caption and an obvious
one renders none; toggling `ui.tool_explanations = false` removes both the
schema property and the prompt instruction.

**Depends on:** the shipped v0.1/v0.2 product only. Every seam this phase needs
already exists and is used, not reshaped (guiding principle): the
`UiEvent`/`Command` channels (both `#[non_exhaustive]`), the `PermissionGate`
oneshot+mpsc round-trip (the exact template for `ask_user`'s blocking call), the
`Tool` trait + `default_registry()`, the single `ToolSpec → provider` choke
point in `Engine::build_request`, the file-based prompt with a `VERSION`
constant, the permission-prompt UI (rich + degraded) and the picker row idiom.
**Fully developable on macOS** — no sandbox involved; `ask_user` deliberately
touches no filesystem or network.

**Note — T-9 was a deferred v0.1 item now graduating.** The model-authored
explanation line was considered on 2026-07-07 and deferred pending a leaner
design; Requirements/Design/Spec v0.5 formalized it as T-9 (on by default,
config-defeatable) with exactly that lean design. This phase is its named door.

**Legend:** `[ ]` todo · `[~]` in progress · `[x]` done · `[!]` blocked

---

## Progress summary

| Group | Status | Notes |
|---|---|---|
| 1. Tool-call explanation — schema injection + prompt + event (T-9 provider/engine) | [x] | Done 2026-07-11; injection at `build_request` (both adapters); `ui.tool_explanations` default true; VERSION 2→3 + `tool_explanation.md`; `ToolStarted.explanation`; 7 tests |
| 2. Tool-call explanation — UI caption (T-9 TUI + degraded) | [x] | Done 2026-07-11; `ConvItem::Tool.explanation`; dim `↳` caption (rich) / `-` lead (line); absent→none; replay reads it from transcript args; 5 tests |
| 3. ask_user — round-trip plumbing + the tool (T-8 engine) | [x] | Done 2026-07-11; `AskId`, `AskAnswer`, `AskUserRequest`/`AskUserAnswer`, `AskGate` mirror of the permission gate (2nd channel + select arm + pending Vec), `AskUser` transcript event, `ask_user` tool registered; 4 tests |
| 4. ask_user — the question prompt UI (T-8 rich TUI) | [x] | Done 2026-07-11; `AskPrompt` state + `on_ask_key` + `render_ask` (dim_accent, never safety band); options + free-text; Enter never auto-answers; Esc→declined; motion stilled; 7 tests |
| 5. ask_user — degraded-mode question prompt (T-8 line) | [x] | Done 2026-07-11; `render_ask` (calm, no banner) + `parse_ask_answer` (number/text/empty→decline); single `Pending` enum so permission & question can't cross wires; no-ANSI sweep covers it; 3 tests |
| 6. End-to-end, exit criterion, docs & SFD bookkeeping | [x] | Done 2026-07-11; combined offline test; README; Spec decision = **no bump** (v0.5 already covers T-8/T-9); memory updated; 1 test |

**Overall Phase 4: COMPLETE** (6 / 6 groups). Branch
`phase4/ask-user-and-explanation` off `version0.2` (Phase 3 merged); not yet merged.

---

## Sequencing rationale (implementation-doc decision)

The plan lists ask_user first, but the two features are fully independent (no
shared types), so group order is free. This TODO leads with **T-9** (groups
1–2) because it is the smaller, lower-risk, self-contained slice — schema
injection + a caption line — and lands a green shippable increment before the
larger **T-8** round-trip machinery (groups 3–5). Group 6 proves both together
against the plan's "Done when". Each group leaves the tree green and, per the
guiding principle, ships the offline `FakeProvider`-driven tests of Tech Spec
§14.5 for its slice, with degraded-mode parity wherever it adds a UI surface.

---

## Platform & constraints (read first)

- **No new dependencies.** The whole phase is engine/config/TUI logic over the
  existing crate set (Tech Spec §12; guiding principle). Nothing here should
  want a new crate.
- **Hard constraints unchanged.** HC-1 (safe Rust), HC-3 (panic-free core),
  HC-6 (tool failures are data — `ask_user` returns a structured decline, never
  a harness error), HC-7 (both features are transcript events). `#![forbid(
  unsafe_code)]` everywhere; `#![deny(clippy::unwrap_used, expect_used)]` on
  core/providers/tools/sandbox — in-lib `#[cfg(test)]` uses `match { Ok=>.., Err
  =>panic!() }` helpers, never `.expect()` (codebase convention).
- **Additive-only across the boundary.** `UiEvent`/`Command`/`TranscriptEvent`
  are `#[non_exhaustive]`; new variants are not breaking. The explanation rides
  in existing `args` and the transcript already records full `args`, so **no
  `transcript::SCHEMA_VERSION` bump** (Tech Spec §5.4). The new `ask_user`
  transcript variant is additive — old readers warn-skip — so **no schema bump**
  there either (transcript.rs notes this for `ModelSwitch`/`EffortChange`).
- **The safety band is reserved.** `theme::safety_band()` is the
  outside-project-root permission signal and nothing else, ever (Design §2/§5).
  The question prompt (T-8) MUST use calm styling — the same styling the
  *routine* (non-outside-root) permission prompt uses — never the band. Diluting
  the one loud signal is a design defect.
- **`ask_user` bypasses the sandbox but flows through the `Tool` trait** (Tech
  Spec §5.2): it is a pure engine↔frontend round-trip, so it never touches
  `ToolCtx::sandbox`/filesystem, but it is a real `Tool` in `default_registry()`
  and reaches the frontend only through a gate mirroring the permission gate.

---

## 1. Tool-call explanation — schema injection + prompt + event  *(T-9; Tech Spec §5.4, §3.1; Design §4.5)*

The provider/engine half: make the model *able and asked* to explain, gated by
config, and surface the explanation to the frontend.

**Done 2026-07-11.** Injection lives in `Engine::build_request` (the one
`ToolSpec→ToolSchema` bridge), so both wire adapters get it with no per-adapter
code. The instruction is a new `prompts/tool_explanation.md` (not baked into
`system.md`), appended by `Engine::effective_system` only when the toggle is on
— so with it off, `system.md` is byte-identical to v2's and no `explanation`
property is sent. `VERSION` 2→3 (the prompt *set* gained a file; the actual
bytes already vary by project instructions, so this matches how VERSION works).
Extraction (`explanation_from_args`) is unconditional and trims blanks to
`None`. 7 tests (3 in-lib helper unit tests, 3 engine integration via
`FakeProvider::last_request`, 1 config parse/merge).

- [x] **`ui.tool_explanations` config key, default `true`** (Tech Spec §8,
      Requirements T-9). No `[ui]` table exists yet — add a `UiConfig`
      (all-optional) to `ConfigFile` (`crates/emberly/src/config.rs`), resolve
      onto `Resolved` (default true), following the `reasoning` view-key
      pattern. Provenance (`config show`) must remain correct.
- [x] **Thread the toggle to the one choke point.** `EngineConfig`
      (`crates/emberly-core/src/engine.rs`) gains `tool_explanations: bool`;
      `main.rs` populates it from `resolved`; `Engine` stores it. This is the
      wire T-9 needs — nothing about `ui.*` reaches the engine today.
- [x] **Inject the optional `explanation` property at the single
      `ToolSpec → provider` point** — `Engine::build_request` (where
      `self.tools.specs()` becomes `Vec<ToolSchema>`). When
      `tool_explanations` is on, mutate each spec's `input_schema`
      (`serde_json::Value`) to add a string `explanation` property (a `Value`
      mutation on `["properties"]["explanation"]`; **not** added to `required`).
      Covers **both** adapters at once — no edit to anthropic.rs/openai.rs
      `build_body`, which stay generic over `ToolSchema`. When off, the property
      is never added, so the model is never prompted and no tokens are spent.
- [x] **Bump `prompts::VERSION` 2 → 3** and add the instruction to
      `crates/emberly-core/prompts/system.md`: fill `explanation` briefly and
      **only** for calls whose intent is not self-evident; add a
      `prompts/CHANGELOG.md` entry. The instruction is emitted **only when the
      toggle is on** — if the property is omitted, the prompt must not ask for
      it (Requirements T-9: "no tokens spent"). *(Decide at build: gate the
      instruction by composing the system prompt, since VERSION is a compile
      constant — the instruction text can be conditional on `tool_explanations`
      when assembling the system message. Prompt wording is drafted for owner
      review at this group.)*
- [x] **`explanation: Option<String>` on `UiEvent::ToolStarted`** (§3.1). The
      engine extracts it from the tool-call `args` (the `explanation` key, if
      present and non-empty) when emitting `ToolStarted`, then the tool ignores
      it during `execute` (no tool declares `deny_unknown_fields`).
- [x] **Transcript:** confirm `tool_call` already records full `args` so the
      explanation persists with **no `SCHEMA_VERSION` bump** (Tech Spec §5.4);
      add a test asserting the round trip through the transcript.
- [x] Tests (`FakeProvider`, §14.5): schema property present when on / absent
      when off; prompt instruction present iff on; engine surfaces a supplied
      `explanation` onto `ToolStarted`; a call with no `explanation` in args
      yields `None`; `explanation` never lands in `required`.
- [x] `cargo fmt` + `clippy` + `test` green; commit.

---

## 2. Tool-call explanation — UI caption  *(T-9; Design §4.5; Tech Spec §9)*

The call stays the headline; the explanation is the caption.

- [x] **Rich TUI:** `ConvItem::Tool` carries `explanation: Option<String>`
      (from `ToolStarted`); render a **single dimmed caption line directly
      under the call**. Absent when `None` — **no placeholder**. Never styled as
      a result or error; never color-only (Design §4.5/§7) — the dim + position
      under the call carries it without relying on colour.
- [x] **Degraded/line mode** (`line.rs`): the caption renders too (plain,
      dimmed-by-convention prefix), absent when none — degraded parity, no ANSI.
- [x] Tests: caption rendered when present / absent when none (both frontends);
      line-mode emits no ANSI escape with an explanation in the stream; caption
      is not the result line.
- [x] `cargo fmt` + `clippy` + `test` green; commit.

---

## 3. ask_user — round-trip plumbing + the tool  *(T-8; Tech Spec §5.2, §3.1/§3.2)*

Mirror the permission-gate blueprint exactly; the only real difference is the
richer reply payload (a chosen option / free text / decline, not Allow/Deny).

- [x] **Id + boundary types.** `AskId(pub u64)` in `id.rs` (sibling of
      `PermissionId`, engine-minted, monotonic). A self-contained rendering
      payload in `types.rs` (`AskUserRendering { question, options: Vec<String>
      }`) and an answer type (`AskUserAnswer` — a selected option index / free
      text, or **declined**; model as `enum { Answer(String), Declined }` or
      `Option<String>` with `None` = declined — decide in-group, favour an
      explicit `Declined` for clarity in the transcript).
- [x] **New UiEvent/Command variants** (both `#[non_exhaustive]`, additive):
      `UiEvent::AskUserRequest { id, question, options }` and
      `Command::AskUserAnswer { id, answer }`.
- [x] **`AskGate` — the second gate.** In `gate.rs`, an `AskUserAsk { request,
      reply: oneshot::Sender<AskUserOutcome> }` and an `AskGate { asks:
      mpsc::Sender<AskUserAsk> }` mirroring `ChannelGate`; fail **closed to a
      structured decline** if the engine is gone (HC-6 — never an error). Add an
      `ask_user(question, options) -> AskUserOutcome` method to `ToolCtx`
      (`ctx.rs`) + a `AskUserGate` trait in `emberly-tools` (parallel to
      `PermissionGate`); this is the tool's only channel for this round trip.
- [x] **Engine wiring.** A second `mpsc` (`ask_rx`) created in `Engine::new`
      and threaded to `run_one_tool_call`; a `PendingAsk`-equivalent local
      `Vec`; an extra `Some(ask) = ask_rx.recv()` arm **and** a
      `Command::AskUserAnswer` arm in the same `tokio::select!` that already
      drives the tool future + permission asks. Handlers mirror
      `on_permission_ask` (mint id, emit `AskUserRequest`, stash pending) and
      `answer_permission` (match id, `reply.send(answer)`, write transcript).
      Cancel/frontend-gone paths resolve every pending ask to `Declined`.
- [x] **Transcript:** `TranscriptEvent::AskUser { question, options, answer }`
      (question+options+answer/decline — §3.2), adjacent to the permission
      variants; additive, **no schema bump**.
- [x] **The `ask_user` tool** in `crates/emberly-tools/src/builtin/ask_user.rs`,
      registered in `default_registry()`. `spec()` declares `question:
      string` (required) + `options: array<string>` (optional); `execute`
      deserializes, calls `ctx.ask_user(...)`, and returns a `ToolOutcome`
      carrying the typed answer, or a structured `{declined:true}` content on
      decline (HC-6). Touches no filesystem/sandbox. `describe()` → a short
      `ask: <question>` summary.
- [x] Tests (`FakeProvider`, §14.5, no terminal): a scripted `ask_user` tool
      call blocks the loop; injecting `Command::AskUserAnswer` resumes with the
      answer as tool-result content; a decline yields the structured
      `{declined:true}`; the transcript records question + answer/decline;
      frontend-gone → decline, loop still resumable.
- [x] `cargo fmt` + `clippy` + `test` green; commit.

---

## 4. ask_user — the question prompt UI (rich TUI)  *(T-8; Design §5.1, §6.4)*

> Looks and behaves unlike a permission prompt: it invites input and has **no
> dangerous default**. Mirror the permission-prompt *machinery* (owns the main
> area, keyboard-exclusive, every exit yields an answer over the oneshot), borrow
> the picker's row-selection idiom for options, add a free-text line. Do **not**
> overload `OverlayContent::Choices` (that entangles blocking semantics with the
> dismissable overlay stack).

- [x] **State + dispatch:** `pending_ask: Option<(AskId, AskUserRendering)>` on
      `App`, set on `AskUserRequest`; a dedicated `on_ask_key`; slot it into the
      `on_key` precedence next to the permission prompt (a question prompt is
      never opened over a permission prompt and vice-versa — one blocking
      decision surface at a time).
- [x] **Calm styling — never the safety band.** `render_ask` takes over the
      main area using the routine `dim_accent()` border + `accent()`/`chrome()`
      heading/body — the exact calm treatment of a non-outside-root permission
      prompt, minus any `error()`/`safety_band()` path. A render test asserts the
      safety band is **absent**.
- [x] **Options + free text.** When the model offered options, render them as a
      selectable list (↑/↓ + a selected index, borrowing the picker idiom) with
      a number/letter to pick; a free-text answer is **always** available (reuse
      `LineEditor` for the field, grapheme-correct per §6.2).
- [x] **No unsafe default** (Design §5.1): **Enter never auto-selects** on the
      user's behalf — Enter submits the *typed* text (or the *explicitly moved-to*
      selection), never a default option. **Esc → `Declined`** (a real answer
      over the oneshot, "user declined to answer"). A stray key is ignored.
- [x] **No motion** while the question is up (Design §6.4) — the ticker is
      gated off exactly as for the permission prompt.
- [x] **Strings:** a new `strings::ask_user` module (kept **separate** from
      `strings::permission` so it is obvious it does not share the safety
      vocabulary): title, prompt hint, decline hint.
- [x] Tests (`TestBackend`): calm render (no safety band, question + options +
      input line shown); Enter-doesn't-auto-answer; Esc→`Declined`; moving the
      selection then Enter picks that option; free-text submit; motion gated off.
- [x] `cargo fmt` + `clippy` + `test` green; commit.

---

## 5. ask_user — degraded-mode question prompt (line)  *(T-8; Design §5.1, §7)*

Full parity — the interaction is identical in meaning, ASCII-only.

- [x] `line.rs`: `render_ask` (blank line, calm ASCII heading — **no** capitals
      safety banner, this is not a safety prompt; the question, numbered options
      if any, a free-text prompt, and a plain "Esc/empty = declined" hint) +
      `parse_ask_answer` (a number/letter → that option; a non-empty line →
      free-text; empty → `Declined`).
- [x] **Two pending slots.** The run loop currently tracks a single pending
      permission id; add a second slot (or a small `Pending` enum) so a question
      prompt and a permission prompt cannot collide, and the right parser
      consumes the next stdin line.
- [x] Tests: no ANSI escape over a stream containing an `AskUserRequest`;
      numbered-option parse; free-text parse; empty → declined.
- [x] `cargo fmt` + `clippy` + `test` green; commit.

---

## 6. End-to-end, exit criterion, docs & SFD bookkeeping  *(Tech Spec §14.5; SFD G-10/G-11/G-16)*

**Done 2026-07-11.** Combined test `explanation_and_ask_user_together` proves
both features in one offline session; the per-group tests cover the decline
path and the toggle-off path. README section added. **Spec decision: no bump**
— v0.5 already specifies T-8 (§5.2), T-9 (§5.4), and the events (§3.1/§3.2), and
the implementation introduced no deviation from them (`AskId`/`AskAnswer`/
`AskUser`/`AskUserRequest`/`AskUserAnswer` are direct realizations of the named
spec items), so Req/Design companion pins are untouched (G-16). Memory
`tool-explanation-deferred` updated to "implemented".

- [x] **Combined offline round trip** proving the plan's "Done when": one
      `FakeProvider` session that (a) makes an `ask_user` call → loop blocks →
      answer resumes with the answer, and a second where a dismiss returns a
      structured decline; (b) scripts a non-obvious tool call that renders its
      dim explanation and an obvious one that renders none; (c) asserts
      `ui.tool_explanations = false` removes **both** the schema property and the
      prompt instruction.
- [x] **Honest scope note:** offline coverage rides on `FakeProvider`; the live
      half (a real provider actually authoring `explanation` and calling
      `ask_user`) is an owner-run manual/nightly smoke (Tech Spec §14.4), same
      posture as Phase 3's reasoning paths. Record it here, don't claim it.
- [x] **README:** short "Asking you a question · tool-call explanations"
      section.
- [x] **Spec bump decision (owner).** Spec v0.5 already fully specifies T-8
      (§5.2), T-9 (§5.4), and the events (§3.1/§3.2) — expected outcome is **no
      bump**. If implementation surfaces an additive refinement (as Phase 3 did),
      surface it and bump minor with owner approval; refresh Req/Design companion
      pins only if the Spec bumps (G-16).
- [x] **Memory:** update `tool-explanation-deferred` → implemented in v0.2
      Phase 4 (T-9), so it is no longer "deferred".
- [x] Full suite + `fmt` + `clippy` green; final commit.

---

## Decisions log

- **Branch:** `phase4/ask-user-and-explanation` off `version0.2` (Phase 3
  merged).
- **Group order T-9 → T-8** (implementation-doc call): smaller independent slice
  first; features share no types.
- **Single injection choke point** = `Engine::build_request` (the
  `ToolSpec → ToolSchema` bridge), not the two adapter `build_body`s — one edit
  covers both wire formats and keeps the adapters generic.
- **`explanation` is optional and off `required`** — a call omitting it is
  normal, and the UI shows no placeholder (Design §4.5).
- **Toggle off = zero tokens** — property omitted **and** prompt instruction
  omitted; the config key defeats the feature entirely (Requirements T-9).
- **`ask_user` mirrors the permission gate** (oneshot+mpsc+pending-Vec+`select!`
  arm), not the overlay/picker stack — it is a turn-blocking, id-correlated
  round trip where every exit path must yield an answer; the picker's "Esc =
  silently nothing" semantics don't fit.
- **Question prompt is calm, never the safety band** (Design §2/§5.1); **no
  unsafe default**, Esc = structured decline (Design §5.1, refines T-8).
- **No schema bumps:** `explanation` rides in `args`; the `ask_user` transcript
  variant is additive and warn-skipped by old readers (Tech Spec §5.4, §3.2).
- **T-9 graduates a deferred v0.1 item** — see the note at the top; update the
  memory at close.
