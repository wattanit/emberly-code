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
| 5. Config, prompts & provenance (§7, §8) | [x] | provenance-tracked load; AGENTS/CLAUDE (+notice); family prompts; `config show`; 5 tests |
| 6. `emberly init` (C-2) | [x] | materializes `.agents/` (config/prompts/permissions/.gitignore); no-clobber; 2 tests |
| 7. CLI surface & key moments (§10) | [x] | testable `parse_args`; `--model`/`--provider` (cli tier); first-run orientation; 4 tests |
| 8. macOS Seatbelt (§6.3, §16) | [x] | `sandbox-exec` profile (no FFI/dep → HC-1/HC-2); same seam as Landlock; `Confined{seatbelt}`; 5 escape + 3 profile tests |
| 9. Release pipeline (§13) | [ ] | musl x86_64/aarch64 + aarch64-darwin; name-collision + `--plain` smoke |
| 10. Replay tests & exit criterion (§14.2) | [ ] | recorded JSONL fixtures + schema forward-compat; full sweep |

**Overall Phase 5: groups 0–8 done; 9–10 remain.** Branch `phase-5-sessions`
(current work on `sandbox`). Group 8 (macOS Seatbelt) landed on darwin 25.3 —
`sandbox: seatbelt` confirmed live; escape suite green.

### Prompt refactor (owner request, 2026-07-07)

Default prompts extracted from inline Rust consts to files, per two decisions
(core home + include_str!; version const + CHANGELOG + transcript stamp):

- `crates/emberly-core/prompts/{system.md, compact.md}` — authored as prose,
  embedded at build via `include_str!` (zero runtime dep; HC-2 safe). One home
  (core) instead of split across binary+engine.
- `emberly_core::prompts` module: `SYSTEM`/`COMPACT` (+ trimmed `system()`/
  `compact()`), and `VERSION` (independent of the crate version).
- `prompts/CHANGELOG.md` tracks prompt changes; `VERSION` is stamped into
  `session_start` as `prompts_version` (transcript **SCHEMA_VERSION → 2**, added
  as a `#[serde(default)]` field — old v1 transcripts still read).
- `config.rs` default + `init` materialization now read the core prompts; the
  `.agents/prompts/<name>.<family>.md` override path is unchanged.
- Content is a faithful port (no behavior change); improving the prose is now a
  file edit + `VERSION` bump, reviewable independently of code.

### Session navigation (owner request, 2026-07-07)

Make sessions navigable, not just persisted. Four asks, three commits:

- **CLI list + resume hint** (`55f8811`): `resume::list_sessions` summarizes
  every transcript (id, title, provider/model, age, events, interrupted);
  `emberly sessions` prints them newest-first with a per-session resume line.
  The clean-exit summary now prints the session id, path, and the exact
  `emberly resume <id>` command.
- **Engine in-session switching** (`832d442`): `Command::NewSession` /
  `Command::ResumeSession`, handled at idle. The engine ends the current
  transcript cleanly, rolls a fresh (or reopens the target) `FileTranscript`,
  resets/replaces the conversation, and writes the new `session_start` (fresh
  only). Target files are created/read *before* the old session ends, so IO
  failure leaves the running session intact (HC-7). A shared
  `Arc<RwLock<PathBuf>>` (`active_session_path`) keeps the host's panic/exit
  path pointed at the current session across switches.
- **TUI session menu** (`39fd1bd`): `/session` opens an interactive picker
  (selectable rows, Enter resumes; current session marked, can't resume itself)
  instead of a details dump. `/new` (alias `/clear`) starts a fresh session in
  place. Both switches are gated to idle in the frontend (refused mid-turn with
  a notice). The frontend orchestrates the switch and clears + reseeds the
  timeline; resume restores the prior conversation so the pane isn't blank.

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

- [x] **Project instructions (C-1):** `load_project_instructions` reads
      `AGENTS.md` (native); `CLAUDE.md` only when AGENTS.md is absent; both
      present → AGENTS.md wins with a one-time notice. Folded into the system
      prompt under a `# Project instructions` heading (pinned; never compacted).
- [x] **Prompts (C-1, P-7):** `load_prompt` resolves `.agents/prompts/
      <name>.<family>.md` then `<name>.md`, else the built-in default; `family_of`
      maps the model (claude / gpt / generic). Covers `system` and `compact`.
- [x] **Provenance (C-3):** `load` tracks per-field source (env > project >
      global > default) and records non-default pieces as `ConfigProvenance`.
      Fed into `session_start` (via `EngineConfig.config_provenance`) and
      `config show`.
- [x] **`emberly config show`:** prints values, prompt/instruction sources, an
      overrides list, and secret status (set/unset — never printed).
- [x] Session-start header (Design §8.2): plain-mode banner prints one line per
      override + any notice; silent about pure defaults. (Rich-mode surfacing is
      a later refinement; the data is in the transcript regardless.)
- [x] The system prompt + `/compact` prompt override flow to the engine
      (`EngineConfig.system` + new `summary_prompt`; `summarize` uses it or the
      built-in default).
- [x] Tests: AGENTS-wins + notice, CLAUDE fallback + none, family variant +
      fallback, layered-config provenance + system-prompt assembly. `config
      show` smoke verified.

---

## 6. `emberly init` (C-2, Design §8.1)

- [x] `init.rs` materializes `.agents/`: `config.toml` (commented), `prompts/
      {system,compact}.md` (the baked-in defaults — `DEFAULT_SYSTEM_PROMPT` +
      `emberly_core::SUMMARY_PROMPT`, now public/DRY), `permissions.toml`
      (documented Phase-2 rule-file template, reserving the pattern), and
      `.agents/.gitignore`. `write_if_absent` never clobbers.
- [x] **UX (Design §8.1):** lists exactly what was created (paths relative to
      the root) + one next command; "Nothing to do" when already set up.
- [x] `.agents/.gitignore` ignores `sessions/` (transcripts local); config +
      prompts stay shareable.
- [x] `init` subcommand wired in `main.rs`. Tests: creates the expected tree
      (+ system prompt is the default); re-run preserves an existing file while
      still creating the rest. Idempotent smoke verified.

---

## 7. CLI surface & key moments (Tech Spec §10, Design §8)

- [x] First-party `parse_args` → `Cli { Version, Init, ConfigShow, Run(RunOpts) }`
      (no extra dep, HC-2). Covers `emberly`, `init`, `resume [id]`,
      `config show`, `--plain`, `--model`, `--provider`, `--version`.
      `--model`/`--provider` are the highest-precedence tier (`cli`), recorded
      in provenance.
- [x] **First run** (Design §8.1): no `.agents/config.toml` still works on
      defaults and prints a one-line orientation pointing at `emberly init`.
- [x] **Session start / end** lines: plain banner shows model + overrides +
      notices; clean exit prints duration + transcript path (group 2). Start
      id/title live in the transcript.
- [x] Tests: `parse_args` table — subcommands short-circuit, flags accumulate,
      `resume` takes an optional id, missing values / unknown args error cleanly.
      Live smoke: `--provider/--model` override with `cli` provenance.
- Note: title/cost in the clean-exit line remain a later refinement (engine-side
      data); the README/`emb` alias docs land with the release group (9).

---

## 8. macOS Seatbelt (§6.3, Tech Spec §16) — the spike  **[x] COMPLETE**

- [x] **ABI-free approach (HC-1):** children run under
      `/usr/bin/sandbox-exec -p <profile>` — no `sandbox_init` FFI, no new
      dependency (std + system binary), so HC-1/HC-2 stay clean. Profile
      (`confine::macos::seatbelt_profile`): `(allow default)`, then a blanket
      write-deny, then re-allow writes under the project root, with `.git/`
      carved back out (HC-5) unless genuine git (§6.4), plus HC-4 approved paths
      and `/dev`. The harness process itself is never confined.
- [x] Wired into `emberly-sandbox` behind the **same seam as Landlock**: the
      new `confine::confined_invocation` dispatches per platform, so
      `HostSandbox` and the bash tool are unchanged. Probe reports the real
      `SandboxStatus` (`Confined { backend: "seatbelt" }` / `Unavailable`); the
      built binary prints `sandbox: seatbelt` at startup on this host.
- [x] **Degradation (§6.7):** the probe actually applies a trivial profile to a
      child (`seatbelt_available`), so a host where `sandbox-exec` is missing or
      blocked reports `Unavailable` and takes the existing degraded path
      (allowlist suspended, auto modes locked, per-action prompting, policy-level
      labeling) — no new degradation code needed.
- [x] `sandbox-exec` deprecation risk noted in `confine::macos`. The spike did
      **not** overrun; Seatbelt ships for v1 macOS.
- [x] Tests: `crates/emberly/tests/seatbelt_escape.rs` (macOS-gated, 5 tests) —
      in-root write ok, outside-root denied, `.git` write denied, git-writable
      allows `.git`, genuine `git commit` succeeds under the writable profile and
      is denied under the default; reads still work. Plus 3 profile unit tests.
      All green on this host (macOS 15 / darwin 25.3).

### Seatbelt — key findings

- **SBPL is last-match-wins** (specificity by order), so unlike Landlock
  (additive-only) it can genuinely *subtract* `.git/` from a writable root — the
  macOS profile is simpler and, unlike the Linux default profile, lets bash
  create brand-new top-level entries under the root.
- **Canonicalize the root.** Seatbelt matches rules against the *resolved* path;
  `/var`, `/tmp`, `$TMPDIR` are symlinks into `/private`, so an unresolved path
  silently never matches (safe-closed: it over-denies). `seatbelt_profile`
  resolves the root and every approved path.
- **Scope is the write lines.** Reads stay broad (`allow default`); the enforced
  hard lines are no-write-outside-root and no-`.git`-write. Read-confinement is a
  documented hardening follow-up (Landlock also denies outside-root reads).
- **No self-exec shim needed** on macOS: `sandbox-exec` *is* the wrapper, so
  `maybe_run_sandbox_shim` stays a Linux-only no-op.

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
- **Seatbelt via `sandbox-exec`, not FFI** (HC-1) — **done** (group 8). No
  deferral needed; the spike was short (SBPL profile + one dispatch fn, no new
  dep). macOS ships kernel-level child confinement for v1, same seam as Landlock.
- **Phase 2 (Landlock) remains deferred** until a Linux machine; Seatbelt gives
  macOS its safety tier in the meantime.
- **Deferred beyond v1** (designed-for, not built): auto-compaction (one
  threshold check in the accounting path), model-generated session titles,
  MCP/LSP/Skills, Windows.
