# Phase 2 — In-app config & prompt editing — TODO & Progress

**Milestone:** M6 group 2 (Tech Spec §15). Edit configuration and prompt
files from within a running session, without dropping to a separate shell
(C-5). Editing honors the tier model (C-1) and provenance (C-3): writes land
in the project tier, and the user sees where a value comes from before
changing it.
**Satisfies:** C-5; Tech Spec §8; Design §3.3, §4.3, §4.6. Pinned to Req
v0.5 / Design v0.5 / Spec v0.4.
**Goal:** `/config` and `/prompt` open the file for editing (quick overlay
or `$EDITOR`); a save takes effect on the running session, and any
restart-only change is named at save time.

**Depends on:** Phase 1 (merged) — the overlay machinery, the command
registry, and the injected-capability pattern (`ProviderFactory`) are the
templates this phase reuses.

**Legend:** `[ ]` todo · `[~]` in progress · `[x]` done · `[!]` blocked

---

## Progress summary

| Group | Status | Notes |
|---|---|---|
| 1. `$EDITOR` handoff (suspend/run/restore) | [x] | Done 2026-07-10; `edit` module + guard suspend/resume; 3 tests |
| 2. Edit commands & targets (`/config`, `/prompt`) | [x] | Done 2026-07-10; rich TUI; 3 tests (line mode → group 6) |
| 3. Quick-edit overlay (single value / short prompt) | [-] | Deferred — fast-follow (owner) |
| 4. Provenance-before-edit + project-tier writes | [x] | Done 2026-07-10; new-vs-existing notice; project-tier only |
| 5. Reload semantics (Live vs RestartRequired) | [x] | Done 2026-07-10; ConfigReloader + ReloadConfig; 2 tests |
| 6. Tests, degraded mode, docs, exit criterion | [x] | Done 2026-07-10; line-mode `/config`/`/prompt`/`/reload`, README, exit met |

**Overall Phase 2: COMPLETE (2026-07-10)** (group 3 deferred as a
fast-follow). In-app `/config` and `/prompt` edit via `$EDITOR` with
provenance and project-tier writes; edits apply live to the running session
(`/reload` on demand); line mode reaches the same via path + `/reload`.
Workspace clippy + fmt clean; 21 test binaries green. Next: Phase 3 —
reasoning effort & the thinking trail (P-9, P-10).

---

## 1. `$EDITOR` handoff  *(Design §4.3, §4.6)*

The full-fidelity escape hatch does not exist yet (`/view` currently opens an
in-TUI text overlay). Build it once, reuse it for config and prompts.

- [x] `edit` module: `resolve_editor` (`$VISUAL` → `$EDITOR`, pure/testable),
      `run_editor(path)` / `run_program(program, path)` spawning the editor as
      a foreground child on the file and waiting. Returns a structured
      `EditStatus` (`Edited` / `NoEditor` / `Failed`) — never an `Err`.
- [x] `TerminalGuard::suspend`/`resume`: leave raw mode + alternate screen for
      the child, then re-enter and clear for a redraw (shared `enter_modes`).
      Restore is idempotent + panic-safe (HC-3), unchanged.
- [x] Tests: editor resolution (VISUAL/EDITOR/empty), a fake-editor script
      that appends to the file (proves the handoff runs on the file), and a
      failing editor → `Failed` not a crash.
- [ ] Line mode: run the editor directly (no alt screen) — wired in group 2
      alongside the line-mode commands.

## 2. Edit commands & targets  *(C-5; Design §3.3)*

- [x] `/config` — edits `.agents/config.toml`; seeds it from the shared init
      template (`init::CONFIG_TEMPLATE`, now `pub`, single source) when absent,
      never clobbering an existing file. Returns `Action::EditFile`.
- [x] `/prompt [name]` — edits `system` (default) / `compact`; seeds from the
      baked-in default (`emberly_core::prompts`) when the project has no
      override (C-1/C-2); unknown name → notice. `/prompt` arg parsed in
      `run_slash` (like `/model`).
- [x] Reachable via palette + `/name` (registry entries `config`, `prompt`).
      No dedicated keybinding (owner: palette + slash only).
- [x] `Action::EditFile(PathBuf)` handled in `tui.rs`: **pause the input
      reader** (refactored to poll+flag so the editor gets the keystrokes),
      `suspend` → `run_editor` → `resume`, then `note_edit` reports the
      outcome. `config_template` threaded `main.rs → frontend::run →
      tui::run → App`.
- [x] Tests: seed-then-edit without clobbering, prompt seed-from-default +
      unknown-name rejection, `note_edit` no-editor notice.

## 3. Quick-edit overlay  *(Design §4.6)*

- [ ] An **editable** overlay (reuse the `OverlayContent` + `LineEditor`
      machinery) for a single config value or a short prompt, for edits not
      worth an `$EDITOR` round trip. Enter saves, Esc cancels.
- **Deferred to a fast-follow (owner decision 2026-07-10).** v0.2 ships the
  `$EDITOR` path (groups 1–2); the in-TUI editable overlay comes later. Left
  here so the design intent (reuse `OverlayContent` + `LineEditor`) is
  recorded.

## 4. Provenance-before-edit + project-tier writes  *(C-1, C-3)*

- [x] Before an edit, a provenance notice (C-3) distinguishes **editing an
      existing project value** from **creating a fresh override seeded from the
      default/template** — so the user knows whether they are about to override
      a baked-in default. (Full default/global/project tiers remain available
      via `emberly config show`; the edit path scopes to project-vs-default,
      which is the actionable distinction.)
- [x] Writes always land in the **project tier** (`.agents/config.toml`,
      `.agents/prompts/<name>.md`), never global config or the baked-in
      defaults (C-1). The path is shown in the notice.

## 5. Reload semantics  *(Tech Spec §8)*

The engine caches `system`/`summary_prompt` (and the picker's profile set)
at construction. A save must take effect without a restart where it can.

- [x] **Live** — system/compact prompts (and AGENTS.md, since `config::load`
      recomputes the combined system prompt) + the `[providers.*]` set.
      **RestartRequired** — `sandbox.require` (trust later): detected by the
      reloader diffing against the launch value and surfaced as a note.
- [x] Mechanism (mirrors `ProviderFactory`): `ConfigReloader` trait +
      `ReloadedConfig` (core); host `ConfiguredReloader` re-runs `config::load`
      and rebuilds the factory/prompts. `Command::ReloadConfig` → engine
      applies `system`/`summary_prompt`/factory, emits `ProfilesChanged` when
      the set changed (picker refresh) + a `Notice` listing what changed and
      any restart note; a reload error is a `HarnessError`, current config
      kept. The TUI sends `ReloadConfig` after a successful `$EDITOR` edit.
- [x] Applies to subsequent turns only: the active provider/model is left
      as-is (use `/model` to switch), and no prior turn or transcript line is
      rewritten.
- [x] Tests: reload applies + reports live changes + restart note + emits
      `ProfilesChanged`; no-reloader path is a calm notice.

## 6. Tests, degraded mode, docs & exit criterion

- [x] Unit tests: `config_target`/`prompt_target` (seed-from-default,
      already-exists, unknown-name), the App `/config`/`/prompt` seeding +
      provenance notices, `note_edit`, and the engine reload-diff/report.
- [x] `$EDITOR` handoff tested via a fake-editor script (appends to the file);
      `resolve_editor` unit-tested; terminal suspend/restore is the
      `TerminalGuard` path, exercised by the interactive smoke.
- [x] Degraded-mode parity: line mode has no editor handoff (stdin is the
      reader / often a pipe); `/config` and `/prompt` seed + print the path
      and `/reload` applies — smoked live. `/reload` also added to the rich
      TUI as a manual escape hatch.
- [x] README: an "Editing config and prompts from a session" section — the
      commands, project-tier writes, live apply, restart-only naming, and the
      plain-mode behavior.
- [x] **Exit criterion met:** `/prompt` (or `/config`) via `$EDITOR` writes to
      the project tier, a provenance notice precedes the edit, the change is
      applied to the running session on save (system prompt next turn; new
      profile in the `/model` picker), and a restart-only change
      (`sandbox.require`) is named — verified by unit tests + the live line-mode
      smoke; lint gates and the suite are green (21 binaries).

---

## Decisions & notes log

Confirmed with the owner 2026-07-10:

- **`$EDITOR` handoff first; in-overlay editor is a fast-follow.** Group 1–2
  (the `$EDITOR` path) is the v0.2 deliverable; group 3 (the editable
  overlay) is deferred within this phase, not built now.
- **Live-reload split accepted:** prompts (system/compact) and the
  `[providers.*]` set reload **Live**; startup-only keys
  (`sandbox.require`, later trust) are **RestartRequired** and named at save
  time. Honest fallback if hot-swap proves fiddly: apply on next session and
  always say what changed.
- **Palette + slash only** for `/config` / `/prompt` (no dedicated
  keybinding), consistent with Design §10's open question.
