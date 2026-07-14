# Phase 4 — Skill system (FR-7, T-15) — TODO & Progress

**Milestone:** M8 phase 4 (Tech Spec §15) — the 0.4 capability-parity set. Fourth
phase; it **reuses Phase 3's progressive-disclosure machinery** (TOML `+++`
frontmatter, a pinned catalog, `Option`-based trust-gating of the project scope, the
sidebar section, the `config_dir()` XDG helper) rather than rebuilding it. Skills add
discovery + precedence + an invoke tool on top of that store apparatus.
**Satisfies:** FR-7, T-15; Tech Spec §8.2, §5.2, §7, §6.7, §3.1; Design §4.9, §3.1;
FR-1, HC-2. Pinned to **Req v0.7 / Design v0.7 / Spec v0.8** (all `approved`).
**Goal:** Named, progressively-disclosed capability folders the model discovers and
invokes on demand. At startup the engine scans two scopes, builds a **skill catalog**
(`SkillMeta{name, description, origin}` per skill) pinned into context so the model
always knows what it can invoke; the `skill` tool loads one skill's `SKILL.md` **body**
(and lists its bundled resource paths) on invoke — metadata standing, body on demand.
The `skill` tool **executes nothing**: running a skill's bundled script is a separate,
ordinary permission- and sandbox-gated `bash` call (FR-7 honesty clause). Project
skills load **only under a trusted root** (FR-1); on a name collision **project
overrides user-global**, and the shadow is surfaced by `emberly config show`.

**Depends on:** the shipped product (M1–M7) **plus Phase 3 (memory, merged)** — the
seam it mirrors most closely. Reuse directly:
- **Frontmatter splitter** — `emberly_core::memory::split_frontmatter` (`+++` TOML
  fence, `crates/emberly-core/src/memory.rs:262`); `EntryMeta`/`serialize_entry`
  (:16, :286) are the shape to imitate for `SKILL.md`. **Reuse `split_frontmatter`
  directly, or extract it + the `toml::from_str::<Meta>` step to a shared
  `frontmatter` module** — decide in the notes log (leaning to a small extraction so
  `memory` and `skills` share one parser; low-risk, both `+++`).
- **Name validation/slug** — `emberly_tools::memory::slug` (`crates/emberly-tools/src/memory.rs:114`,
  re-exported `lib.rs:33`) rejects `..`/absolute/separators; reuse to sanitize the
  requested skill name before catalog lookup.
- **Scope/origin enum** — `MemoryScope{User,Project}` (`crates/emberly-tools/src/memory.rs:17`)
  is the exact precedent for `SkillOrigin{User,Project}` (origin drives the Design §4.9
  tool line and precedence).
- **Trust-gated `Option<PathBuf>` project dir** — `MemoryStore{user_dir, project_dir:
  Option<PathBuf>}` (`crates/emberly-core/src/memory.rs:31`), `dir_for` (:44); binary
  threading `user_memory_dir`/`project_memory_dir` into `EngineConfig`
  (`crates/emberly/src/main.rs:514`–:558), `build_memory_store` (`engine.rs:2718`).
  **Mirror exactly** for skills.
- **XDG dir helper** — `config::memory_dir()` (`crates/emberly/src/config.rs:1026`) →
  add `skills_dir() = config_dir().join("skills")`.
- **Pinned block** — `effective_system()` memory branch (`engine.rs:2285`),
  `render_memory_block` (`engine.rs:2705`); cached-index fields
  (`memory_user_index`/`memory_project_index` `engine.rs:584`), refresh
  (`refresh_memory_indexes` :2344).
- **Gate for a data-returning, non-permission-gated op** — `MemoryGate`
  (`crates/emberly-tools/src/memory.rs:80`), `ToolCtx` wiring (field :63, no-op default
  `DropMemoryGate` :91, `with_memory_gate` :124, accessor `memory_op` :195),
  `MemoryGateImpl`/`MemoryAsk` channel+oneshot (`crates/emberly-core/src/gate.rs:165`),
  engine gate field (`engine.rs:503`), channel (`:609`), select-loop arm (`:1545`),
  handler `on_memory_op` (`:2356`), install in `make_ctx` (`:2338`).
- **UiEvent + sidebar** — `MemoryStatus` (`crates/emberly-core/src/event.rs:185`),
  emit at session start (`engine.rs:713`) + after op (`:2371`); TUI `App` fields
  (`crates/emberly-tui/src/app.rs:317`), `apply_event` arm (:675), sidebar Memory
  section (`crates/emberly-tui/src/render.rs:707`), session reset (`app.rs:1663`).
- **Config** — `ConfigFile.memory` (`config.rs:57`), `MemoryConfigFile` (:119), merge
  (:359), `Resolved.memory` (:411) + resolve (:772), core `MemoryConfig`
  (`engine.rs:115`), `EngineConfig.memory` (:216), `config show` provenance (:668).
- **Bash tool (for skill scripts)** — `BashTool` name `"bash"`
  (`crates/emberly-tools/src/builtin/bash.rs:152`), registered `builtin/mod.rs:40`,
  permission+sandbox gated (`ctx.authorize`, `ctx.sandbox().bash_invocation`) — the
  `skill` tool never touches this; the model calls `bash` itself.
- **Registry** — `default_registry()` currently registers 11 tools
  (`crates/emberly-tools/src/builtin/mod.rs:34`); `skill` becomes the 12th.
- **Tests** — memory store unit tests (`crates/emberly-core/src/memory.rs:292`), slug
  tests (`crates/emberly-tools/src/memory.rs:157`), engine round-trips + untrusted test
  (`crates/emberly-core/tests/engine_loop.rs:3777`, `:3843`), `last_request().system`
  pin assertion pattern.

**Key structural facts (from the codebase):**
1. **Skills are discovered/read-only, not model-authored.** Unlike memory (the model
   *writes* entries), the model never creates or edits a skill — it only *invokes* one.
   So there is **no write/mutate path and no mutation gate**; the catalog is built once
   at startup (and re-derived on a new session/root), pinned, and emitted via
   `SkillsAvailable`. The only model-facing tool is `skill` (invoke = load body).
2. **The `skill` tool executes nothing and is not permission-gated.** It reads
   instruction text from already-trust-resolved dirs and returns it — like `recall`, a
   read the permission model does not guard. Running a skill's bundled script is the
   model making a **separate ordinary `bash` call**, fully permission- and sandbox-gated
   (FR-7 honesty clause, Tech Spec §8.2). The `skill` line must never imply a script ran.
3. **Trust-gating is the same structural `Option` as memory** — the binary passes the
   project skills dir as `Some` only under a trusted root, `None` otherwise; the engine
   never re-checks trust. A project skill from an untrusted root is **neither cataloged
   nor invocable** (Tech Spec §6.7, Design §4.9 "silently absent"). Testable by
   constructing with `project_skills_dir: None`.
4. **Precedence: project overrides user-global** on a name collision (project ships a
   tuned variant), mirroring config precedence. The shadowed user skill is surfaced by
   `emberly config show` — this resolves the Requirements §13 / Tech Spec §16 precedence
   open item (project wins, surfaced), so the surfacing is in scope, not optional.
5. **`SkillMeta`/`SkillsAvailable`/`skills.enabled` are greenfield** — none exist yet
   (confirmed). Placement mirrors memory: shared `SkillMeta`/`SkillOrigin` in
   `emberly-tools`, the `SkillsAvailable` event beside `MemoryStatus` in
   `emberly-core/event.rs`, discovery in a new `emberly-core/src/skills.rs`.

**Legend:** `[ ]` todo · `[~]` in progress · `[x]` done · `[!]` blocked

---

## Progress summary

| Group | Status | Notes |
|---|---|---|
| 1. Shared types (`SkillMeta`/`SkillOrigin`) + `SkillGate` + `skill` tool skeleton (`emberly-tools`) | [ ] | invoke-only; returns body + resource paths; not permission-gated; reuse `slug` |
| 2. Skill discovery/catalog module: scan dirs, parse `SKILL.md`, precedence + shadow, resolve body (`emberly-core`) | [ ] | new `skills.rs`; reuse `split_frontmatter`; project overrides user; list bundled resources |
| 3. Engine wiring: gate impl + select-loop arm + `on_skill_invoke` + build catalog at start + pin + `SkillsAvailable` + reset (`emberly-core`) | [ ] | catalog is engine state; pin via `effective_system()`; emit at start & session switch |
| 4. Trust-gated project scope + skills dirs from the binary (`emberly`) | [ ] | user-global always; project dir `Some` only under trust; `skills_dir()` XDG helper |
| 5. Config: `[skills] enabled` + resolve/provenance + `EngineConfig` + `config show` shadow notice (`emberly`/`emberly-core`) | [ ] | mirror `[memory]`; shadow surfacing resolves §16 precedence open item |
| 6. Events + TUI: `SkillsAvailable` UiEvent + sidebar Skills section + read-only body view + origin on tool line + degraded (`emberly-core`/`emberly-tui`) | [ ] | list name/description/origin; select → `SKILL.md` body read-only (§4.2 overlay) |
| 7. Tests (offline, deterministic — §14.7) + exit criterion | [ ] | discovery, precedence+shadow, body-on-invoke, untrusted absent, script via bash only |

**Overall Phase 4: NOT STARTED.**

---

## 1. Shared types + `SkillGate` + `skill` tool skeleton  *(T-15; Tech Spec §5.2, §8.2)*

Shared types + the invoke tool live in `emberly-tools`; catalog discovery + the gate
impl land in `emberly-core` (groups 2/3) — the established split.

- [ ] New `crates/emberly-tools/src/skills.rs` (beside `memory.rs`): `enum
      SkillOrigin { User, Project }` (`#[serde(rename_all = "snake_case")]`, mirror
      `MemoryScope` :17) and `struct SkillMeta { name: String, description: String,
      origin: SkillOrigin }` (`Clone, Debug, Serialize, Deserialize, PartialEq`).
      Re-export from `emberly-tools/src/lib.rs` (like `TaskItem`/`MemoryScope`). Define
      the gate's return type `SkillInvocation { body: String, resources: Vec<String>,
      origin: SkillOrigin }` and a `SkillError`.
- [ ] Add a `SkillGate` trait + `ToolCtx` wiring (`ctx.rs`, mirror `MemoryGate` exactly):
      field `skill: Arc<dyn SkillGate>` (:63 region), no-op default `DropSkillGate`
      returning `Err(SkillError)` (:91), `with_skill_gate` builder (:124), accessor
      `invoke_skill(&self, name: String) -> Result<SkillInvocation, SkillError>` (:195).
      Signature returns data (like `memory_op`), fail-closed to `Err` mapped to
      `ToolOutcome::failure` (HC-6), never a panic.
- [ ] New `crates/emberly-tools/src/builtin/skill.rs` — `struct SkillTool` implementing
      `Tool`, shaped like `builtin/memory.rs`: module doc stating "reads instruction text
      only; executes nothing; running a bundled script is a separate `bash` call under
      §6; not permission-gated"; `spec()` name `"skill"`, description (C-4, overridable
      per C-1), schema `{name: string}` required. `execute()` sanitizes `name` via `slug`
      reuse (defense), calls `ctx.invoke_skill(name)` and **never** `ctx.authorize`; maps
      `SkillInvocation` to a `ToolOutcome::success` whose **content** is the `SKILL.md`
      body followed by a "Bundled resources:" list of the resource paths, and whose
      **summary** carries origin for the Design §4.9 line (`skill · pdf-fill · user`). An
      unknown skill → clean `ToolOutcome::failure` (HC-6), not a crash.
- [ ] Register in `builtin/mod.rs`: `mod skill;` + `pub use` and
      `registry.register(Arc::new(SkillTool::new()))` in `default_registry()` (:34,
      after `MemoryTool` :46 — the 12th tool). Advertised via `registry.specs()`.

## 2. Skill discovery / catalog module  *(FR-7, T-15; Tech Spec §8.2, HC-2)*

New `crates/emberly-core/src/skills.rs` — pure over injected dir paths so it is
unit-testable against temp dirs.

- [ ] `struct SkillCatalog { user_dir: PathBuf, project_dir: Option<PathBuf> }`
      (`project_dir` `None` on an untrusted root — structural fact 3). Each skill is a
      subdirectory `<dir>/<name>/` containing `SKILL.md`.
- [ ] **Frontmatter parse (reuse Phase 3).** Read each `<name>/SKILL.md`, split with the
      shared `+++` splitter (reuse `memory::split_frontmatter` or the extracted shared
      helper — notes log), parse the frontmatter with `toml::from_str` into `struct
      SkillManifest { name: String, description: String }`. A folder without a readable
      `SKILL.md` or without required frontmatter is skipped (warn, dim line — not a
      crash), not cataloged.
- [ ] **Discovery + precedence.** `discover() -> (Vec<SkillMeta>, Vec<ShadowNotice>)`:
      scan user-global then project (project only if `project_dir.is_some()`); build the
      catalog keyed by `name`. On a collision **project overrides user-global** (keep the
      project `SkillMeta`, record a `ShadowNotice{name, shadowed_origin: User}` for
      `config show` — structural fact 4). `origin` is set per the winning scope.
- [ ] **Resolve a body on invoke.** `invoke(name) -> Option<SkillInvocation>`: look up
      the winning skill's folder, read the full `SKILL.md` body (post-frontmatter), and
      list bundled resource paths — every file in `<name>/` except `SKILL.md`, returned
      as project-relative for a project skill and absolute for a user-global skill (so the
      model can `read_file`/`bash` them; reading a user-global resource is an ordinary
      outside-root read that prompts under HC-4 — honesty clause, note in log). Missing
      skill → `None`.
- [ ] Catalog rendering for the pinned block: `render_catalog(&[SkillMeta]) -> String`
      (one line per skill `- <name> — <description> (<origin>)`), mirroring
      `render_memory_block`. Empty catalog → empty string (no section).

## 3. Engine wiring: gate impl, handler, build, pin, event, reset  *(T-15; Tech Spec §3.1, §7, §6.7)*

All in `crates/emberly-core/src/`.

- [ ] Gate impl in `gate.rs`: `SkillGateImpl` + `SkillAsk { name, reply:
      oneshot::Sender<Result<SkillInvocation, SkillError>> }` (channel+oneshot, mirror
      `MemoryGateImpl`/`MemoryAsk` :165). Fail-closed to `Err` on disconnect.
- [ ] Engine state: hold the `SkillCatalog`, the built `Vec<SkillMeta>`, the pinned
      catalog string, and the shadow notices. Construct from `EngineConfig` (group 5) via
      a `build_skill_catalog(config)` mirroring `build_memory_store` (`engine.rs:2718`);
      `None`/empty when `skills.enabled` is false. Add the gate field (`engine.rs:503`
      region), channel create + `run` tuple, and the `make_ctx` install
      (`.with_skill_gate(...)` :2338).
- [ ] Select-loop arm: `Some(s) = skill_rx.recv() => self.on_skill_invoke(s).await`
      beside the `memory_rx` arm (`engine.rs:1545`) **and** the other responsive select
      loops. Drain pending on frontend disconnect.
- [ ] `on_skill_invoke` handler: resolve the body via the catalog and reply over the
      oneshot; an unknown/untrusted-absent skill returns `Err`/a rejection the tool maps
      to HC-6 data (never a panic). **Read-only — emits nothing** (the catalog does not
      change on invoke; contrast memory's `on_memory_op` which refreshes `MemoryStatus`).
- [ ] **Pin the catalog into the sent context.** In `effective_system()`
      (`engine.rs:2285`, the exact seam memory/task-list use) append the rendered catalog
      when `skills.enabled` and the catalog is non-empty — a `render_skills_block` beside
      `render_memory_block` (:2705). **Only metadata is pinned; bodies load via the
      `skill` tool** (Tech Spec §7 progressive disclosure). `windowed_messages()`/
      `pinned_count()` unchanged.
- [ ] **`SkillsAvailable` at session start + on switch.** Build the catalog and emit
      `UiEvent::SkillsAvailable { skills }` (group 6) in `Engine::new` (mirror the
      `MemoryStatus` `try_send` at `engine.rs:713`) and re-derive it in `adopt_session`
      (:964, where memory refreshes) so a new root re-scans project skills. **Fix the
      memory gap while here:** memory does not re-emit `MemoryStatus` on session switch
      (deviation noted); do emit `SkillsAvailable` on switch so the TUI Skills section is
      never stale.
- [ ] **No new transcript type (HC-7).** A skill invoke is already a `tool_call`/
      `tool_result` pair; the instruction body is in the result, the skill files are
      user-inspectable on disk. No `SCHEMA_VERSION` bump. Record in log.

## 4. Trust-gated project scope + skills dirs from the binary  *(FR-7, FR-1; Tech Spec §6.7, §8.2)*

All in `crates/emberly/src/main.rs` (+ a `config.rs` path helper) — mirror the memory
wiring at `main.rs:514`–:558 exactly.

- [ ] Add `skills_dir()` beside `memory_dir()` (`config.rs:1026`):
      `config_dir().map(|d| d.join("skills"))` — the XDG user-global skills root.
- [ ] In `main.rs`, after the trust gate (`main.rs:344`): compute `user_skills_dir =
      config::skills_dir()` (always `Some` when a home exists) and `project_skills_dir =
      Some(project_root.join(".agents").join("skills"))` (the trust gate exited on
      decline, so a running session is trusted; keep `None` as the structural
      untrusted-fallback for future non-interactive paths, exactly like
      `project_memory_dir` :558). Thread both into `EngineConfig` (group 5).
- [ ] Do **not** create the dirs eagerly — discovery tolerates a missing dir as an empty
      catalog (an absent `~/.config/emberly/skills` or `.agents/skills` is normal).

## 5. Config: `[skills]` + provenance + `config show` shadow notice  *(T-15; Tech Spec §8, §8.2)*

- [ ] `crates/emberly/src/config.rs`: add `pub skills: SkillsConfigFile` to `ConfigFile`
      (:57 region, beside `memory`) with `struct SkillsConfigFile { enabled: Option<bool>
      }` (mirror `MemoryConfigFile` :119). Merge project-over-global (:359 block).
- [ ] Resolve into `emberly_core::SkillsConfig { enabled: bool (default true) }` at
      `config.rs:772` (beside the memory resolve), add to `Resolved` (:411) and the engine
      wiring (`main.rs:524`). Store **locations are fixed, not config** (Tech Spec §8),
      so `enabled` is the only knob; dirs come from group 4.
- [ ] `EngineConfig` (`engine.rs:216` region) gains `skills: SkillsConfig`,
      `user_skills_dir: Option<PathBuf>`, `project_skills_dir: Option<PathBuf>`.
- [ ] Provenance / `config show` (C-3): add a `skills.enabled` block (mirror
      `memory.enabled` :668). **Plus the shadow notice (Tech Spec §8.2, resolves the §16
      precedence open item):** in the `config show` path, run skill discovery and print
      any `ShadowNotice` — "skill `<name>`: project shadows user-global" — so a shadowed
      user skill is visible, not silent. (Discovery scans project skills only under trust,
      consistent with §6.7; note the resolver in the log if `config show` outside a
      trusted session needs special handling.)

## 6. Events + TUI: `SkillsAvailable` + sidebar + body view  *(T-15; Design §4.9, §3.1, §7)*

No new per-invoke UiEvent (Tech Spec §3.1: skill invocations flow through the existing
`ToolStarted`/`ToolFinished`); `SkillsAvailable` carries the catalog for the sidebar.

- [ ] `UiEvent`: add `SkillsAvailable { skills: Vec<emberly_tools::SkillMeta> }`
      (`event.rs:185`, beside `MemoryStatus`; `#[non_exhaustive]`, serde `tag = "kind"` →
      `"skills_available"`).
- [ ] TUI `App`: add `skills: Vec<SkillMeta>` (`app.rs:317` region) + a `SkillsAvailable`
      arm in `apply_event` (:675); clear on new session (:1663).
- [ ] Sidebar **Skills** section in `render_sidebar()` (`render.rs`, after the Memory
      section ~:716): list each skill `name — description (origin)` grouped/annotated by
      origin (Design §4.9: origin is how the user reads trust). Empty catalog → no section
      (Design §3.1). No color-only signal (Design §7).
- [ ] **Read-only body view (Design §4.9, in-scope per plan).** Selecting a Skills-row
      opens a read-only overlay showing that skill's `SKILL.md` body — reuse the existing
      overlay pattern (Design §4.2, the diff/inspector overlay in `render.rs`/`app.rs`).
      This is the first such inspector (memory shipped counts-only); if the overlay proves
      large, the **minimum in-scope surface is the list with name/description/origin** and
      the body view can land as a follow-up — **decide in the notes log**, defaulting to
      shipping the read-only view since the plan names it.
- [ ] **Origin on the tool line (Design §4.9).** `skill · <name> · <origin>` rides the
      tool `summary` from group 1, rendered by the existing `ToolFinished` path — confirm
      the `·`-separated form. A skill invoke is quiet tool activity, never harness voice.
- [ ] Plain/degraded frontend (`line.rs`): skill invokes already render via the
      tool-finish line; `SkillsAvailable` needs no plain surface (no sidebar). Keep
      `degraded_output_has_no_ansi_escapes` green.

## 7. Tests + exit criterion  *(Tech Spec §14.7 offline, deterministic)*

- [ ] **Discovery unit test (`emberly-core` over temp dirs):** a `<name>/SKILL.md` with
      `+++` frontmatter is cataloged as `SkillMeta{name, description, origin}`; a folder
      without `SKILL.md`/frontmatter is skipped; bundled resource paths are listed on
      invoke and the body is post-frontmatter text.
- [ ] **Precedence + shadow:** a skill present in both scopes resolves to the **project**
      variant with `origin: Project` and produces a `ShadowNotice` for the user copy
      (assert `config show` surfacing).
- [ ] **Body-on-invoke via `FakeProvider`** (mirror the memory round-trip
      `engine_loop.rs:3777`): a scripted `skill` call returns the `SKILL.md` body +
      resource list in the `tool_result`; the catalog (metadata) is pinned in
      `last_request().system` while the body is **not** pinned (progressive disclosure).
- [ ] **Catalog pinned / `skills.enabled=false`:** with skills enabled the catalog is in
      the system prompt; with `skills.enabled = false` no catalog is pinned and the `skill`
      tool is effectively empty (unknown-skill failures).
- [ ] **Untrusted project absent:** construct with `project_skills_dir: None`; a project
      skill is neither cataloged (`SkillsAvailable` omits it, not pinned) nor invocable
      (invoke → HC-6 failure), while user-global skills still work (Design §4.9; Tech Spec
      §6.7).
- [ ] **`skill` executes nothing / not permission-gated:** a `skill` invoke never raises
      a `PermissionRequest` and never spawns a process (assert no `authorize`, no
      `sandbox` call); running a bundled script is a *separate* `bash` call that **does**
      hit the permission gate (assert the bash path still prompts — FR-7 honesty clause).
- [ ] **Exit criterion (Phase 4 done when):** discovery builds the catalog; `skill` loads
      only the body on invoke (metadata standing); project-over-user precedence resolves
      with a shadow notice; an untrusted-root project skill is absent from the catalog;
      and a bundled script runs only through the ordinary permission-gated `bash` path
      (Tech Spec §14.7, FR-7/§6.7). Workspace clippy-clean under the §1 lint policy;
      offline suite green. No new external dependency (`toml` already in the tree, HC-2).

---

## Decisions & notes log

> Fill in as the phase proceeds (mirrors the v0.1–v0.3, Phase 1–3 logs).

- **No mutation gate — skills are read-only.** Unlike memory (write/update/remove), the
  model never authors a skill, so there is only a data-returning `SkillGate` (invoke =
  load body), structurally the memory `recall` op without the write ops. It never calls
  `ctx.authorize` — not permission-gated (reads instruction text from trust-resolved
  dirs). _If the Spec should absorb the gate/outcome shape, it flows back as a version
  bump per G-24, not an edit here._
- **Frontmatter parser: reuse vs. extract.** `split_frontmatter` is `pub` in
  `emberly-core::memory`; skills can import it, or it + the `toml::from_str::<Meta>` step
  extract to a shared `emberly-core::frontmatter` module. Leaning to a small extraction so
  neither feature "owns" the other's parser. _Decide in group 2; keep it low-churn._
- **Trust-gating is structural (mirrors memory).** Binary passes `project_skills_dir:
  None` on an untrusted root; the engine never re-checks trust. Testable via `None`.
  Chosen for the same reason as memory — trust is a pre-engine gate (Tech Spec §6.7).
- **Precedence: project wins, shadow surfaced.** Resolves the Requirements §13 / Tech
  Spec §16 precedence open item. Deliberately *not* invoking a shadowed user skill by a
  qualified name in this phase (the §16 sub-question "should a shadowed user skill be
  invocable by qualified name") — **left as an open item, resolver = owner**; default is
  project shadows user entirely, surfaced in `config show`.
- **Resource paths cross the root boundary honestly.** A user-global skill's bundled
  resources are listed as absolute paths; reading/running them is an ordinary
  outside-root `read_file`/`bash` that prompts under HC-4/§6 — the `skill` tool grants no
  privileged path (FR-7 honesty clause). Project-skill resources are project-relative and
  read without a prompt (§6.2).
- **Sidebar body view is the first inspector overlay.** Memory shipped counts-only and
  punted its editable overlay; the plan puts the skills **read-only** body view in scope
  (§4.9). Reuse the Design §4.2 overlay pattern. _If it balloons, ship list-only and
  follow up with the body view — record the call in group 6._
- **Emit `SkillsAvailable` on session switch (fixes a memory gap).** Merged memory does
  not re-emit `MemoryStatus` on `adopt_session`; do emit `SkillsAvailable` on switch so
  the sidebar never goes stale after a model/root change. _Consider a follow-up to make
  memory symmetric (out of scope here)._
- **No transcript type / no bytes duplicated (HC-7).** A skill invoke is a `tool_call`/
  `tool_result` pair; skill files are user-inspectable on disk. Resume re-scans the store.
- **Open items carried in (Tech Spec §16 v0.8, this phase's resolver):** confirm the
  `config show` shadow notice is clear enough, and decide whether a shadowed user skill
  should ever be invocable by a qualified name (default: no).
