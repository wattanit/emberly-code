# Phase 1 — Completion gate (S-6) — TODO & Progress

**Milestone:** M9 phase 1 (Tech Spec §15) — the 0.4.1 cross-project feature set.
The **sibling of the S-5 loop guardrail**: S-5 stops a loop that *re-treads*, S-6
stops one that *lands early*. This phase reuses the S-5 halt mechanism
(detect → emit → park on a user command) almost verbatim, on the opposite branch
of the turn loop.
**Satisfies:** S-6; Tech Spec §7, §3.1/§3.2, §6.1, §8; Design §8.7; HC-6, HC-7;
Requirements §6, §11. Pinned to **Req v0.8 / Design v0.8 / Spec v0.9** (all
`approved`).
**Goal:** Let a session hold the agent loop to registered pass/fail checks
before it may declare a task done. On a completion attempt each registered check
runs; while any fails the loop re-opens with the failure as a tool-result (HC-6);
after `max_attempts` failed attempts the harness **halts to the user**
(keep-going / steer / stop / **finish-override**), exactly as S-5 does. A check
command runs through the **sandboxed `bash` path but un-prompted** (config
registration in a trusted root is the authorization — no privileged path around
§6). **Inert until a check is registered:** a session with no checks behaves
exactly as today. `emberly-sandbox` is untouched.

**Depends on:** the shipped product (M1–M8). Every seam this phase plugs into
already exists:
- **The S-5 guardrail to mirror** (`crates/emberly-core/src/engine.rs`):
  loop-state fields `:497-514` (init `:703-710`), `LoopConfig` tunables `:56-75`,
  `evaluate_loop()` detector `:1542-1592`, `reset_loop_window()` `:1596-1600`,
  **`await_loop_resolution()` halt-and-park `:1606-1635`**, `resolution_label()`
  `:455-460`.
- **The turn-completion hook point** — `engine.rs:1277-1279`:
  `if tool_calls.is_empty() { return; }` inside `run_turn` (loop at `:1262`).
  **This `return` is where the gate must evaluate before allowing "done".**
- **The enum trio to mirror:** `UiEvent::LoopHalted`
  (`emberly-core/src/event.rs:87-92`), `Command::ResolveLoop`
  (`.../command.rs:38-41`), `TranscriptEvent::LoopHalt` (`.../transcript.rs:144-152`,
  additive → no `SCHEMA_VERSION` bump), `enum LoopResolution{Resume,Stop,Steer}`
  (`.../types.rs:82-95`). All internally-tagged + `#[non_exhaustive]`.
- **Sandboxed bash execution** (`crates/emberly-tools/src/builtin/bash.rs`):
  `execute` `:175`, the **`ctx.authorize(...)` block to BYPASS `:182-191`**, the
  confined spawn `:193-234` via `ctx.sandbox().bash_invocation(&cmd, root)`
  (`.../sandbox.rs:26-30`), OS confinement `emberly-sandbox/src/confine.rs:106`.
- **`[loop]` config** to parallel (`crates/emberly/src/config.rs`): file struct
  `:94-101`, top-level field `:43-45`, merge `:357-366`, resolve into core
  `:936-951`, core field `:473-474`, wired at `crates/emberly/src/main.rs:547`.
  Provenance model blocks: `[truncate]` `:656-671`, `[context]` `:674-760`.
- **Tool-result reduction** to reuse for the failing-check output tail:
  `reduce_output()` (`crates/emberly-tools/src/reduce.rs:64-74`), applied in the
  pipeline at `engine.rs:2146-2157`.
- **The TUI loop-break surface** to mirror — rich: `LoopHaltPrompt`
  (`crates/emberly-tui/src/app.rs:306-321`), field `:394-396`, ingestion
  `:668-670`, keyboard gate `:821-824` + `on_loop_halt_key` `:1289-1335`,
  `resolve_loop()` `:1337-1339`; render `render.rs:1502-1569`, footer hint
  `:1209-1213`; strings `strings.rs:55-70`. Plain: `line.rs:119`,
  `render_loop_halt` `:280-285`, `parse_loop_resolution` `:480-490`, blocking
  input `:573-603`.

**Key structural facts (from the codebase):**
1. **Un-prompted ≠ uncontained.** Confinement lives entirely in
   `sandbox().bash_invocation()` / `confine.rs`, *independent* of the permission
   gate. A gate-check command reuses that spawn path (`bash.rs:193-234`) while
   simply **not** calling `ctx.authorize` (skip `bash.rs:182-191`) — so it is
   sandboxed exactly like `bash` but never raises a `PermissionRequest`. This is
   the whole meaning of "no privileged path around §6" (S-6 honesty clause).
2. **The gate hooks the branch S-5 ignores.** `evaluate_loop()` deliberately
   returns `None` for a no-tool-call turn (`engine.rs:1548`) and S-5 halts *after*
   tools ran (`:1296-1311`). S-6 is the mirror image: it fires on the
   `tool_calls.is_empty()` branch (`:1277-1278`), the natural terminate point.
3. **`[loop]` has no `config show` provenance** (a pre-existing gap — only the
   `rename` at config.rs:44 and a test reference it). **Do not copy that gap** —
   add a real provenance `record(...)` block for `[completion]`, modeled on
   `[truncate]`/`[context]`.

**Legend:** `[ ]` todo · `[~]` in progress · `[x]` done · `[!]` blocked

---

## Progress summary

| Group | Status | Notes |
|---|---|---|
| 1. `CompletionCheck` + `CompletionConfig` types + registry field (`emberly-core`) | [x] | mirror `LoopConfig` (`engine.rs:56-75`); `Vec<CompletionCheck>` populated at startup |
| 2. Enum trio + `CheckResult`/`GateResolution` types (`event`/`command`/`transcript`/`types`) | [x] | `CompletionGateHalted`, `ResolveCompletionGate`, `completion_check`+`completion_gate_halt` (additive, no SCHEMA bump) |
| 3. Sandboxed un-prompted check execution (`emberly-core` + reuse `bash_invocation`) | [x] | reuse `sandbox().bash_invocation`; **never** `ctx.authorize`; reduce output tail via `reduce_output("bash", …)` |
| 4. Gate evaluation on the completion attempt + failed-attempt counter (`engine.rs:1277`) | [x] | run checks; any fail → append tool-result (HC-6) + re-open loop; pass → terminate |
| 5. Bounded-attempt halt + `await_completion_resolution` (mirror `await_loop_resolution`) | [x] | `resume`/`steer`/`stop`/`finish`; `finish` writes `override: true` |
| 6. Config: `[completion]` + `[[completion.check]]` + **provenance** (`emberly`) | [x] | file struct + merge + resolve + `config show` record (don't repeat the `[loop]` gap) |
| 7. TUI: completion-gate halt surface (rich + plain) + sidebar gate status | [x] | mirror `LoopHaltPrompt`; 4th "finish anyway" choice; status shown only when checks registered |
| 8. Tests (offline, deterministic — §14.8) + exit criterion | [x] | fail re-opens, pass terminates, contained-not-prompted, halt+resolutions, override, inert gate |

**Overall Phase 1: DONE (2026-07-15).** All 8 groups complete. Workspace
builds clean, `cargo clippy --workspace --all-targets` is fully clean (zero
warnings), `cargo fmt --check` is clean, and the offline `FakeProvider` suite
is green: `emberly-core` (`engine_loop.rs`, 108 tests, up from 100 pre-phase),
`emberly` (4 new `[completion]` config tests), and `emberly-tui` (16 new
rich + degraded completion-gate tests) all pass.

---

## 1. `CompletionCheck` + config types + registry field  *(S-6; Tech Spec §7)* — DONE

Establish the types and the per-session registry the engine evaluates.

- [x] In `emberly-core` (beside `LoopConfig` at `engine.rs:56-75`) define
      `struct CompletionCheck { name: String, command: String, expect_exit: i32 }`
      and `struct CompletionConfig { enabled: bool, max_attempts: usize }` with a
      `Default` (`enabled: true`, `max_attempts: 3`). `#[derive(Clone, Debug,
      Serialize, Deserialize)]`. **Defaults live in core**, matching the
      `LoopConfig` convention (config supplies `Option`s, core owns the default).
- [x] Engine holds the registry: add `completion_config: CompletionConfig`,
      `completion_checks: Vec<CompletionCheck>`, and a per-session
      `completion_attempts: usize` counter beside the loop-state fields
      (`engine.rs:497-514`); init in the constructor (`:703-710`) — checks
      populated from config at startup, `completion_attempts = 0`.
- [x] **Registration hook (designed-for, Requirements S-6).** Expose a small
      `register_completion_check(&mut self, CompletionCheck)` on the engine so a
      future frontend/tool can register programmatically; config is the shipped
      caller. Do **not** build a frontend registrant now — just leave the seam
      (note it in the log).
- [x] Reset `completion_attempts = 0` on `NewSession` and whenever the gate
      passes or the loop makes genuine progress on a re-opened attempt (mirror the
      `reset_loop_window()` discipline at `engine.rs:1596-1600`). *(The
      gate-passes/re-opened-attempt reset lands with group 4's
      `evaluate_completion_gate()`; only the `NewSession`/constructor reset is
      in scope here.)*

## 2. Enum trio + result/resolution types  *(S-6; Tech Spec §3.1/§3.2)* — DONE

Additive variants on the three `#[non_exhaustive]` boundary enums; mirror the
`LoopHalted`/`ResolveLoop`/`LoopHalt`/`LoopResolution` set exactly.

- [x] `types.rs` (beside `LoopResolution` `:82-95`): `struct CheckResult { name:
      String, passed: bool, reason: String }` and `enum GateResolution { Resume,
      Stop, Steer(String), Finish }` — `#[serde(rename_all = "snake_case")]`.
      `Finish` is the **user override** (Design §8.7).
- [x] `event.rs` (beside `LoopHalted` `:87-92`): `UiEvent::CompletionGateHalted {
      failing: Vec<CheckResult>, attempts: usize }` (serde `tag = "kind"` →
      `"completion_gate_halted"`).
- [x] `command.rs` (beside `ResolveLoop` `:38-41`): `Command::ResolveCompletionGate
      { resolution: GateResolution }` (serde `tag = "kind"`). Add the idle-time
      no-op arm next to the `ResolveLoop` stray (`engine.rs:883-886`).
- [x] `transcript.rs` (beside `LoopHalt` `:144-152`, tag key is `"type"`): add
      **two** variants — `CompletionCheck { name, passed, reason }` (one per
      evaluation) and `CompletionGateHalt { failing: Vec<CheckResult>, attempts,
      resolution: Option<String>, #[serde(default)] override_finish: bool }`.
      **Additive — older readers warn-skip, `SCHEMA_VERSION` stays** (copy the
      `LoopHalt` comment + `skip_serializing_if` convention). Confirm the resume
      reader tolerates them (unknown-schema path already skips).

## 3. Sandboxed, un-prompted check execution  *(S-6 honesty clause; Tech Spec §7, §6.1)* — DONE

A check command runs through the identical confined spawn as `bash`, but never
touches the permission gate.

- [x] Add an engine helper `run_completion_check(&self, &CompletionCheck) ->
      CheckResult` in `emberly-core`. It builds the invocation via
      `self.sandbox()` `.bash_invocation(&check.command, root)` and spawns through
      the **same** `tokio::process` path as `bash.rs:193-234` (env_clear +
      allowlist, `process_group(0)`, group-kill guard, `timeout(...)`). **It must
      not construct or route a `PermissionRequest`** — this is the deliberate
      divergence from `bash.rs:182-191`.
- [x] `passed = (exit_code == check.expect_exit)`. On fail, build `reason` from the
      exit status + a **reduced tail** of stdout/stderr via
      `reduce_output("bash", &combined)` (`reduce.rs:64-74`) then the size backstop
      — the same reduction the tool pipeline uses (`engine.rs:2146-2157`), so a
      noisy test log does not flood the re-opened turn.
- [x] Uses the same timeout policy as `bash` (`S-4`); a check that hangs is killed
      by process group exactly as a tool child is. Confinement unavailable
      (degraded, §6.5) does not special-case the gate — the check simply runs under
      whatever the active sandbox status is, like any command.

## 4. Gate evaluation on the completion attempt  *(S-6; Tech Spec §7, HC-6)* — DONE

Hook the natural terminate point; re-open the loop on failure.

- [x] At the completion branch (`engine.rs:1277-1279`), before the `return`,
      insert a call to a new `evaluate_completion_gate()`. **If
      `completion_checks` is empty → return as today** (the gate is inert; zero
      behavior change — assert this in tests).
- [x] `evaluate_completion_gate()` runs every registered check (group 3). If all
      pass → write each result as a `CompletionCheck` transcript event, reset
      `completion_attempts`, allow the `return` (loop terminates).
- [x] If any fails → increment `completion_attempts`; write the `CompletionCheck`
      events; **append the failing results to the conversation as a
      tool-result-shaped assistant-visible message** (HC-6, agent-world per Design
      §8.7) so the model reads the failures and can fix them; then **continue the
      turn loop** (do not `return`) so the model gets another turn — unless the
      attempt cap is hit (group 5). The re-open path must leave a clean message
      boundary (every `tool_use` has its `tool_result`), like compaction's
      boundary rule.

## 5. Bounded-attempt halt + await resolution  *(S-6; Tech Spec §7, §3)* — DONE

Mirror `await_loop_resolution` (`engine.rs:1606-1635`) for the gate.

- [x] When `completion_attempts >= completion_config.max_attempts`, call a new
      `await_completion_resolution(failing, attempts)`: emit
      `UiEvent::CompletionGateHalted { failing, attempts }`, then park on
      `commands_rx.recv()` for `Command::ResolveCompletionGate` — handling
      `SetMode`/`Compact` inline while parked and treating `Cancel`/channel-closed
      as `Stop` (fail-safe, never spins unattended), exactly as
      `await_loop_resolution` does.
- [x] Map the resolution: `Resume` → reset `completion_attempts`, continue the loop
      (try again); `Steer(text)` → inject the steer as a user turn and reset the
      counter (mirror the S-5 steer path); `Stop` → end the turn/session per the
      S-5 `Stop` behavior; **`Finish` → allow termination as "done" over the red
      gate** (the override the owner approved). Write one `CompletionGateHalt`
      transcript event with the resolution label and `override_finish: true` iff
      `Finish`.
- [x] `resolution_label()`-style helper for the four variants (extend/mirror
      `engine.rs:455-460`).

## 6. Config: `[completion]` + `[[completion.check]]` + provenance  *(S-6; Tech Spec §8)* — DONE

All in `crates/emberly/src/config.rs`, paralleling `[loop]` — **but add the
provenance block `[loop]` is missing.**

- [x] File struct: `struct CompletionConfigFile { enabled: Option<bool>,
      max_attempts: Option<usize>, #[serde(default)] check: Vec<CompletionCheckFile> }`
      and `struct CompletionCheckFile { name: String, command: String, expect_exit:
      Option<i32> }`. Top-level `#[serde(default)] pub completion: CompletionConfigFile`
      beside `loop_` (`:43-45`). `[[completion.check]]` parses as the `check` Vec.
- [x] Merge (project over global) at the `merge` fn (`:335`), a `[completion]`
      block modeled on the `[loop]` merge (`:357-366`): scalar fields overwrite if
      `higher.*.is_some()`; **decide check-list merge semantics — project replaces
      vs. concatenates** (lean: project replaces the whole list, mirroring "project
      wins per key"; record the choice in the log).
- [x] Resolve into `emberly_core::CompletionConfig` + the `Vec<CompletionCheck>`
      at the resolve site (`:936-951` neighbourhood): `enabled`/`max_attempts`
      `unwrap_or(default)`; map each `CompletionCheckFile` to a `CompletionCheck`
      with `expect_exit.unwrap_or(0)`. Add the core-config fields beside
      `loop_config` (`:473-474`) and wire them to the engine in `main.rs` (beside
      `:547`), calling `register_completion_check` for each.
- [x] **Provenance / `config show` (C-3):** add a `record(...)` block for
      `completion.enabled` / `completion.max_attempts` (and a count line for
      registered checks), modeled on the `[truncate]` block (`:656-671`). This is
      the gap the `[loop]` table left open — close it here for `[completion]`.

## 7. TUI: completion-gate halt surface + sidebar status  *(S-6; Design §8.7, §7)* — DONE

Mirror the loop-break surface; add the "finish anyway" choice and the
registered-checks status line.

- [x] **Rich (`emberly-tui`):** a `CompletionGatePrompt` mirroring `LoopHaltPrompt`
      (`app.rs:306-321`), field `pending_completion_gate` (`:394-396`), ingestion
      of `UiEvent::CompletionGateHalted` (`:668-670`), keyboard-ownership gate
      (`:821-824`) + an `on_completion_gate_key` handler modeled on
      `on_loop_halt_key` (`:1289-1335`) with **four** choices: `g` keep-going, `s`
      stop, `t`/Enter steer, **`f` finish-anyway**. `resolve_completion_gate()`
      emits the command (`:1337-1339` analog). Add to the "decision open" predicate
      (`:965-971`).
- [x] Rendering: `render_completion_gate` modeled on `render_loop_halt`
      (`render.rs:1502-1569`) — harness-voice block naming the failing checks and
      attempt count, the four choices (or steer field), footer hint (`:1209-1213`).
      **"Finish anyway" labeled as an override**, never "checks passed" (Design
      §8.7). Strings: a `mod completion_gate` beside `mod loop_halt`
      (`strings.rs:55-70`).
- [x] **Plain/degraded (`line.rs`):** `UiEvent::CompletionGateHalted` →
      `render_completion_gate` (mirror `:280-285`), a `parse_gate_resolution`
      (mirror `:480-490`; `finish`/`f`/`4` → Finish), and the blocking-input arm
      (mirror `:573-603`) emitting `Command::ResolveCompletionGate`. Capitalized
      deliberate choices; keep the no-ANSI degraded test green.
- [x] **Sidebar gate status (Design §8.7, §3.1):** when checks are registered,
      show a dim line naming them and their last result (pass/fail), beside the
      sandbox/context status. **Absent entirely when no checks are registered** —
      no "None" stub (mirror the Tasks/Memory "shown only when present" rule). Feed
      it from the `CompletionCheck` results the engine already emits/records (a
      small `UiEvent` carrying last-results, or fold into an existing status event —
      **decide in the log**).

## 8. Tests + exit criterion  *(Tech Spec §14.8 offline, deterministic)* — DONE

Mirror the S-5 loop tests and the §14.8 gate coverage. All offline via
`FakeProvider`.

- [x] **Fail re-opens, pass terminates:** a `FakeProvider` script that attempts
      completion with a registered check failing asserts the failure is appended as
      a tool-result and the loop re-opens; a passing check lets the turn terminate
      normally.
- [x] **Contained but not prompted:** a check command runs through the sandboxed
      `bash` path and **never** raises a `PermissionRequest` (assert no `authorize`
      call on the check path — the divergence from `bash.rs:182-191`).
- [x] **Bounded halt + resolutions:** drive `max_attempts` failures and assert
      `CompletionGateHalted` fires; each of `resume`/`steer`/`stop`/`finish`
      behaves correctly; `finish` writes a `CompletionGateHalt` transcript event
      with `override_finish: true`.
- [x] **Inert gate:** with no registered checks, loop termination is byte-for-byte
      unchanged from today (no gate evaluation, no events) — the "behaves exactly as
      today" guarantee (S-6).
- [x] **HC-7:** `CompletionCheck` / `CompletionGateHalt` events are appended, never
      rewrite a prior line; resume tolerates them.
- [x] **Degraded parity:** the plain frontend renders the halt with ASCII,
      capitalized choices, and no ANSI.
- [x] **Exit criterion (Phase 1 done when):** the failing/passing/contained/
      bounded-halt/override/inert behaviors above all pass (Tech Spec §14.8, S-6);
      workspace clippy-clean under the §1 lint policy; offline suite green.

---

## Decisions & notes log

> Fill in as the phase proceeds (mirrors the v0.1–v0.4 phase docs' logs).

- **Un-prompted but sandboxed.** Gate checks reuse `sandbox().bash_invocation()`
  + the confined spawn (`bash.rs:193-234`) while skipping the `ctx.authorize`
  block (`bash.rs:182-191`). Containment is in `bash_invocation`/`confine.rs`,
  independent of the permission gate — so a check is contained exactly like
  `bash` yet never prompts. This is the concrete realization of S-6's "no
  privileged path around §6." _If the Spec should absorb the exact execution
  shape as feedback, it flows back as a version bump (G-24), not an edit here._
- **Gate hooks the no-tool-call branch.** The gate fires at `engine.rs:1277-1278`
  (the natural terminate point), the mirror of S-5 which halts after tools ran
  (`:1296`) and ignores the no-tool branch (`:1548`). _Confirm the re-open leaves
  a clean tool_use/tool_result boundary._
- **Check-list merge semantics (group 6).** Leaning to **project replaces** the
  whole `[[completion.check]]` list (mirrors "project wins per key") rather than
  concatenating global + project. _Decide during group 6; record here._
- **Close the `[loop]` provenance gap for `[completion]`.** `[loop]` merges and
  resolves but is never surfaced in `config show` (a pre-existing gap). Do **not**
  copy it — `[completion]` gets a real `record(...)` block. _Optionally flag the
  `[loop]` gap to the owner as a separate cleanup; out of scope for this phase._
- **Sidebar gate-status event (group 7).** Whether to add a dedicated
  `UiEvent` carrying last check-results for the sidebar, or fold it into an
  existing status event. _Decide during group 7._
- **`finish` = override.** Owner decision (2026-07-15): the user may end a task as
  done over a still-failing gate; recorded with `override_finish: true`, never
  presented as "checks passed" (Design §8.7). The gate binds the model's claim of
  done, never the user's authority (S-6).
- **Registration hook left unused.** The engine exposes
  `register_completion_check` for a future frontend/tool registrant (Requirements
  S-6), but only config drives it this phase. _Open item carried in (Tech Spec
  §16 v0.9): validate the hook against a real non-config registrant when one
  exists._
- **Group 1 done (2026-07-15).** `CompletionCheck`/`CompletionConfig` added
  beside `LoopConfig` (`engine.rs`), `#[derive(Clone, Debug, Serialize,
  Deserialize)]` as specified — no current caller serializes them, but the seam
  matches the registration hook's future non-config use. `EngineConfig` gained
  `completion_config`/`completion_checks`; both of its literal construction
  sites (`emberly/src/main.rs`, `emberly-core/tests/engine_loop.rs`) were
  updated — `main.rs` wires `CompletionConfig::default()` + an empty check list
  as a **placeholder** until group 6 resolves them from `[completion]` config.
  `completion_attempts` inits to 0 in the constructor and resets to 0 in
  `adopt_session` (the `NewSession`/`/resume` reset point — mirrors
  `next_permission_id = 0` there, not `reset_loop_window()`, since loop-state
  itself isn't reset on session switch today, a pre-existing S-5 fact this
  phase doesn't touch). Workspace builds and the full test suite (`cargo test
  --workspace`) is green; `cargo clippy --workspace --all-targets` shows exactly
  one expected transitional warning (`completion_config` unread) that clears
  once group 4 reads it.
- **Group 2 done (2026-07-15).** `CheckResult`/`GateResolution` added beside
  `LoopResolution` in `types.rs`; `UiEvent::CompletionGateHalted`,
  `Command::ResolveCompletionGate`, and the `CompletionCheck`/
  `CompletionGateHalt` transcript variants added beside their S-5 mirrors —
  all additive, `SCHEMA_VERSION` untouched. Added the idle-time no-op arm for
  `ResolveCompletionGate` next to `ResolveLoop`'s in `engine.rs`'s top-level
  command dispatch. None of these are constructed or matched-on anywhere yet
  (group 3 builds the check runner that produces a `CheckResult`; group 4/5
  wire the emit/park/transcript-write sites) — `#[non_exhaustive]` +
  non-exhaustive `if let`/match patterns elsewhere mean nothing needed
  updating to accept the new variants. Workspace builds and the full test
  suite is green; `cargo clippy --workspace --all-targets` still shows only
  the one group-1 transitional warning, no new ones.
- **Group 3 done (2026-07-15).** `Engine::run_completion_check` added
  (`engine.rs`, beside `await_loop_resolution`), spawning through
  `self.sandbox_spawn.bash_invocation()` — the same field the `bash` tool
  reaches via `ctx.sandbox()` — with the identical env-scrub/process-group/
  timeout/group-kill shape as `bash.rs`, and **no `PermissionRequest` is ever
  constructed on this path** (confirmed by inspection; a `PermissionRequest`
  round-trips through `ctx.authorize`/the gate channels, neither of which
  `run_completion_check` touches — group 8 adds the assertion test). Failure
  `reason` runs the exact `reduce_output("bash", …)` → `truncate_output(…,
  &self.truncate)` pipeline `ingest_tool_result` uses. Timeout uses
  `emberly_tools::DEFAULT_TIMEOUT_SECS` (120s, `bash`'s own default) — a
  `CompletionCheck` has no per-check timeout field in its config shape, so
  "same timeout policy as bash" here means the same enforcement mechanism
  *and* the same default ceiling, not a configurable one. Extracted
  `DEFAULT_ENV_ALLOWLIST`/`DEFAULT_TIMEOUT_SECS` as `pub const`s in
  `emberly-tools::builtin::bash` (both re-exported from the crate root) so
  `BashTool::default()` and the gate's check runner read one source of truth
  instead of two copies of the same literals. The group-kill guard is
  duplicated as a small private `CompletionCheckKillGuard` in `engine.rs`
  rather than reused from `bash.rs`, since that struct is private to
  `emberly-tools::builtin::bash` and widening its visibility across the crate
  boundary for a ~15-line guard wasn't worth the surface; added `rustix` (an
  already-workspace-vetted, already-used-elsewhere dependency) to
  `emberly-core`'s `Cargo.toml` for this. No caller wires this method into the
  turn loop yet — group 4 hooks it at the `tool_calls.is_empty()` branch and a
  real fail/pass round-trip test lands in group 8, per the phase's own
  structure; testing a private method with no caller isn't reachable from the
  integration-test file today. Workspace builds and the full test suite is
  green; `cargo clippy --workspace --all-targets` shows the group-1 warning
  plus three new, equally transitional dead-code warnings on the unused
  `run_completion_check`/`CompletionCheckKillGuard` — all four clear once
  group 4 lands.
- **Group 4 done (2026-07-15).** The `tool_calls.is_empty()` branch
  (`engine.rs`) now calls `evaluate_completion_gate()` and either `return`s
  (`CompletionGateOutcome::Terminate`) or `continue`s the enclosing `run_turn`
  loop (`CompletionGateOutcome::ReOpen`) instead of always returning.
  `evaluate_completion_gate` runs every registered check, writes one
  `CompletionCheck` transcript event per evaluation (win or lose, HC-7), and
  on any failure increments `completion_attempts` and pushes
  `Message::user_text(render_gate_failure(&failing))` — a synthetic user-role
  message formatted like ordinary tool-result content (`"<name>: <reason>"`
  per failing check), since there is no real `tool_use` this turn to pair a
  genuine `ContentBlock::ToolResult` against; this is what "tool-result-shaped"
  means in practice (Design §8.7's "ordinary tool-result content"). No special
  message-boundary handling was needed: a no-tool-call turn already has no
  dangling `tool_use`, so appending the next message is inherently a clean
  boundary. The attempt-cap check is deliberately absent — `evaluate_loop`-style,
  group 5 will intercept before the `ReOpen` path once `completion_attempts >=
  max_attempts`. Added two integration tests
  (`tests/engine_loop.rs`): `completion_gate_inert_with_no_checks_registered`
  (byte-for-byte parity — no `completion_check` events, normal termination) and
  `completion_gate_failing_check_reopens_then_passing_check_terminates` (a
  marker-file check that fails once then passes, driving both halves of the
  round trip through one `FakeProvider` script — asserts both scripted turns
  ran, exactly 2 `completion_check` transcript events with the right
  pass/fail order, and the model got its second turn after the reopen).
  Workspace builds and the full test suite (102 tests in `engine_loop.rs`, up
  from 100) is green; `cargo clippy --workspace --all-targets` is back down to
  the single group-1 transitional warning (`completion_config` unread —
  group 5's `max_attempts` check clears it).
- **Group 5 done (2026-07-15).** `evaluate_completion_gate` now takes
  `commands_rx`; once a failing attempt pushes `completion_attempts` to
  `completion_config.max_attempts` it calls the new
  `await_completion_resolution` (mirrors `await_loop_resolution` exactly:
  emit, park on `commands_rx.recv()`, `SetMode`/`Compact` handled inline,
  `Cancel`/channel-closed → `Stop`) and maps the four `GateResolution`
  variants — `Resume`/`Steer` reset `completion_attempts` and return `ReOpen`
  (mirroring the S-5 steer path exactly, including `record_user_message` for
  `Steer`); `Stop`/`Finish` both return `Terminate`, distinguished only by the
  `override_finish` flag on the `CompletionGateHalt` transcript event (the
  loop doesn't need to tell them apart — only the audit record does). Added
  `gate_resolution_label` beside `resolution_label`. This closes out the
  `completion_config` field, so **the workspace now builds with zero clippy
  warnings** (the transitional ones from groups 1/3 are gone). Added five more
  integration tests mirroring the existing S-5 halt tests
  (`spawn_with_gate`/`drive_resolving_gate`, analogs of
  `spawn_with_loop`/`drive_resolving_loop`): `completion_gate_halts_once_then_
  resume_tries_again` (a counter-file check that fails twice then passes —
  proves the halt fires exactly once, not on every failed attempt, and resume
  lets the model earn a real pass), `completion_gate_stop_ends_the_turn`,
  `completion_gate_finish_overrides_the_red_gate` (asserts
  `override_finish: true` is recorded), `completion_gate_steer_injects_a_
  message_and_tries_again`, and — pulling one of group 8's own exit-criterion
  items forward since it was cheap to prove right now —
  `completion_gate_check_execution_never_prompts` (these tests' default rules
  ask on every ordinary `bash` call; a check registered under `max_attempts: 1`
  still raises zero `PermissionRequest`s, confirming the S-6 honesty-clause
  divergence end-to-end rather than by inspection alone). Workspace builds
  clean; `engine_loop.rs` is up to 107 tests (from 102); `cargo clippy
  --workspace --all-targets` is fully clean, zero warnings.
- **Group 6 done (2026-07-15).** `CompletionConfigFile`/`CompletionCheckFile`
  added beside `LoopConfig` in `emberly/src/config.rs`; top-level `completion`
  field beside `loop_`. **Check-list merge semantics — decided: project
  replaces the whole list** (mirrors "project wins per key"; a project that
  wants the global checks too repeats them), confirmed by
  `completion_check_list_replaces_rather_than_concatenates_on_merge`; scalar
  fields (`enabled`/`max_attempts`) merge normally, overwriting only when the
  higher tier sets them. Resolved into `emberly_core::CompletionConfig` +
  `Vec<CompletionCheck>` (`expect_exit.unwrap_or(0)`) beside `loop_config` in
  `Resolved`, and wired into `main.rs`'s `EngineConfig` literal — replacing
  group 1's `CompletionConfig::default()`/empty-list placeholder with the real
  resolved values (`register_completion_check` stays the seam for a future
  non-config registrant; the shipped path threads checks through
  `EngineConfig.completion_checks` directly, same as every other resolved
  config field — no behavior difference, simpler than a second registration
  path for the one caller that exists). **Closed the `[loop]` provenance gap
  for `[completion]`** (a pre-existing gap on `[loop]`, left as-is — flagging
  it is a separate cleanup, out of scope here): `completion.enabled` and
  `completion.max_attempts` each get a `record(...)` block on deviation,
  modeled on `[truncate]`'s, plus a `completion.check` count line ("N
  registered") whenever any check is registered, regardless of whether
  `max_attempts` deviates. Added 4 unit tests in `config.rs` (parse+merge
  scalars, list-replaces-not-concatenates, `load()`-level resolve +
  provenance with a project check registered, and the fully-unconfigured
  inert-default case with zero completion provenance noise). Workspace builds
  clean; `cargo clippy --workspace --all-targets` stays fully clean.
- **Group 7 done (2026-07-15).** Rich TUI: `CompletionGatePrompt` beside
  `LoopHaltPrompt` (`app.rs`) with `failing`/`attempts`/`steering`/`editor`;
  `pending_completion_gate` field, `UiEvent::CompletionGateHalted` ingestion,
  keyboard-ownership gate, `on_completion_gate_key` (4 choices: `g`/`s`/`t`
  or Enter/`f`), `resolve_completion_gate()`, and the `is_deciding()` predicate
  all mirror their S-5 counterparts exactly. `render_completion_gate`
  (`render.rs`) mirrors `render_loop_halt` — harness-voice block, failing
  checks + attempt count, four choices, footer hint wired into both the
  overlay dispatch in `frame()` and the status-bar hint in `render_status`.
  `mod completion_gate` added to `strings.rs` beside `mod loop_halt`, with
  `FINISH_ANYWAY = "finish anyway (override)"` — never phrased as success.
  Plain/degraded (`line.rs`): `render_completion_gate`, `parse_gate_resolution`
  (`finish`/`f`/`finish anyway`/`4` → `Finish`), a `Pending::CompletionGate`
  variant, and the blocking-input arm all mirror the loop-halt equivalents;
  plain output stays ASCII/no-ANSI throughout (inherent to the whole file).
  **Sidebar gate status — decided:** a new `UiEvent::CompletionStatus {
  checks: Vec<CheckResult> }`, emitted from `evaluate_completion_gate` after
  every evaluation (win or lose) alongside the `completion_check` transcript
  writes — not folded into an existing status event, since neither
  `MemoryStatus` nor `SkillsAvailable`'s shape fits a list of named pass/fail
  results. **Decision: the line is absent until the gate has run at least
  once** (there is no "registered but not yet run" rendering — a check has
  no meaningful pass/fail before its first evaluation, and inventing one risks
  the honesty clause); `completion_status` clears on session switch like
  `memory_user`/`skills`. Rendered as `gate_status_line` beside `sandbox_line`
  in the sidebar (not down with Tasks/Memory/Skills — Design §8.7 places it
  "alongside sandbox and context status"), warning-toned (not error) on any
  fail, mirroring `sandbox_line`'s partial/unavailable styling. Added 7 rich
  `app.rs` tests (mirroring every `loop_halt_*` test) + 1 status test, and 2
  degraded `line.rs` tests (harness-voice render + `parse_gate_resolution`
  coverage). Workspace builds clean; `cargo clippy --workspace --all-targets`
  stays fully clean.
- **Group 8 done (2026-07-15).** Most of the exit-criterion coverage already
  landed inline with groups 4–5 (fail-reopens/pass-terminates, inert gate,
  contained-not-prompted, and all four bounded-halt resolutions each got a
  dedicated test as its mechanism was built, rather than batched at the end).
  This group added the two pieces that specifically needed the full surface
  in place: **HC-7** — `completion_gate_transcript_events_are_additive_for_
  resume` (mirrors `todo_transcript_event_is_additive_for_resume`): a real
  `FileTranscript`-backed session hits a bounded halt, and
  `emberly_core::resume::read_records` reads the `completion_check`/
  `completion_gate_halt` records back without error (the resume reader's
  generic `_ => {}` fallthrough for view-reconstruction already tolerated
  them, same as `LoopHalt` — confirmed rather than assumed). **Degraded
  parity** — the group 7 `line.rs` tests confirm ASCII, no ANSI, ordinary
  (non-bracketed-single-letter) full-word choices, and never "checks passed"
  phrasing. **Exit criterion:** ran `cargo build --workspace` (clean),
  `cargo test --workspace` (every crate green — 108 tests in `engine_loop.rs`,
  4 in `emberly`'s config tests, 16 new in `emberly-tui`), `cargo clippy
  --workspace --all-targets` (zero warnings, satisfying the §1 lint policy —
  no `unwrap`/`expect`, no stray `too_many_arguments`), and `cargo fmt --check`
  (clean, after one `cargo fmt` pass mid-phase — safe here since every
  modified file was this phase's own work, not a concurrent merge). **Phase 1
  is DONE.**
