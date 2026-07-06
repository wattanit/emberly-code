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
| 2. Centralized theme & string table | [x] | theme.rs (named roles + reserved safety band) + strings.rs; 3 theme tests |
| 3. Grapheme-aware text engine + line editor | [x] | text.rs + editor.rs; Thai stacked-mark fixtures; 16 tests; expect-verified |
| 4. Layout: main pane + sidebar + status bar | [x] | render.rs: two-pane + sidebar + status; auto-collapse <100; scrollback; bash:bash fixed |
| 5. Markdown subset + syntax highlighting | [x] | markdown.rs: first-party md + syntect (fancy-regex); span-preserving wrap; 8 tests |
| 6. Diffs first-class (inline + overlay) | [x] | diffview.rs + FileDiff plumbing; inline (capped) + Ctrl+O overlay; /view → group 8 |
| 7. The permission prompt | [x] | scrollable full-content, loud outside-root, deny-default, diff-rendered; 5 render/behavior tests |
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

- [x] `theme.rs`: a `Palette` of named colour roles (background/raised, accent/
      dim-accent, primary, chrome, success/error/warning, safety fg+bg) with
      Design §2 candidate values, and a `Theme { palette, color }` whose role
      methods build ratatui `Style`s. Theming later = a different `Palette`
      behind the same roles — the values are the only thing that changes.
- [x] **Reserved safety styling** as its own role, `Theme::safety_band()` —
      loud fg-on-deep-red bold band, documented as outside-root-only, never
      decorative (Design §2, §5). A test asserts it is a distinct bg band.
- [x] Light-terminal legibility fallback: `Palette::light()` / `Theme::light()`
      (same roles, dark-on-light values). Selection hook is present; auto
      light/dark detection is out of scope for v1.
- [x] `strings.rs`: every chrome string (brand, permission, status/sidebar,
      mode names, keybinding hints, markers) in one table — no scattered
      literals. Voice rules applied (terse, lower-case chrome, no exclamation
      marks). The **permission-critical strings are shared** by the TUI and the
      line frontend, so degraded-mode parity is structural, not accidental.
- [x] `tui.rs` render + `line.rs` permission prompt refactored onto theme +
      strings; `App` carries the active `Theme` as the single source. `plain()`
      theme (colour off) exists for tests / future headless reuse.

---

## 3. Grapheme-aware text engine + line editor  *(Requirements §2.1; Design §6.2; Tech Spec §9)*

- [x] `text.rs`: cluster-wise measurement (`unicode-segmentation` +
      `unicode-width`) — `width`, `cluster_count`, `prev/next_boundary`,
      `col_at`/`byte_at_col`, word-aware `wrap`, and `slice_cols` (cluster-
      aligned horizontal scroll). No `str::len`/chars-as-columns anywhere.
- [x] **First-party line editor** (`editor.rs`, grapheme-aware): insert/delete
      by cluster (backspace removes a base + all its stacked marks), left/right
      by cluster, home/end (logical-line), word left/right + delete-word-back,
      Ctrl+K kill-to-end, multi-line up/down keeping the display column,
      Shift+Enter newline, bracketed paste (`insert_str`), and up/down history
      with draft stash. Wired into `App` (replaces the plain `String`); keys
      routed in `on_key`; input rendered with a 2-col gutter, vertical +
      per-line horizontal scroll, and a grapheme-correct terminal cursor.
- [x] Unit tests on Thai stacked-mark fixtures ("ที่" = 3 scalars, 1 cluster, 1
      column; "ไทย" = 3 columns): cluster count/width, boundary stepping, col↔
      byte round-trip, wrap (word-break, hard-break, blank lines), plus editor
      backspace/motion/word-ops/multiline/submit/history. 16 tests total.
      Interactive path expect-verified (Thai input → clear → clean exit + restore).

---

## 4. Layout: main pane + sidebar + status bar  *(Design §3; Tech Spec §9)*

- [x] **Main pane** — conversation with user prompts, assistant text, tool
      activity, notices; permission prompt takes over the pane (group 7 makes
      it scrollable). **Scrollback** via PageUp/PageDown over rows pre-wrapped
      through the text engine (exact row math); a dim "↓ more" hint when
      scrolled up; jumps to bottom on submit. Mouse-wheel deferred (needs mouse
      capture, which fights terminal selection — noted). **`bash: bash` fixed**:
      tool rows render the summary verb phrase with no redundant `tool:` prefix.
- [x] **Right sidebar** (`render_sidebar`, width 32, Ctrl-B toggles): wordmark +
      version · session title (or "untitled session") · `~`-abbreviated root ·
      model block (provider/model, context % with warn/error colour ≥75/≥90,
      cost "est." when known, sandbox status — dim confined / warning
      partial+unavailable / "—" unknown) · **modified files** (path fit to
      width + green `+adds` / red `-dels`, or "—"). Extension sections omitted,
      not stubbed.
- [x] **Status bar** (bottom, full width): mode · context % · state-dependent
      hints (permission keys while a prompt is open).
- [x] **Auto-collapse below 100 columns**: sidebar hidden; context % and mode
      remain on the status bar (their only home when collapsed). Reaching the
      rest by command is group 8 (`/files`, `/session`).
- [x] Box-drawing minimal: conversation/input use a border block; the sidebar
      uses a single LEFT separator; content is never trapped in a full box.
- [x] Rendering moved to a dedicated `render` module (pure over `&App`); `tui`
      is now just the driver. 4 render unit tests (fit/ellipsis, cluster-aware
      fit, `~` abbreviation, conversation wrap row count). Visually verified via
      a 120-col PTY smoke (sidebar shown, streamed reply, clean exit + restore).

---

## 5. Markdown subset + syntax highlighting  *(Design §4.1; Tech Spec §9)*

- [x] `markdown.rs`: first-party pass — fenced code, `**bold**`, `` `inline
      code` ``, `-`/`*`/`+`/`1.` lists (→ `•`/`n.`), `#`..`######` headings as
      bold. Tables/links/images/nested exotica pass through as plain text.
- [x] Syntax highlighting via `syntect` (fancy-regex, loaded once via
      `OnceLock`) for fenced blocks; syntect colours mapped onto ratatui while
      keeping our background. Neutral base16 theme (blues/greens/greys, no
      orange) so code never competes with the ember accent (Design §4.1).
- [x] Streaming-aware: only **closed** fences are highlighted; an unclosed
      (still-streaming) fence renders as plain text (Design §4.1 note). Assets
      load lazily on first closed fence, so startup is unaffected. (Per-frame
      re-highlight of closed blocks is acceptable at typical sizes; caching is
      a noted future optimization.)
- [x] Prose wraps via a **span-preserving** wrap over the group-3 text engine
      (flatten to graphemes+style, word-break, coalesce) so **bold** survives a
      line break; Thai-/CJK-correct. Code lines are not reflowed (clip at the
      pane edge — reflowed code is unreadable). 8 markdown tests (bold split,
      literal inline code, heading, list, plain passthrough, cross-style wrap,
      closed-fence highlight, open-fence-plain).

---

## 6. Diffs first-class (inline + overlay) + `$EDITOR` hatch  *(Design §4.2–§4.3)*

- [x] `diffview.rs`: first-party unified-diff rendering — file header (chrome),
      `@@` hunk header (dim accent), `+` green / `-` red via semantic roles,
      context dimmed. `+`/`-`/`@@` prefixes carry meaning without colour
      (degraded parity). `render_unified` + `render_unified_capped`. 3 tests.
- [x] **Plumbing**: `FileChange.diff: Option<String>` populated by edit/write
      (they already own both sides — reuse `emberly-tools::diff::unified_diff`);
      engine emits an additive `UiEvent::FileDiff { path, unified }` alongside
      `FileModified`. `line.rs` ignores it for now (degraded diff → group 10).
- [x] **Inline** on execute: `ConvItem::Diff` rendered in the conversation,
      capped at 20 rows with a "… N more — Ctrl+O to view" pointer. (Edit
      permission prompts render the diff in group 7 via `diffview`.)
- [x] **Overlay**: a scrollable, Esc-dismiss pane overlay (`Overlay` +
      `overlays` stack, modal for navigation; input can't leak into the editor
      while open). Opened with **Ctrl+O** for the most-recently-modified file's
      latest diff; centered `Clear`ed pane, ↑↓/PgUp/PgDn scroll, "↓ more" hint.
      Sidebar-entry *selection* to pick which file lands with the command system
      (group 8); true session-**cumulative** diff (vs. latest-edit) needs a
      baseline the transcript provides (Phase 5) — currently shows the latest
      change, noted.
- [~] `/view` `$EDITOR` hatch: **deferred to group 8**. It is a command *and*
      requires pausing the input reader while the child editor owns the tty
      (two readers on one tty otherwise) — that coordination belongs with the
      command system. The overlay already covers in-TUI diff review (Design
      §4.2's preferred path); `/view` is the §4.3 full-fidelity escape hatch.

---

## 7. The permission prompt — the most important screen  *(Design §5, §7; HC-4)*

> Built with the most care in the phase. Saying yes must require having seen
> what you are saying yes to. Rich and degraded modes carry identical guarantees.

- [x] **Full content, always.** The prompt takes over the whole main area (no
      input box while deciding); header (banner/heading/why/paths) is pinned,
      and the full command or diff **scrolls within the prompt** (↑↓ / PgUp /
      PgDn / Space / Home). Never truncated to fit; the footer shows "↓ N more —
      scroll to review" until the end is reached.
- [x] **Escalation is visually loud.** Outside-root uses the reserved
      `safety_band` banner *and* an error-coloured prompt border — unmistakable
      at a glance. Verified by a render test.
- [x] **Choices:** Deny (default) · Allow once (`y`) · Allow this session (`s`).
      Enter/Esc/`d`/`n` → **Deny**; allow is deliberate. Session-grant
      *persistence* is Phase 2's rule engine — the choice is wired, decision
      returns to the engine as today.
- [x] **Forbidden patterns** enforced: no timeout-to-approve (input-driven
      only); Enter maps to *deny*, never "approve focused"; one prompt = one
      action; **no auto-scroll** (scrolling is user-driven; the animation ticker
      is off during prompts — group 9). A stray key is *ignored*, so there is no
      accidental decision in either direction. Covered by behavior + render tests.
- [x] **Why line:** the dimmed `reason` from the engine (which rule / "outside
      project root") shown in the header.
- [x] Degraded rendering already shares the prompt's strings and guarantees via
      `line.rs` (group 2/10) — capitals banner, deny-default, deliberate key.
- [x] Edit/write prompts render their `detail` (a unified diff) through
      `diffview` with diff colours; commands render as wrapped plain text. 5
      tests (full-content+deny-default, loud outside-root, more-below indicator,
      diff rendering, scroll-doesn't-decide) via ratatui `TestBackend`.

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
