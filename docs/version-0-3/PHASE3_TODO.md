# Phase 3 — Compaction completion: `/compact` surface + automatic compaction (FR-4) — TODO & Progress

**Milestone:** M7 group 3 (Tech Spec §15) — the 0.3 context-economy set, **layer
4** of Requirements §8.4 (compaction, manual and automatic — the last and only
layer that calls the model, at a clean boundary).
**Satisfies:** FR-4 and the §8.3 `/compact` command-surface gap; Tech Spec §7,
§3.2 (`trigger`), §8, §9, §15 (M7); Design §3.3, §8.6, §6.1; HC-7 (log
untouched). Pinned to Req v0.6 / Design v0.6 / Spec v0.7 (all `approved`).
**Goal:** Finish compaction on both ends. (a) Wire the missing `/compact`
command surface into the registry so it is reachable via palette, `/command`,
and keybinding. (b) Add automatic compaction: when context usage crosses a high
threshold, schedule a compaction at the next clean boundary — same mechanism as
manual, differing only in trigger — on by default, disableable, no thrash, and
surfaced in the harness voice.

**Depends on:** the shipped product (M1–M6) and Phases 1–2. The compaction
**engine** already exists (shipped M5): `Engine::compact()`
(`crates/emberly-core/src/engine.rs:740`), `summarize()` (`:812`),
`Command::Compact` (`command.rs:66`), the `compact_requested` queue (`:431`, set
`:997/:1193/:1270`, drained at the post-turn clean boundary `:589–594`), pinned
content + clean-boundary + summarization-failure fallback all in place. Phase 2
added the `[context]` config (`ContextConfig`, `engine.rs:74`) and made
`emit_context_usage()` (`engine.rs:2046`) count the **windowed** view — the exact
hook the auto threshold needs. What is missing is the TUI surface, the trigger,
and the config knobs.

**Legend:** `[ ]` todo · `[~]` in progress · `[x]` done · `[!]` blocked

---

## Progress summary

| Group | Status | Notes |
|---|---|---|
| 1. Wire the `/compact` command surface (registry + slash + palette + key) | [ ] | closes the existing gap |
| 2. Config: `context.auto_compact` (true) + `context.auto_compact_threshold` (0.85) | [ ] | |
| 3. Automatic-compaction trigger in the accounting path (+ no-thrash latch) | [ ] | |
| 4. `trigger: manual \| auto` transcript field (additive, no schema bump) | [ ] | |
| 5. Harness-voice surfacing (turn count + auto reason) | [ ] | |
| 6. Tests (offline, deterministic — §14.6) + exit criterion | [ ] | |

**Overall Phase 3: NOT STARTED.**

---

## 1. Wire the `/compact` command surface  *(Requirements §8.3; Design §3.3; Tech Spec §9, §15)*

The engine services `Command::Compact` today, but **no TUI code ever sends it** —
only tests do (`engine_loop.rs:392`). Close the gap; copy the `/model` wiring
pattern.

- [ ] Add `AppCommand::Compact` to the command enum
      (`crates/emberly-tui/src/commands.rs:12`) and a `CommandSpec` entry in
      `COMMANDS` (`:61`) — `name: "compact"`, a one-line description, so it flows
      automatically into the palette fuzzy list, `/help` (`app.rs:1952`), and
      `commands::by_name` slash lookup (the single source of truth, Tech Spec §9,
      Design §3.3).
- [ ] Dispatch: add the arm in `App::run_command()` (`app.rs:1494`) that sends
      `Action::Command(Command::Compact)` to the engine (the missing link;
      `/model` reaches `open_model_picker` at `:1552`, `/compact` just fires the
      command). Reachable via `/compact` (slash, `run_slash` `app.rs:1426`) and
      the palette (Ctrl+P → Enter, `app.rs:1675`).
- [ ] **Third way — keybinding (Design §3.3).** Assign a keybinding (via
      `CommandSpec.key`, handled in `App::on_key()` `app.rs:687`) or, if no free
      key is worth it, confirm palette + slash is the intended reach for compact
      as it is for `/model` — record the choice in the notes log. (The specific
      key is a minor, tunable decision — Design §10.)
- [ ] Guard against confusion with the existing `/prompt compact`
      (`commands.rs:123`, which edits the *summarization prompt*, not a trigger) —
      distinct names, distinct help lines.

## 2. Config: `auto_compact` + `auto_compact_threshold`  *(FR-4; Tech Spec §7, §8)*

Extend the Phase 2 `[context]` section. Follow the same file→engine path
window_turns took.

- [ ] Add to `ContextConfigFile` (`crates/emberly/src/config.rs:86`) and
      `ContextConfig` (`engine.rs:74`): `auto_compact: bool` (**default `true`**)
      and `auto_compact_threshold: f64` (**default `0.85`**, Tech Spec §7/§8).
      Wire through the resolver (`config.rs:599`), `merge` (`:304`), provenance
      (`:508`), and startup (`main.rs:512`).
- [ ] `context.auto_compact = false` disables the trigger entirely (manual
      `/compact` only). Threshold is "initial; tune with use" (Tech Spec §16,
      Requirements §13). Validate the threshold is in `(0.0, 1.0]`; an
      out-of-range value is a clear config error, not a silent clamp.
- [ ] Live-reload is **not** required for these keys — context config is not in
      `reload_config()` today (`engine.rs:1547`); treat auto-compaction keys as
      RestartRequired (C-5 reload model) unless trivially live. Note in the log.

## 3. Automatic-compaction trigger in the accounting path  *(FR-4; Tech Spec §7)*

One threshold check in the accounting path, reusing the **exact** manual queue —
"the only difference from `/compact` is the trigger" (Requirements §8.7, Tech
Spec §7).

- [ ] In `emit_context_usage()` (`engine.rs:2046`), after the pct is computed:
      if `self.context.auto_compact` and pct crosses
      `auto_compact_threshold`, request a compaction by the **same
      `compact_requested` queue** the mid-turn manual path uses (`:997`), so it
      drains at the next clean boundary (`:589–594`). No new scheduling path.
- [ ] **Track the trigger.** Since manual and auto both raise the queue, record
      which raised it (e.g. replace/augment `compact_requested: bool` with a
      pending-trigger `Option<CompactTrigger>`), so `compact()` writes the right
      `trigger` (group 4). Manual wins if both are pending (a user `/compact`
      is never downgraded to `auto`).
- [ ] **No thrash (Tech Spec §7).** A hysteresis latch: after an auto-compaction
      fires it does **not** re-fire until usage has fallen below the threshold and
      re-crossed it. The compaction itself drops usage well below the threshold,
      so re-arming is natural; the latch just prevents re-requesting on every
      accounting event while still above the line. Set the latch's initial state
      so a resumed already-full session compacts once at the first boundary, not
      repeatedly.
- [ ] Everything else is unchanged: same pinned content, same clean-boundary
      rule, same summarization-failure fallback (`compact()` `:740`). Auto never
      compacts mid-tool-round — it waits for the boundary exactly as a queued
      manual `/compact` does.

## 4. `trigger: manual | auto` transcript field  *(FR-4; Tech Spec §3.2)*

- [ ] Add `trigger: CompactTrigger` (`Manual | Auto`) to
      `TranscriptEvent::Compaction` (`transcript.rs:178`) with `#[serde(default)]`
      defaulting to `Manual` — additive, older readers warn-skip it, **no
      `SCHEMA_VERSION` bump** (stays `2`, `transcript.rs:25`); mirror the
      `LoopHalt.resolution` additive pattern (`transcript.rs:147`). Absent reads
      as `manual` (Tech Spec §3.2).
- [ ] Write it in `compact()` (`engine.rs:797`) from the pending trigger (group
      3). It is audit-only: resume/replay (`resume.rs:103`) ignores `trigger` (the
      view rebuild does not depend on why a compaction happened, HC-7).
- [ ] Extend the serde round-trip (`event_model.rs:183`) to cover both trigger
      values and the default-on-absent.

## 5. Harness-voice surfacing  *(Design §8.6, §6.1)*

Compaction is a harness-world moment, announced in the harness voice, never as
model output (Design §8.6). It reuses `UiEvent::CompactionStatus{message}`
(`event.rs:173` → `app.rs:654` → `ConvItem::Notice`).

- [ ] Revise the completion message to carry the **turn count** (Design §8.6):
      manual → "Compacted N turns into a summary."; automatic → the same with its
      reason, e.g. "Context was near full — compacted N turns to keep going." The
      count is the replaced range (`replaced_to − replaced_from`, already computed
      in `compact()`); the message branches on the trigger. No alarm styling —
      staying under budget is routine housekeeping (§1.2 "calm under load").
- [ ] Keep the existing spinner line ("compacting…") and the
      summarization-failure line; only the success line gains the count/reason.
- [ ] The summary stays inspectable — it is ordinary conversation content,
      openable via `/view` (Design §4.3/§8.6); no change needed, just confirm.
- [ ] Degraded mode (§7): the notice is plain ASCII, no color-only meaning
      (already via `markers::NOTICE`).

## 6. Tests + exit criterion  *(Tech Spec §14.6 offline, deterministic)*

- [ ] **Manual trigger:** extend
      `compact_summarizes_the_middle_and_records_the_event`
      (`engine_loop.rs:373`) to assert the `Compaction` event now carries
      `trigger = manual` and the message includes the turn count.
- [ ] **Automatic trigger (the FR-4 test):** drive `ContextUsage` past the
      threshold via `FakeProvider` usage numbers (`fake.rs:123`,
      `StreamEvent::Usage` → authoritative tokens, `engine.rs:1053`); assert a
      `Compaction` event with `trigger = auto` fires **at the next clean
      boundary** (not mid-tool-round) and that it **does not thrash** (fires once,
      not on every subsequent accounting event while still above the line).
- [ ] **Disabled:** `context.auto_compact = false` never auto-fires even above
      the threshold; manual `/compact` still works.
- [ ] **TUI surface:** a test that `/compact` (slash) and the palette entry both
      dispatch `Command::Compact` to the engine, and that `compact` appears in the
      help/registry listing (Design §3.3).
- [ ] **Exit criterion (Phase 3 done when):** `/compact` is reachable via palette,
      `/command`, and keybinding and compacts at a clean boundary; an
      automatic-compaction test drives usage past the threshold and asserts a
      `compaction` event with `trigger = auto` fires at the next clean boundary and
      does not thrash (Tech Spec §14.6, FR-4). Workspace clippy-clean under the §1
      lint policy; offline suite green.

---

## Decisions & notes log

> Fill in as the phase proceeds (mirrors the v0.1/v0.2 phase docs' logs).

- **`/compact` keybinding (group 1)** — _decide at implementation._ Assign a free
  key (Design §3.3's third way) or match `/model`'s palette-+-slash-only reach;
  record which and why.
- **Pending-trigger representation (group 3)** — _decide at implementation._
  Bias: replace `compact_requested: bool` with `pending_compaction:
  Option<CompactTrigger>` so the trigger rides the existing queue with no second
  flag; manual outranks auto if both are pending.
- **No-thrash latch (group 3)** — hysteresis: re-arm only after usage drops below
  the threshold. The compaction drops usage well under the line, so re-arming is
  automatic; confirm the post-compaction `emit_context_usage()` (`engine.rs:806`)
  does not immediately re-trigger.
- **Auto-compaction config not live-reloaded** — context keys are RestartRequired
  today (`reload_config` does not touch `self.context`); acceptable for FR-4,
  revisit if in-app tuning of the threshold is wanted (C-5).
