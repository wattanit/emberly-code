# Phase 1 — Core Engine — TODO & Progress

**Milestone:** M1 (Tech Spec §15) — *engine loop + fake provider +
read/bash/edit + per-action prompts (no sandbox yet), line-mode output.*
**Goal:** Prove the core. One end-to-end "the agent does a task" slice,
driven by a scripted fake provider, under the panic-free lint gate.

**Depends on:** Phase 0 (workspace scaffolding + CI gates) complete.

**Legend:** `[ ]` todo · `[~]` in progress · `[x]` done · `[!]` blocked

---

## Progress summary

| Group | Status | Notes |
|---|---|---|
| 0. Prerequisite (Phase 0) | [x] | Workspace + lint gates green (2026-07-06) |
| 1. Event model & channels | [x] | Done 2026-07-06; 4 round-trip tests green |
| 2. Provider trait + FakeProvider | [x] | Done 2026-07-06; 8 tests green |
| 3. Tool trait + ToolCtx | [x] | Done 2026-07-06; 6 tests green |
| 4. Tools: read / write / edit / bash | [x] | Done 2026-07-06; 14 tests green |
| 5. Truncation at ingestion | [x] | Done 2026-07-06; 5 tests green |
| 6. Permission gate (rule-layer) | [x] | Done 2026-07-06 (with group 7) |
| 7. Agent loop | [x] | Done 2026-07-06; 5 engine tests green |
| 8. Line-mode frontend | [x] | Done 2026-07-06; 5 tests green |
| 9. Supervisor skeleton | [x] | Done 2026-07-06 |
| 10. Tests & exit criterion | [x] | Done 2026-07-06; 48 tests, binary runs |

**Overall Phase 1: COMPLETE (2026-07-06).** All 10 groups done; 48 tests
green; `emberly` runs end-to-end in line mode under the panic-free lint gate.

---

## 0. Prerequisite — Phase 0 foundation

> Not part of M1 proper, but Phase 1 cannot start until these are green.

- [x] Cargo workspace with the six crates (`emberly-core`,
      `emberly-providers`, `emberly-tools`, `emberly-sandbox`, `emberly-tui`,
      `emberly`) and one-way dependency direction wired *and verified*.
- [x] Lint gates: `#![forbid(unsafe_code)]` everywhere;
      `#![deny(clippy::unwrap_used, clippy::expect_used)]` on core/providers/
      tools/sandbox. `anyhow` binary-only; `thiserror` in libs. Verified the
      gate bites (an `unwrap` in core is a hard compile error).
- [x] Foundational dependency set declared centrally in
      `[workspace.dependencies]`, `Cargo.lock` committed (54 packages, no C
      deps). Heavier per-layer deps (reqwest/ratatui/syntect/landlock/…) added
      by the phase that introduces them — see decisions log.
- [x] CI workflow (`.github/workflows/ci.yml`): fmt + clippy-as-errors + test
      + `x86_64-unknown-linux-musl` static build + `cargo deny` + `cargo vet`.
      `deny.toml` bans the canonical C deps (openssl/native-tls/libgit2).
      *(musl build + deny/vet run in CI; not exercised on the macOS host.)*

---

## 1. Event model & channels  *(A-1, A-3; Tech Spec §2, §3)*

- [x] Define `UiEvent` enum (non-exhaustive, `Serialize`): all §3.1 variants
      including `Cost/SandboxStatus/ModeChanged/CompactionStatus` (declared,
      exercised from later phases). → `event.rs`
- [x] Define `TranscriptEvent` enum (`Serialize`/`Deserialize`) with the `v`
      schema version field, wrapped in `TranscriptRecord { v, ts, #[flatten]
      event }` producing `{"v":1,"ts":…,"type":…,…}`. → `transcript.rs`
- [x] Define `Command` enum (in): user input, permission answer, cancel;
      `set_mode`/`compact` declared, handled later. → `command.rs`
- [x] Engine owns state; `channels::channel()` returns paired
      `EnginePorts`/`FrontendPorts` over `mpsc` (bounded, backpressure). No
      `Arc<Mutex<_>>`. → `channels.rs`
- [x] Round-trip serialization test for every event/command/transcript variant
      + wire-shape assertion + Thai-content round-trip. → `tests/event_model.rs`
      (4 tests green)

**Supporting types added:** `id.rs` (`SessionId`/`ToolCallId`/`PermissionId`),
`types.rs` (`Mode`, `SandboxStatus`, `PermissionRendering`,
`PermissionDecision`, `TokenUsage`).

## 2. Provider trait + FakeProvider  *(A-2, P-1; Tech Spec §4.1, §14.1)*

- [x] `Provider` trait (`provider.rs`): `id`, `model_info`,
      `stream_completion`, `count_tokens`. Normalized `CompletionRequest`
      (system + `Message`/`ContentBlock` + `ToolSchema`) in `message.rs`.
- [x] `CompletionStream` (`stream.rs`) yielding normalized `StreamEvent`
      (`TextDelta`, `ToolCall{Start,Delta,End}`, `Usage`, `Done`). **Design
      refinement:** the spec's `Err` event is modeled as the `Err` arm of the
      stream item `Result<StreamEvent, ProviderError>` — idiomatic, and keeps
      `StreamEvent` `Clone`/`Serialize`. Documented in `stream.rs`. No wire
      types cross the boundary (P-1).
- [x] `ProviderError` (`error.rs`, `thiserror`) with `is_retryable()` /
      `retry_after()` classification pre-wired for the Phase 3 retry policy.
- [x] `FakeProvider` (`fake.rs`): queue of `ScriptedResponse`s, one per
      completion; helpers for text / tool-call / mid-stream-error /
      mid-stream-drop / connect-error. → `tests/fake_provider.rs` (8 tests).
- [x] `count_tokens` chars/4 (`div_ceil`), flagged `approximate` (P-6).

**Layering decision:** `ToolCallId` and `TokenUsage` now live in
`emberly-providers` (they originate on the provider wire / are the accounting
unit); `emberly-core` re-exports them, so there is one definition and no
boundary translation. `Pricing::estimate_usd` added for Phase 3 cost (P-6).

## 3. Tool trait + ToolCtx  *(HC-6, T-7; Tech Spec §5.1)*

- [x] `Tool` trait (`tool.rs`): `spec() -> ToolSpec`, `async execute(args,
      ctx) -> ToolOutcome`; never returns a harness-level `Err`. Object-safe
      via `async-trait` so `Arc<dyn Tool>` works (T-7).
- [x] `ToolOutcome` (`tool.rs`): `{ ok, content, summary }` with
      `success`/`failure`/`denied` constructors (HC-6). Full content returned;
      truncation is engine-side at ingestion (group 5), not the tool's job.
- [x] `ToolCtx` (`ctx.rs`): project root, `TruncateConfig`, and the gate;
      `authorize()` is the single path to permission. Sandbox handle noted as
      the Phase 2 addition — the tool interface won't change when it lands.
- [x] `ToolRegistry` (`registry.rs`): name-indexed; `register`/`get`/`specs`/
      `names`; engine iterates `specs()` to build provider `ToolSchema`s.
- [x] Tests: `tests/tool_trait.rs` (6) — allow/deny gate, denial-as-data
      (HC-6), bad-args-as-data, registry ops.

**Layering decision:** the permission **gate is a trait in `emberly-tools`**
(`PermissionGate` + `PermissionRequest`/`PermissionOutcome`), implemented by
`emberly-core`. Keeps `tools` independent of `sandbox`; core enriches the
request with the matched-rule reason, runs the UI round trip, and collapses
the user's richer `PermissionDecision` into allow/deny for the tool.

## 4. Tools: read / write / edit / bash  *(T-1, T-2, T-3, T-4; Tech Spec §5.2)*

- [x] `read_file` (`builtin/read.rs`): `resolve_in_root` (canonicalize
      existing prefix → symlink-safe, collapse `..`, classify outside-root);
      in-root reads free, outside-root reads ask (HC-4). Full content returned;
      engine truncates at ingestion (group 5).
- [x] `edit_file` (`builtin/edit.rs`): exact match-and-replace; **no match** vs
      **N matches found** distinguished; no-match includes a `similar`-based
      closest-line hint (T-3); optional `replace_all`; unified diff in the
      prompt (Design §5).
- [x] `bash` (`builtin/bash.rs`): `tokio::process`, 120s default / 600s ceiling
      timeout; child in its own **process group** (`process_group(0)`, unix);
      `kill_on_drop` kills the leader on timeout (S-4 — harness never hangs);
      env scrubbed to PATH/HOME/LANG/TERM allowlist. Non-zero exit = data.
- [x] `write_file` (`builtin/write.rs`): create/replace; parent dirs inside
      root; **hard-refuses `.git/`** at the tool layer (HC-5, no gate);
      `FileChange` (adds/dels) attached for the engine to emit `FileModified`.
- [x] All four authorize through `ctx.authorize()` (read only when
      outside-root, per §6.2). Path resolution in `path.rs`, diff/hint helpers
      in `diff.rs`. → `tests/builtin_tools.rs` (14 tests).

**Deferred to Phase 2 (noted in `bash.rs`):** killing the whole descendant
*tree* needs a group-kill signal (a vetted syscall dep like `rustix`/`nix`),
decided alongside the sandbox. Phase 1 sets the process group + leader-kill +
timeout, which satisfies "a hung child must not hang the harness." Avoids an
unlisted dependency and `unsafe` (HC-1).

## 5. Truncation at ingestion  *(§8.1; Tech Spec §5.3)*

- [x] `truncate_output(&str, &TruncateConfig) -> Truncation` (`truncate.rs`):
      over `max_lines` (400) → keep head (150) + tail (100) with
      `[... N lines elided — /view to open full output ...]`.
- [x] Deterministic, pure, **no model call**. Second byte-wise pass for
      still-oversized output (huge single lines), UTF-8-boundary safe.
- [x] `Truncation` reports `truncated` + `original_lines`/`original_bytes`;
      `full_output_ref` sidecar is set by the transcript layer in Phase 5, not
      here (as planned). → 5 inline unit tests (incl. multibyte boundary).

**Note:** engine calls `truncate_output` when ingesting a tool result
(group 7). Byte-elision marker reads "N bytes elided"; line-elision "N lines
elided".

## 6. Permission gate — rule-layer only  *(§6.6, HC-6; Tech Spec §6.1 partial)*

- [x] Per-action ask flow (`gate.rs` + `engine.rs`): tool's `authorize()`
      sends a `PermissionAsk` (request + reply oneshot) over an internal
      channel; the engine emits `UiEvent::PermissionRequest{id, rendering}`,
      stores the reply keyed by id, and resolves it on `Command::PermissionAnswer`.
- [x] **Deny returns to the model as data** via `ToolOutcome::denied`, ingested
      as a `tool_result` with `is_error=true`; the model then continues (proven
      by `denied_tool_feeds_failure_and_model_continues`). Never a silent drop.
- [x] `ChannelGate` **fails closed** (Deny) if the engine/reply is gone. The
      deny-as-default *keypress* mapping is a group 8 frontend concern; the gate
      contract makes deny the safe fallback.
- [x] No OS sandbox / `permissions.toml` / persisted grants (Phase 2). The
      Phase-1 gate always prompts for whatever reaches it (in-root reads don't).

**Concurrency (Tech Spec §2):** no `Arc<Mutex<_>>`. The tool future borrows
only Arc clones (not `&mut self`), so the engine `select!`s it against
`commands_rx` and the ask channel within one task. `build_rendering` adds the
matched-rule "reason" (Design §5).

## 7. Agent loop  *(Tech Spec §2, §3)*

- [x] Sequential loop (`engine.rs` `run_turn`): build `CompletionRequest` →
      `consume_stream` (emit deltas, accumulate tool calls) → run tool calls
      through gate + tool → truncate + append `tool_result` → loop until `Done`
      with no tool calls.
- [x] One in-flight completion at a time; tool calls run sequentially.
- [x] Cancel handled at each await point (streaming and tool exec); the bash
      child is killed via `kill_on_drop` when the exec future is dropped.
      Canceled/remaining tool calls get backfilled `tool_result`s so the
      conversation stays well-formed.
- [x] Mid-stream drop → `StreamEnd::Dropped` → HarnessError, partial text kept,
      no crash (Phase 3 adds retry). Mid-stream error → HarnessError.
- [x] Emits `FileModified{path, adds, dels}` from the tool's `FileChange`;
      `ContextUsage` after each step. → `tests/engine_loop.rs` (5 tests).

## 8. Line-mode frontend  *(A-1; Tech Spec §9 degraded contract)*

- [x] `emberly-tui/line.rs`: `LineRenderer::render` (pure, writes to any
      `Write`) + async `run(FrontendPorts)` driver. ASCII markers, no color,
      no cursor repositioning.
- [x] Renders deltas (stream, no newline), tool start/finish, file changes,
      harness errors, and the permission prompt: **full content** (untruncated
      detail), OUTSIDE-PROJECT banner in caps, `[Enter] DENY` default, `[y]`
      explicit approve. `parse_permission_answer` defaults to deny.
- [x] Input driver: stdin lines → `UserInput`; while a prompt is open the next
      line is the answer; `/cancel` → `Command::Cancel`. → 5 unit tests.
- [x] Seed of degraded/`--plain` mode and the headless-frontend contract; the
      rich `ratatui` TUI is Phase 4.

## 9. Supervisor skeleton  *(HC-3; Tech Spec §10)*

- [x] `emberly/main.rs`: `#[tokio::main]` + `install_panic_hook` (top-level
      hook), supervises the spawned engine task (checks `JoinError::is_panic`).
- [x] Clean exit: harness-voice error to stderr + non-zero exit on failure.
      Line mode makes no terminal changes to restore; the hook is the attach
      point for TUI terminal-restore (Phase 4).
- [x] Transcript persistence + `abnormal_exit` deferred to Phase 5; the panic
      hook marks where they attach.

## 10. Tests & exit criterion  *(A-2)*

- [x] Unit tests across groups: truncation math (5), edit no-match/N-match (in
      builtin_tools), env scrubbing + timeout kill (bash tests).
- [x] Scripted `FakeProvider` integration test `full_workflow_read_edit_
      permission_bash`: read (no prompt) → edit (prompt) → bash (prompt) →
      terminates, file edited. Plus `denied_tool_feeds_failure_and_model_
      continues` for the deny path.
- [x] Denial-as-data (HC-6): asserted in the denied-tool test (failure result,
      model continues).
- [x] Cancel test `cancel_during_bash_stops_promptly`: cancel kills the sleep
      in <3s (0.5s actual), not its 5s timeout.
- [x] Whole tree passes under the `unwrap_used`/`expect_used` deny gate; 48
      tests green; `emberly --plain` smoke run verified end-to-end.

---

## Phase 1 exit criterion (from IMPLEMENTATION_PLAN.md)

> A scripted fake-provider session completes a multi-step task (read a file,
> propose an edit, prompt for permission, run a bash command) end-to-end in
> line mode, with denials routed back to the model as data, all under the
> panic-free lint gate.

- [x] **Exit criterion met.** Proven by `full_workflow_read_edit_permission_bash`
      + `denied_tool_feeds_failure_and_model_continues` (deny→data), all under
      the panic-free lint gate. `emberly` also runs the loop end-to-end in line
      mode (placeholder provider; live providers are Phase 3).

---

## Notes / decisions log

- **2026-07-06:** `write_file` lands in Phase 1 (not deferred). Its `.git/`
  refusal is a tool-layer check that does not depend on the OS sandbox, so
  there is no reason to wait for Phase 2. (Owner decision.)
- **2026-07-06:** `full_output_ref` sidecar file writing stays deferred to
  Phase 5; Phase 1 implements truncation math + marker only. (Owner decision.)

- **2026-07-06 (Phase 0):** Deferred the heavier leaf dependencies
  (`reqwest`, `ratatui`, `crossterm`, `syntect`, `landlock`, `ignore`,
  `grep-searcher`, `globset`, `similar`, `unicode-*`, `nucleo-matcher`) to the
  phases that introduce them, rather than adding the full Tech Spec §12 set at
  scaffold time. Rationale: keeps the audited tree small and reviewable, and
  each addition gets `cargo vet`/`deny` acceptance when it actually lands.
  Versions are pinned centrally in `[workspace.dependencies]` as they arrive.
- **2026-07-06 (Phase 0):** `cargo vet` CI job is `continue-on-error` until the
  audit-import set is seeded, then tightened to required. Recorded so it is not
  mistaken for a permanently-soft gate.

*(Continue recording deviations, deferrals, and decisions here as work
proceeds — e.g. the first-party-vs-crate SSE question if it surfaces early.)*
