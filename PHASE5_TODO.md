# Phase 5 — Sessions, Config & Release — TODO & Progress

**Milestone:** M5 (Tech Spec §15) — *durable sessions, resume, manual
compaction, init/config provenance, macOS confinement, and the release
pipeline.*
**Goal:** Prove v1. Everything so far has been ephemeral — a session lives only
in memory and vanishes on exit. Phase 5 makes it **durable and recoverable**:
an append-only transcript that is ground truth (HC-7), resume after a crash,
`/compact` to survive long tasks, `init`/`config show` so configuration is
inspectable, macOS Seatbelt so bash is finally OS-confined *on this machine*,
and a release pipeline that ships static binaries.

**Depends on:** Phases 1, 3, 4 (all merged). The transcript **schema** already
exists (`emberly-core::transcript` — `TranscriptRecord`/`TranscriptEvent`,
`SCHEMA_VERSION`, `ConfigProvenance`, defined in Phase 1); this phase writes,
reads, and acts on it. Config resolution + secrets exist (`emberly::config`,
Phase 3); this phase completes the two-tier + provenance + prompts story.

**Legend:** `[ ]` todo · `[~]` in progress · `[x]` done · `[!]` blocked

---

## Progress summary

| Group | Status | Notes |
|---|---|---|
| 0. Prerequisites: session identity & the writer seam | [x] | `TranscriptSink`/`NoopSink`/`CaptureSink`; session id + `.agents/sessions/` |
| 1. Transcript persistence (HC-7) | [x] | `FileTranscript` (append + per-event fsync) + sidecars; engine write points; 4 tests |
| 2. Supervisor completion (HC-3, S-2) | [x] | panic hook + error path append `abnormal_exit` + resume hint; clean-exit summary |
| 3. Resume (§8.2, §3.3) | [x] | `resume.rs` replay + engine seeding + display seeding; `resume [id]` + offer-on-launch; 5 tests + e2e |
| 4. Manual `/compact` (§8.3, §7) | [x] | clean-boundary gate; summarize via provider; pinned + keep-recent; fallback truncate; 1 test |
| 5. Config, prompts & provenance (§7, §8) | [ ] | AGENTS.md/CLAUDE.md; prompt dir + per-family variants; `config show`; provenance |
| 6. `emberly init` (C-2) | [ ] | materialize `.agents/` defaults; considerate UX (Design §8.1) |
| 7. CLI surface & key moments (§10) | [ ] | `init`/`resume`/`config show`/`--model`/`--provider`; session header + clean-exit line |
| 8. macOS Seatbelt (§6.3, §16) | [ ] | confine children via `sandbox-exec` (no FFI → HC-1); SandboxStatus on macOS |
| 9. Release pipeline (§13) | [ ] | musl x86_64/aarch64 + aarch64-darwin; name-collision + `--plain` smoke |
| 10. Replay tests & exit criterion (§14.2) | [ ] | recorded JSONL fixtures + schema forward-compat; full sweep |

**Overall Phase 5: NOT STARTED.** Branch `phase-5-sessions` off `main`.

---

## Platform & constraint notes (read first)

- **Fully developable on macOS.** Persistence, resume, compaction, config, init,
  CLI, and release wiring are OS-agnostic. **Seatbelt (group 8) is macOS-native**
  — this is the phase where bash finally gets OS confinement *on the dev
  machine*, since Phase 2 (Landlock) stays deferred until a Linux box.
- **HC-1 (no unsafe) constrains Seatbelt.** The classic `sandbox_init` is a
  deprecated C API needing FFI → forbidden. Plan: spawn children under the
  system `/usr/bin/sandbox-exec -p <profile>` binary (no FFI, no `unsafe`), or
  decline and degrade honestly per §6.7. Seatbelt is the **spike** of this phase
  (Tech Spec §16 flags the API as old/thin) — it may partially defer.
- **HC-3/HC-7 are the spine.** The transcript writer fsyncs **per event** so the
  abnormal-exit path has almost nothing to lose. Nothing ever rewrites a line.
- **HC-2 unchanged.** No new C deps expected; if a release step needs one, stop.
- **Secrets never persist** — API keys never enter the transcript; request logs
  redact auth headers (already true; keep it true).

---

## 0. Prerequisites: session identity & the writer seam

- [x] **Session identity:** `SessionId::new()` at startup; file
      `.agents/sessions/<id>.jsonl`; sidecars under `.agents/sessions/<id>-outputs/`;
      dirs created on open/first sidecar.
- [x] **`TranscriptSink` trait** in `emberly-core` (`record(&Record)`,
      `sidecar(call_id, content) -> Option<ref>`), `Send + Sync` (the engine
      borrows `&self` across awaits, so it must stay `Sync`). `NoopSink` default,
      `CaptureSink` (Arc<Mutex<Vec>>) for tests, `FileTranscript` for real I/O.
- [x] **Timestamps** stamped at the write edge (`write_transcript` →
      `OffsetDateTime::now_utc()`); the record type is time-carrying, logic isn't.
- [x] **Decision recorded:** engine-owned sink (in `EngineConfig`), one write
      site per durable event — no second event stream. Default `no_transcript()`
      keeps existing tests unchanged.

---

## 1. Transcript persistence (HC-7, §8.2, Tech Spec §3.2)

- [x] `FileTranscript`: open `<id>.jsonl` append-only; one `TranscriptRecord`
      per line; **flush + `sync_data` per event** (S-2). Marks itself unhealthy
      after a write error (reported once, not per event). Never rewrites a line.
- [x] Engine write points: `session_start` (provider/model/root/sandbox/
      provenance), `user_message` (+`original_task`), `assistant_message`,
      `tool_call`, `tool_result` (+truncated/sidecar), `permission_request`,
      `permission_decision`, `session_title`, `session_end`. (`mode_change` →
      Phase 2; `compaction` → group 4; `abnormal_exit` → group 2.)
- [x] **Sidecars for truncated output:** on a truncated `tool_result`, the full
      output spills to `<id>-outputs/<call_id>.txt` and `full_output_ref` is set.
      (Wiring the `/view`-full hatch to it is group 7.)
- [x] **Session title:** first user message clipped to 60 chars (`clip_title`);
      emitted as `session_title` on the first turn.
- [x] Tests: `CaptureSink` asserts the durable sequence for a scripted session
      (incl. `session_end` on channel close); `FileTranscript` round-trips
      (write → read → parse) and sidecars in a temp dir. Live smoke: a real
      `--plain` run writes `session_start…session_end` to disk.

---

## 2. Supervisor completion (HC-3, S-2, Tech Spec §10)

- [x] Abnormal-exit paths: the **panic hook** and the `run() -> Err` path both
      append `abnormal_exit { reason }` and print the resume hint (non-zero exit
      via the existing error path). Terminal restore is already handled by the
      TUI guard's hook (Phase 4), which wraps this one and runs first.
- [x] Clean-exit path: the engine records `session_end` on channel close; the
      binary prints a one-line summary (`session ended · <mm:ss> · transcript:
      <path>`). Title/cost live engine-side — surfacing them is a later
      refinement (would need a final summary event).
- [x] **Panic-reachable handle:** a `static SESSION_PATH: OnceLock<PathBuf>` set
      when the transcript opens. The hook cannot hold `&mut engine`, so it calls
      `emberly_core::append_abnormal_exit(path, reason)` — a standalone
      reopen-append (safe because every prior event was fsynced).
- [x] Test: `append_abnormal_exit_adds_a_trailing_line` writes a session,
      reopens it, and asserts `abnormal_exit` is the intact trailing line.
      (Binary panic-path is mechanically wired; forced-panic smoke → group 10.)

---

## 3. Resume (§8.2, Tech Spec §3.3, Design §8.3)

- [x] **Replay:** `resume::read_records` (lenient) → `rebuild_conversation`
      folds assistant text + following `tool_call`s into one assistant message
      and `tool_result`s into tool messages. The engine is seeded via
      `EngineConfig.initial_conversation` (+ `resuming`, which skips a fresh
      `session_start` and keeps appending to the same file); the frontend
      timeline is seeded via `App::seed_history`.
- [x] **Compaction as a view transform:** a `compaction` record truncates the
      rebuilt messages to `replaced_from` and appends the summary as a user
      message — the JSONL is untouched. Tested.
- [x] **Forward-compat:** `read_records` warns-and-skips unknown `type` /
      newer-`v` lines (never crashes), reporting a newer schema distinctly.
      Tested with a good + unknown + newer fixture.
- [x] **`emberly resume [id]`:** id → that session; no id → `latest_session`.
- [x] **Offer-on-launch (Design §8.3):** interactive launches call
      `offer_resume` — if the newest session is `interrupted()`, prompt "Found
      an interrupted session … resume? (y/N)"; only "y" resumes. Never
      auto-resumes; silent when nothing is interrupted.
- [x] Tests: `rebuild_conversation` (tool turn), compaction collapse,
      `interrupted` (clean/crash/kill/empty), `read_records` warn-skip. **E2E
      smoke:** run → resume appends to the same file (1 `session_start`, 2
      `session_end`), banner shows "resumed session (N earlier events)".

---

## 4. Manual `/compact` (§8.3, Tech Spec §7)

- [x] **Clean-boundary gate:** idle `/compact` runs immediately (already a
      boundary); a `/compact` seen mid-turn (in `consume_stream` / the tool-call
      select) sets `compact_requested` and runs after the turn returns — when
      every `tool_use` has its `tool_result`.
- [x] **Pinned, never compacted:** the original task (first conversation
      message) is kept; the system prompt lives outside the conversation.
      (AGENTS.md/CLAUDE.md pinning arrives with group 5.)
- [x] **Summarize with the current provider** via `SUMMARY_PROMPT` (original
      task / decisions / files / state / next steps); the middle is rendered to
      text and streamed, collecting the reply. (Prompt is overridable in group 5.)
- [x] **Rebuild the view:** `[pinned original][summary-as-user-msg][last
      KEEP_RECENT messages]` (KEEP_RECENT = 6; config wiring group 5). Records a
      `Compaction { summary, replaced_from, replaced_to }`; the JSONL is
      untouched. Replay splices the same range (group 3, updated to match).
- [x] **Failure fallback:** on a summarize error (or empty reply), the middle is
      dropped behind a placeholder summary with a visible `CompactionStatus`
      warning — recorded as a `compaction` so resume stays consistent. A full
      context never yields a stuck session.
- [x] Emits `CompactionStatus` UiEvents ("compacting…", "compacted — …",
      no-op/fallback notices) that Phase 4 renders as notices.
- [x] Test (via `FakeProvider`): four turns then `/compact` rebuilds the view
      and records `Compaction` with the model's summary; the UI is told. (Empty
      middle → "nothing to compact" no-op.)

---

## 5. Configuration, prompts & provenance (§7, §8; C-1/C-3/C-4/P-7)

- [ ] **Project instructions (C-1):** load `AGENTS.md` (native); if absent, read
      `CLAUDE.md`; if both present, **AGENTS.md wins with a one-time notice**.
      Fold into the system prompt as pinned context.
- [ ] **Prompts directory (C-1, P-7):** markdown prompts overridable in
      `.agents/prompts/`; per-model-family variants resolve `<name>.<family>.md`
      then fall back to `<name>.md`. At least the system prompt + the `/compact`
      summarization prompt live here.
- [ ] **Provenance tracking (C-3):** while resolving config, record each active
      non-default piece as `ConfigProvenance { piece, source }`. Feed into the
      `session_start` transcript event and `config show`.
- [ ] **`emberly config show`:** print every active piece with its provenance
      tier (default / global / project / env). Secrets never printed (show
      "set"/"unset" only).
- [ ] Session-start header (Design §8.2): silent about defaults; one dimmed
      provenance line per override.
- [ ] Tests: AGENTS-wins-over-CLAUDE precedence + notice; family variant
      resolution + fallback; provenance for a layered config; `config show`
      output shape; secrets never appear.

---

## 6. `emberly init` (C-2, Design §8.1)

- [ ] Materialize active defaults into `.agents/`: `config.toml` (commented),
      `prompts/` (the built-in prompts), and — reserving the pattern —
      `permissions.toml` (the Phase 2 rule file, written as a documented default
      even though the rule engine is Phase 2). Never overwrite existing files
      without consent.
- [ ] **UX (Design §8.1):** print exactly what it created, where, and the one
      next command worth knowing. No walls of text — "a considerate colleague,
      not an installer wizard."
- [ ] `.gitignore` guidance: `.agents/sessions/` (transcripts) should be ignored
      by default; `.agents/config.toml`/prompts are shareable. Init writes/append
      a `.agents/.gitignore` accordingly.
- [ ] Tests: init into a temp dir creates the expected tree; re-running is safe
      (no clobber); the printed summary lists what was made.

---

## 7. CLI surface & key moments (Tech Spec §10, Design §8)

- [ ] Real arg parsing (subcommands + flags), replacing the hand-rolled loop:
      `emberly` (start in cwd), `emberly init`, `emberly resume [id]`,
      `emberly config show`, `--plain`, `--model <m>`, `--provider <p>`,
      `--version`. Keep it dependency-light (first-party parse or a tiny crate —
      check HC-2). `--model`/`--provider` are the highest-precedence overrides.
- [ ] **First run** (Design §8.1): no config just works (baked-in defaults) and
      prints a two-line orientation pointing at `emberly init`.
- [ ] **Session start / end** lines (Design §8.2/§8.3) — wire to the transcript
      (start id/title, end summary with duration + cost + transcript path).
- [ ] Tests: arg parsing table (each subcommand/flag → parsed intent); unknown
      arg errors cleanly.

---

## 8. macOS Seatbelt (§6.3, Tech Spec §16) — the spike

- [ ] **ABI-free approach (HC-1):** confine spawned children by launching them
      under `/usr/bin/sandbox-exec -p <profile>` rather than the `sandbox_init`
      C FFI. Profile: read broadly, write only under the project root, deny
      `.git/` writes, deny network unless configured — mirroring the Phase 2
      Landlock intent. The harness process itself is never confined.
- [ ] Wire into `emberly-sandbox` behind the same seam Phase 2 will use for
      Landlock, so `bash` runs confined on macOS. Report real `SandboxStatus`
      (`Confined { backend: "seatbelt" }` / `Unavailable { reason }`).
- [ ] **Degradation (§6.7):** if `sandbox-exec` is unavailable or the profile
      fails, notify, suspend the bash allowlist, lock auto modes, keep
      per-action prompting — the honest degraded mode.
- [ ] `sandbox-exec` is deprecated by Apple but still present and works; note
      the risk. **If the spike overruns, defer Seatbelt** (like Landlock) and
      ship v1 macOS with permission-prompt-only guarding, clearly labeled — do
      not block the rest of Phase 5 on it.
- [ ] Tests: a macOS-gated escape test (write outside root / `.git` write via
      bash is blocked when confined); degraded path asserts the labeling.

---

## 9. Release pipeline (Tech Spec §13)

- [ ] Release build targets: `x86_64-unknown-linux-musl`,
      `aarch64-unknown-linux-musl` (fully static, HC-2), `aarch64-apple-darwin`;
      best-effort `x86_64-apple-darwin`. Windows deferred (documented).
- [ ] Release workflow (tag-triggered): build all targets, strip, produce
      archives + checksums. Reuse the existing CI gates.
- [ ] **Release checklist:** name-collision check (crates.io/Homebrew/distro/
      PATH — Design §1.1), a `--plain` smoke run, and the HC-2 C-free guard on
      each artifact.
- [ ] Version/`--version` output finalized; `README` install docs (incl. the
      suggested `emb` alias, not created by the tool).

---

## 10. Replay tests, fixtures & exit criterion (§14.2, A-2)

- [ ] **Recorded JSONL fixtures** committed under a test fixtures dir: a normal
      session, a compacted session, an abnormally-exited session, and a
      **schema-forward-compat** fixture (a record with `v = SCHEMA_VERSION + 1`
      / unknown `type`) that must warn-not-crash.
- [ ] Replay tests assert the rebuilt conversation view for each fixture.
- [ ] Round-trip test: run a scripted `FakeProvider` session with the file sink,
      then resume from the written file → identical view.
- [ ] fmt + clippy clean; HC-2 guard passes; full suite green.
- [ ] **Manual smoke (owner):** real session → quit → `emberly resume` restores
      it; kill mid-session (`kill -9`) → next launch offers resume; `/compact` on
      a long session round-trips; `emberly init` + `config show` report correct
      provenance; (macOS) a bash write outside root is blocked when confined.

**Exit criterion (plan §Phase 5 "Done when"):** a session survives an abnormal
exit and resumes cleanly; `/compact` round-trips through the transcript;
`init`/`config show` report correct provenance; macOS confinement passes an
escape-test analogue (or is explicitly, visibly degraded); all release targets
build.

---

## Decisions log

- **Branch:** `phase-5-sessions` off `main` (Phases 1/3/4 merged).
- **Transcript schema already exists** (Phase 1) — this phase is the writer +
  reader + actors, not a redesign. `SCHEMA_VERSION` stays 1 unless an event
  shape changes (then bump + a forward-compat fixture).
- **Writer lives in the engine behind a `TranscriptSink` trait** (fs impl behind
  it; no-op default) so core stays unit-testable and there's one write site per
  durable event. (Confirm in group 0.)
- **Seatbelt via `sandbox-exec`, not FFI** (HC-1). It is the phase's spike and
  may defer to keep v1 moving — macOS then ships permission-prompt-only,
  labeled, exactly like the current Landlock deferral.
- **Phase 2 (Landlock) remains deferred** until a Linux machine; Seatbelt gives
  macOS its safety tier in the meantime.
- **Deferred beyond v1** (designed-for, not built): auto-compaction (one
  threshold check in the accounting path), model-generated session titles,
  MCP/LSP/Skills, Windows.
