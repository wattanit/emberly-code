# Phase 2 — Session scratch space (FR-8, T-17) — TODO & Progress

**Milestone:** M11 phase 2 (Tech Spec §15) — the 0.4.3 feature set.
**Satisfies:** FR-8, T-17; Tech Spec §5.2, §8.3, §10; Design §4.12, §8.8.
Pinned to **Req v0.10 / Design v0.10 / Spec v0.11** (all `approved`).

Written as-built (see `IMPLEMENTATION_PLAN.md`'s "How this plan was
written") — unlike Phase 1, this one *was* scoped and approved upstream
through the SFD process before any code, the normal prospective order.

## Group 1 — Foundation docs (drafted and approved before any code)

- [x] Requirements v0.9 → v0.10: new **FR-8** (§8.9 — lifecycle, no
      auto-cleanup, the CLI-reclaim requirement) and **T-17** (§5 —
      `scratch_write` tool, content-fields-never-a-path pattern, not
      permission-gated), added to the 0.4.3 scope list, a new Open
      Questions entry on the clean command's confirmation behavior.
- [x] Design Guideline v0.9 → v0.10: §4.12 (scratch-write as a quiet dim
      line, same family as memory/skills, explicitly marked illustrative
      per the SFD standard's G-3) and §8.8 (`emberly clean`'s voice,
      mirroring `init` — prints what it found and removed, no ceremony).
- [x] Tech Spec v0.10 → v0.11: §8.3 (layout, the tool's schema, the
      `ScratchStore`/`ScratchGate` design), §5.2 (built-in tool table row),
      §10 (the `emberly clean` CLI verb), M11, two new Open Items (no
      non-interactive flag yet, no size/retention cap yet). Corrected a
      wrong assumption before it shipped: glob/grep don't skip "hidden
      directories" (`.hidden(false)` in the actual `WalkBuilder` config) —
      they honor `.gitignore`, which is the real reason scratch content
      won't surface by default.
- [x] All three companion-version pins refreshed in lockstep. (commit
      `5a49dca`)

## Group 2 — The tool (`emberly-tools`)

- [x] `scratch.rs`: `ScratchRequest{name, content}`, `ScratchOutcome`,
      `ScratchGate` trait, `DropScratchGate` no-op default,
      `validate_name` — same security checks as memory's `slug` (empty,
      path separators, `..`, leading `~`) but **not** transforming the
      name, since a scratch file's extension and case must survive
      verbatim (`analysis.py`, not `analysis-py`).
- [x] `builtin/scratch.rs`: `ScratchWriteTool` — `Tool` impl, JSON schema
      (`{name, content}`, both required), `describe()` (`"scratch ·
      {name}"`), `execute()` validates early then calls
      `ctx.scratch_write(...)`.
- [x] `ctx.rs`: `scratch: Arc<dyn ScratchGate>` field,
      `with_scratch_gate` builder, `scratch_write()` passthrough —
      mirroring the `memory`/`skill` gate pattern exactly.
- [x] Registered in `default_registry()`; exported from `lib.rs`.
- [x] Unit tests: name validation preserves case/extension, rejects path
      separators / `..` / home paths / empty.

## Group 3 — The store (`emberly-core`)

- [x] `scratch.rs`: `ScratchStore{dir}` + `execute()` (re-validates the
      name, asserts the resolved path stays under `dir` — belt-and-braces
      over the tool's own guard, mirroring `MemoryStore`), lazy
      `create_dir_all` on first write only. `impl ScratchGate for
      ScratchStore` directly — **no channel/oneshot round trip** to the
      engine loop, unlike memory/task-list/skill: a scratch write has no
      side effect on any other engine-owned state (no cached index, no
      sidebar status to refresh), so the gate can act synchronously
      instead of asking the engine loop to act on its behalf. This was a
      deliberate simplification decided during implementation, not
      specified in the Tech Spec draft (which only committed to the
      schema-constrained *validation* pattern, not the plumbing mechanism).
- [x] `engine.rs`: `scratch_dir_for(project_root, session_id)` helper
      (`.agents/scratch/<session-id>/` — no new `EngineConfig` field
      needed, unlike memory's two scope dirs, since scratch has only ever
      one location); `scratch_store: Arc<ScratchStore>` field constructed
      in `Engine::new` and rebuilt in `adopt_session` (covers both `/new`
      and `/resume`, the two places `session_id` changes); wired into
      `make_ctx()`.
- [x] `.gitignore`: `.agents/scratch/` added.
- [x] Unit tests: lazy directory creation, replace-on-rewrite, invalid
      name rejected without touching the filesystem.
- [x] Integration tests (`engine_loop.rs`): file lands under the right
      session directory with the right content; never raises a
      `PermissionRequest`; a path-escape name fails as structured data
      (HC-6) without creating the scratch tree at all.

## Group 4 — `emberly clean` (the `emberly` binary)

- [x] `clean.rs`: `clean(scratch_root, session_id)` — no id scans every
      immediate subdirectory of `.agents/scratch/`; an id targets just
      that one. Reports total size (`format_size`: B/KiB/MiB), asks a
      plain `y`/`n` (`confirm()` — same safe-default-decline-when-
      non-interactive shape as the trust prompt), deletes on `y`.
      `dir_size` is defensively recursive even though scratch files are
      always flat by construction (the tool rejects any name with a path
      separator).
- [x] `main.rs`: `Cli::Clean(Option<String>)`, parsed with the same
      optional-trailing-id shape as `resume [id]`; dispatched before
      `Cli::Run`.
- [x] Unit tests: `scan_all` finds and sorts every session directory
      (and is empty-not-erroring on a missing root), `dir_size` sums
      correctly, `format_size` picks the right unit, CLI parse accepts
      both `clean` and `clean <id>`.
- [x] Verified live against a real temp project (not just unit tests):
      empty scratch root → "nothing to clean"; populated + no arg →
      correct total across sessions; populated + a specific id → scoped
      correctly; non-interactive stdin (even piped `"y"`) → declines and
      preserves the files, matching the trust prompt's own precedent.

## Group 5 — TUI

- [x] No new rendering code needed — confirmed the existing
      `ConvItem::Tool` header line in `render.rs` renders **any**
      `ToolOutcome.summary` string generically; `"scratched · {name} ·
      {bytes}B written"` flows through it exactly like memory's
      `"remembered · ..."` already does.

**Done when:** ✅ all five groups above shipped; `cargo build/test/
clippy(-D warnings)/fmt` clean at each commit (verified 2026-07-30).
