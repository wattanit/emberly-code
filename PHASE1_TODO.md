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
| 5. Truncation at ingestion | [ ] | |
| 6. Permission gate (rule-layer) | [ ] | |
| 7. Agent loop | [ ] | |
| 8. Line-mode frontend | [ ] | |
| 9. Supervisor skeleton | [ ] | |
| 10. Tests & exit criterion | [ ] | |

**Overall Phase 1: not started.**

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

- [ ] On appending a tool result: if over `truncate.max_lines` (400) or
      `truncate.max_bytes` (64 KiB), keep head (150) + tail (100), insert
      `[... N lines elided — /view to open full output ...]`.
- [ ] Deterministic, per-event, **no model call**. Applies to bash output and
      file reads alike.
- [ ] `full_output_ref` sidecar wiring stubbed (real file lands Phase 5);
      truncation math and marker are complete and tested here.

## 6. Permission gate — rule-layer only  *(§6.6, HC-6; Tech Spec §6.1 partial)*

- [ ] Per-action ask flow: tool requests → engine emits
      `UiEvent::PermissionRequest{id, rendering}` → waits for
      `Command`-carried answer (deny / allow-once).
- [ ] **Deny returns to the model as a structured tool result** ("user denied
      this command"), never a silent drop (§6.6, HC-6).
- [ ] Enter/Esc default maps to **deny**; approval is a distinct deliberate
      key (Design §5 — even in line mode).
- [ ] No OS sandbox, no `permissions.toml`, no session-persist grants yet
      (those are Phase 2). Simple in-memory per-action decision only.

## 7. Agent loop  *(Tech Spec §2, §3)*

- [ ] Sequential loop: send `CompletionRequest` → consume `StreamEvent`s →
      on tool call, run through gate + tool → append `ToolOutcome` (truncated)
      → continue until `Done` with no pending tool calls.
- [ ] One in-flight completion at a time.
- [ ] Cancel (`Command`) handled at the next await point; a running bash child
      is killed by process group.
- [ ] Mid-stream drop from the provider: partial assistant text kept + marked
      interrupted (full retry policy is Phase 3; here just don't crash).
- [ ] Emit `FileModified{path, adds, dels}` when edit/write changes a file.

## 8. Line-mode frontend  *(A-1; Tech Spec §9 degraded contract)*

- [ ] Minimal append-only line output consuming `UiEvent`, producing
      `Command`. ASCII markers, no cursor repositioning.
- [ ] Renders assistant deltas, tool start/finish, and the permission prompt
      (full content, explicit approve key) in line form.
- [ ] This is **not** the real TUI (Phase 4); it is the driving/observing
      harness for the loop and the seed of degraded mode.

## 9. Supervisor skeleton  *(HC-3; Tech Spec §10)*

- [ ] Binary installs a panic hook + supervises the engine task.
- [ ] On abnormal path: restore terminal state, exit cleanly non-zero.
- [ ] Transcript persistence + `abnormal_exit` event are Phase 5 — leave the
      hook point, don't implement persistence here.

## 10. Tests & exit criterion  *(A-2)*

- [ ] Unit tests for: truncation math, edit no-match/N-match messages, env
      scrubbing, process-group kill on timeout.
- [ ] Scripted `FakeProvider` integration test: multi-step task —
      read a file → propose an edit → permission prompt (test both **deny**
      → data-back-to-model and **allow** paths) → run a bash command → loop
      terminates.
- [ ] Denial-as-data assertion (HC-6): denied tool call produces a structured
      tool result the model sees.
- [ ] Cancel test: cancel mid-bash kills the child process group.
- [ ] Whole tree passes under `clippy::unwrap_used`/`expect_used` deny gate.

---

## Phase 1 exit criterion (from IMPLEMENTATION_PLAN.md)

> A scripted fake-provider session completes a multi-step task (read a file,
> propose an edit, prompt for permission, run a bash command) end-to-end in
> line mode, with denials routed back to the model as data, all under the
> panic-free lint gate.

- [ ] **Exit criterion met.**

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
