# Phase 4 — Full TUI — TODO & Progress

**Milestone:** M4 (Tech Spec §15) — *the real `ratatui` interface: panes,
sidebar, palette, diff overlay, Thai text handling, degraded-mode parity.*
**Goal:** Prove the Design Guideline. Turn the Phase 1 line-mode seed into the
warm, calm, two-pane terminal interface Design §2–§7 describes — a main
conversation pane, a collapsible sidebar, a fuzzy command palette, first-party
markdown + syntax highlighting + diffs, grapheme-correct Thai text everywhere,
purposeful motion, and a **tested** degraded mode that doubles as the headless
contract.

**Depends on:** Phase 1 (the `UiEvent`/`Command` channel boundary, the
`FrontendPorts`, and the line-mode `LineRenderer` all exist and are proven) and
Phase 3 (live providers, so context %/cost/usage events carry real data). Both
are merged to `main`. **Fully developable on macOS** — no OS sandbox involved;
Phase 2 (Landlock) remains deferred until a Linux machine.

**Legend:** `[ ]` todo · `[~]` in progress · `[x]` done · `[!]` blocked

---

## Progress summary

| Group | Status | Notes |
|---|---|---|
| 0. Prerequisites & dependencies | [x] | ratatui 0.29 / syntect 5.3 (fancy-regex) / unicode-*; HC-2 verified C-free |
| 1. Frontend abstraction & terminal lifecycle | [x] | seam + RAII guard + panic-safe restore (HC-3); loop + view-model; PTY-verified |
| 2. Centralized theme & string table | [ ] | one theme file, one strings module (Design §2, §6.2) |
| 3. Grapheme-aware text engine + line editor | [ ] | Thai width/wrap/cursor; the input box (Tech Spec §9) |
| 4. Layout: main pane + sidebar + status bar | [ ] | auto-collapse <100 cols (Design §3) |
| 5. Markdown subset + syntax highlighting | [ ] | first-party md; syntect fancy-regex (Design §4.1) |
| 6. Diffs first-class (inline + overlay) | [ ] | own both sides; `/view` `$EDITOR` hatch (Design §4.2–§4.3) |
| 7. The permission prompt | [ ] | **the most important screen** (Design §5) |
| 8. Command palette + command registry | [ ] | Ctrl+P fuzzy; single registry (Design §3.3) |
| 9. Motion | [ ] | single ~12fps ticker; spinner/glow (Design §6.4) |
| 10. Degraded mode parity | [ ] | `--plain`/`NO_COLOR`/`TERM=dumb` (Design §7) |
| 11. Tests, fixtures & exit criterion | [ ] | Thai + degraded + permission-prompt guarantees |

**Overall Phase 4: NOT STARTED.** Branch `phase-4-tui` off `main`.

---

## Platform & safety notes (read first)

- **All macOS-developable.** This is presentation code over the existing event
  boundary; nothing here needs Landlock or Linux.
- **HC-1 (safe Rust):** `ratatui`, `crossterm`, `syntect`, `unicode-*` are all
  usable under `#![forbid(unsafe_code)]` in *our* crate — the forbid applies to
  first-party code only. Recheck at group 0 that none forces an `unsafe` shim.
- **HC-2 (no C deps):** `syntect` **must** use the **`fancy-regex`** backend
  (pure Rust). Its default `onig` backend is **Oniguruma (C)** and is
  forbidden — pull `syntect` with `default-features = false` and select
  `default-fancy` / `regex-fancy`. Re-run the CI C-crypto/C-dep guard after
  adding deps.
- **HC-3 (panic-free, clean exit):** raw mode + alternate screen **must** be
  restored on every exit path — normal, error, *and panic*. The panic hook in
  `main.rs` (currently a stub) must leave the terminal usable before printing.
  A RAII terminal guard is the mechanism; the panic hook calls the same
  teardown. This is the one place a TUI most often leaves users with a broken
  shell — treat it as a hard requirement, not polish.
- **`str::len` / chars-as-columns is banned** in all layout math (Tech Spec §9).
  Every width/wrap/cursor calculation goes through the group-3 text engine.

---

## 0. Prerequisites & dependencies  *(§10 policy; HC-1/HC-2; Tech Spec §9, §12)*

- [x] Added to `[workspace.dependencies]`, pinned: `ratatui = "0.29"`
      (`default-features = false`, `["crossterm"]`); `syntect = "5.2"`
      (`default-features = false`, `["default-fancy"]` → pure-Rust fancy-regex,
      resolved 5.3.0 + fancy-regex 0.16); `unicode-segmentation = "1"`;
      `unicode-width = "0.2"`; `nucleo-matcher = "0.3"` (registered, wired in at
      group 8). **`crossterm` NOT declared separately** — used via
      `ratatui::crossterm` so there is one crossterm version (0.28.1) in the
      graph, no duplicate-version split.
- [x] `emberly-tui/Cargo.toml`: added ratatui/syntect/unicode-*; still
      `emberly-tui -> core` only (no new internal edges). Added tokio `time`
      feature for the group-9 animation ticker.
- [x] **HC-2 verified:** `cargo tree --workspace -e normal` is C-free — no
      onig/openssl/ring/aws-lc; `fancy-regex 0.16.2` confirmed as syntect's
      engine; `ring` remains dormant (not in the normal graph). CI guard grep
      extended with `onig`/`onig-sys`/`onig_sys`; `deny.toml` bans `onig` +
      `onig_sys`.
- [x] `cargo build`/`clippy`/`fmt` green; full suite still 74 tests passing.
- [x] **Decision recorded:** the TUI is event-driven, not immediate-mode-first —
      it keeps a local view-model updated by `UiEvent`s and redraws on
      event/tick/input; it never blocks the engine (redraw and input handling
      are independent of streaming). ratatui's immediate-mode draw call is just
      the render step over that view-model each frame.

---

## 1. Frontend abstraction & terminal lifecycle  *(A-1; HC-3; Design §7; Tech Spec §9, §10)*

- [x] `Frontend` seam: `frontend::{FrontendKind, detect, run}` dispatches to the
      rich TUI (`tui::run`) or line-mode (`line::run`) over the same
      `FrontendPorts` (A-1). Line mode is retained as the degraded/headless path.
- [x] **Terminal guard (RAII):** `terminal::TerminalGuard::enter()` enters raw
      mode + alternate screen + hides the cursor + enables bracketed paste;
      `Drop` calls `restore_terminal()`. PTY smoke confirmed the exact enter/
      leave escape sequences (`1049h/2004h/25l` up, `2004l/1049l/25h` down).
- [x] **Panic-safe restore (HC-3):** the guard installs a panic hook that calls
      `restore_terminal()` *before* the binary's existing hook prints the calm
      bug notice — so the message lands on a usable terminal, not inside the
      cleared alternate screen. Same restore fn as `Drop`; idempotent, so the
      "hook + unwind Drop" double-call is safe. (Live forced-panic check folded
      into group 11's manual smoke; transcript flush still Phase 5 — marker
      kept.)
- [x] Frontend selection in `main.rs`: `frontend::detect(force_plain)` — rich by
      default; line mode on `--plain`, `NO_COLOR`, `TERM=dumb`, or non-tty
      stdout. One place decides; group 10 finalizes/tests the predicate. Banner
      printed only in plain mode (the alt screen would wipe it in rich mode).
- [x] Main event loop: `select!` over engine `UiEvent`s and terminal input
      (read on a dedicated OS thread → channel, since `crossterm::event::read`
      blocks). Redraw on event/input/resize; never blocks the engine. The
      animation-tick arm is added in group 9.
- [x] Local view-model (`app::App`): session meta, conversation log, streaming
      flag, input buffer, context %/cost, sandbox, mode, modified files, pending
      permission, sidebar visibility. Updated by the pure `apply_event` reducer;
      6 reducer/input unit tests (accumulate deltas, tool-finish correlation,
      modified-file upsert, permission deny-default + deliberate-allow, submit).

---

## 2. Centralized theme & string table  *(Design §2, §6.2)*

- [ ] One `theme` module: named **roles**, not scattered colors — background/
      raised-surface, ember accent + dim accent, primary text, secondary/chrome,
      semantic allow/deny/warning, diff add/del. Candidate values from Design
      §2 (tune by eye later). Theming later = swapping values into roles.
- [ ] **Reserved safety styling** as its own role, used *only* by the
      outside-root permission band (group 7). A lint/comment so it is never
      reused decoratively (Design §2, §5).
- [ ] Light-terminal legibility fallback path (Design §2).
- [ ] One `strings` module/table: every interface string (English, v1). No
      scattered literals — cheap discipline, leaves localization open (Design
      §6.2). Voice rules (§6.1–§6.2): second person, present tense, terse
      chrome (2–5 words), no exclamation marks; error shape *what → why → next*.

---

## 3. Grapheme-aware text engine + line editor  *(Requirements §2.1; Design §6.2; Tech Spec §9)*

- [ ] `text` module: cluster-wise measurement (`unicode-segmentation` +
      `unicode-width`) — `display_width(&str)`, wrap-to-width, and
      cluster-index ↔ column mapping. **Never** `str::len`, **never**
      chars-as-columns. Thai combining vowel/tone marks are zero-width and must
      not consume a column; used everywhere (input, wrap, overlay scroll).
- [ ] **First-party line editor** (grapheme-aware): insert/delete by cluster,
      left/right by cluster, home/end, word ops, Emacs-style basics; multi-line
      via Shift+Enter; bracketed paste; history (up/down). Cursor column via the
      text engine so it lands correctly amid Thai clusters.
- [ ] Unit tests on Thai fixtures with **stacked** vowel/tone marks: width,
      wrap boundaries, and cursor motion. These are the §14 correctness anchors
      — write them here, reuse in group 11.

---

## 4. Layout: main pane + sidebar + status bar  *(Design §3; Tech Spec §9)*

- [ ] **Main pane** — the conversation: user prompts, assistant text (group 5),
      tool activity, diffs (group 6), permission prompts (group 7). Owns
      scrollback (PageUp/Down, wheel). Fold in the known **`> bash: bash`**
      cosmetic fix here: tool-activity lines render `verb + summary`, not a
      redundant `tool: tool` label.
- [ ] **Right sidebar** (fixed width, collapsible keybinding), in order:
      wordmark + version · session title (renamable) · project root (`~`-abbrev)
      · model block (provider/model, context %, session cost "est.", sandbox
      status — dimmed when confined, warning styling + reason when degraded) ·
      **modified files** (path + `+adds/-dels`, selectable → cumulative diff
      overlay, group 6). Extension sections (MCP/LSP/Skills) absent in v1 —
      reserve the pattern, not empty stubs.
- [ ] **Status bar** (bottom, one line): current mode · context % · 3–5
      contextual keybinding hints (dimmed), hints change with state (e.g. the
      permission prompt's keys while it's open).
- [ ] **Auto-collapse below 100 columns** (Tech Spec §9): sidebar hides;
      context % and mode migrate to the status line. Everything in the sidebar
      also reachable via commands (`/files`, `/session`) so nothing is
      sidebar-exclusive (Design §3.2).
- [ ] Box-drawing minimal: section dividers + the sidebar separator only;
      content never trapped in full boxes (Design §2).

---

## 5. Markdown subset + syntax highlighting  *(Design §4.1; Tech Spec §9)*

- [ ] **First-party** minimal markdown pass (no full md parser for chat flow):
      fenced code blocks, bold, inline code, bulleted/numbered lists, headings
      as bold + spacing. Everything else (tables, images, links, nested exotica)
      passes through as **plain unmangled text**.
- [ ] Syntax highlighting via `syntect` (**fancy-regex** backend) for fenced
      blocks. Accent-free scheme — highlighting uses neutral/semantic range so
      code never competes with the ember accent (Design §4.1).
- [ ] Streaming-aware: assistant text arrives as deltas; render incrementally
      without re-highlighting the whole buffer per delta (perf + no flicker).
      Highlight closed code fences; show open/streaming fences as plain until
      closed.
- [ ] Wrapping via the group-3 text engine (Thai-safe inside prose and code).

---

## 6. Diffs first-class (inline + overlay) + `$EDITOR` hatch  *(Design §4.2–§4.3)*

- [ ] First-party unified diff rendering from the edit tool's before/after (we
      own both sides — no external diff binary; reuse `emberly-tools` diff
      helpers where they fit). File-path header + line numbers; green additions
      / red deletions via semantic roles; ASCII `+`/`-` prefixes carry meaning
      without color (degraded parity, group 10).
- [ ] **Inline** in the main pane when an edit executes, and inside edit
      permission prompts (group 7).
- [ ] **Overlay** (pane overlay, scrollable, Esc to dismiss) for a file's
      cumulative session diff, opened from the sidebar modified-files list.
      Overlay scrolling uses the group-3 engine.
- [ ] `/view` escape hatch: open any assistant message (raw markdown) read-only
      in `$VISUAL` → `$EDITOR` → fallback print-to-pane with notice. Same
      mechanism reused for full untruncated tool outputs behind truncation
      markers (Design §4.3; Requirements §8.1).

---

## 7. The permission prompt — the most important screen  *(Design §5, §7; HC-4)*

> Built with the most care in the phase. Saying yes must require having seen
> what you are saying yes to. Rich and degraded modes carry identical guarantees.

- [ ] **Full content, always.** Complete command / complete diff / all affected
      paths. Never truncated to fit — long content **scrolls within the
      prompt**. If unscrolled to the end, the approve hint indicates there is
      more below.
- [ ] **Escalation is visually loud.** Outside-project-root (HC-4) uses the
      **reserved safety styling** (group 2) — distinct color band + explicit
      plain-language line: "This affects files OUTSIDE your project." Impossible
      to mistake for a routine prompt at a glance.
- [ ] **Choices:** Deny (safe default) · Allow once · Allow for this session
      (where the rule layer permits — session-grant persistence itself is
      Phase 2's rule engine; wire the choice, note the dependency). Default
      keypress (Enter/Esc) → **Deny**. Approval is a distinct, deliberate key.
- [ ] **Forbidden patterns** (enforce + test): no timeout-to-approve; no "Enter
      approves whatever is focused"; no batching distinct actions under one
      approval; **no auto-scroll** moving content out from under the user while
      the prompt is open (motion is off here — group 9).
- [ ] **Why line:** one dimmed line stating which rule matched or "outside
      project root" — teaches the model in situ.
- [ ] Reuse the line-mode prompt's guarantees as the degraded rendering (group
      10) — capitals banner, deny-default, deliberate key — so both modes are
      provably equivalent.

---

## 8. Command palette + command registry  *(Design §3.3; Tech Spec §9)*

- [ ] **Single command registry** = one source of truth: name, keybinding,
      one-line description, handler. Serves the palette, `/command` parsing, and
      `/help`. All functionality reachable three ways: palette, slash-command,
      keybinding.
- [ ] **Ctrl+P** palette: fuzzy match over the registry, keybindings + one-line
      descriptions shown; Enter runs, Esc dismisses.
- [ ] Slash-command parser in the input box routes to the same registry.
- [ ] v1 command set (wire what exists; stub Phase-5 ones as "coming soon" only
      if listed): `/help`, `/commands`, `/view`, `/files`, `/session`,
      `/cancel`, sidebar toggle, mode toggle (mode itself is Phase 2), quit.
      New-user help teaches the palette first (Design §3.3).

---

## 9. Motion — few, small, purposeful  *(Design §6.4; Tech Spec §9)*

- [ ] **Single animation ticker** (~12fps) drives everything; never blocks
      input or streaming. Animation state carries **zero** information (ambient
      only).
- [ ] Sanctioned motion only: ember-**pulse** spinner (dimmed verb phrase —
      "thinking"/"running tests"/"reading files"; elapsed time after 5s, Design
      §6.3); subtle accent brightness pulse while streaming; brief overlay
      ease-in (1–2 frames); modified-file sidebar settle.
- [ ] **Skipped entirely** when: `motion = false` config key, degraded mode
      (group 10), or a permission prompt is open (group 7). No looping animation
      on an idle screen except the prompt cursor.
- [ ] Test: with motion off, one static render per state change; the ticker is
      not spawned.

---

## 10. Degraded mode parity  *(Design §7; Tech Spec §9)*

- [ ] Degraded predicate (finalize group 1): `--plain` flag OR `NO_COLOR` OR
      `TERM=dumb` OR non-tty stdout → line-mode frontend. Rich TUI otherwise.
- [ ] Degraded rendering (extend the existing `LineRenderer`): no color, no
      box-drawing, no spinner (plain "working…" lines), ASCII-only markers,
      append-only, **no cursor repositioning**. Diff `+`/`-` and the words
      ALLOW/DENY carry meaning without color — color/unicode are enhancement,
      never sole carrier, everywhere.
- [ ] Permission prompt keeps every guarantee in degraded mode: full content,
      **OUTSIDE-PROJECT-ROOT banner in capitals**, deliberate approve key
      (already true in line-mode — assert it stays true).
- [ ] `--plain` promoted from Phase 1's no-op to the real forcing flag; the
      line-oriented path is the de facto headless-frontend contract (A-1).
- [ ] Degraded mode is a **supported, tested** configuration, not best-effort.

---

## 11. Tests, fixtures & exit criterion  *(Tech Spec §14; A-1)*

- [ ] **Thai fixtures** (stacked vowel/tone marks): width, wrap, cursor motion,
      and that content renders/wraps correctly in main pane, input, and overlay.
- [ ] **Degraded-mode tests:** line output is append-only/ASCII; degraded
      predicate picks the right frontend per env.
- [ ] **Permission-prompt guarantees** as assertions in both rich and degraded
      renderings: full content shown, deny is default, outside-root banner
      present, no forbidden pattern (no timeout, no batching).
- [ ] View-model reducer tests: feeding a canned `UiEvent` sequence produces
      the expected sidebar/status/modified-files state (pure, no terminal).
- [ ] Layout tests: sidebar auto-collapses below 100 columns and info migrates
      to the status line.
- [ ] fmt + clippy (unwrap/expect gates) clean; HC-2 guard passes with the new
      deps; full workspace test suite green.
- [ ] **Manual smoke (macOS, live provider):** drive a full real session —
      stream markdown + highlighted code, run a tool with a permission prompt,
      view an inline diff and a sidebar cumulative-diff overlay, type Thai in the
      input, use Ctrl+P, toggle the sidebar, resize below 100 cols, run
      `--plain`. Panic/Ctrl-C leaves the terminal usable.

**Exit criterion (plan §Phase 4 "Done when"):** the TUI drives a full session;
Thai-fixture and degraded-mode tests pass; the permission prompt meets every
Design §5 guarantee in **both** rich and degraded modes.

---

## Decisions log

- **Branch:** `phase-4-tui` off `main` (Phase 3 merged).
- **Line mode is not thrown away** — it becomes the degraded/`--plain`/headless
  frontend (Design §7, A-1). The rich TUI is the *second* implementation over
  the same `FrontendPorts`, which is exactly the A-1 seam Phase 1 built for.
- **`syntect` = fancy-regex backend only** (pure Rust). Oniguruma (`onig`) is C
  and forbidden by HC-2; CI guard extended to catch `onig`/`onig_sys`.
- **Terminal restore is an HC-3 obligation**, wired via an RAII guard + the
  panic hook, verified by forcing a panic — not left as polish.
- **Reserved safety styling** (group 2) is used by the outside-root permission
  band and nothing else, ever (Design §2/§5).
- **Cosmetic `> bash: bash`** redundant tool-start label (carried over from
  Phase 3) is fixed in group 4's tool-activity rendering.
- **Deferred to Phase 5** (note, don't build): transcript flush on the panic
  path, session-title auto-generation, mode toggle *behavior* (Phase 2 rule
  engine + sandbox gating), session-grant persistence. Wire the surfaces; leave
  the markers.
