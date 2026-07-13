# Phase 6 Part 2 — Memory & Skills inspectors (Design §4.9) — TODO & Progress

**Milestone:** M8 follow-up (Tech Spec §15). **A gap-closing task, not a new requirement.**
The Memory/Skills *inspector overlays* are already specified in **Design §4.9** and mandated
by **Requirements FR-6 / FR-7**, but Phases 3–4 shipped the sidebar as **counts/catalog
display only** — no inspector overlay, no `/memory` / `/skills` palette command. This
closes that gap. Split out from Phase 6 (mouse) because it is orthogonal to pointer support
and independently mergeable; it is *conceptually* Phase 6's "part 2" only because Phase 6's
sidebar-click target (`PHASE6_TODO.md`, group 4) is inert until these inspectors exist.
**Satisfies:** Design §4.9 ("Inspectors, not black boxes"), §4.2 (overlay pattern), §4.6
(in-app edit path), §3.3 ("reachable three ways"); Requirements FR-6 (memory
inspect/edit/delete), FR-7 (skill body inspectable before it runs), C-5 (in-app editing),
C-3/C-1 (provenance/tier on edit), FR-1 (untrusted project content not shown); Tech Spec
§8.1 (memory store/layout), §8.2 (skills layout), §8.6 (progressive disclosure).
Pinned to **Req v0.7 / Design v0.7 / Spec v0.8** (all `approved`). **No foundation-doc
change** — this is downstream implementation catching up to already-approved docs.

**Goal.** Make memory and skills *inspectable and (for memory) correctable* from inside a
session, exactly as Design §4.9 states:
> **Inspectors, not black boxes.** The sidebar Memory section opens an overlay listing
> entries grouped by scope, each editable/deletable via the in-app edit path (§4.6)…
> The Skills section lists available skills by name, description, and origin; selecting one
> shows its instruction body read-only, so "what could this skill tell the model to do" is
> always inspectable before it ever runs.

And the FR-6 mandate:
> The user can **inspect, edit, and delete memory directly** — memory is never a place the
> harness hides state the user cannot see.

**Key structural facts (from the codebase):**
1. **There is no TUI→engine path for memory list/edit/delete.** `MemoryGate`/`MemoryGateImpl`
   are driven only from *tool execution* (`memory_rx` arm, `engine.rs:1617`). `FrontendPorts`
   is just `commands_tx` + `events_rx` (`channels.rs:26`), and the `Command` enum
   (`command.rs:17`–:83) has **no** memory/skill variants. Inspector mutations need **new
   `Command` variants handled at the engine idle loop** (mirror `ReloadConfig`/`SetEffort`,
   `engine.rs:842`) plus **response `UiEvent`s** — never direct TUI writes into the memory
   store (FR-6: memory is harness-owned/schema-constrained; the model/user supply content,
   the harness performs the write).
2. **Memory has no list operation.** `MemoryOp` is `Write | Update | Remove | Recall`
   (`emberly-tools/src/memory.rs:17`), and `MemoryStatus` carries **counts only**
   (`event.rs:190`). The store can scan (`build_index` `memory.rs:227`) but exposes no list
   API. Add a listing capability (engine/store-side, group 1).
3. **Skill bodies are lazy (progressive disclosure, §8.6).** `SkillMeta`/`SkillsAvailable`
   carry name/description/origin, **not** the body (`emberly-tools/src/skills.rs:25`,
   `event.rs:196`); the body loads via `SkillCatalog::invoke(name)` (`core/src/skills.rs:123`),
   a tool-time path. The inspector must fetch the body **through the engine** (so project
   precedence + untrusted-root gating still apply, `core/src/skills.rs:99`, :34) — not by
   the TUI reading `SKILL.md` itself.
4. **The edit path is `$EDITOR` handoff (§4.6 full-edit).** `Action::EditFile`
   (`app.rs:227`) → `edit::run_editor` (`tui.rs:101`) → on save `Command::ReloadConfig`;
   `/config` and `/prompt` use it (`app.rs:1376`,:1403). Memory edit **mirrors this but
   commits via a memory `Update` command, not `ReloadConfig`** (`ReloadConfig` only reloads
   config/prompts, `command.rs:60`).
5. **Trust is inherited for free (FR-1).** On an untrusted root the store's `project_dir`
   is `None` (`memory.rs:49`) and the skill catalog's project location is skipped — so the
   inspector's project section is simply empty/absent with no extra logic. Keep it that way.
6. **`adopt_session` does not re-emit `MemoryStatus`** (`engine.rs:1022`) though it does
   re-emit `SkillsAvailable` (:1030). Fix while here so the sidebar/inspector isn't stale
   after `/resume`.

**Depends on / seams to reuse:**
- Memory store — `MemoryStore`, `MemoryEntry{meta,body}`/`EntryMeta{name,description,type_}`
  (`core/src/memory.rs:15`), ops `execute` (:58), `status_counts` (:170), index scan
  (`build_index` :227), frontmatter split (:reused).
- Memory types/gate — `MemoryScope`/`MemoryOp`/`MemoryRequest`/`MemoryOutcome`
  (`tools/src/memory.rs:17`), `MemoryGateImpl` (`gate.rs:162`), `on_memory_op`
  (`engine.rs:2459`).
- Skill catalog — `SkillCatalog::discover`/`invoke` (`core/src/skills.rs:59`,:123),
  `SkillMeta` (`tools/src/skills.rs:25`), `refresh_skill_catalog` (`engine.rs:2446`),
  `SkillGateImpl` (`gate.rs:200`).
- Events — `MemoryStatus`/`SkillsAvailable` (`event.rs:190`,:196), emit sites
  (`engine.rs:758`,:772; adopt_session :1022,:1030), app storage (`app.rs:321`,:325; apply
  :688,:692), sidebar render (`render.rs:724`,:735).
- Commands/channels — `Command` enum (`command.rs:17`), idle loop (`engine.rs:842`),
  `FrontendPorts` (`channels.rs:26`), TUI send (`tui.rs:86`).
- Overlays — `OverlayContent{Diff,Text,Sessions,Choices}` (`app.rs:98`), `push_overlay`
  (:1447), `open_text_overlay` (:1182), pickers (:1194–:1281), `on_overlay_key` (:1760),
  `on_session_picker_key` (:1797), `on_choice_picker_key` (:1857); render (`render.rs:184`).
- Palette/slash — `AppCommand` + `COMMANDS` (`commands.rs:12`,:63), `run_slash`/`run_command`
  (`app.rs:1463`,:1532), palette Enter (:1708), `help_text` (:1995).
- Edit path — `edit::run_editor` (`edit.rs`), `Action::EditFile` (`app.rs:227`), `edit_config`
  (`app.rs:1376`), `EditFile` loop handling (`tui.rs:101`).
- Tests — memory store units (`memory.rs:299`), skill units (`skills.rs:219`), engine
  integration (`tests/engine_loop.rs:3834` memory, :4034 skills), TUI app/overlay tests
  (`app.rs` `#[cfg(test)]` ~:2008), render (`render.rs:1302`).

**Legend:** `[ ]` todo · `[~]` in progress · `[x]` done · `[!]` blocked

---

## Progress summary

| Group | Status | Notes |
|---|---|---|
| 1. Engine/store data model: memory listing + skill-body fetch (`emberly-core`, `emberly-tools`) | [ ] | add list capability; reuse `SkillCatalog::invoke` for body |
| 2. TUI→engine command + event plumbing: new `Command`/`UiEvent` variants; adopt_session fix (`emberly-core`) | [ ] | idle-loop handlers mirroring `ReloadConfig`; no direct TUI writes |
| 3. Memory inspector overlay: `/memory`, grouped list, view/edit/delete (`emberly-tui`) | [ ] | edit via `$EDITOR`→`Update`; delete confirmed; scope/origin shown (FR-6, §4.6) |
| 4. Skills inspector overlay: `/skills`, list, read-only body (`emberly-tui`) | [ ] | body fetched via engine (progressive disclosure); origin visible (FR-7) |
| 5. Degraded parity + rendering + strings (`emberly-tui`) | [ ] | overlays are rich-only; define plain-mode `/memory` `/skills` behavior |
| 6. Tests (offline — §14.7) + exit criterion (`emberly-core`, `emberly-tui`) | [ ] | round-trips, trust-absence, delete-confirm, body-not-pinned preserved |

**Overall Phase 6 Part 2: NOT STARTED.**

---

## 1. Engine/store data model — memory listing + skill-body fetch  *(FR-6, FR-7, §8.6)*

- [ ] **Memory listing.** Add a store-side list that returns entry *summaries* (not bodies —
      keep progressive disclosure): `MemoryStore::list_entries(scope) -> Vec<EntrySummary
      { name, description, type_, scope }>`, reusing the directory scan in `build_index`
      (`core/src/memory.rs:227`). **Decide (notes log):** expose this as a store method
      driven by a new engine command (preferred — keeps the *model-facing* `MemoryOp` enum
      unchanged; the model never needs `List`) **vs.** adding `MemoryOp::List` to the tool
      enum. Lean to a store method + engine command, since listing is a TUI/user action, not
      a model turn.
- [ ] **Memory body read** for the edit/view step reuses the existing `MemoryOp::Recall`
      semantics (`memory.rs:105`) — read the body + origin on demand for one named entry.
- [ ] **Skill-body fetch** reuses `SkillCatalog::invoke(name)` (`core/src/skills.rs:123`),
      which already loads the post-frontmatter body (and lists bundled resources) under the
      engine's trust/precedence rules. No new discovery logic — the inspector fetch is a
      read-only invoke-for-display (it must **not** run any bundled script — invoking a
      skill's *body for display* is not executing its scripts; §4.9 / FR-7).
- [ ] Keep bodies **out of standing context** — listing/summary only in the catalog/index;
      bodies flow only in response to an explicit inspector fetch (preserve the
      `engine_loop.rs:4121` "body is NOT pinned" invariant).

## 2. TUI→engine command + event plumbing  *(C-5; the harness-owned-store rule, FR-6)*

- [ ] **New `Command` variants** (`core/src/command.rs:17`), handled at the **idle loop**
      (`engine.rs:842`, mirroring `ReloadConfig`/`SetEffort`):
      - `Command::MemoryList` — request the grouped entry summaries.
      - `Command::MemoryMutate { op: Update | Remove, scope, name, description?, type_?,
        body? }` — commit an edit or a delete. Route through the **same** validated path as
        `on_memory_op` (`engine.rs:2459`) — extract a shared `execute_memory_op` so tool and
        inspector share one code path, re-emitting `MemoryStatus` after (FR-6: harness
        performs the write; name re-validated via `slug`).
      - `Command::InspectSkill { name }` — fetch a skill body for display.
- [ ] **New response `UiEvent`s** (`core/src/event.rs`): `MemoryEntries { user:
      Vec<EntrySummary>, project: Vec<EntrySummary> }` and `SkillBody { name, origin, body }`
      (+ optional resource paths). The frontend sees oneshot gate replies only as events, so
      these carry the async results back to the overlay.
- [ ] **Fix `adopt_session`** (`engine.rs:1022`) to re-emit `MemoryStatus` after
      `refresh_memory_indexes()` — matching the `SkillsAvailable` re-emit at :1030 — so the
      inspector/sidebar are correct after `/resume`.
- [ ] Wire `commands_tx`/`events_rx` as today (`tui.rs:86`, app `apply_event`
      `app.rs:688`); no channel-shape change (still `FrontendPorts`).

## 3. Memory inspector overlay  *(Design §4.9, §4.6; FR-6, C-5, C-3, FR-1)*

- [ ] **Palette + slash command.** Add `AppCommand::Memory` (`commands.rs:12`) + a `COMMANDS`
      entry (`commands.rs:63`) `"memory" — "Inspect, edit, and delete stored memory"`, and a
      `run_command` arm (`app.rs:1532`) → `open_memory_inspector()`. Auto-appears in
      `help_text` (`app.rs:1995`) and is thus "reachable three ways" (§3.3); the Phase 6
      sidebar click becomes the optional fourth way once merged.
- [ ] **Open + list.** `open_memory_inspector()` sends `Command::MemoryList`; on
      `UiEvent::MemoryEntries`, push an overlay listing entries **grouped by scope**
      (user-global, then project), **origin on every line** (FR-6/§4.9 — origin is how the
      user reads trust), showing `name — description`. **Decide (notes log):** a new
      `OverlayContent::MemoryEntries { … , selected }` vs. reusing `Choices` with a new
      `ChoiceKind::MemoryEntry` — a new variant is cleaner because Enter here has two
      actions (edit / delete), not the single "apply" of the pickers.
- [ ] **View / edit an entry (§4.6 full-edit).** Selecting an entry shows its body
      (`Recall`) and its scope/provenance (C-3 — the user sees which scope before changing
      it). Edit uses the `$EDITOR` handoff exactly like `edit_config` (`app.rs:1376` →
      `Action::EditFile` → `tui.rs:101`), but on save commits via
      `Command::MemoryMutate { op: Update, … }` — **not** `ReloadConfig`. Edits land in the
      entry's existing scope (C-1: never silently mutate another tier).
- [ ] **Delete an entry — confirmed.** Deletion is destructive, so it requires an explicit
      confirm step (a `y/N` line in the overlay), never a single unconfirmed key/click; on
      confirm send `Command::MemoryMutate { op: Remove, … }`. On success the store re-emits
      `MemoryStatus`; refresh the open list.
- [ ] **Untrusted project root (FR-1).** The project group is simply absent when the store's
      `project_dir` is `None` — inherited from the engine, no TUI-side trust check. Show only
      what exists; absence is the correct quiet signal (§4.9).

## 4. Skills inspector overlay  *(Design §4.9; FR-7, §8.6)*

- [ ] **Palette + slash command.** Add `AppCommand::Skills` + `COMMANDS` entry `"skills" —
      "List available skills and inspect a skill's instructions"` + `run_command` arm →
      `open_skills_inspector()` (same registration pattern as group 3).
- [ ] **List.** `open_skills_inspector()` lists from the already-cached `app.skills`
      (`SkillsAvailable`, `app.rs:325`) — name, description, **origin** (user vs project;
      FR-7 trust) — as a selectable overlay (mirror the session picker `app.rs:1797`).
- [ ] **Read-only body on select.** Selecting a skill sends `Command::InspectSkill { name }`;
      on `UiEvent::SkillBody`, open the body **read-only** via `open_text_overlay`
      (`app.rs:1182`, `OverlayContent::Text`) with a title naming the skill + origin. This is
      the §4.9 promise: "what could this skill tell the model to do is always inspectable
      before it ever runs." **No edit** — a skill is an on-disk folder authored externally;
      editing is out of scope (note in log). Fetching the body for display never runs a
      bundled script (FR-7 — running a script is still an ordinary permission-gated command).

## 5. Degraded parity + rendering + strings  *(Design §7, §4.9)*

- [ ] **Overlay rendering.** Grouped memory list + skill list reuse the selectable-list
      render (`render.rs:184`, session/choice pattern); the skill body + memory-view reuse
      the `Text` wrap+scroll render. Scope/origin headers are dimmed chrome, not accent
      (§2); status/keys ("[Enter] view · [e] edit · [d] delete · [Esc] close") go through the
      strings module, not inline literals.
- [ ] **Degraded / plain mode (§7).** Overlays are a **rich-TUI** affordance; the plain
      frontend (`line.rs`) is append-only with no overlays. **Decide (notes log):** in plain
      mode, `/memory` and `/skills` print the list inline (append-only, read-only text), and
      skill-body / memory-view print inline too; **edit still works via `$EDITOR`** (it
      suspends the TUI regardless of mode) but **delete is confirmed inline**. Lean to:
      degraded gives full *inspection* inline (parity with §4.9's "inspectable") and keeps
      edit/delete available via the same commands, since FR-6's inspect/edit/delete is a
      requirement, not a rich-only nicety. Do not make inspection rich-only.
- [ ] All new strings centralized (Tech Spec §9 strings rule); ASCII-safe in degraded mode
      (no color-only meaning, §7).

## 6. Tests + exit criterion  *(Tech Spec §14.7 offline, deterministic)*

- [ ] **Engine round-trips** (`tests/engine_loop.rs`, beside :3834/:4034):
      - `Command::MemoryList` → `UiEvent::MemoryEntries` with correct scope grouping.
      - `Command::MemoryMutate{Update}` edits the body and re-emits `MemoryStatus`; the
        pinned one-line index reflects the change; body stays off standing context.
      - `Command::MemoryMutate{Remove}` deletes and re-emits updated counts.
      - `Command::InspectSkill` → `UiEvent::SkillBody` returns the body; assert the body is
        **still not** in the system prompt (progressive disclosure preserved, cf. :4121).
      - `adopt_session` re-emits `MemoryStatus` (regression for the current gap).
- [ ] **Store units** (`memory.rs:299`): `list_entries` returns summaries (no bodies) for
      each scope; empty project when `project_dir` is `None`.
- [ ] **TUI** (`app.rs` `#[cfg(test)]`): `/memory` and `/skills` open their overlays; memory
      list groups by scope with origin; selecting a memory entry shows body + offers
      edit/delete; **delete requires confirmation** (a single key does not delete);
      selecting a skill issues `InspectSkill` and renders the returned body read-only.
- [ ] **Trust (FR-1)** (`engine_loop.rs` or `app.rs`): with an untrusted project root, the
      inspector shows **no** project memory and **no** project skills.
- [ ] **Degraded parity** (`line.rs:672` neighborhood): plain-mode `/memory` `/skills`
      produce inspectable, ASCII-only output with no ANSI; inspection is not rich-only.
- [ ] **Exit criterion (Part 2 done when):** memory can be listed, viewed, edited (via
      `$EDITOR`, committed through the engine), and deleted (confirmed) from a running
      session; skills can be listed and their instruction bodies viewed read-only; both are
      reachable via `/memory` and `/skills` (§3.3) with project content hidden on an
      untrusted root (FR-1); progressive disclosure is preserved (bodies never pinned);
      offline suite green; workspace clippy-clean under §1; no new external dependency
      (HC-2). **On merge, Design §4.9 is fully realized and Phase 6's sidebar-click target
      (`PHASE6_TODO.md` group 4) becomes wireable.**

---

## Decisions & notes log

> Fill in as the work proceeds.

- **Scope: gap-closing, not new scope.** Every capability here is already in Design §4.9 /
  FR-6 / FR-7 / C-5 — this is implementation catching up to approved docs, so **no
  foundation-doc version bump**. If building it surfaces a genuine HOW-change, that flows
  back as a Spec bump (SFD G-11), never a shadow spec.
- **Split from Phase 6 (mouse) on purpose.** Orthogonal to pointer support and independently
  mergeable. The only linkage: Phase 6's deferred sidebar-click target becomes wireable once
  this lands (both branch off `version0.4`; order of merge doesn't matter).
- **Mutations go through the engine, never direct TUI writes (FR-6).** Memory is
  harness-owned/schema-constrained; the inspector sends `Command::MemoryMutate` and the
  engine performs the write via the shared `execute_memory_op` path — same category as the
  transcript and trust store. _Extract the shared path from `on_memory_op` so tool and
  inspector cannot diverge._
- **Listing: store method + engine command, not a model-facing `MemoryOp::List`.** Listing
  is a user/TUI action; keep the model's tool enum unchanged. _Confirm during group 1._
- **Overlay variant for the memory list.** Leaning to a dedicated
  `OverlayContent::MemoryEntries` over reusing `Choices`, because Enter has two follow-on
  actions (edit / delete) plus a confirm step, unlike the single-apply pickers. _Decide in
  group 3._
- **Skill inspector is read-only.** Skills are externally-authored on-disk folders; the
  inspector shows the body to judge trust before invocation (§4.9/FR-7) but does not edit
  them. Fetching the body for display is not script execution (FR-7). _If skill editing is
  ever wanted, it is a separate task._
- **Delete is confirmed.** Destructive; requires an explicit confirm (never a lone
  key/click), consistent with the "mouse never weakens a decision" spirit (§3.4) and the
  general care around irreversible actions.
- **Degraded mode still inspects (FR-6 is a requirement, not a nicety).** Plain mode prints
  the lists/bodies inline (append-only) and keeps edit (`$EDITOR`) and confirmed delete;
  only the *overlay presentation* is rich-only. _Confirm the plain-frontend surface in
  group 5._
- **Bonus fix carried here:** `adopt_session` re-emitting `MemoryStatus` (`engine.rs:1022`)
  — a pre-existing staleness gap, cheap to fix alongside the inspector.
- **Trust is inherited (FR-1).** Untrusted root ⇒ no `project_dir` / no project skills ⇒
  empty project section, no extra TUI logic. _Add a test so it cannot regress silently._
