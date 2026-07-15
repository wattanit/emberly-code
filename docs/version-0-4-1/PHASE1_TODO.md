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
| 1. `CompletionCheck` + `CompletionConfig` types + registry field (`emberly-core`) | [ ] | mirror `LoopConfig` (`engine.rs:56-75`); `Vec<CompletionCheck>` populated at startup |
| 2. Enum trio + `CheckResult`/`GateResolution` types (`event`/`command`/`transcript`/`types`) | [ ] | `CompletionGateHalted`, `ResolveCompletionGate`, `completion_check`+`completion_gate_halt` (additive, no SCHEMA bump) |
| 3. Sandboxed un-prompted check execution (`emberly-core` + reuse `bash_invocation`) | [ ] | reuse `sandbox().bash_invocation`; **never** `ctx.authorize`; reduce output tail via `reduce_output("bash", …)` |
| 4. Gate evaluation on the completion attempt + failed-attempt counter (`engine.rs:1277`) | [ ] | run checks; any fail → append tool-result (HC-6) + re-open loop; pass → terminate |
| 5. Bounded-attempt halt + `await_completion_resolution` (mirror `await_loop_resolution`) | [ ] | `resume`/`steer`/`stop`/`finish`; `finish` writes `override: true` |
| 6. Config: `[completion]` + `[[completion.check]]` + **provenance** (`emberly`) | [ ] | file struct + merge + resolve + `config show` record (don't repeat the `[loop]` gap) |
| 7. TUI: completion-gate halt surface (rich + plain) + sidebar gate status | [ ] | mirror `LoopHaltPrompt`; 4th "finish anyway" choice; status shown only when checks registered |
| 8. Tests (offline, deterministic — §14.8) + exit criterion | [ ] | fail re-opens, pass terminates, contained-not-prompted, halt+resolutions, override, inert gate |

**Overall Phase 1: NOT STARTED.**

---

## 1. `CompletionCheck` + config types + registry field  *(S-6; Tech Spec §7)*

Establish the types and the per-session registry the engine evaluates.

- [ ] In `emberly-core` (beside `LoopConfig` at `engine.rs:56-75`) define
      `struct CompletionCheck { name: String, command: String, expect_exit: i32 }`
      and `struct CompletionConfig { enabled: bool, max_attempts: usize }` with a
      `Default` (`enabled: true`, `max_attempts: 3`). `#[derive(Clone, Debug,
      Serialize, Deserialize)]`. **Defaults live in core**, matching the
      `LoopConfig` convention (config supplies `Option`s, core owns the default).
- [ ] Engine holds the registry: add `completion_config: CompletionConfig`,
      `completion_checks: Vec<CompletionCheck>`, and a per-session
      `completion_attempts: usize` counter beside the loop-state fields
      (`engine.rs:497-514`); init in the constructor (`:703-710`) — checks
      populated from config at startup, `completion_attempts = 0`.
- [ ] **Registration hook (designed-for, Requirements S-6).** Expose a small
      `register_completion_check(&mut self, CompletionCheck)` on the engine so a
      future frontend/tool can register programmatically; config is the shipped
      caller. Do **not** build a frontend registrant now — just leave the seam
      (note it in the log).
- [ ] Reset `completion_attempts = 0` on `NewSession` and whenever the gate
      passes or the loop makes genuine progress on a re-opened attempt (mirror the
      `reset_loop_window()` discipline at `engine.rs:1596-1600`).

## 2. Enum trio + result/resolution types  *(S-6; Tech Spec §3.1/§3.2)*

Additive variants on the three `#[non_exhaustive]` boundary enums; mirror the
`LoopHalted`/`ResolveLoop`/`LoopHalt`/`LoopResolution` set exactly.

- [ ] `types.rs` (beside `LoopResolution` `:82-95`): `struct CheckResult { name:
      String, passed: bool, reason: String }` and `enum GateResolution { Resume,
      Stop, Steer(String), Finish }` — `#[serde(rename_all = "snake_case")]`.
      `Finish` is the **user override** (Design §8.7).
- [ ] `event.rs` (beside `LoopHalted` `:87-92`): `UiEvent::CompletionGateHalted {
      failing: Vec<CheckResult>, attempts: usize }` (serde `tag = "kind"` →
      `"completion_gate_halted"`).
- [ ] `command.rs` (beside `ResolveLoop` `:38-41`): `Command::ResolveCompletionGate
      { resolution: GateResolution }` (serde `tag = "kind"`). Add the idle-time
      no-op arm next to the `ResolveLoop` stray (`engine.rs:883-886`).
- [ ] `transcript.rs` (beside `LoopHalt` `:144-152`, tag key is `"type"`): add
      **two** variants — `CompletionCheck { name, passed, reason }` (one per
      evaluation) and `CompletionGateHalt { failing: Vec<CheckResult>, attempts,
      resolution: Option<String>, #[serde(default)] override_finish: bool }`.
      **Additive — older readers warn-skip, `SCHEMA_VERSION` stays** (copy the
      `LoopHalt` comment + `skip_serializing_if` convention). Confirm the resume
      reader tolerates them (unknown-schema path already skips).

## 3. Sandboxed, un-prompted check execution  *(S-6 honesty clause; Tech Spec §7, §6.1)*

A check command runs through the identical confined spawn as `bash`, but never
touches the permission gate.

- [ ] Add an engine helper `run_completion_check(&self, &CompletionCheck) ->
      CheckResult` in `emberly-core`. It builds the invocation via
      `self.sandbox()` `.bash_invocation(&check.command, root)` and spawns through
      the **same** `tokio::process` path as `bash.rs:193-234` (env_clear +
      allowlist, `process_group(0)`, group-kill guard, `timeout(...)`). **It must
      not construct or route a `PermissionRequest`** — this is the deliberate
      divergence from `bash.rs:182-191`.
- [ ] `passed = (exit_code == check.expect_exit)`. On fail, build `reason` from the
      exit status + a **reduced tail** of stdout/stderr via
      `reduce_output("bash", &combined)` (`reduce.rs:64-74`) then the size backstop
      — the same reduction the tool pipeline uses (`engine.rs:2146-2157`), so a
      noisy test log does not flood the re-opened turn.
- [ ] Uses the same timeout policy as `bash` (`S-4`); a check that hangs is killed
      by process group exactly as a tool child is. Confinement unavailable
      (degraded, §6.5) does not special-case the gate — the check simply runs under
      whatever the active sandbox status is, like any command.

## 4. Gate evaluation on the completion attempt  *(S-6; Tech Spec §7, HC-6)*

Hook the natural terminate point; re-open the loop on failure.

- [ ] At the completion branch (`engine.rs:1277-1279`), before the `return`,
      insert a call to a new `evaluate_completion_gate()`. **If
      `completion_checks` is empty → return as today** (the gate is inert; zero
      behavior change — assert this in tests).
- [ ] `evaluate_completion_gate()` runs every registered check (group 3). If all
      pass → write each result as a `CompletionCheck` transcript event, reset
      `completion_attempts`, allow the `return` (loop terminates).
- [ ] If any fails → increment `completion_attempts`; write the `CompletionCheck`
      events; **append the failing results to the conversation as a
      tool-result-shaped assistant-visible message** (HC-6, agent-world per Design
      §8.7) so the model reads the failures and can fix them; then **continue the
      turn loop** (do not `return`) so the model gets another turn — unless the
      attempt cap is hit (group 5). The re-open path must leave a clean message
      boundary (every `tool_use` has its `tool_result`), like compaction's
      boundary rule.

## 5. Bounded-attempt halt + await resolution  *(S-6; Tech Spec §7, §3)*

Mirror `await_loop_resolution` (`engine.rs:1606-1635`) for the gate.

- [ ] When `completion_attempts >= completion_config.max_attempts`, call a new
      `await_completion_resolution(failing, attempts)`: emit
      `UiEvent::CompletionGateHalted { failing, attempts }`, then park on
      `commands_rx.recv()` for `Command::ResolveCompletionGate` — handling
      `SetMode`/`Compact` inline while parked and treating `Cancel`/channel-closed
      as `Stop` (fail-safe, never spins unattended), exactly as
      `await_loop_resolution` does.
- [ ] Map the resolution: `Resume` → reset `completion_attempts`, continue the loop
      (try again); `Steer(text)` → inject the steer as a user turn and reset the
      counter (mirror the S-5 steer path); `Stop` → end the turn/session per the
      S-5 `Stop` behavior; **`Finish` → allow termination as "done" over the red
      gate** (the override the owner approved). Write one `CompletionGateHalt`
      transcript event with the resolution label and `override_finish: true` iff
      `Finish`.
- [ ] `resolution_label()`-style helper for the four variants (extend/mirror
      `engine.rs:455-460`).

## 6. Config: `[completion]` + `[[completion.check]]` + provenance  *(S-6; Tech Spec §8)*

All in `crates/emberly/src/config.rs`, paralleling `[loop]` — **but add the
provenance block `[loop]` is missing.**

- [ ] File struct: `struct CompletionConfigFile { enabled: Option<bool>,
      max_attempts: Option<usize>, #[serde(default)] check: Vec<CompletionCheckFile> }`
      and `struct CompletionCheckFile { name: String, command: String, expect_exit:
      Option<i32> }`. Top-level `#[serde(default)] pub completion: CompletionConfigFile`
      beside `loop_` (`:43-45`). `[[completion.check]]` parses as the `check` Vec.
- [ ] Merge (project over global) at the `merge` fn (`:335`), a `[completion]`
      block modeled on the `[loop]` merge (`:357-366`): scalar fields overwrite if
      `higher.*.is_some()`; **decide check-list merge semantics — project replaces
      vs. concatenates** (lean: project replaces the whole list, mirroring "project
      wins per key"; record the choice in the log).
- [ ] Resolve into `emberly_core::CompletionConfig` + the `Vec<CompletionCheck>`
      at the resolve site (`:936-951` neighbourhood): `enabled`/`max_attempts`
      `unwrap_or(default)`; map each `CompletionCheckFile` to a `CompletionCheck`
      with `expect_exit.unwrap_or(0)`. Add the core-config fields beside
      `loop_config` (`:473-474`) and wire them to the engine in `main.rs` (beside
      `:547`), calling `register_completion_check` for each.
- [ ] **Provenance / `config show` (C-3):** add a `record(...)` block for
      `completion.enabled` / `completion.max_attempts` (and a count line for
      registered checks), modeled on the `[truncate]` block (`:656-671`). This is
      the gap the `[loop]` table left open — close it here for `[completion]`.

## 7. TUI: completion-gate halt surface + sidebar status  *(S-6; Design §8.7, §7)*

Mirror the loop-break surface; add the "finish anyway" choice and the
registered-checks status line.

- [ ] **Rich (`emberly-tui`):** a `CompletionGatePrompt` mirroring `LoopHaltPrompt`
      (`app.rs:306-321`), field `pending_completion_gate` (`:394-396`), ingestion
      of `UiEvent::CompletionGateHalted` (`:668-670`), keyboard-ownership gate
      (`:821-824`) + an `on_completion_gate_key` handler modeled on
      `on_loop_halt_key` (`:1289-1335`) with **four** choices: `g` keep-going, `s`
      stop, `t`/Enter steer, **`f` finish-anyway**. `resolve_completion_gate()`
      emits the command (`:1337-1339` analog). Add to the "decision open" predicate
      (`:965-971`).
- [ ] Rendering: `render_completion_gate` modeled on `render_loop_halt`
      (`render.rs:1502-1569`) — harness-voice block naming the failing checks and
      attempt count, the four choices (or steer field), footer hint (`:1209-1213`).
      **"Finish anyway" labeled as an override**, never "checks passed" (Design
      §8.7). Strings: a `mod completion_gate` beside `mod loop_halt`
      (`strings.rs:55-70`).
- [ ] **Plain/degraded (`line.rs`):** `UiEvent::CompletionGateHalted` →
      `render_completion_gate` (mirror `:280-285`), a `parse_gate_resolution`
      (mirror `:480-490`; `finish`/`f`/`4` → Finish), and the blocking-input arm
      (mirror `:573-603`) emitting `Command::ResolveCompletionGate`. Capitalized
      deliberate choices; keep the no-ANSI degraded test green.
- [ ] **Sidebar gate status (Design §8.7, §3.1):** when checks are registered,
      show a dim line naming them and their last result (pass/fail), beside the
      sandbox/context status. **Absent entirely when no checks are registered** —
      no "None" stub (mirror the Tasks/Memory "shown only when present" rule). Feed
      it from the `CompletionCheck` results the engine already emits/records (a
      small `UiEvent` carrying last-results, or fold into an existing status event —
      **decide in the log**).

## 8. Tests + exit criterion  *(Tech Spec §14.8 offline, deterministic)*

Mirror the S-5 loop tests and the §14.8 gate coverage. All offline via
`FakeProvider`.

- [ ] **Fail re-opens, pass terminates:** a `FakeProvider` script that attempts
      completion with a registered check failing asserts the failure is appended as
      a tool-result and the loop re-opens; a passing check lets the turn terminate
      normally.
- [ ] **Contained but not prompted:** a check command runs through the sandboxed
      `bash` path and **never** raises a `PermissionRequest` (assert no `authorize`
      call on the check path — the divergence from `bash.rs:182-191`).
- [ ] **Bounded halt + resolutions:** drive `max_attempts` failures and assert
      `CompletionGateHalted` fires; each of `resume`/`steer`/`stop`/`finish`
      behaves correctly; `finish` writes a `CompletionGateHalt` transcript event
      with `override_finish: true`.
- [ ] **Inert gate:** with no registered checks, loop termination is byte-for-byte
      unchanged from today (no gate evaluation, no events) — the "behaves exactly as
      today" guarantee (S-6).
- [ ] **HC-7:** `CompletionCheck` / `CompletionGateHalt` events are appended, never
      rewrite a prior line; resume tolerates them.
- [ ] **Degraded parity:** the plain frontend renders the halt with ASCII,
      capitalized choices, and no ANSI.
- [ ] **Exit criterion (Phase 1 done when):** the failing/passing/contained/
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
