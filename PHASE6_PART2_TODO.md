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
| 1. Engine/store data model: memory listing + skill-body fetch (`emberly-core`, `emberly-tools`) | [x] | `EntrySummary` + `MemoryStore::list_entries`; Recall/invoke reused as-is |
| 2. TUI→engine command + event plumbing: new `Command`/`UiEvent` variants; adopt_session fix (`emberly-core`) | [x] | idle-loop handlers; shared `execute_memory_op`; adopt_session `MemoryStatus` fix; 7 round-trip tests |
| 3. Memory inspector overlay: `/memory`, grouped list, view/edit/delete (`emberly-tui`) | [x] | `OverlayContent::MemoryEntries`; view/edit(`$EDITOR`→`Update`)/confirmed-delete; +`MemoryView`/`MemoryBody`; 9 app tests |
| 4. Skills inspector overlay: `/skills`, list, read-only body (`emberly-tui`) | [x] | `OverlayContent::SkillList` from cached catalog; Enter→`InspectSkill`→read-only body+resources; origin per row; 6 app tests |
| 5. Degraded parity + rendering + strings (`emberly-tui`) | [ ] | overlays are rich-only; define plain-mode `/memory` `/skills` behavior |
| 6. Tests (offline — §14.7) + exit criterion (`emberly-core`, `emberly-tui`) | [ ] | round-trips, trust-absence, delete-confirm, body-not-pinned preserved |

**Overall Phase 6 Part 2: Groups 1–4 done; Groups 5–6 remain (degraded parity + strings, and the offline test suite / exit criterion — engine round-trips and rich-TUI tests already landed alongside their groups).**

---

## 1. Engine/store data model — memory listing + skill-body fetch  *(FR-6, FR-7, §8.6)*

- [x] **Memory listing.** Added `MemoryStore::list_entries(scope) -> Vec<EntrySummary
      { name, description, type_, scope }>` (`core/src/memory.rs`), a **store method** (not a
      model-facing `MemoryOp::List` — confirmed decision below). Refactored the `build_index`
      scan into a shared `scan_entry_metas(dir)` helper; both `build_index` and
      `list_entries` use it (no duplicated dir-scan). `EntrySummary` lives in
      `core/src/memory.rs` beside `MemoryEntry`, derives Serialize/Deserialize/Clone/Eq (so
      it can ride a `UiEvent` in group 2) and **has no `body` field** — progressive
      disclosure is structural, not convention.
- [x] **Memory body read** for the edit/view step reuses the existing `MemoryOp::Recall`
      semantics (`memory.rs`) unchanged — read the body + origin on demand for one named
      entry. No new code (verified).
- [x] **Skill-body fetch** reuses `SkillCatalog::invoke(name)` (`core/src/skills.rs:130`)
      as-is — already loads the post-frontmatter body + resource paths under trust/precedence
      rules. No new discovery logic; the group-2 `InspectSkill` command calls it. Invoking a
      skill body **for display** runs no bundled script (§4.9 / FR-7).
- [x] Bodies stay **out of standing context** — `list_entries` reads metadata only
      (`scan_entry_metas` drops the body); `EntrySummary` carries no body. Bodies flow only
      via an explicit `Recall`/`InspectSkill` fetch (group 2). The `engine_loop.rs:4121`
      "body is NOT pinned" invariant is untouched.

## 2. TUI→engine command + event plumbing  *(C-5; the harness-owned-store rule, FR-6)*

- [x] **New `Command` variants** (`core/src/command.rs`), handled at the **idle loop**
      (`engine.rs:842`, alongside `ReloadConfig`/`SetEffort`):
      - `Command::MemoryList` → `emit_memory_entries()`.
      - `Command::MemoryMutate { op, scope, name, description?, type_?, body? }` → builds a
        `MemoryRequest` and calls the shared `execute_memory_op` (below); a `Rejected`
        outcome surfaces as a `Notice` so the user sees why. `op` is the full `MemoryOp`
        enum (docs restrict inspector use to `Update`/`Remove`); reused the tool's enum
        rather than a new one.
      - `Command::InspectSkill { name }` → `inspect_skill()` (catalog `invoke`).
      - The three mid-turn command matches (`engine.rs:1389/1585/1667`) already have
        catch-all arms, so these idle-only commands are safely ignored mid-turn (gated by
        the frontend like `SetEffort`).
- [x] **Extracted `execute_memory_op(req) -> MemoryOutcome`** from `on_memory_op` — the
      single validated write path (store `execute` → `refresh_memory_indexes` → emit
      `MemoryStatus` → soft-cap warn). Both the `memory` tool (`on_memory_op` now just calls
      it + replies over the oneshot) and the inspector's `MemoryMutate` share it, so they
      **cannot diverge** (FR-6). The store's `slug` guard re-validates the name.
- [x] **New response `UiEvent`s** (`core/src/event.rs`): `MemoryEntries { user:
      Vec<EntrySummary>, project: Vec<EntrySummary> }` and `SkillBody { name, origin, body,
      resources }`. An unknown/disabled/untrusted-absent skill emits a `Notice` instead of a
      `SkillBody`, so the inspector never opens an empty overlay.
- [x] **Fixed `adopt_session`** (`engine.rs`) to re-emit `MemoryStatus` after
      `refresh_memory_indexes()` — matching the `SkillsAvailable` re-emit just below — so the
      inspector/sidebar are correct after `/resume` and `/new`.
- [x] Channels unchanged (still `FrontendPorts` = `commands_tx` + `events_rx`); no
      channel-shape change. TUI wiring is group 3.

## 3. Memory inspector overlay  *(Design §4.9, §4.6; FR-6, C-5, C-3, FR-1)*

- [x] **Palette + slash command.** Added `AppCommand::Memory` + `COMMANDS` entry
      `"memory" — "Inspect, edit, and delete stored memory"` + `run_command` arm →
      `open_memory_inspector()`. Auto-appears in `help_text` (iterates `COMMANDS`) and the
      palette — "reachable three ways" (§3.3), asserted in a test.
- [x] **Open + list.** `open_memory_inspector()` returns `Action::Command(MemoryList)`; on
      `UiEvent::MemoryEntries` the overlay opens (or refreshes in place). Entries are
      **grouped by scope** (user then project) with a dimmed scope **header per group**
      (origin is how the user reads trust) and `name — description` rows. **Decided:** a
      dedicated `OverlayContent::MemoryEntries { user, project, selected, confirm_delete }`
      (not a `Choices` reuse) — Enter/`e`/`d` have distinct actions plus a confirm step.
- [x] **View / edit an entry (§4.6 full-edit).** Added `Command::MemoryView` +
      `UiEvent::MemoryBody` (the per-entry body fetch — conceptually group 2, landed here).
      Enter → view (read-only `Text` overlay titled `name · scope`); `e` → edit. Edit fetches
      the body then hands off to `$EDITOR` in the frontend loop (`run_memory_edit`), and on
      save commits via `Command::MemoryMutate { op: Update, … }` — **not** `ReloadConfig`.
      Edits land in the entry's existing scope (C-1); **`description`/`type_` ride through
      unchanged** so a body edit never erases metadata (see notes — body-only edit for v1).
- [x] **Delete an entry — confirmed.** `d` arms a `y/N` confirm rendered in the overlay hint;
      **a lone key never deletes** (asserted); only `y` sends `Command::MemoryMutate {
      op: Remove, … }`, any other key cancels. After any mutate the frontend re-issues
      `MemoryList` (the engine re-emits only `MemoryStatus`), refreshing the open list.
- [x] **Untrusted project root (FR-1).** The project group is simply empty when the store's
      `project_dir` is `None` — `list_entries` returns empty for it; no TUI-side trust check.
      The renderer omits an empty group's header, so an untrusted root shows only user memory.

## 4. Skills inspector overlay  *(Design §4.9; FR-7, §8.6)*

- [x] **Palette + slash command.** Added `AppCommand::Skills` + `COMMANDS` entry `"skills" —
      "List available skills and inspect a skill's instructions"` + `run_command` arm →
      `open_skills_inspector()`. Auto-listed in `/help`/palette (reachable three ways, §3.3).
- [x] **List.** `open_skills_inspector()` pushes `OverlayContent::SkillList { skills, selected }`
      from the already-cached `app.skills` (**no engine round-trip** — the catalog is
      standing context). Rows show `name — description` with a dimmed `(origin)` suffix
      (user vs project; FR-7 trust). Empty catalog → an empty overlay with a calm message.
- [x] **Read-only body on select.** Enter sends `Command::InspectSkill { name }`; on
      `UiEvent::SkillBody` the body opens **read-only** via `open_text_overlay` titled
      `name · origin`, with any bundled resource paths appended under a "bundled files:"
      header. **No edit/delete** — `e`/`d` are inert (asserted); skills are externally-authored
      folders. Fetching the body for display runs no bundled script (FR-7).

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

- [x] **Engine round-trips** (`tests/engine_loop.rs`, "inspector round-trips" section) —
      done early alongside group 2 (7 tests, all green):
      - `memory_list_command_groups_entries_by_scope` — `MemoryList` → `MemoryEntries` with
        correct scope grouping + case-insensitive sort + origin tag.
      - `memory_list_project_empty_when_untrusted` — project empty when `project_dir` None.
      - `memory_mutate_update_edits_body_and_reemits_status` — edits body on disk, re-emits
        `MemoryStatus`, pinned index shows new description, **body stays off** the system
        prompt.
      - `memory_mutate_remove_deletes_and_reemits_counts` — deletes + re-emits count 0.
      - `inspect_skill_returns_body_and_stays_off_context` — `InspectSkill` → `SkillBody`
        with origin + resources; body **not** pinned in the system prompt.
      - `inspect_skill_unknown_emits_notice_not_body` — unknown skill → `Notice`, no
        `SkillBody`.
      - `adopt_session_reemits_memory_status` — regression for the `/resume` `/new` gap.
- [ ] **Store units** (`memory.rs:299`): `list_entries` returns summaries (no bodies) for
      each scope; empty project when `project_dir` is `None`.
- [x] **TUI** (`app.rs` `#[cfg(test)]`): memory side (9 tests) — `/memory` reachable three
      ways; grouped-by-scope overlay with origin; Enter issues `MemoryView`; view opens a
      read-only body overlay; edit stages a pending `$EDITOR` handoff carrying metadata;
      **delete requires confirmation** (a lone key does not delete); re-list refreshes in
      place. Skills side (6 tests) — `/skills` reachable three ways; overlay from the cached
      catalog with per-row origin; Enter issues `InspectSkill`; `SkillBody` opens read-only
      with resources; `e`/`d` inert (no edit/delete); empty catalog handled.
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
  is a user/TUI action; keep the model's tool enum unchanged. _CONFIRMED (group 1):_ added
  `MemoryStore::list_entries`; `MemoryOp` stays `Write|Update|Remove|Recall`. `EntrySummary`
  placed in `core/src/memory.rs` (not `emberly-tools`) — it is a TUI/engine listing type the
  model never sees, and `event.rs` (core) can carry it directly on the group-2 `UiEvent`.
- **Group 1 verification (local toolchain caveat).** `cargo test -p emberly-core` green (47
  lib unit incl. 2 new `list_entries` tests + 92 `engine_loop`). CI runs **stable**
  (`ci.yml` → `dtolnay/rust-toolchain@stable`), not the pinned 1.96; the library
  `deny(unwrap_used)` gate is on **non-test** code (`cargo clippy -p emberly-core --lib`
  clean — no new warnings), and local 1.96 `--all-targets`/`fmt` noise on untouched files is
  toolchain skew, not a regression.
- **Overlay variant for the memory list.** _CONFIRMED (group 3):_ dedicated
  `OverlayContent::MemoryEntries { user, project, selected, confirm_delete }`. `selected`
  indexes the flattened user-then-project list; the renderer inserts dimmed scope headers.
- **Group 3 decisions:**
  - **`MemoryView`/`MemoryBody` added in group 3, not group 2.** The per-entry body fetch
    the inspector's view/edit needs was implicit in the group-1/2 spec ("reuse `Recall`") but
    had no command/event. Added `Command::MemoryView { scope, name }` →
    `UiEvent::MemoryBody { scope, name, body }` (engine `emit_memory_body` runs a read-only
    store `Recall`). Mirrors `InspectSkill`/`SkillBody`. Kept off `MemoryOp` (user/TUI read).
  - **Body-only edit (metadata preserved).** The `$EDITOR` handoff edits the markdown body;
    `description`/`type_`/`name`/`scope` ride through the resulting `MemoryMutate{Update}`
    unchanged (the store's `write_entry` would otherwise reset description to empty). Editing
    metadata in the overlay is out of scope for v1 — a later nicety, not a requirement.
  - **Async edit handoff via a staged field.** `apply_event(MemoryBody)` can't return an
    `Action`, so an edit-intent body reply sets `pending_memory_edit`; the frontend loop
    drains it (`take_pending_memory_edit`) right after `apply_event` and runs the `$EDITOR`
    handoff off the input path. View-intent replies just push a read-only `Text` overlay.
  - **Refresh-after-mutate lives in the loop.** The engine re-emits only `MemoryStatus` after
    a mutate (it doesn't know the inspector is open); the frontend loop re-issues
    `MemoryList` after any `MemoryMutate` (delete and edit-commit), which refreshes the open
    overlay in place via `apply_memory_entries`.
  - **Confirmed delete.** `d` arms `confirm_delete`; only `y`/`Y` deletes, every other key
    cancels (deny-by-default for a destructive action). A lone key never deletes (tested).
- **Skill inspector is read-only.** Skills are externally-authored on-disk folders; the
  inspector shows the body to judge trust before invocation (§4.9/FR-7) but does not edit
  them. Fetching the body for display is not script execution (FR-7). _If skill editing is
  ever wanted, it is a separate task._ _CONFIRMED (group 4): `e`/`d` are inert on the skills
  overlay (asserted); Enter is the only action → `InspectSkill`._
- **Group 4 decisions:**
  - **Dedicated `OverlayContent::SkillList { skills, selected }`** (parallels `MemoryEntries`
    but simpler — one action). The catalog is already standing context (`app.skills` from
    `SkillsAvailable`), so opening the list needs **no engine round-trip**; only the body is
    fetched on demand (`InspectSkill` → `SkillBody`), preserving §8.6 progressive disclosure.
  - **Origin per row.** Skills mix user/project in one flat list (precedence already
    resolved at discovery), so origin is a dimmed `(origin)` suffix per row, not a group
    header (contrast memory, where scope is the grouping).
  - **Body view appends bundled resource paths** under a "bundled files:" header — FR-7:
    what the skill bundles is part of "what it could tell the model to do". Read-only; the
    paths are not executed (running a bundled script remains an ordinary permission-gated
    `bash` call).
- **Delete is confirmed.** Destructive; requires an explicit confirm (never a lone
  key/click), consistent with the "mouse never weakens a decision" spirit (§3.4) and the
  general care around irreversible actions.
- **Degraded mode still inspects (FR-6 is a requirement, not a nicety).** Plain mode prints
  the lists/bodies inline (append-only) and keeps edit (`$EDITOR`) and confirmed delete;
  only the *overlay presentation* is rich-only. _Confirm the plain-frontend surface in
  group 5._
- **Bonus fix carried here:** `adopt_session` re-emitting `MemoryStatus` (`engine.rs:1022`)
  — a pre-existing staleness gap, cheap to fix alongside the inspector. _DONE (group 2)._
- **Group 2 decisions (confirmed):** (a) `execute_memory_op` extracted from `on_memory_op`
  as the single write path — tool + inspector share it (FR-6). (b) `MemoryMutate` reuses the
  tool's `MemoryOp` enum rather than a new inspector-only enum; docs restrict inspector use
  to `Update`/`Remove`. (c) A rejected mutate surfaces as a `Notice`; a missing/untrusted
  skill on `InspectSkill` also surfaces as a `Notice` (never an empty overlay). (d) After a
  mutate the engine only re-emits `MemoryStatus`; the TUI re-issues `MemoryList` to refresh
  the open list (group 3) — keeps the engine from guessing the overlay is open. (e) `clippy`
  note: `deny(unwrap_used)` fires only under `cargo clippy`, not `cargo test`/`build`, and
  CI's stable clippy exempts test modules — new tests follow existing `.unwrap()` style.
- **Trust is inherited (FR-1).** Untrusted root ⇒ no `project_dir` / no project skills ⇒
  empty project section, no extra TUI logic. _Add a test so it cannot regress silently._
