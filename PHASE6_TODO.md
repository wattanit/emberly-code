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
| 1. `ui.mouse` config + threading + single capture gate (rich && ui.mouse) + crossterm feature (`emberly`/`emberly-tui`) | [ ] | mirror `tool_explanations` + the §6.4 ticker control point; gate the *existing* capture |
| 2. Wheel-scroll parity: route wheel to the palette list; confirm overlay/permission/conversation (`emberly-tui`) | [ ] | small gap — `on_scroll` doesn't route to the open palette today |
| 3. Click hit-testing infrastructure: retained `HitMap` of `Rect → Target` (`emberly-tui`) | [ ] | layout is transient in draw; build a hit-map from render geometry |
| 4. Click = focus+Enter on keyboard-parity surfaces: palette rows, picker rows, reasoning expand, modified-files diff (`emberly-tui`) | [ ] | each maps to an existing handler; Memory/Skills row-open deferred (no keyboard parity yet) |
| 5. Permission-prompt click safety + native Shift-selection passthrough + `ui.mouse=false` off switch (`emberly-tui`) | [ ] | click reuses `on_permission_key`; never auto-approve; preserve terminal copy |
| 6. Tests (offline — §14.7) + exit criterion (`emberly-tui`) | [ ] | capture-off predicate, wheel routing, click→action, click-never-approves, degraded |

**Overall Phase 6: NOT STARTED.**

---

## 1. `ui.mouse` config + threading + single capture gate  *(Design §3.4; Tech Spec §9, §8)*

- [ ] `crates/emberly/src/config.rs`: add `pub mouse: Option<bool>` to `UiConfig` (:66,
      beside `tool_explanations` :74). Merge (:343 block), resolve default **`true`**
      (:892 pattern — `merged.ui.mouse.unwrap_or(true)`), provenance for a non-default
      (:612 pattern), and surface in `config show`. Add a `mouse` field to `Resolved`
      (or a `ui_mouse: bool`), mirroring how `tool_explanations` is carried.
- [ ] Thread it to the TUI. `tool_explanations` is engine-only, so `ui.mouse` is the first
      `[ui]` flag the TUI needs — pass it through `frontend::run` → `tui::run` → `App`
      (mirror the `reasoning` thread, `main.rs:618` → `tui.rs:45`). Store `mouse_enabled:
      bool` on `App` (or pass to `tui::run` for the capture decision only).
- [ ] **Single capture gate (Tech Spec §9, the §6.4-ticker pattern).** In `enter_modes`
      (`terminal.rs:48`) make `EnableMouseCapture` conditional on one predicate `rich &&
      ui.mouse`. Since `enter_modes` is only reached from `tui::run` (rich), the predicate
      reduces to `ui.mouse` at that call site — pass a `capture_mouse: bool` into
      `TerminalGuard::enter(...)`/`enter_modes` and emit `EnableMouseCapture` only when
      set; **always** `DisableMouseCapture` on teardown (harmless if never enabled, and
      safe across `resume`). Degraded/`--plain` never enters `tui::run`, so it is
      structurally capture-free (`frontend.rs:72`) — do not touch `line.rs`.
- [ ] Confirm the crossterm mouse-event reader is enabled. Mouse events already arrive
      (`tui.rs:125`), riding ratatui's crossterm backend via feature unification; make it
      explicit — declare the needed feature on the `crossterm` workspace dep
      (`Cargo.toml:57`) so it is not accidentally dropped. No new external crate (HC-2;
      `crossterm` already locked, Tech Spec §12).

## 2. Wheel-scroll parity  *(Design §3.4)*

Wheel scroll mostly works (`on_scroll` routes overlay → permission → conversation,
`app.rs:959`). Close the one gap and confirm the rest.

- [ ] Route the wheel to the **command palette** list when it is open: `on_scroll`
      (`app.rs:959`) currently ignores the palette (modal at `on_key` :727 but absent from
      the scroll router), so a long palette cannot be wheeled. Add a palette branch (scroll
      `palette.selected`/a view offset) ahead of the overlay branch, matching the modal
      priority in `on_key`.
- [ ] Confirm (with a test, group 6) the existing routing: open overlay → wheel scrolls
      the overlay (`Overlay.scroll`); permission prompt up → wheel scrolls
      `permission_scroll`; otherwise the conversation `scroll`. This is the "scrolls the
      focused pane or open overlay" requirement (Design §3.4) — mostly present, just
      unverified and missing the palette case.

## 3. Click hit-testing infrastructure  *(Design §3.4)*

The crux: turn a click `(column, row)` into a target. Layout is transient (`render.rs:40`),
so introduce a retained hit-map.

- [ ] Define `enum ClickTarget { PaletteRow(usize), ChoiceRow(usize), ReasoningToggle,
      ModifiedFile(usize)/OpenDiff, ConversationBody, OverlayBody, PermissionChoice(Allow|
      Session|Deny), … }` and a `HitMap(Vec<(Rect, ClickTarget)>)` with `hit(col,row) ->
      Option<ClickTarget>` (topmost match wins, honoring the same modal priority as
      `on_key`: palette > overlay > permission > sidebar/conversation).
- [ ] **Populate the hit-map from render geometry.** `render::frame(f, &app)` takes `&App`
      today, so choose the plumbing (**decide in notes log**):
      (a) have `frame` build and return a `HitMap` (the `tui` loop stores it on `App` after
      each `draw`), or (b) a separate `layout(app, area) -> HitMap` that recomputes the
      same `Layout::split` deterministically and is called alongside draw.
      Leaning to (a) — one geometry source of truth, no drift — accepting that `frame`
      then returns the map (or writes it via a `&mut HitMap` out-param). Record the choice.
- [ ] For the sidebar (one `Paragraph`, `render.rs:755`) and the wrapped conversation
      (`conversation_lines()` `render.rs:344`), map row ranges to targets as the lines are
      built, so a click resolves to the right row without per-row widgets.
- [ ] Add the click arm in the event loop (`tui.rs:130`, replacing `_ => continue`):
      `MouseEventKind::Down(Left)` → `app.on_click(mouse.column, mouse.row)` → redraw. Keep
      wheel arms unchanged. Ignore other kinds (drag/move/right — but see group 5 for
      Shift-drag passthrough).

## 4. Click = focus + Enter on keyboard-parity surfaces  *(Design §3.4; §3.3, §4.4, §4.2)*

`on_click(col,row)` dispatches via the hit-map to the **existing** keyboard handler for
each target — the mouse adds no capability the keyboard lacks (the invariant).

- [ ] **Command-palette row** → select that row and run it, reusing the `on_palette_key`
      Enter path (`app.rs:1708` → `run_slash`). A click on row *i* sets `palette.selected =
      i` then activates — "focus + Enter".
- [ ] **Picker rows (model/effort/mode)** → select + apply, reusing `on_choice_picker_key`
      Enter (`app.rs:1857` → `Command::SwitchModel`/`SetEffort`/`SetMode`). Click a choice
      row = highlight + confirm.
- [ ] **Collapsed reasoning trail** → expand/collapse, reusing `toggle_reasoning`
      (`app.rs:1357`, the Ctrl+R action). (The inline task list is always-expanded today —
      no collapse affordance exists, so there is nothing to toggle; a click is a no-op
      unless/until a task-list collapse lands — note in log, do not invent one.)
- [ ] **Modified-files entry** → open its diff, reusing the existing diff-overlay open
      (`open_last_diff` `app.rs:1168`). Today only the *last* modified file is keyboard-
      openable (Ctrl+O). A click naming a *specific* row would exceed keyboard parity
      unless per-file open exists for the keyboard too. **Decide (notes log):** either
      (a) scope the click to "open the diff overlay" (parity-safe, opens the same view Ctrl+O
      does) or (b) add a keyboard-selectable modified-files list *first* so click-a-row has a
      keyboard twin. Default to (a) to preserve the invariant this phase.
- [ ] **Out of scope this phase — Memory/Skills inspector click targets (honesty clause).**
      Design §4.9 *does* specify inspectors: the sidebar Memory section "opens an overlay
      listing entries grouped by scope, each editable/deletable" and the Skills section
      "selecting one shows its instruction body read-only" (tied to the FR-6/FR-7 trust
      rationale — memory the user cannot see/correct is memory they cannot trust). **But
      those inspector overlays were never built in Phase 3/4** — today the sidebar Memory/
      Skills sections are counts/catalog lines only, there is no inspector overlay, and no
      `/memory` / `/skills` palette command. So the blocker for a mouse click here is **the
      overlay itself does not exist yet**, not a missing keyboard-selection model. This is a
      pre-existing Phase 3/4 gap against Design §4.9, **independent of Phase 6**. Once the
      inspector overlay lands and is palette-openable (§3.3 "reachable three ways"), a
      sidebar click opens it parity-safely — the *exact* modified-files-diff pattern
      (group 4 above), the mouse being the §3.4 "fourth, optional way." Until then, a click
      on those sidebar rows is an inert no-op (they carry no keyboard action to mirror). Do
      **not** build the inspector or a mouse-only selection in this phase.

## 5. Permission-prompt click safety + native selection + off switch  *(Design §3.4, §5, §7)*

- [ ] **Permission clicks reuse `on_permission_key` (`app.rs:1000`), never a new path.** A
      click on the Allow/Session/Deny affordance dispatches the *same* decision the key
      produces. **Guarantees that must hold (Design §3.4/§5):** no click "approves whatever
      is focused"; no hover-to-approve (only an explicit click on the affordance, like an
      explicit keypress); **no click-through past unscrolled content** — if the body has
      content below the fold, the approve affordance still indicates it and a click cannot
      bypass it, exactly as the key path enforces; clicking Allow is as deliberate as the
      approve key. A click *outside* the affordances is inert (never a default-approve).
- [ ] **Native Shift-drag selection passes through (Design §3.4).** With capture on, the
      terminal's own Shift-modified click-drag-to-copy must still work. crossterm delivers
      Shift-modified drags as events carrying the Shift modifier — **do not consume** those
      (let the terminal handle selection); only act on unmodified left-clicks and wheel.
      Document this in `/help` (Design §3.4). **Open item (Tech Spec §16, this phase's
      resolver):** Shift-passthrough is terminal-dependent — verify across the target
      terminals and document those where `ui.mouse = false` is the only way to get native
      selection.
- [ ] **`ui.mouse = false` releases the mouse entirely** (Design §3.4): capture never
      enabled (group 1), so the terminal owns the pointer unconditionally — for users who
      prefer native selection everywhere. Verify no click/scroll handling runs when capture
      is off (it can't — no events arrive).

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
