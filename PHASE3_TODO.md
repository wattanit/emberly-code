# Phase 3 — Persistent memory (FR-6, T-13) — TODO & Progress

**Milestone:** M8 phase 3 (Tech Spec §15) — the 0.4 capability-parity set. Third
phase; the first to add **harness-managed filesystem persistence** for the model
(like the transcript and trust store, not an agent write). It establishes the
*progressive-disclosure store* machinery — TOML frontmatter, a pinned index,
trust-gating of the project scope, a sidebar inspector — that **Phase 4 (skills)
reuses**.
**Satisfies:** FR-6, T-13; Tech Spec §8.1, §5.2, §7, §6.7, §3.1; Design §4.9, §3.1,
§8.4; HC-4 (unwidened), FR-1, HC-2. Pinned to **Req v0.7 / Design v0.7 / Spec v0.8**
(all `approved`).
**Goal:** Durable facts that survive across sessions — an index auto-loaded into
pinned context each session, entries written by the model through a
**schema-constrained** tool that never widens HC-4 (the model supplies content,
never a path; the harness performs the write into its own store). Two scopes:
user-global (always loaded) and project (loaded **only under a trusted root**, FR-1).
Progressive disclosure: only the one-line-per-entry index is standing context; entry
**bodies** load on demand via the tool's `recall` op.

**Independent of Phase 2.** Per the plan's dependency summary, Phases 1/2/5 are
mutually independent and 3→4 form their own chain; Phase 3 builds on the shipped
product (M1–M7) and Phase 1's task-list machinery only, touching none of Phase 2's
image code. It can be implemented before or after Phase 2. (Both write into the same
crates but different modules; a merge conflict is unlikely and, if any, confined to
the `ConfigFile`/`default_registry`/`UiEvent` addition sites.)

**Depends on:** the shipped product (M1–M7). **Phase 1's task-list (`todo`) is the
closest end-to-end analog** — gate trait → channel+oneshot → select-loop arm →
engine state → `UiEvent` → transcript → `effective_system()` pin → sidebar section.
Memory differs by adding filesystem I/O (mirroring the transcript/trust/permissions
writers) and by pinning an *index string* rather than a full list. The seams:
- Gate to mirror — `TaskListGate` (`crates/emberly-tools/src/task_list.rs:41`),
  `DropTaskListGate` no-op (:54); the data-returning `RecallGate`
  (`.../recall.rs:30`) for the `recall` op shape. `ToolCtx` wiring —
  `with_task_list_gate`/`set_task_list` (`.../ctx.rs:100`, :145); fields + no-op
  defaults in `ToolCtx::new` (`.../ctx.rs:56`, :78).
- Gate impl to mirror — `TaskListGateImpl`/`TaskListAsk` (channel+oneshot,
  `crates/emberly-core/src/gate.rs:129`); `RecallGateImpl` (:101).
- Engine — gate field (`engine.rs:466`), state field (`task_list` :536), channel
  create + return tuple (`Engine::new` :554/:560, `run` signature :657), select-loop
  arm (:1462, the `task_rx.recv() => on_task_list_set` line :1466), handler
  `on_task_list_set` (:1628), `make_ctx` gate install (:2212), session reset
  (`adopt_session` clears `task_list` ~:894).
- **Pinned context** — `effective_system()` (`engine.rs:2186`) is where the task list
  is appended; the memory index(es) append the same way. `render_task_list_block`
  (:2510) is the block-renderer precedent. `windowed_messages()`/`pinned_count()`
  (:2098/:2084) need **no** change (index rides the system prompt, not the message
  pins).
- Harness file I/O to mirror — trust store writer `save_store_at` (create parent,
  `toml::to_string`, write, 0600 — `crates/emberly/src/trust.rs:187`), permissions
  append `append_rule_block` (`engine.rs:2596`), `FileTranscript::create` dir-creation
  (`crates/emberly-core/src/transcript.rs:305`).
- Trust gate (pre-engine) — `trust::gate(&canonical_root)` (`crates/emberly/src/main.rs:344`);
  `Gate::Declined => return Ok(())` exits before any session. `global_trust_dirs`
  (`config.rs:936`), `TrustConfig` (:65).
- XDG config dir — `config_dir()` (`config.rs:909`), `global_trust_path` (:929) as the
  `~/.config/emberly/<x>` precedent → memory dir = `config_dir()?.join("memory")`.
- Project `.agents/` — computed in the binary (`main.rs:338`,
  `sessions_dir = root/.agents/sessions`) → project memory = `root/.agents/memory`.
- Existing `toml` parse sites — `config.rs:262` (`toml::from_str`), `trust.rs:179/192`.
  Project-instruction loading (whole-file, no frontmatter) — `load_project_instructions`
  (`config.rs:792`) folded into the system prompt (:620); this is the C-1 precedent
  memory sits *beside* (agent-authored, not user-authored).
- `UiEvent` — enum (`event.rs:20`), `TaskListUpdated` precedent (:177); emit path
  `Engine::emit` (`engine.rs:2256`); TUI `apply_event` `TaskListUpdated` arm
  (`crates/emberly-tui/src/app.rs:665`), app state field `tasks` (:314), unknown-event
  `_ => {}` (:669).
- TUI sidebar — `render_sidebar()` Tasks section (`crates/emberly-tui/src/render.rs:688`,
  after Modified-files :662); inline `ConvItem::TaskList` (:504); plain frontend
  `TaskListUpdated` (`crates/emberly-tui/src/line.rs:171`).
- Config — `ConfigFile` (`config.rs:19`), `ContextConfigFile` (:83), merge
  (context :312), resolve (:667), provenance `record` (:757) + context block (:523),
  `Resolved` (:334), engine wiring (`main.rs:524`).
- Registry — `default_registry()` (`crates/emberly-tools/src/builtin/mod.rs:30`);
  also `main.rs:516`, `engine_loop.rs:55`.
- Tests — `FakeProvider` (`crates/emberly-providers/src/fake.rs:122`, `last_request`
  :166); engine round-trip template `todo_round_trip_emits_event_and_transcript`
  (`crates/emberly-core/tests/engine_loop.rs:3419`), pin assertion via `last_request()`
  (:3521+); tool unit tests with mock gates (`crates/emberly-tools/tests/builtin_tools.rs:499`).

**Key structural facts (from the codebase):**
1. **No boolean marks a tool "not permission-gated" — it just never calls
   `ctx.authorize`.** `todo`/`recall`/`ask_user` reach the engine through a `ctx` gate
   method and never authorize. `memory` is the same: harness-managed persistence that
   does not widen HC-4 (FR-6 honesty clause). Project-scope writes land inside the root
   and follow the §6.2 no-prompt read/write rule. **Mirror `TaskListGate`, never
   `edit`/`write`.**
2. **The `memory` `recall` OP is not the `recall` TOOL (T-10).** T-10's `recall` tool
   (already shipped, `builtin/recall.rs`) returns dropped *conversation turns*. Memory's
   `recall` is an *op value* of the `memory` tool (`op: recall`) returning an *entry
   body*. Same word, different mechanism — do not conflate the two gates.
3. **Trust-gating is realized by the binary passing the project memory dir as
   `Option`, not by a runtime re-check.** `trust::gate` runs pre-engine and *exits* on
   decline (`main.rs:344`), so a live session already implies a trusted root. The engine
   never re-checks trust. Project memory is gated by the binary passing
   `project_memory_dir: Some(..)` **only when trusted** and `None` otherwise; user-global
   is always `Some`. This keeps the honesty clause structural and — crucially — testable:
   construct the engine with `project_memory_dir: None` to assert project memory is absent
   (Design §4.9 "silently absent, not half-loaded").
4. **The engine owns the live store, not the binary.** Writes happen mid-session and
   must regenerate the index, refresh the pinned context, and re-emit `MemoryStatus`, so
   the engine holds the store (dirs + cached index text + counts). The binary only
   supplies the two dir paths (gated) and the config; loading and all mutation are engine-
   side, mirroring how the engine owns the transcript writer.
5. **Frontmatter split + slugify are greenfield** — no existing splitter or slugify in
   the workspace (only `transcript.rs:392` `sanitize`, a filename cleaner). Both are
   first-party, pure-Rust (HC-2), reusing the existing `toml` crate for the frontmatter
   table — deliberately no YAML dependency (Tech Spec §8.1).

**Legend:** `[ ]` todo · `[~]` in progress · `[x]` done · `[!]` blocked

---

## Progress summary

| Group | Status | Notes |
|---|---|---|
| 1. Shared types + name→slug/validate + `MemoryGate` trait + `memory` tool skeleton (`emberly-tools`) | [ ] | mirror `TaskListGate`; reject `..`/absolute/separators in `name` (HC-4 boundary, testable) |
| 2. Memory store: dirs, frontmatter split, entry read/write, `MEMORY.md` index gen (`emberly-core`) | [ ] | new `memory.rs`; `toml` frontmatter, no YAML (HC-2); index = one line per entry |
| 3. Engine wiring: gate impl + select-loop arm + `on_memory_op` + load-at-start + pin index + reset (`emberly-core`) | [ ] | FS write engine-side; refresh index; emit `MemoryStatus`; append index in `effective_system()` |
| 4. Trust-gated project scope + store dirs from the binary (`emberly`) | [ ] | user-global always; project dir `Some` only under trusted root; XDG `config_dir()/memory` |
| 5. Config: `[memory] enabled + max_index_entries` + resolve/provenance + `EngineConfig` (`emberly`/`emberly-core`) | [ ] | mirror `[context]`; `max_index_entries` soft warn (§16 open item) |
| 6. Events + TUI: `MemoryStatus` UiEvent + sidebar Memory section + origin on tool line + degraded (`emberly-core`/`emberly-tui`) | [ ] | counts (not bodies) in sidebar; `user`/`project` origin per Design §4.9 |
| 7. Tests (offline, deterministic — §14.7) + exit criterion | [ ] | write/recall round-trip; path-escape rejected; index pinned; project absent when untrusted |

**Overall Phase 3: NOT STARTED.**

---

## 1. Shared types + name validation/slug + `MemoryGate` + `memory` tool skeleton  *(T-13; Tech Spec §5.2)*

Shared types and the schema-constrained tool live in `emberly-tools` (as `TaskItem`
does); the store I/O and gate impl land in `emberly-core` (group 2/3) — the established
split.

- [ ] New `crates/emberly-tools/src/memory.rs` (beside `task_list.rs`): define
      `enum MemoryScope { User, Project }` and `enum MemoryOp { Write, Update, Remove,
      Recall }` (`#[serde(rename_all = "snake_case")]`), a `MemoryRequest { op, scope,
      name, description: Option<String>, type_: Option<String>, body: Option<String> }`
      (`#[serde(rename = "type")]` for the reserved word), and a `MemoryOutcome`
      returned by the gate: `Written { user: usize, project: usize }` /
      `Recalled { body: Option<String>, origin: MemoryScope }` /
      `Rejected { reason: String }` (the count refresh feeds `MemoryStatus`). Re-export
      from `emberly-tools/src/lib.rs`.
- [ ] **Name → slug + validation (the HC-4 boundary, made testable).** A first-party
      `fn slug(name: &str) -> Result<String, MemoryNameError>` in `memory.rs`: reject an
      empty name, any path separator (`/`, `\\`), `..`, and absolute-path markers
      *before* slugging; lowercase, replace runs of non-`[a-z0-9]` with `-`, trim. Pure,
      no filesystem — a unit test asserts `../etc/passwd`, `/abs`, `a/b`, `a\\b` are all
      rejected (Tech Spec §8.1: "the model cannot escape it"). The engine re-applies this
      as the authoritative guard before any write (defense in depth, group 3).
- [ ] Add a `MemoryGate` trait + `ToolCtx` wiring (`ctx.rs`): field + no-op default
      (`DropMemoryGate` returning `MemoryOutcome::Rejected`/an error, mirroring
      `DropTaskListGate` :54), `with_memory_gate` builder (:100) and a
      `memory_op(&self, req: MemoryRequest) -> Result<MemoryOutcome, MemoryError>`
      accessor (:145). Fail-closed to `Err` the tool maps to `ToolOutcome::failure`
      (HC-6), never a panic.
- [ ] New `crates/emberly-tools/src/builtin/memory.rs` — `struct MemoryTool`
      implementing `Tool`, shaped like `builtin/task_list.rs`: module doc stating
      "harness-managed persistence (like the transcript/trust store); does not widen HC-4;
      not permission-gated"; `spec()` name `"memory"`, description (C-4, overridable per
      C-1), JSON schema for `{op, scope, name, description?, type?, body?}` with `op`/
      `scope` enums and `name` required. `execute()` validates the name via `slug(..)`
      (early reject → `ToolOutcome::failure`), calls `ctx.memory_op(req)` and **never**
      `ctx.authorize`; maps `MemoryOutcome` to a `ToolOutcome` whose summary carries the
      **scope origin** for the Design §4.9 line (`remembered · project · "<desc>"`,
      `recalled · user`). A `Recalled { body: None }` (missing entry) is a clean HC-6
      failure, not a crash.
- [ ] Register in `builtin/mod.rs`: `mod memory;` + `pub use` and
      `registry.register(Arc::new(MemoryTool::new()))` in `default_registry()` (:30,
      beside `TaskListTool` :40). Advertised to the model via `registry.specs()` — no other
      change.

## 2. Memory store: layout, frontmatter, entries, index  *(T-13; Tech Spec §8.1, HC-2)*

New `crates/emberly-core/src/memory.rs` — the store logic the engine calls. Pure over an
injected directory path so it is unit-testable against a temp dir.

- [ ] `struct MemoryStore { user_dir: PathBuf, project_dir: Option<PathBuf> }`
      (`project_dir` is `None` on an untrusted root — structural fact 3). Entry files are
      `<slug>.md`; the index is `<dir>/MEMORY.md`.
- [ ] **Frontmatter split (greenfield, `toml` crate — HC-2, no YAML).**
      `fn split_frontmatter(text: &str) -> Option<(&str, &str)>` recognizing a leading
      `+++\n … \n+++\n` (TOML fence) — TOML's conventional delimiter, and unambiguous vs.
      YAML's `---` which also delimits markdown thematic breaks. Parse the fence body with
      `toml::from_str` into `struct EntryMeta { name: String, description: String,
      #[serde(rename = "type")] type_: Option<String> }`. Write entries by serializing
      `EntryMeta` with `toml::to_string` between `+++` fences, then the markdown body.
      **Record the fence choice (`+++` vs `---`) in the notes log** — if the Spec §8.1
      wording should pin the delimiter, that is G-24 feedback, not an edit here.
- [ ] Ops over a scope dir (create dir on first write, mirror `save_store_at`
      `create_dir_all` `trust.rs:189`): `write`/`update` (write `<slug>.md`; update is
      write-if-exists semantics — decide overwrite-vs-error in the log), `remove` (delete
      `<slug>.md`; missing = clean no-op or Rejected, decide in log), `recall` (read and
      return the body of `<slug>.md`, or `None`).
- [ ] `regenerate_index(dir)`: after any mutation, rewrite `MEMORY.md` as one line per
      entry `- <name> — <description>` sorted stably, by scanning `<dir>/*.md` (skip
      `MEMORY.md` itself) and reading each frontmatter. `load_index(dir) -> (String,
      usize)` returns the index text + entry count for pinning and `MemoryStatus`.
- [ ] Bytes/paths never escape the scope dir: join `slug` onto the fixed `dir` and assert
      the result stays under `dir` (belt-and-braces over the group-1 slug guard).

## 3. Engine wiring: gate impl, handler, load, pin, reset  *(T-13; Tech Spec §3.1, §7, §6.7)*

All in `crates/emberly-core/src/`.

- [ ] Gate impl in `gate.rs`: `MemoryGateImpl` + `MemoryAsk { req, reply:
      oneshot::Sender<MemoryOutcome> }` (channel+oneshot, mirroring `TaskListGateImpl`
      :129 but returning `MemoryOutcome` like `RecallGateImpl` :101 returns data).
      Fail-closed to `Err` on disconnect.
- [ ] Engine state: hold the `MemoryStore`, the two cached index strings, and the two
      counts. Construct from `EngineConfig` (group 5) with the trust-gated dirs (group 4).
      Add the gate field (`engine.rs:466`), channel create + `run` tuple (`Engine::new`
      :554/:560, `run` :657), and the `make_ctx` install (`.with_memory_gate(...)` :2219).
- [ ] Select-loop arm: `Some(m) = memory_rx.recv() => self.on_memory_op(m).await` beside
      the `task_rx` arm (`engine.rs:1466`) **and** any other select loops that must stay
      responsive during a tool run (mirror the Phase 1 wiring). Drain pending on frontend
      disconnect.
- [ ] `on_memory_op` handler: re-validate the name (authoritative slug guard), route to
      the `MemoryStore` op **on the correct scope** — a `Project` op when `project_dir` is
      `None` returns `Rejected { reason: "project memory unavailable (untrusted root)" }`
      (HC-6 data, never a panic). On a mutation, `regenerate_index` + reload both cached
      indexes/counts, then **emit `UiEvent::MemoryStatus { user, project }`** (group 6) via
      `emit()` (`engine.rs:2256`). Ack/return the `MemoryOutcome` over the oneshot.
- [ ] **No new transcript type (Tech Spec §3.2, HC-7).** A memory write is already a
      `tool_call`/`tool_result` pair — the model's content is in the args and the outcome
      in the result; the durable fact lives in the user-inspectable `<slug>.md` file, not
      duplicated into the JSONL. Confirm the existing `ToolResult` transcript line records
      the op summary; **no `SCHEMA_VERSION` bump**. Record this choice in the log.
- [ ] **Load at session start.** In `Engine::new`/session adoption, load both indexes
      into the cached state (user-global always; project only if `project_dir.is_some()`)
      and emit the initial `MemoryStatus` (Tech Spec §8.1 "at session start the engine
      loads both indexes"). On resume, the same load runs — memory is disk-backed, so a
      resumed session re-reads the current store (independent of the transcript).
- [ ] **Pin the index(es) into the sent context.** In `effective_system()`
      (`engine.rs:2186`, the exact seam the task list uses) append a rendered memory block
      when `memory.enabled` and an index is non-empty — a `render_memory_block(user_idx,
      project_idx)` beside `render_task_list_block` (:2510), labeling scope. **Bodies are
      NOT pinned** (Tech Spec §7 — only the index; bodies load via the `recall` op). Warn
      (dim harness line, once) when an index exceeds `memory.max_index_entries` (the §16
      soft-cap open item) — do not truncate.
- [ ] Reset on new session (`adopt_session` ~:894, where `task_list` clears): re-point the
      store to the (possibly new) project root and reload; user-global is unchanged.

## 4. Trust-gated project scope + store dirs from the binary  *(FR-6, FR-1; Tech Spec §6.7, §8.1)*

All in `crates/emberly/src/main.rs` (+ a `config.rs` path helper).

- [ ] Add `memory_dir()` beside `global_trust_path` (`config.rs:929`):
      `config_dir().map(|d| d.join("memory"))` — the XDG user-global store (`config.rs:909`
      `config_dir` already handles `XDG_CONFIG_HOME`/`HOME`).
- [ ] In `main.rs`, after the trust gate (`main.rs:344`) and before building the engine
      config: compute `user_memory_dir = config::memory_dir()` (always `Some` when a home
      exists) and `project_memory_dir = trust_granted-or-already-trusted ?
      Some(project_root.join(".agents").join("memory")) : None`. **Because the gate exits
      on decline, a running session is trusted** — but pass `None` when the root is not
      trusted so the gating is structural and future-proof (structural fact 3). Thread both
      into `EngineConfig` (group 5).
- [ ] Do **not** create the dirs eagerly — the store creates a scope dir on first write
      (group 2), so an empty store leaves no directories and loads an empty index.

## 5. Config: `[memory]` + resolve/provenance + `EngineConfig`  *(T-13; Tech Spec §8)*

- [ ] `crates/emberly/src/config.rs`: add `pub memory: MemoryConfigFile` to `ConfigFile`
      (:19, beside `context` :51) with `struct MemoryConfigFile { enabled: Option<bool>,
      max_index_entries: Option<usize> }` (mirror `ContextConfigFile` :83). Merge
      project-over-global field-by-field (:312 block).
- [ ] Resolve into a new `emberly_core::MemoryConfig { enabled: bool (default true),
      max_index_entries: usize (default — pick an initial soft cap, §16) }` at
      `config.rs:667` (beside the `context` resolve), and add it to `Resolved` (:334) +
      the engine wiring (`main.rs:524`). Store *locations are fixed, not config* (Tech Spec
      §8: "Memory and skill store locations are fixed"), so only the two behavior knobs are
      configurable — the dirs come from group 4.
- [ ] `EngineConfig` (`emberly-core`, `engine.rs` near the `context` field) gains
      `memory: MemoryConfig`, `user_memory_dir: Option<PathBuf>`,
      `project_memory_dir: Option<PathBuf>`.
- [ ] Provenance / `config show` (C-3): add `memory.enabled` + `memory.max_index_entries`
      blocks to the provenance machinery (`config.rs:523`–:604 pattern, `record` :757) —
      speech about deviations, silent on defaults (handbook §5).

## 6. Events + TUI: `MemoryStatus` + sidebar + origin  *(T-13; Design §4.9, §3.1, §7)*

No new per-op UiEvent (Tech Spec §3.1: memory writes/recalls flow through the existing
`ToolStarted`/`ToolFinished`); `MemoryStatus` carries only the counts for the sidebar.

- [ ] `UiEvent`: add `MemoryStatus { user: usize, project: usize }` (`event.rs:20`,
      after `TaskListUpdated` :177; `#[non_exhaustive]` keeps it non-breaking, serde
      `tag = "kind"` → `"memory_status"`).
- [ ] TUI `App`: add `memory_user: usize` / `memory_project: usize` (`app.rs:314` region)
      and a `MemoryStatus` arm in `apply_event` (:665, beside `TaskListUpdated`); clear on
      new session.
- [ ] Sidebar **Memory** section in `render_sidebar()` (`render.rs`, after the Tasks block
      ~:705, before `render_widget`): show per-scope counts (`Memory  user 3 · project 5`)
      — **counts, not bodies** (Design §4.9 "count from `MemoryStatus`"; the editable
      entry inspector overlay via the Design §4.6 in-app path is a later affordance — a
      read-only count section is the in-scope surface here, and an empty store shows no
      section). No color-only signal (Design §7).
- [ ] **Origin on the tool line (Design §4.9).** The `user`/`project` origin and the
      `remembered`/`recalled` verb ride the tool `summary` set in group 1 (from the
      `MemoryOutcome`), rendered by the existing `ToolFinished` path — no render change
      beyond confirming the `·`-separated form matches Design §4.9
      (`remembered · project · "<desc>"`, `recalled 2 memories`).
- [ ] Plain/degraded frontend (`line.rs`): memory ops already render via the tool-finish
      line (`line.rs:84`); `MemoryStatus` needs no plain-mode surface (no sidebar). Keep
      `degraded_output_has_no_ansi_escapes` green.

## 7. Tests + exit criterion  *(Tech Spec §14.7 offline, deterministic)*

- [ ] **Slug/validation unit test (`emberly-tools`, the HC-4 boundary made a test):**
      `slug` rejects `..`, absolute paths, and `/`/`\\` separators in `name`, and slugs a
      normal name deterministically.
- [ ] **Store unit test (`emberly-core` over a temp dir):** write → the `<slug>.md` file
      exists with correct `+++` frontmatter + body; `regenerate_index` produces the
      one-line-per-entry `MEMORY.md`; `recall` returns the body; `remove` deletes it and
      updates the index; `split_frontmatter` round-trips.
- [ ] **Write/recall round-trip via `FakeProvider`** (mirror `todo_round_trip...`
      `engine_loop.rs:3419`): a scripted `memory` write emits `MemoryStatus` with the new
      counts, and a subsequent `recall` op returns the entry body in the `tool_result`.
- [ ] **Index pinned:** after a write, `FakeProvider::last_request()` (`fake.rs:166`)
      shows the memory index in the system prompt (mirror the pin_task_list assertion);
      with `memory.enabled = false` it is absent.
- [ ] **Project memory absent under an untrusted root:** construct the engine with
      `project_memory_dir: None`; a `memory` op with `scope: project` returns the HC-6
      `Rejected` result and the project index never enters the pinned context, while
      user-global still works (Design §4.9 "silently absent, not half-loaded"; Tech Spec
      §6.7).
- [ ] **Not permission-gated:** a `memory` op never raises a `PermissionRequest` (assert
      no `authorize` path; mirror the `todo`/`recall` not-gated assertions).
- [ ] **HC-4 boundary at the engine:** a `memory` op whose `name` escapes (constructed to
      bypass the tool, hitting `on_memory_op`) is rejected by the authoritative guard — the
      write never lands outside the scope dir.
- [ ] **HC-7 / no bytes duplicated:** the fact lives in `<slug>.md`; the transcript records
      only the `tool_call`/`tool_result` pair; resume re-reads the store from disk.
- [ ] **Exit criterion (Phase 3 done when):** a write/recall round-trip works; a
      path-escape attempt in `name` (`..`, absolute, separators) is rejected (the HC-4
      boundary made a test); the index loads as pinned context; and project-scope memory
      does **not** load under an untrusted root (Tech Spec §14.7, FR-6/§6.7). Workspace
      clippy-clean under the §1 lint policy; offline suite green. (No new external
      dependency — `toml` is already in the tree, HC-2.)

---

## Decisions & notes log

> Fill in as the phase proceeds (mirrors the v0.1–v0.3, Phase 1, and Phase 2 logs).

- **Gate shape.** `memory` reaches the engine via a new `MemoryGate` returning a
  `MemoryOutcome` — structurally a hybrid of `TaskListGate` (ack + engine mutation, not
  permission-gated) and `RecallGate` (returns data, for the `recall` op). It never calls
  `ctx.authorize` — the only mechanism that marks a tool gated (confirmed: no boolean flag
  exists). _If the Spec should absorb the gate/outcome shape, it flows back as a version
  bump per G-24, not an edit here._
- **Trust-gating is structural, not a runtime re-check.** The binary passes
  `project_memory_dir: None` on an untrusted root; the engine never re-checks trust
  (`trust::gate` already exited on decline). Chosen because trust is a pre-engine gate
  (Tech Spec §6.7) and this keeps the honesty clause testable (construct with `None`).
  _Record if a future non-interactive "proceed-untrusted" mode ever lets a session run
  untrusted — then this `None` path becomes load-bearing at runtime, not just future-proofing._
- **`memory` recall op vs the `recall` tool (T-10).** Deliberately kept distinct: the
  memory op returns an entry *body*; the T-10 tool returns dropped *conversation turns*.
  Same word, two gates. _Watch the tool descriptions so the model does not confuse them._
- **Frontmatter delimiter.** Leaning to `+++` (TOML's conventional fence) over `---`
  (YAML/markdown-ambiguous), Tech Spec §8.1 says "TOML frontmatter" without pinning the
  fence. _Confirm in group 2; if §8.1 should pin the delimiter, that is G-24 feedback._
- **Index pinned via the system prompt, not a message pin.** The memory index is engine
  state, so it rides `effective_system()` (the task-list precedent) rather than the message
  pin prefix (`pinned_count`/`windowed_messages`). Bodies are never pinned (Tech Spec §7).
- **No transcript type / no bytes in the JSONL (HC-7).** The durable fact is the
  user-inspectable `<slug>.md`; the transcript keeps only the `tool_call`/`tool_result`
  pair. Resume re-reads the store from disk. _Consequence: editing a memory file between
  sessions is honored on next load — intended (Tech Spec §8.1 "plain text the user edits")._
- **Sidebar shows counts now; the editable overlay is a later affordance.** Design §4.9
  describes an editable entry inspector via the §4.6 in-app path; this phase ships the
  read-only count section (the `MemoryStatus` surface) and leaves the overlay to a
  follow-up, since files are already user-editable on disk. _Record if the owner wants the
  overlay in-scope for 0.4._
- **Open items carried in (Tech Spec §16 v0.8, this phase's resolver):**
  `memory.max_index_entries` is a soft warn threshold — determine the point at which a large
  always-pinned index itself wants the §7 economy (description truncation or an on-demand
  index tier); pick the initial default in group 5 and tune with use (Requirements §13).
