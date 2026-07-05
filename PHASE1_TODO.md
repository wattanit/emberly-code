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
| 2. Provider trait + FakeProvider | [ ] | |
| 3. Tool trait + ToolCtx | [ ] | |
| 4. Tools: read / write / edit / bash | [ ] | |
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

- [ ] `Provider` trait: `id`, `model_info`, `stream_completion`,
      `count_tokens`. Normalized `CompletionRequest` (messages + tool
      schemas).
- [ ] `CompletionStream` yielding normalized `StreamEvent`:
      `TextDelta`, `ToolCallStart/Delta/End`, `Usage`, `Done`, `Err`.
      No wire types cross the boundary.
- [ ] `ProviderError` (`thiserror`).
- [ ] `FakeProvider`: constructed from a script of canned `StreamEvent`s;
      supports text, tool calls, error, and mid-stream drop scenarios.
- [ ] `count_tokens` chars/4 approximation (trigger-grade, P-6).

## 3. Tool trait + ToolCtx  *(HC-6, T-7; Tech Spec §5.1)*

- [ ] `Tool` trait: `spec() -> ToolSpec` (name, description, JSON schema),
      `async execute(args, ctx) -> ToolOutcome`. `execute` never returns a
      harness-level `Err`.
- [ ] `ToolOutcome`: structured success | structured failure (HC-6).
- [ ] `ToolCtx`: project root, truncation config, permission-gate handle.
      *(sandbox handle field declared; wired in Phase 2.)* Transport-agnostic
      so a future MCP adapter implements `Tool` (T-7).
- [ ] Tool registry the engine iterates to build provider tool schemas.

## 4. Tools: read / write / edit / bash  *(T-1, T-2, T-3, T-4; Tech Spec §5.2)*

- [ ] `read_file`: path-normalize (resolve symlinks, collapse `..`) then
      root-check; bounded output via truncation (group 5).
- [ ] `edit_file`: exact string match-and-replace; failure messages
      distinguish **no match** vs **N matches found**; no-match includes a
      `similar`-based closest-region fuzzy hint (T-3).
- [ ] `bash`: `tokio::process`, default 120s timeout configurable up to a
      ceiling; child in its own **process group** so timeout/cancel kills the
      whole tree (S-4); scrubbed env allowlist (PATH/HOME/LANG/TERM +
      config additions).
- [ ] `write_file`: creates or replaces a file; creates parent dirs inside
      root only; refuses `.git/` paths at the tool layer (HC-5 tool-level
      check — does not depend on the OS sandbox). Emits `FileModified`.
- [ ] All four route execution *through* the permission gate; none can
      bypass it.

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
