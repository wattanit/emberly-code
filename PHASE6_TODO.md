# Phase 6 — TUI mouse support (Design §3.4) — TODO & Progress

**Milestone:** M8 phase 6 (Tech Spec §15) — the final 0.4 phase. **Design-owned; no
Requirements FR/HC/T id** — it realizes Design §3.4 (pointer interaction), Requirements
§2.1 (additive-never-exclusive), and Tech Spec §9/§8 (`ui.mouse`). It comes last because
pointer click-select is most useful once the surfaces it targets exist (the Tasks section
from Phase 1, the Memory/Skills sidebar from Phases 3–4).
**Satisfies:** Design §3.4, §5, §7; Tech Spec §9, §8 (`ui.mouse`); Requirements §2.1.
Pinned to **Req v0.7 / Design v0.7 / Spec v0.8** (all `approved`).
**Goal:** Pointer interaction as **additive convenience, never a new authority**. The
governing rule (Design §3.4): *anything the mouse can do, the keyboard can already do,
and the mouse can do nothing the keyboard cannot.* Wheel/trackpad scrolls the focused
pane or open overlay; a click is a shortcut for "focus + Enter" on interactive rows.
Capture is gated on `ui.mouse` (**default `true`**) **and** rich mode via one control
point, off in degraded/`--plain`/`NO_COLOR`/`TERM=dumb`. Native Shift-drag selection
passes through. A click on a permission prompt may land on Deny/Allow but **never**
synthesizes an approval (no click-through, no hover-to-approve).

**Independent of the other phases' mechanisms; last by design.** It wires clicks to
surfaces the prior phases built and is otherwise self-contained in `emberly-tui` + one
`ui.mouse` config knob.

**Key structural facts (from the codebase) — this phase is NOT greenfield:**
1. **Mouse capture is already unconditionally ON in rich mode, and wheel scroll already
   works.** `enter_modes()` executes `EnableMouseCapture` with no gate
   (`crates/emberly-tui/src/terminal.rs:48`–:57), teardown does `DisableMouseCapture`
   (:101–:115), and the event loop already routes `ScrollUp`/`ScrollDown` to
   `app.on_scroll` (`crates/emberly-tui/src/tui.rs:125`–:133 → `app.rs:959`). **Do not
   re-add capture.** Phase 6 (a) *gates* the existing capture on `ui.mouse` && rich, and
   (b) adds *click* handling (`tui.rs:130` is currently `_ => continue`).
2. **The keyboard-parity invariant is the spine (Design §3.4).** Every click must map to
   an **existing keyboard action**. A surface with no keyboard equivalent must **not**
   gain one via the mouse — that would be new authority. Consequence for scope: the
   command palette, the model/effort/mode pickers, the reasoning-trail expand, the
   modified-files diff open, and wheel-scroll everywhere all have keyboard handlers to
   reuse; **the Memory/Skills sidebar sections are display-only today (no keyboard row
   selection, no inspector overlay)** — so clicking them has nothing to invoke and is
   **out of scope until a keyboard-selectable sidebar lands** (group 4 decision + open
   item; do not invent mouse-only selection).
3. **Hit-testing is the real new work.** All layout is computed transiently inside the
   draw call via `Layout::split` (`render.rs:40`–:54); **no `Rect`s are retained in
   `App`**, and the sidebar is one `Paragraph` of pre-built lines (`render.rs:755`), not
   per-row widgets. Mapping a click `(col,row)` → target needs a retained hit-map (group
   3).
4. **The single control point (Tech Spec §9).** Gate capture exactly like the §6.4
   animation ticker (`tui.rs:53`–:73, `app.is_animating()` `app.rs:881`) — one predicate,
   `rich && ui.mouse`, so capture is off whenever `ui.mouse = false`, degraded, or
   `--plain`. Degraded never enters `tui::run` at all (`frontend::decide`
   `frontend.rs:34`), so plain mode is structurally capture-free.
5. **Permission safety (Design §3.4/§5).** A click may land on Deny/Allow but must reuse
   `on_permission_key` semantics (`app.rs:1000`–:1036 — only `y`/`s` approve, Enter/Esc
   deny): no click-through past unscrolled content (the approve affordance still indicates
   content below), no hover-to-approve, Allow as deliberate as the approve key.

**Depends on / seams to reuse:**
- Terminal modes / capture — `enter_modes` (`terminal.rs:48`), `restore_terminal`
  (:101), entry `tui.rs:41`; crossterm features (`Cargo.toml:53`–:57 — `bracketed-paste`;
  the `events`/mouse reader currently rides ratatui's backend via feature unification,
  confirm/declare explicitly, group 6).
- Event loop / dispatch — async `tokio::select!` (`tui.rs`), key dispatch (:83), **mouse
  arm** (:125–:133), input reader poll (:189).
- Frontend split / degraded — `frontend::decide` (`frontend.rs:34`), env detection (:50),
  runner (:72); plain frontend `line.rs` (never enters rich).
- Focus / scroll model — implicit modal priority in `on_key` (`app.rs:725`: palette →
  overlay → permission → …), scroll fields `scroll`/`permission_scroll`/`Overlay.scroll`
  (`app.rs:342`, :331, :94), wheel routing `on_scroll` (:959), keyboard scroll (:824).
- Interactive surfaces + their keyboard handlers (the click targets):
  - Command palette — `commands::COMMANDS` (`commands.rs:64`), open (`app.rs:783`), render
    (`render.rs:94`), `on_palette_key` Enter→`run_slash` (`app.rs:1694`, :1708).
  - Pickers (model/effort/mode) — `open_*_picker` (`app.rs:1194`/:1225/:1257),
    `on_choice_picker_key` (:1857), render `choice_picker_lines` (`render.rs:286`).
  - Reasoning trail — collapsed line render (`render.rs:386`), `toggle_reasoning`
    (`app.rs:1357`, Ctrl+R :792).
  - Modified-files diff — sidebar render (`render.rs:679`), `open_last_diff`
    (`app.rs:1168`, Ctrl+O :788).
  - Permission prompt — render (`render.rs:917`, footer default deny :1213),
    `on_permission_key` (`app.rs:1000`).
- Overlays — stack `overlays: Vec<Overlay>` (`app.rs:352`), `OverlayContent` (:98),
  `push_overlay` (:1447), `on_overlay_key` (:1775), `centered()` (`render.rs:318`).
- Config — `[ui]` `UiConfig` (`config.rs:66`), `tool_explanations` precedent (parse :74,
  merge :343, resolve :892, provenance :612, thread `main.rs:545`); TUI receives
  `reasoning` via `frontend::run`→`tui::run`→`app` (`main.rs:618`, `tui.rs:45`).
- Tests — `on_scroll` routing test (`app.rs:2562`), permission deny-default test (:2608),
  render `TestBackend::draw` (`render.rs:1330`), degraded no-ANSI (`line.rs:672`),
  frontend-mode test (`frontend.rs:98`).

**Legend:** `[ ]` todo · `[~]` in progress · `[x]` done · `[!]` blocked

---

## Progress summary

| Group | Status | Notes |
|---|---|---|
| 1. `ui.mouse` config + threading + single capture gate (rich && ui.mouse) + crossterm feature (`emberly`/`emberly-tui`) | [x] | mirrored `tool_explanations` + the §6.4 ticker control point; gated the *existing* capture; no new dep (`Cargo.lock` unchanged) |
| 2. Wheel-scroll parity: route wheel to the palette list; confirm overlay/permission/conversation (`emberly-tui`) | [x] | added palette branch to `on_scroll` (moves `selected`, == Up/Down); confirmed overlay/permission/conversation routing with tests |
| 3. Click hit-testing infrastructure: retained `HitMap` of `Rect → Target` (`emberly-tui`) | [x] | `hit.rs` (`HitMap`/`ClickTarget`); `frame` builds it (out-param); `on_click` = focus+Enter; tui click arm; palette+choice rows wired (rest → group 4) |
| 4. Click = focus+Enter on keyboard-parity surfaces: palette rows, picker rows, reasoning expand, overlay rows, sidebar (`emberly-tui`) | [x] | palette/choice (grp 3) + session/memory/skill overlay rows + reasoning toggle + sidebar diff/memory/skills open (sidebar-geometry refactor); all via key/command twins |
| 5. Permission-prompt click safety + native Shift-selection passthrough + `ui.mouse=false` off switch (`emberly-tui`) | [x] | affordances reuse `on_permission_key`; body/header inert (no click-through/approve); Shift-gate + `/help` doc; single `mouse_capture_enabled` predicate |
| 6. Tests (offline — §14.7) + exit criterion (`emberly-tui`) | [ ] | capture-off predicate, wheel routing, click→action, click-never-approves, degraded |

**Overall Phase 6: IN PROGRESS — Groups 1–5 done (capture gate + wheel parity + click hit-testing + full click dispatch incl. sidebar + permission click safety). Remaining: group 6 (consolidated tests + exit criterion).**

---

## 1. `ui.mouse` config + threading + single capture gate  *(Design §3.4; Tech Spec §9, §8)*

- [x] `crates/emberly/src/config.rs`: added `pub mouse: Option<bool>` to `UiConfig` (beside
      `tool_explanations`, with a §3.4 doc comment). Merge block mirrors `tool_explanations`;
      `Resolved` gains `pub mouse: bool` resolved `merged.ui.mouse.unwrap_or(true)`; provenance
      recorded under `"ui.mouse"` only on a deviation from the default (the `record()`/
      `source_of` pattern), so `config show`'s generic "overrides (non-default sources)" list
      surfaces it automatically — no bespoke `show` code, exactly like `tool_explanations`.
      Documented `# mouse = true` in the `init` `[ui]` config template. Test
      `ui_mouse_parses_and_merges` added (parse / merge / independence from
      `tool_explanations`).
- [x] Threaded to the TUI: `resolved.mouse` → `frontend::run(..)` → `tui::run(.., mouse)` →
      `TerminalGuard::enter(mouse)` (mirrors the `reasoning` thread). **Not** stored on `App` —
      `App` needs no capture knowledge; the flag is consumed at the terminal-guard call site
      only (per the group-1 "or pass to `tui::run` for the capture decision only" option). The
      plain frontend ignores it (degraded is structurally capture-free — `frontend.rs` Rich arm
      only; `line.rs` untouched).
- [x] **Single capture gate (Tech Spec §9, the §6.4-ticker pattern).** `TerminalGuard` now
      carries `capture_mouse: bool`; `enter_modes(capture_mouse)` emits `EnableMouseCapture`
      **only when set**, and `resume` re-applies the stored flag (so an `$EDITOR` handoff
      restores the same decision). Teardown keeps the **unconditional** `DisableMouseCapture`
      (harmless if never enabled, safe across `resume`). Reaching `tui::run` already means rich,
      so `rich && ui.mouse` reduces to `ui.mouse` at the call site (documented inline). `line.rs`
      untouched.
- [x] Made the crossterm `events` reader **explicit**: added `"events"` to the `crossterm`
      workspace-dep feature list (it was already active as a crossterm default; naming it keeps
      key/resize/**mouse** capture from being dropped if defaults ever change). **No new external
      crate and `Cargo.lock` is unchanged** (HC-2; `crossterm` already locked, Tech Spec §12).

## 2. Wheel-scroll parity  *(Design §3.4)*

Wheel scroll mostly works (`on_scroll` routes overlay → permission → conversation,
`app.rs:959`). Close the one gap and confirm the rest.

- [x] Routed the wheel to the **command palette** when open: added a palette branch to
      `on_scroll` **ahead of the overlay branch**, matching the modal priority in `on_key`
      (palette > overlay > permission > conversation). The palette has no separate view
      offset — its render windows the list around `selected` — so the branch moves `selected`
      (wheel-up → up, wheel-down → down, clamped to `commands::matches(query).len()-1`), which
      is **exactly the Up/Down key action** (keyboard parity, §3.4). One item per notch.
- [x] Confirmed the existing routing with tests. `wheel_routes_to_the_open_palette` (new):
      wheel moves the palette selection like Down/Up, clamps at the top, and never leaks to
      the conversation. `wheel_scrolls_a_permission_prompt_without_deciding` (new): the wheel
      scrolls `permission_scroll`, never `scroll`, and **never decides** (Design §5/§3.4).
      `wheel_scrolls_conversation_and_routes_to_overlay` (existing) already covers overlay +
      conversation. All green (169 `emberly-tui` tests). _Group 6 can reference these rather
      than re-adding._

## 3. Click hit-testing infrastructure  *(Design §3.4)*

The crux: turn a click `(column, row)` into a target. Layout is transient (`render.rs:40`),
so introduce a retained hit-map.

- [x] Defined `ClickTarget` + `HitMap` in a new `crate::hit` module. `ClickTarget` starts
      minimal — `PaletteRow(usize)`, `ChoiceRow(usize)` — and each later group **extends** the
      enum as it wires a surface (avoids `dead_code` on unconstructed variants; §1 lint
      policy). `HitMap` is `Vec<(Rect, ClickTarget)>` with `push`/`clear`/`hit(col,row) ->
      Option<ClickTarget>`; `hit` scans **back-to-front so the topmost region wins**. Modal
      priority is enforced structurally (below), not by relying on push order alone. 4 unit
      tests (contains/half-open edges, miss, topmost-wins, clear).
- [x] **Populated from render geometry — plumbing (a) via a `&mut HitMap` out-param.**
      `render::frame(f, &app, &mut HitMap)` builds the map as it draws (one geometry source of
      truth, no drift). Chosen the out-param over a return value because ratatui's
      `Terminal::draw` closure must return `()` — so `frame` can't return the map through it;
      the `tui` loop's `redraw` helper makes a fresh `HitMap`, draws, and stores it on
      `App.hit_map`. **Modal priority** is enforced by each modal renderer calling
      `hit.clear()` before pushing its own regions (`render_overlay`, `render_palette`), so the
      **topmost interactive layer owns the map** and a click can never fall through a modal to
      the pane behind it (asserted: a read-only Text overlay leaves nothing clickable).
- [~] Sidebar (modified files) and conversation (reasoning toggle) row→target mapping —
      **moved to group 4** alongside their dispatch (they need parallel target-maps in
      `conversation_lines()`/`render_sidebar` and are meaningless without dispatch). Group 3
      populates the two clean 1-line-per-row surfaces: **palette rows** (`render_palette`) and
      **choice-picker rows** (`render_overlay` Choices). Sessions/memory/skills overlay rows
      also → group 4 (multi-line rows). Permission affordances → group 5.
- [x] Added the click arm in the event loop (replacing `_ => continue`):
      `MouseEventKind::Down(Left)` with **no modifiers** → `app.on_click(col,row)` → routed
      through the shared `handle_action` (same path as keys) → `redraw`. Wheel arms unchanged;
      modified/other kinds fall to `_ => continue` (no redraw, so a native Shift-drag stays
      smooth — the Shift gate is here already; group 5 verifies/documents it). Extracted
      `handle_action` so a key and a click share one Action-dispatch path (the §3.4 invariant
      in code).

## 4. Click = focus + Enter on keyboard-parity surfaces  *(Design §3.4; §3.3, §4.4, §4.2)*

`on_click(col,row)` dispatches via the hit-map to the **existing** keyboard handler for
each target — the mouse adds no capability the keyboard lacks (the invariant).

- [x] **Command-palette row** → select that row and run it, reusing the `on_palette_key`
      Enter path. **Done in group 3** — `on_click` sets `palette.selected = row` then
      dispatches a synthetic `Enter` to `on_palette_key` (literal "focus + Enter"; zero
      duplicated dispatch). Parity test: click row 1 == Down + Enter.
- [x] **Picker rows (model/effort/mode)** → select + apply, reusing `on_choice_picker_key`
      Enter. **Done in group 3** — `on_click` calls `set_choice_selection(row)` then a
      synthetic `Enter` to `on_choice_picker_key`. Parity test: click row 2 == Down·Down +
      Enter.
- [x] **Collapsed/expanded reasoning trail** → toggle, reusing `toggle_reasoning` (the Ctrl+R
      action). `conversation_lines` now reports the **most recent** reasoning header's line
      index (out-param — overwritten each item so it ends at the last, matching Ctrl+R's
      "toggle the most recent"); `render_conversation` pushes a `ReasoningToggle` region when
      it is on screen (the conversation is pre-wrapped, so screen row == line offset). Only the
      *last* trail is clickable, so the click never exceeds Ctrl+R's parity. (Inline task list
      is always-expanded — nothing to toggle; left alone per the note below.) Tests:
      `click_reasoning_toggle_matches_ctrl_r` (app), `reasoning_trail_is_clickable` (render).
- [x] **Overlay picker rows: sessions / memory / skills** → select + Enter, reusing
      `on_session_picker_key` / `on_memory_inspector_key` / `on_skills_inspector_key`. The four
      overlay list-builders now return a `row_of_line` map (header/spacer → `None`);
      `render_overlay` pushes one region per selectable line via a `RowKind` → `ClickTarget`
      map (unified with group 3's choice path). `on_click` dispatches each via the
      synthetic-Enter pattern. Tests: `click_memory_row_is_focus_plus_enter` (app),
      `memory_inspector_rows_populate_the_hit_map` (render); sessions/skills share the identical
      mechanism.
- [x] **Modified-files entry → open its diff.** Decision **(a)** (parity-safe): a click on the
      modified-files section opens the same diff overlay Ctrl+O opens (`open_last_diff`); per-file
      open has no keyboard twin, so any click opens the same view the key does. Enabled by the
      sidebar-geometry refactor below.
- [x] **Memory/Skills inspector *rows* clickable + sidebar-section open (honesty clause fully
      resolved).** Part 2 shipped the `/memory` `/skills` overlays; group 4 makes their **rows**
      clickable (view/inspect) *and* — via the refactor below — makes the **sidebar Memory/Skills
      sections** clickable to open the inspector (the `/memory` `/skills` command twins). This is
      Design §4.9's "sidebar Memory click opens the inspector," realized as §3.4's "fourth,
      optional way."
- [x] **Sidebar-geometry refactor (DONE — enables the sidebar clicks above).** The sidebar was
      one `Paragraph` with `Wrap{trim:false}`; some lines wrapped, so screen-row → logical-line
      was unreliable. Fixed by rendering the sidebar **without wrap** (ratatui truncates
      overflow → each logical line is exactly one screen row), and `fit`-truncating the one
      remaining un-fit line (skill descriptions; full text stays in the `/skills` inspector).
      `render_sidebar` now records each clickable section's line range and pushes an `OpenDiff`
      / `OpenMemoryInspector` / `OpenSkillsInspector` region — gated on `interactive` (the base
      layer) so the sidebar is inert behind a permission/ask/loop prompt (§5); an overlay/
      palette on top clears the map. `on_click` dispatches each to its exact key/command twin
      (`open_last_diff`, `run_command(Memory)`, `run_command(Skills)`). _Minor behavior change:
      very large token/cost lines now truncate instead of wrapping — negligible for a 32-col
      status pane; noted in the log._ Tests: `sidebar_sections_populate_the_hit_map`,
      `sidebar_is_not_clickable_behind_a_permission_prompt` (render),
      `click_sidebar_sections_match_their_keyboard_actions` (app).

## 5. Permission-prompt click safety + native selection + off switch  *(Design §3.4, §5, §7)*

- [x] **Permission clicks reuse `on_permission_key`, never a new path.** `render_permission`
      registers **only** the three footer affordances (Allow/Session/Deny) as click targets —
      and only their **text** (the padding between them is inert, so a stray near-miss never
      approves). `on_click` maps each to the exact key (`y`/`s`/Enter) and calls
      `on_permission_key(id, …)`, so a click is byte-for-byte the key's decision. **Guarantees
      held:** (a) no click "approves whatever is focused" — a non-affordance click resolves to
      nothing (`render_permission` clears the hit-map first → no click-through to the
      conversation/sidebar behind; body/header push nothing → inert); (b) no hover-to-approve
      (only `Down(Left)` acts); (c) **no click-through past unscrolled content** — the footer's
      "↓ N more" notice is unchanged and approval reuses the key path (which never blocked on
      scroll, only *indicated*), so the click has exactly the key's power, no more; (d) Allow is
      as deliberate as the approve key. Tests: `click_permission_affordances_match_their_keys`,
      `a_click_off_the_permission_affordances_never_decides` (app),
      `permission_affordances_are_the_only_click_targets` (render — body row inert).
- [x] **Native Shift-drag selection passes through (Design §3.4).** The tui click arm consumes
      **only** `Down(Left)` with `mouse.modifiers.is_empty()` (added in group 3); Shift/Ctrl/
      Alt-modified clicks and all drags fall to `_ => continue` — not consumed, not redrawn — so
      the terminal's own Shift-drag-to-copy works and stays smooth. Documented in `/help` (new
      Mouse section: wheel/click/Shift/off-switch). Test: `help_documents_the_mouse_and_shift_passthrough`.
      _Open item (Tech Spec §16) carried to release verification: Shift-passthrough is
      terminal-dependent — verify across target terminals; where a terminal doesn't honor it,
      `ui.mouse = false` is the escape hatch. Not blocking (offline suite green)._
- [x] **`ui.mouse = false` releases the mouse entirely** (Design §3.4). Made the gate an
      explicit single control point: `terminal::mouse_capture_enabled(rich, ui_mouse) = rich &&
      ui_mouse`, called by `tui::run` with `rich = true`, so `false` ⇒ no `EnableMouseCapture`
      ⇒ no mouse events arrive ⇒ neither `on_click` nor `on_scroll` ever runs (verified by
      construction; the predicate is unit-tested). Degraded runs `line::run`, which never
      touches capture. Test: `mouse_capture_gated_on_rich_and_ui_mouse` (terminal).

## 6. Tests + exit criterion  *(Tech Spec §14.7 offline, deterministic)*

- [ ] **Capture gated (the §14.7 mouse test, Tech Spec §15 exit):** the capture predicate
      is `false` when `ui.mouse = false` and in degraded mode (assert via the predicate /
      that `line::run` never calls `EnableMouseCapture`, and that `tui::run` skips it when
      `ui.mouse=false`).
- [ ] **Wheel routing:** extend `wheel_scrolls_conversation_and_routes_to_overlay`
      (`app.rs:2562`) — a wheel event scrolls the focused pane, the open overlay, the
      permission body, and (new) the open palette.
- [ ] **Click selects an interactive row:** a synthesized click on a palette row / picker
      row activates the same action the Enter key would (assert the resulting `Command` /
      state change); a click to expand the reasoning trail toggles it.
- [ ] **Click never synthesizes a permission approval (Design §3.4/§5):** with a permission
      prompt up, a click never yields Allow unless it lands on the Allow affordance and,
      when content is unscrolled, cannot bypass the same guard the key path enforces; a
      click elsewhere leaves the prompt pending (defaults never auto-approve). Sits beside
      `permission_defaults_to_deny_on_enter` (`app.rs:2608`).
- [ ] **Degraded parity:** the plain frontend renders unchanged with no mouse capture and
      no ANSI (keep `degraded_output_has_no_ansi_escapes` `line.rs:672` green); no feature
      depends on the pointer to carry meaning (Design §7).
- [ ] **Exit criterion (Phase 6 done when):** a mouse unit test asserts capture is disabled
      under `ui.mouse=false` and in degraded mode, that a wheel event scrolls the focused
      pane, that a click selects an interactive row, and that a click never synthesizes a
      permission approval (Tech Spec §14.7, Design §3.4/§5). Workspace clippy-clean under
      the §1 lint policy; offline suite green. No new external dependency (`crossterm`
      already locked, HC-2). **This is the last 0.4 phase — on merge, M8 is complete.**

---

## Decisions & notes log

> Fill in as the phase proceeds (mirrors the v0.1–v0.3, Phase 1–5 logs).

- **Group 1 DONE (config + threading + capture gate).** `ui.mouse` (default `true`) mirrors the
  `tool_explanations` pattern end-to-end: `UiConfig` field → merge → `Resolved.mouse` →
  provenance (`"ui.mouse"`, surfaced generically by `config show`) → `init` template doc. Threaded
  `resolved.mouse` → `frontend::run` → `tui::run` → `TerminalGuard::enter(capture_mouse)`; the
  existing unconditional `EnableMouseCapture` is now conditional on the flag, `resume` re-applies
  it, teardown still always `DisableMouseCapture`s. Made the crossterm `events` feature explicit
  (no new dep; `Cargo.lock` unchanged, HC-2). Not stored on `App` — capture is decided only at the
  guard call site. Verified: `emberly`+`emberly-tui` build clean; config tests (13, incl. new
  `ui_mouse_parses_and_merges`) and the full `emberly-tui` suite (167) green; `emberly-tui --lib`
  clippy clean (the only workspace clippy warnings are the 2 pre-existing `emberly-core/engine.rs`
  `too_many_arguments`, unrelated).
- **Group 2 DONE (wheel-scroll parity).** `on_scroll` gained a palette branch at the top of
  the modal chain (mirroring `on_key`'s palette > overlay > permission > conversation order).
  Because the palette render windows the list around `selected` (no separate scroll offset),
  the wheel moves `selected` — the same action as the Up/Down keys, so wheeling the palette is
  keyboard-parity by construction. Overlay/permission/conversation routing was already present
  and is now covered by tests. No new state, no config. 2 tests added (169 `emberly-tui` green).
- **Group 3 DONE (click hit-testing infrastructure).** New `crate::hit` module: `ClickTarget`
  (starts minimal — `PaletteRow`/`ChoiceRow` — extended per group to avoid `dead_code`) +
  `HitMap` (topmost-wins `hit()`). `render::frame(f, &app, &mut HitMap)` builds the map from
  live geometry (out-param, because ratatui's `draw` closure returns `()`); the `tui` loop's
  `redraw` stores it on `App.hit_map`. **Modal priority via `hit.clear()`** in each modal
  renderer → the topmost layer owns the map, no click-through (tested with a read-only overlay).
  `on_click` is **literal "focus + Enter"**: it resolves the target, sets the selection, then
  dispatches a synthetic `Enter` to the *existing* key handler — so the mouse cannot diverge
  from the keyboard by construction (the §3.4 invariant, in code). Extracted `handle_action` so
  keys and clicks share one Action path. Wired palette + choice rows (clean 1-line rows);
  sessions/memory/skills/reasoning/modified-files → group 4, permission → group 5. Shift-gate on
  the click already present (`modifiers.is_empty()`); group 5 verifies. +10 tests (179 green),
  `emberly-tui` clippy-clean, no new dep.
- **Group 3/4 boundary rebalanced (recorded per G-11).** The TODO put "populate all surfaces"
  in group 3 and "dispatch" in group 4. Rebalanced so each commit is coherent and tested:
  **group 3** = framework + the two clean 1-line-per-row surfaces (palette, choices) *with*
  dispatch (the synthetic-Enter pattern made dispatch trivial, so splitting it off added no
  value); **group 4** = the multi-line / conversation-embedded / sidebar surfaces (sessions,
  memory/skills rows, reasoning toggle, modified files, sidebar-open) with their population +
  dispatch; **group 5** = permission affordances. Net scope unchanged; only the commit seam
  moved.
- **Hit-map plumbing decision (resolves the group-3 open item).** Chose the `&mut HitMap`
  out-param over a `frame` return value: ratatui's `Terminal::draw` takes an `FnOnce(&mut Frame)`
  whose return is discarded, so a returned map can't escape the closure — the out-param is the
  clean way to get geometry out of the draw. Single source of truth preserved (built *in* the
  draw, never a second recompute).
- **Group 4 DONE (click dispatch on the overlay + conversation surfaces).** Wired
  session/memory/skill **overlay rows** (the four list-builders now return a `row_of_line`
  map; `render_overlay` pushes one region per selectable line via `RowKind → ClickTarget`,
  unifying group 3's choice path) and the **reasoning-trail toggle** (`conversation_lines`
  reports the last reasoning header's line index; `render_conversation` pushes a
  `ReasoningToggle` region). All dispatch via the synthetic-Enter pattern → parity by
  construction. +4 tests (183 green), `emberly-tui` clippy-clean, no new dep.
- **Group 5 DONE (permission click safety + Shift passthrough + off switch).** Permission
  clicks are the sharpest safety surface, so they get the strictest treatment: `render_permission`
  clears the hit-map (no click-through to anything behind the modal) and registers **only** the
  three footer affordances, and only their **text** (inter-affordance padding is inert). `on_click`
  maps Allow/Session/Deny → `y`/`s`/Enter and calls `on_permission_key` verbatim, so a click is
  identical to the key — a non-affordance click resolves to nothing (never a default-approve),
  there's no hover-to-approve, and no click-through past the "↓ N more" indicator (the click has
  exactly the key's power). Shift/modified clicks are already left to the terminal (group-3 gate)
  for native selection, now documented in `/help`. The off switch is an explicit, unit-tested
  control point `terminal::mouse_capture_enabled(rich, ui_mouse)`; `false` ⇒ no capture ⇒ no
  events ⇒ no click/scroll handling. +5 tests (191 green), clippy-clean, no new dep. _Terminal-
  dependent Shift-passthrough stays a release-verification open item (Tech Spec §16), non-blocking._
- **Sidebar clicks DONE (owner: do it now).** The initial group-4 deferral was resolved the
  same phase: the sidebar-geometry refactor landed. Root cause was the sidebar's
  `Paragraph.wrap(Wrap{trim:false})` — wrapping made screen-row → logical-line unreliable.
  **Fix: render the sidebar without wrap** (ratatui truncates overflow, so each logical line is
  exactly one screen row → line index `i` sits at screen row `inner.y + i`), and `fit`-truncate
  the one remaining un-fit line (skill descriptions). `render_sidebar` records each clickable
  section's line range and pushes `OpenDiff` / `OpenMemoryInspector` / `OpenSkillsInspector`,
  gated on `interactive` (base layer only — inert behind a permission/ask/loop prompt, §5).
  `on_click` dispatches each to its exact twin (`open_last_diff` = Ctrl+O; `run_command(Memory/
  Skills)` = `/memory` `/skills`). Modified-files uses decision **(a)** (open the same overlay
  Ctrl+O opens — per-file open has no keyboard twin). _Behavior change: very large token/cost
  lines truncate instead of wrapping — negligible for a 32-col status pane; the sidebar is
  informational and everything is reachable elsewhere (Design §3.1)._ +3 tests (186 green).
- **Memory/Skills honesty-clause fully resolved.** The original group-4 note said memory/skills
  clicks were out of scope because the inspector overlays didn't exist. Part 2 shipped them;
  group 4 now makes both the inspector **rows** (view/inspect) *and* the **sidebar sections**
  (open the inspector) clickable — parity-safe, the exact modified-files-diff pattern. Design
  §4.9's "sidebar Memory click opens the inspector" is realized (§3.4's fourth, optional way).
- **Part 2 (memory/skills inspectors) already merged into this branch — group 4 sidebar-click
  target is now UNBLOCKED.** `PHASE6_TODO.md` groups 2/4 and the notes below were written before
  `phase6b/memory-skills-inspector` landed; the inspectors now exist and are palette-openable
  (`/memory`, `/skills`), so a sidebar Memory/Skills click has a keyboard twin (open the same
  overlay, the modified-files-diff pattern) and is no longer an inert no-op. _Owner decision
  (2026-07-14): **fold it in now.** Done — the sidebar-geometry refactor + sidebar clicks
  (modified-files diff, Memory/Skills open) landed in group 4 (see the "Sidebar clicks DONE"
  note above)._
- **Not greenfield — gate, don't add.** Mouse capture is already unconditionally on in
  rich mode and wheel scroll already routes. Phase 6 gates the existing capture on
  `rich && ui.mouse` (one control point, §9) and adds click handling. _Do not duplicate
  `EnableMouseCapture`._
- **Keyboard-parity invariant governs scope (Design §3.4).** Every click maps to an
  existing keyboard action; the mouse adds no new authority.
- **Memory/Skills sidebar clicks: blocked on an unbuilt overlay, not on parity.** Design
  §4.9 specifies Memory/Skills *inspector overlays* (memory entries editable/deletable;
  skill bodies read-only), but Phase 3/4 shipped the sidebar as counts/catalog display
  only — no inspector overlay, no `/memory` / `/skills` palette command. So the sidebar
  today is informational, matching the "palette-driven, sidebar-informational" mental
  model in practice, even though Design intends an inspector. The inspector is a
  **pre-existing Phase 3/4 gap against §4.9, independent of Phase 6.** When it is built it
  should be palette-openable (§3.3); at that point a sidebar click opens it exactly like
  the modified-files diff click (§3.4's "fourth, optional way"). This phase does not build
  it, and a click on those rows is a harmless no-op until it exists. _Flag the §4.9 gap to
  the owner as its own follow-up; do not fold it into Phase 6._
- **Modified-files click: open-the-overlay vs. per-row selection.** Default to opening the
  same diff overlay Ctrl+O opens (parity-safe); a per-file click-a-row needs a
  keyboard-selectable list first. _Decide in group 4; default keeps the invariant._
- **Hit-map plumbing (group 3).** Leaning to `render::frame` producing the `HitMap` (one
  geometry source of truth, no drift) over a separate recompute pass that could diverge
  from the draw. _Confirm the signature change is clean; if `&mut` out-param is tidier than
  a return value, use that._
- **Task-list click is a no-op today.** The inline task list has no collapse affordance
  (always expanded), so there is nothing for a click to toggle. _Left as a no-op; if a
  task-list collapse is added later, wire the click then — do not add a collapse solely
  for the mouse._
- **Shift-drag passthrough is terminal-dependent.** Only unmodified left-clicks/wheel are
  consumed; Shift-modified events are left to the terminal for native selection, documented
  in `/help`. _Open item (Tech Spec §16, this phase's resolver): verify across target
  terminals and document where `ui.mouse = false` is the only way to get native selection._
- **`ui.mouse` is the first `[ui]` flag the TUI consumes.** `tool_explanations` is
  engine-only; `ui.mouse` must reach `tui::run` for the capture decision — thread it like
  `reasoning`. _If more `[ui]` flags follow, consider passing a small `UiFlags` struct
  rather than growing the arg list (out of scope here)._
- **Open items carried in (Tech Spec §16 v0.8, this phase's resolver):** mouse capture /
  native-selection passthrough across terminals (above). No other 0.4 open items remain;
  on this phase's merge the M8 milestone and the 0.4 feature set are complete (Tech Spec
  §15).
