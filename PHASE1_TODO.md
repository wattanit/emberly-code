# Phase 1 — Task-list ("todo") tool (T-11) — TODO & Progress

**Milestone:** M8 phase 1 (Tech Spec §15) — the 0.4 capability-parity set. First
phase; establishes the *engine-state tool → UiEvent → transcript → pinned
context → sidebar section* pattern the later memory/skills phases reuse.
**Satisfies:** T-11; Tech Spec §5.2, §3.1/§3.2, §7, §8; Design §4.7, §3.1, §7;
HC-7. Pinned to **Req v0.7 / Design v0.7 / Spec v0.8** (all `approved`).
**Goal:** Give the model an explicit, ordered, user-visible task list for
multi-step work — model-authored planning made legible — as **pure engine state**
(no filesystem, no network), so like `recall`/`ask_user` it is **not
permission-gated**. Rendered inline + in a sidebar Tasks section, recorded in the
transcript (HC-7), and pinned in the sent context so "what's left" survives
compaction and windowing.

**Depends on:** the shipped product (M1–M7). The seams this phase plugs into all
exist:
- Tool trait / registry — `crates/emberly-tools/src/tool.rs:110`,
  `.../registry.rs`, `.../builtin/mod.rs:28`.
- The two engine-state tools to mirror — `builtin/ask_user.rs`,
  `builtin/recall.rs`; their gates in `crates/emberly-core/src/gate.rs`
  (`AskGate::ask` :69, `RecallGate` :101). **`RecallGate` is the closest analog**
  — a state-touching engine round-trip that never calls `ctx.authorize`.
- `UiEvent` (`crates/emberly-core/src/event.rs:23`), `Command`
  (`.../command.rs:17`), `TranscriptEvent` (`.../transcript.rs:62`) — all
  `#[non_exhaustive]`, so every addition is non-breaking.
- Sent-context assembly — `build_request()` `engine.rs:2111`, `effective_system()`
  `engine.rs:2151`.
- Config — `crates/emberly/src/config.rs` (`ContextConfigFile` :86).
- TUI sidebar — `crates/emberly-tui/src/render.rs` (`render_sidebar()` :532).

**Key structural fact (from the codebase):** there is **no boolean flag** marking
a tool "not permission-gated." A tool is permission-gated *iff* its `execute`
calls `ctx.authorize(...)`. `ask_user`/`recall` simply never do — they reach the
engine through a `ctx` gate method. The `todo` tool likewise never authorizes; it
mutates engine state through a new gate. Mirror `recall`/`ask_user`, never
`edit`/`bash`.

**Legend:** `[ ]` todo · `[~]` in progress · `[x]` done · `[!]` blocked

---

## Progress summary

| Group | Status | Notes |
|---|---|---|
| 1. `TaskItem` type + `TaskListGate` + tool skeleton (`emberly-tools`) | [ ] | mirror `RecallGate`/`AskGate` + `RecallTool`/`AskUserTool` |
| 2. Engine state + gate impl + select-loop wiring + `UiEvent` + transcript (`emberly-core`) | [ ] | `TaskListUpdated` event, `task_list` transcript event (additive, no SCHEMA bump) |
| 3. Pin the task list into the sent context (`effective_system`) | [ ] | gated on `context.pin_task_list`; mirrors the tool-explanation append |
| 4. Config: `context.pin_task_list` (default `true`) + provenance | [ ] | `ContextConfigFile` + merge + resolve + `config show` |
| 5. TUI: sidebar Tasks section + inline render + ASCII fallback | [ ] | rich glyphs `○◐✓`, plain `[ ]/[~]/[x]`; plain frontend prints inline |
| 6. Tests (offline, deterministic — §14.7) + exit criterion | [ ] | round-trip, pinned-across-compaction, degraded parity, not-gated |

**Overall Phase 1: NOT STARTED.**

---

## 1. `TaskItem` type + `TaskListGate` + tool skeleton  *(T-11; Tech Spec §5.2)*

The `todo` tool is a built-in that never touches the filesystem/network; it
mutates engine state through a new gate, exactly as `recall`/`ask_user` reach the
engine. Trait defs live in `emberly-tools`; the impl lands in `emberly-core`
(group 2) — the established split (`PermissionGate`/`AskGate` traits in tools,
`ChannelGate`/gate impls in core).

- [ ] Define `struct TaskItem { text: String, status: TaskStatus }` and
      `enum TaskStatus { Pending, InProgress, Done }` in `emberly-tools` near the
      gate traits (`ctx.rs` / a new `task_list.rs`). `#[derive(Clone, Debug,
      Serialize, Deserialize, PartialEq)]`, `#[serde(rename_all = "snake_case")]`
      on the status. It is shared: `emberly-core` imports it for engine state, the
      `UiEvent`, and the transcript event (core already depends on tools per Tech
      Spec §1). Re-export from `emberly-tools/src/lib.rs`.
- [ ] Add a `TaskListGate` trait + `ToolCtx` wiring in
      `crates/emberly-tools/src/ctx.rs` (mirror `with_ask_gate` :84 /
      `with_recall_gate` :92 and the `ask_user` :124 / `recall` :130 accessors).
      A safe no-op default gate installed in `ToolCtx::new` (:76–77), mirroring
      `DeclineAskGate`/`DeclineRecallGate` (e.g. `DropTaskListGate` that discards).
      Signature: `async fn set_task_list(&self, items: Vec<TaskItem>) ->
      Result<(), TaskListError>` — an ack, not a full round-trip answer (simpler
      than `ask`; closer to `recall`'s "reach the engine and return"). Fail-closed
      to an `Err` the tool maps to a `ToolOutcome::failure` (HC-6), never a panic.
- [ ] New `crates/emberly-tools/src/builtin/task_list.rs` — `struct TaskListTool`
      implementing `Tool` (`tool.rs:110`). Copy the shape of
      `builtin/recall.rs`: module doc stating "touches no filesystem or network,
      bypasses the sandbox, not permission-gated"; `spec()` (name `"todo"`,
      description, JSON schema for `{items: [{text, status}]}`); `execute()` calls
      `ctx.set_task_list(items)` and **never** `ctx.authorize` — returns
      `ToolOutcome::success` summarizing the update (e.g. "task list: 3 items, 1
      in progress"). The model always sends the **full list** (replace, not
      merge, per Tech Spec §5.2).
- [ ] Register in `crates/emberly-tools/src/builtin/mod.rs`: `mod task_list;` +
      `pub use` (:8–24) and `registry.register(Arc::new(TaskListTool::new()))` in
      `default_registry()` (:28, beside `AskUserTool` :36 / `RecallTool` :37). The
      spec is then advertised to the provider via `registry.specs()`
      (`registry.rs:40`) — no other change to make the model see it.
- [ ] Tool description is behavior-critical config (C-4), overridable per C-1 —
      keep the schema/description in the prompts/tool-descriptions source so a
      project can tune it.

## 2. Engine state + gate impl + select-loop wiring + events  *(T-11; Tech Spec §3.1, §3.2)*

The engine holds the current list, emits it to the frontends, and records it.
All in `crates/emberly-core/src/`.

- [ ] Engine state: add `task_list: Vec<TaskItem>` to the `Engine` (near the
      `modified_files`-style trackers; the field is read by group 3's
      `effective_system` and reset on `NewSession`, mirroring the modified-files
      clear at `emberly-tui/src/app.rs:1639` on the engine side).
- [ ] Gate impl in `gate.rs`: a `TaskListGateImpl` mirroring `RecallGate`
      (`gate.rs:101`) / `AskGate` (:69) — sends a `TaskListSet { items, reply }`
      over an mpsc (ack via a `oneshot<()>` so the tool returns success only after
      the engine stored it; fail-closed to `Err` on disconnect, like
      `gate.rs:82/84`).
- [ ] Wire a `Some(x) = task_rx.recv() => self.on_task_list_set(...)` arm into the
      tool-exec `tokio::select!` loop (`engine.rs:1438–1472`, beside the
      `user_asks_rx` arm at :1442) **and** any idle/streaming select loops that
      must stay responsive while a tool runs. Drain pending on frontend
      disconnect (mirror :1466–1469).
- [ ] `on_task_list_set` handler: store `self.task_list = items`, **emit
      `UiEvent::TaskListUpdated { items }`** via `emit()` (`engine.rs:2205`;
      model the call on the `FileModified`/`FileDiff` emission in `finish_tool`
      :1922–1940), **write a `task_list` transcript event** via `write_transcript`
      (`engine.rs:2211`), then ack the oneshot.
- [ ] `UiEvent`: add `TaskListUpdated { items: Vec<TaskItem> }` to
      `event.rs:23` (serde `tag = "kind"`, snake_case → `"task_list_updated"`;
      `#[non_exhaustive]` keeps it non-breaking).
- [ ] `TranscriptEvent`: add a `TaskList { items: Vec<TaskItem> }` variant to
      `transcript.rs:62` (serde `tag = "type"` → `"task_list"`). **Additive —
      older readers warn-skip it, no `SCHEMA_VERSION` bump** (mirror the `AskUser`
      :154–164 / `EffortChange` :176 convention; `SCHEMA_VERSION = 2` at
      transcript.rs:25 stays). Confirm the resume reader tolerates it
      (`resume.rs:47` already skips unknown newer schema; the list is derivable
      re-reading the last `task_list` event, but resume need not reconstruct it —
      it is regenerated the next time the model calls `todo`, and pinning simply
      shows nothing until then; **record this choice in the notes log**).

## 3. Pin the task list into the sent context  *(T-11; Tech Spec §7, §8; Requirements §13 resolved)*

Pinning keeps the list in front of the model across compaction/windowing. The
cleaner seam is the **system-prompt pin path**, not the message-pin path: the
list is engine state, not a conversation turn.

- [ ] In `effective_system()` (`engine.rs:2151`) — which already appends the
      tool-call-explanation instruction onto `self.system` — append a rendered
      task-list block when `self.context.pin_task_list` is set **and** the list is
      non-empty. This is the exact precedent (`tool_explanation`) for extending
      the pinned system content; nothing in the windowed message path
      (`windowed_messages()` :2067 / `pinned_count()` :2046) needs to change.
- [ ] Add `pin_task_list: bool` (default `true`) to `ContextConfig`
      (`engine.rs:72–102`, beside `window_turns` :98). Compaction (`compact()`
      :914) already never touches the system prompt / pinned prefix, so a pinned
      task list is inherently compaction-safe — verify, don't re-implement.
- [ ] Rendered block is deterministic and small (one line per item with a text
      status token, e.g. `- [in_progress] wire the gate`) so the standing context
      cost is trivial (progressive-disclosure spirit, Tech Spec §7). No model call.

## 4. Config: `context.pin_task_list` + provenance  *(T-11; Tech Spec §8)*

All in `crates/emberly/src/config.rs`.

- [ ] Add `pin_task_list: Option<bool>` to `ContextConfigFile` (:86, beside
      `window_turns` :89 / `auto_compact` :93).
- [ ] Merge (project over global) field-by-field at :310–321 — add a
      `pin_task_list` clause mirroring the neighbours.
- [ ] Resolve into `emberly_core::ContextConfig` at :647–668 —
      `pin_task_list: merged.context.pin_task_list.unwrap_or(d.pin_task_list)`
      (`d` = `ContextConfig::default()` at :647).
- [ ] Provenance / `config show` (C-3): add a `context.pin_task_list` block to the
      provenance machinery (:519–579) for parity with the other `context.*` keys.

## 5. TUI: sidebar Tasks section + inline render + ASCII fallback  *(T-11; Design §4.7, §3.1, §7)*

- [ ] TUI state: add a `tasks: Vec<TaskItem>` field to `App`
      (`emberly-tui/src/app.rs`, beside `modified_files` :309) and handle
      `UiEvent::TaskListUpdated` in `App::apply_event` (:486; model on the
      `FileModified` arm :638 / upsert helper :671). Clear on new session (:1639).
- [ ] Sidebar **Tasks** section in `render_sidebar()` (`render.rs:532`): push lines
      after the Modified-files block (:634–658, before the final `render_widget`
      :660), using the `labeled()` helper (:664). One line per item with a status
      glyph; the single in-progress item lightly ember-accented (reuse the
      `sidebar_settling()` accent pattern at :647). Absent (no section, no stub)
      when the list is empty — mirror the modified-files `—` placeholder decision
      but per Design §3.1 the Tasks section is simply not shown when empty.
- [ ] Glyphs: rich `○` pending / `◐` in-progress / `✓` done live in
      `strings.rs` (:103–104 is where rich-mode glyphs live); the ASCII fallbacks
      `[ ]`/`[~]`/`[x]` are used in the plain frontend. **Color is never the sole
      signal** (Design §7) — the glyph/token carries status without color.
- [ ] Inline render: on `TaskListUpdated`, show a compact checklist block in the
      conversation flow (Design §4.7). Either add a `ConvItem::TaskList` variant
      (`app.rs:45`, beside `Tool` :55) with a `conversation_lines()` arm
      (`render.rs:364`), or fold it into the tool-activity render — **decide in the
      notes log** (leaning to a dedicated `ConvItem` so a completed list settles to
      an all-`✓` block rather than vanishing, per Design §4.7).
- [ ] Plain/degraded frontend (`line.rs`): render `TaskListUpdated` as an
      ASCII checklist block (append-only, no cursor repositioning) rather than
      ignoring it via the catch-all — the plain mode has no sidebar, so the inline
      block is the only surface (Design §7). Keep the
      `degraded_output_has_no_ansi_escapes` test (`line.rs:651`) green.

## 6. Tests + exit criterion  *(Tech Spec §14.7 offline, deterministic)*

- [ ] **Round-trip via `FakeProvider`:** a scripted `todo` tool call emits
      `UiEvent::TaskListUpdated` and writes a `task_list` transcript event with the
      full list (replace semantics — a second call replaces, not merges).
- [ ] **Not permission-gated:** the `todo` call never raises a
      `PermissionRequest` (assert no `authorize` path; mirror the `recall`/`ask_user`
      not-gated assertions in the §14.7 suite).
- [ ] **Pinned across compaction:** with `pin_task_list = true`, drive a
      compaction (`FakeProvider` usage numbers past the threshold) and assert the
      rendered system prompt still carries the task-list block afterwards; with
      `pin_task_list = false` it is absent from the sent context.
- [ ] **HC-7:** the `task_list` transcript event is appended, never rewrites a
      prior line; resume tolerates the event (unknown-schema path `resume.rs:47`)
      and does not crash.
- [ ] **Degraded parity:** the plain frontend renders the checklist with ASCII
      markers and no ANSI (extend/keep `line.rs:651`).
- [ ] **Exit criterion (Phase 1 done when):** a `todo` round-trip emits
      `TaskListUpdated` + a `task_list` transcript event and stays pinned across a
      compaction; the sidebar and inline renders show correct glyphs with an
      ASCII-fallback degraded-mode test; the tool is never permission-gated (Tech
      Spec §14.7, T-11). Workspace clippy-clean under the §1 lint policy; offline
      suite green.

---

## Decisions & notes log

> Fill in as the phase proceeds (mirrors the v0.1–v0.3 phase docs' logs).

- **Gate shape.** `todo` reaches the engine via a new `TaskListGate`
  (ack-only oneshot), structurally between `RecallGate` (returns data, no user
  round-trip, not gated) and `AskGate` (user round-trip). It is *not*
  permission-gated because it never calls `ctx.authorize` — the only mechanism
  that marks a tool gated (confirmed: no boolean flag exists). _Confirm on
  implementation; if the Spec should absorb the gate shape as feedback, it flows
  back as a version bump per G-24, not an edit here._
- **Pin via system prompt, not a pinned message.** The list is engine state, so
  it rides in `effective_system()` (the tool-explanation precedent) rather than
  the message pin prefix (`pinned_count`/`windowed_messages`). _Record if this
  diverges from any Spec wording so it can flow back (G-24)._
- **Resume does not reconstruct the list.** The `task_list` transcript event is
  the audit record (HC-7); on resume the pinned block simply shows nothing until
  the model next calls `todo`. Rationale: the list is model-owned working state,
  cheap to regenerate, and reconstructing it would add a resume-replay special
  case for no user-visible gain. _Revisit if a resumed session visibly losing its
  checklist proves annoying in use._
- **Inline vs. tool-activity render (group 5).** Leaning to a dedicated
  `ConvItem::TaskList` so a completed list settles to an all-`✓` block (Design
  §4.7) instead of vanishing. _Decide during group 5._
- **Open item carried in (Requirements §13 / Tech Spec §16):** whether the task
  list is pinned across compaction or may be dropped and re-read — **resolved
  here as pinned by default** (`context.pin_task_list = true`); tune with use.
