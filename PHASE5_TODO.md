# Phase 5 — Safety guardrails: workspace trust & loop-breaking — TODO & Progress

**Milestone:** M6 (Tech Spec §15) — the final, *safety* slice of the 0.2 set.
Gate the agent on a conscious trust decision before it runs in an unfamiliar
folder (FR-1), and guarantee a runaway loop always ends in a user decision, not
silent unbounded spend (S-5).
**Satisfies:** FR-1, S-5; Tech Spec §6.7, §7, §10, §3.1/§3.2 (events); Design
§8.4, §8.5. Pinned to **Req v0.5 / Design v0.5 / Spec v0.5**.
**Goal:** an untrusted root raises the gate and *decline* exits with no session
created; a project-local trust key is ignored; a trusted parent suppresses
re-prompting in a subfolder; `trust revoke` re-arms the gate. A scripted
re-treading loop trips `LoopHalted` while a progressing loop does not, and the
user's choice (resume / stop / steer) is honored and recorded.

**Depends on:** the shipped v0.1/v0.2 product only. Trust reuses the binary's
existing startup flow, the global-config path helpers, and the `keys.toml` 0600
enforcement pattern; the loop guardrail reuses the engine turn loop, the
`ingest_tool_result` file-change/result signal, and the `ask_user`
emit-request-then-await-command pattern (T-8). No new dependencies (Tech Spec
§12). **Fully developable on macOS** — the sandbox itself is untouched (trust is
explicitly *not* containment).

**Legend:** `[ ]` todo · `[~]` in progress · `[x]` done · `[!]` blocked

---

## Progress summary

| Group | Status | Notes |
|---|---|---|
| 1. Workspace trust — store, config, CLI (FR-1 data layer) | [x] | Done 2026-07-11; `trust.rs` store (0600, global-only) + subtree membership; `[trust] trusted_dirs` read only from global tier (project ignored w/ notice); `emberly trust list`/`revoke`; 10 tests |
| 2. Workspace trust — the startup gate + prompt (FR-1 gate) | [x] | Done 2026-07-11; pre-engine gate (canonicalize → check → prompt); decline `return Ok(())` (no session); accept writes store + engine records `TrustDecision`; non-tty declines cleanly; honesty line in-prompt; 2 engine tests |
| 3. Loop guardrail — detection + halt/await in the engine (S-5 core) | [x] | Done 2026-07-11; per-turn signature (normalized args—strips T-9 caption—+ result hash + modified-file set); trips on repeat_window/max_no_progress; `LoopHalted`+`ResolveLoop{Resume/Stop/Steer}`; `LoopHalt` transcript; `[loop]` config→engine; 6 tests |
| 4. Loop guardrail — the halt surface (S-5 UI, both frontends) | [x] | Done 2026-07-11; `LoopHaltPrompt` (menu + steer field), harness-voice `render_loop_halt` (chrome/warning, no safety band); g/s/t + Esc=stop; degraded `parse_loop_resolution`; motion stilled; 10 tests |
| 5. End-to-end, exit criterion, docs & SFD bookkeeping | [ ] | combined offline tests; README; **Spec bump decision (owner)** — trust realized as a pre-engine gate refines §3.1 `TrustRequest`; memory |

**Overall Phase 5: IN PROGRESS** (4 / 5 groups). Branch `phase5/trust-and-loop-guardrail` off
`version0.2` (Phase 4 merged).

---

## Sequencing rationale (implementation-doc decision)

Two independent features (trust touches the binary/config/CLI; the guardrail
touches the engine/frontend), so order is free. This TODO does **trust first**
(groups 1–2) — a self-contained binary+config slice that ships on its own — then
the **loop guardrail** (groups 3–4), then a combined exit (group 5). Within each
feature the data/engine layer precedes the UI, so every group leaves the tree
green with its offline `FakeProvider`/unit tests (Tech Spec §14.5) and
degraded-mode parity where it adds a surface.

---

## Platform & constraints (read first)

- **No new dependencies** (Tech Spec §12). Trust is std-fs + TOML over the
  existing config machinery; the guardrail is engine logic + a hash (reuse a
  `std`/existing hasher — no new crate).
- **Hard constraints unchanged.** HC-1 (safe Rust), HC-3 (panic-free core), HC-7
  (audit trail — trust and halt are transcript events). `#![forbid(unsafe_code)]`
  everywhere; `#![deny(unwrap_used, expect_used)]` on core/…/sandbox — in-lib
  tests use `match { Ok=>.., Err=>panic!() }`, never `.expect()`. The **binary**
  crate allows `expect` (its tests already do).
- **Additive-only across the boundary.** New `UiEvent`/`Command`/
  `TranscriptEvent` variants are additive on `#[non_exhaustive]` enums →
  **no `transcript::SCHEMA_VERSION` bump** (the `AskUser`/`ModelSwitch` note in
  transcript.rs covers this).
- **Trust is a consent gate, NOT containment (FR-1 honesty clause).** It lives in
  the **binary**, outside `emberly-sandbox`; it changes no ruleset, never widens
  HC-4/HC-5, never suspends a permission prompt, never enables an auto mode. The
  prompt says so in a dimmed line. Trust decides *whether* the agent runs here,
  never *what* it may do.
- **The trust store is global-only** (`~/.config/emberly/trust.toml`, 0600). A
  repository must never be able to pre-declare itself trusted — so a `[trust]`
  section in **project** config is ignored (with a one-time notice), and
  `trust.trusted_dirs` is read only from the **global** tier (FR-1).
- **The guardrail is a heuristic** (Requirements S-5). The guarantee is
  *termination-into-a-decision*, not perfect classification; it must **never**
  trip while files change or tool-result hashes differ (genuine progress). Label
  defaults "initial; tune with use" (Tech Spec §7).

---

## 1. Workspace trust — store, config, CLI  *(FR-1; Tech Spec §6.7, §10; Design §8.4)*

The data layer: a global trust store with membership, the config allowlist, and
the management CLI — everything except the startup prompt (group 2).

- [x] **Trust store module** (new, in the `emberly` binary — trust is a binary
      concern, outside `emberly-sandbox`). `~/.config/emberly/trust.toml` via a
      new `config::global_trust_path()` (next to `global_keys_path`). Entries are
      canonical paths with an `accepted` flag + timestamp. Read enforces 0600
      (reuse `enforce_private_permissions`); write creates the dir + file at 0600
      (a **new** create-then-`set_permissions(0o600)` helper — none exists).
- [x] **Membership = subtree trust** (FR-1): a canonicalized root is trusted if
      it *or any ancestor* is an accepted store entry, **or** matches the
      `trust.trusted_dirs` allowlist. `is_trusted(root) -> bool` + `record_trust(
      root)`.
- [x] **`[trust] trusted_dirs` config, GLOBAL TIER ONLY** (FR-1). Read only from
      the global config binding at load (never the project binding); a `[trust]`
      in project `.agents/config.toml` is dropped with a `Resolved.notices`
      warning ("ignoring project [trust] — trust is global-only"). Add a
      `TrustConfig { trusted_dirs: Vec<String> }` but resolve it off the global
      tier explicitly, so the tier separation is visible in code.
- [x] **Trust CLI** (Tech Spec §10): `emberly trust list` (print accepted paths +
      when) and `emberly trust revoke <path>` (canonicalize, remove the entry,
      confirm) — so revoking is never hand-editing a file. New `Cli::TrustList`/
      `Cli::TrustRevoke(String)` variants + a `"trust"` arm in `parse_args` + two
      dispatch arms that `return Ok(())` (mirror the `config show` two-level
      pattern).
- [x] Tests (binary crate — `expect` allowed): store round-trip; subtree
      membership (ancestor match trusts a subdir); allowlist match; **0600
      rejection** of a group-readable store (mirror `rejects_group_readable_keys_
      file`); project `[trust]` ignored; `revoke` removes an entry and re-arms.
- [x] `cargo fmt` + `clippy` + `test` green; commit.

---

## 2. Workspace trust — the startup gate + prompt  *(FR-1; Tech Spec §6.7; Design §8.4)*

The gate itself: checked in the binary **before** any project file is read into
the prompt and **before** any session exists.

- [x] **Canonicalize the root** at startup (`main.rs` does not today) and run the
      gate **between root-determination and `config::load`** — the only point
      that is after the root is known but before AGENTS.md/CLAUDE.md enter the
      prompt (Design §8.4) and before a session/transcript exists.
- [x] **The gate:** trusted (store ∪ allowlist) → proceed silently. Untrusted →
      the prompt. **Accept** → `record_trust(root)` then proceed. **Decline** →
      print one calm line and `return Ok(())` — **no session created** (the
      session file is opened later, so decline naturally starts nothing, FR-1).
- [x] **The prompt** (Design §8.4), a pre-engine stdin prompt (like
      `offer_resume`, works identically in rich and plain since it prints before
      either frontend takes the terminal): names the folder; states plainly what
      agreeing means ("Emberly will read, edit, and run commands in this
      folder"); a one-line nudge to review unfamiliar folders; **safe default =
      decline** (the default keypress does not grant; trusting is deliberate); a
      dimmed honesty line ("trusting this folder does not switch off later
      permission prompts"); ASCII TRUST / DON'T-TRUST framing. **Non-tty →
      declines cleanly** (guard on `is_terminal`, never hang a pipe).
- [x] **Audit record.** On **accept**, write `TranscriptEvent::TrustDecision {
      path, trusted: true }` as an early record once the session sink exists
      (Tech Spec §3.2 `trust_decision`). On **decline** there is no session, so
      the durable record is the store's *absence* of the path (nothing to write);
      note this honestly.
- [x] **Honesty:** the gate lives in the binary, imports nothing from
      `emberly-sandbox`, and touches no ruleset (FR-1 honesty clause) — a code
      comment states it.
- [x] Tests: `is_trusted` short-circuits the gate (trusted → no prompt path
      taken); the decision function returns decline for empty/"n" and accept only
      for a deliberate "trust"/"yes"; `TrustDecision` recorded on accept. The
      interactive prompt end-to-end is an owner manual smoke (needs a tty).
- [x] `cargo fmt` + `clippy` + `test` green; commit.

---

## 3. Loop guardrail — detection + halt/await in the engine  *(S-5; Tech Spec §7, §3.1/§3.2; Design §8.5)*

The engine detects a non-progressing loop at the turn boundary and hands control
back to the user.

- [x] **`[loop]` config → engine.** `LoopConfig { enabled (default true),
      repeat_window (default 3), max_no_progress_turns }` in config.rs
      (`#[serde(rename = "loop")]`, `loop` is reserved); merge per-field; resolve;
      thread to `EngineConfig.loop_config` and into `Engine` (mirror the
      `tool_explanations` path exactly). Defaults labeled "initial; tune with
      use" (Tech Spec §7).
- [x] **Per-turn signature** (Tech Spec §7): for the completed turn, the multiset
      of `(tool_name, normalized-args)` tuples + a hash of the concatenated
      tool-result content + the set of modified-file paths. **Normalize args**:
      strip the T-9-injected `explanation` property and canonicalize key order,
      so a caption change never looks like progress *or* masks a repeat.
      Accumulate in `run_tool_calls`/`ingest_tool_result` (where `outcome.content`
      and `outcome.file_change.path` are in hand).
- [x] **No-progress check** at the turn boundary (the "loop back for another
      completion" point, *before* the next `open_stream_with_retry`): a rolling
      `VecDeque` of the last `repeat_window` signatures; **trip when** they repeat
      tool-call signatures **AND** contribute no new modified file **AND** no new
      distinct result hash (cumulative sets). Never trip while `enabled == false`
      or while progress continues.
- [x] **On trip:** stop issuing provider calls; emit `UiEvent::LoopHalted {
      reason }` + write `TranscriptEvent::LoopHalt { reason, resolution: None }`;
      then **park `run_turn` on `commands_rx`** awaiting a resolution (mirror the
      `ask_user` await, but no oneshot — nothing is blocked, the turn simply waits
      for the user's decision).
- [x] **Resolution** (Design §8.5): a `LoopResolution { Resume, Stop, Steer(
      String) }` type + `Command::ResolveLoop { resolution }`. **Resume** →
      continue the loop (reset the no-progress window so it doesn't instantly
      re-trip). **Stop** → end the turn cleanly. **Steer(text)** → push the text
      as a user message and continue. Update the `LoopHalt` transcript record with
      the chosen resolution (Tech Spec §3.2 "reason + the user's chosen
      resolution").
- [x] Tests (`FakeProvider`): a script of `repeat_window` identical no-progress
      tool-call turns trips `LoopHalted` exactly once; a progressing loop (a new
      `file_change` each turn, or differing result content) does **not** trip;
      `Resume` continues; `Stop` ends; `Steer` injects and continues; `enabled =
      false` never trips.
- [x] `cargo fmt` + `clippy` + `test` green; commit.

---

## 4. Loop guardrail — the halt surface  *(S-5; Design §8.5, §7)*

The halt is a **harness-world** moment — the harness's own out-of-band voice,
not model output, and distinct from the question prompt (that is the *model*
asking; this is the *harness* stepping in).

- [x] **Rich TUI:** a `pending_loop_halt` state on `App`, set on
      `UiEvent::LoopHalted`; a surface that owns the keyboard like a decision
      prompt but in the **harness voice** (Design §6.1) — one calm line ("Stopped:
      the last few steps repeated without progress.") + three choices: **keep
      going** (resume) · **stop here** · **say something** (steer). "Say
      something" opens a free-text field (reuse `LineEditor`) → `Steer(text)`. No
      blame, no alarm styling; **not** the reserved safety band and **not** the
      question-prompt styling. Motion stilled while it is up (extend `is_
      deciding`).
- [x] **Line mode:** a plain harness-voice block + a prompt mapping input to
      resume/stop/steer (e.g. empty = resume? no — the user must choose; a small
      keyed menu, deliberate). Full degraded parity, no ANSI.
- [x] **Strings:** a `strings::loop_halt` module (harness-voice wording), shared
      by both frontends.
- [x] Tests: rich render (harness voice, three choices, not safety band); keys →
      the right `ResolveLoop` commands; steer opens the text field; line-mode
      parse + no-ANSI.
- [x] `cargo fmt` + `clippy` + `test` green; commit.

---

## 5. End-to-end, exit criterion, docs & SFD bookkeeping  *(Tech Spec §14.5; SFD G-10/G-11/G-16/G-24)*

- [ ] **Combined offline tests** proving the plan's "Done when": (trust) an
      untrusted root declines → no session; a trusted ancestor suppresses the
      prompt for a subdir; `revoke` re-arms. (loop) a re-treading script trips and
      each of resume/stop/steer is honored and recorded; a progressing script
      never trips.
- [ ] **README:** short "Trusting a folder" and "When the loop is broken"
      sections.
- [ ] **Spec bump decision (owner) — a real one this phase.** Implementation
      refined the trust design: it is realized as a **pre-engine binary gate**
      (Tech Spec §6.7), so **`UiEvent::TrustRequest{path}` (§3.1) is not used** and
      `trust_decision` is recorded only on accept (no session exists on decline).
      This is a downstream discovery that refines §3.1/§3.2/§6.7 (G-11/G-24) →
      propose a **minor Spec bump v0.5 → v0.6** absorbing it (and confirming the
      loop events land as specified). Surface to the owner; on approval, bump the
      Spec, refresh Req/Design companion pins (G-16), and record the feedback
      item. If declined, leave `TrustRequest` in §3.1 as a still-open door and
      note the divergence.
- [ ] **Memory:** note the trust store location/shape and the loop-guardrail
      defaults if non-obvious (per the memory rules — only what the code doesn't
      already say).
- [ ] Full suite + `fmt` + `clippy` green; final commit. This closes the 0.2
      feature set (M6) — all of P-8/P-9/P-10/T-8/T-9/C-5/C-6/FR-1/S-5.

---

## Decisions log

- **Branch:** `phase5/trust-and-loop-guardrail` off `version0.2` (Phase 4
  merged).
- **Trust is a pre-engine binary gate**, not an engine channel round-trip
  (Tech Spec §6.7: "checked in the binary at startup, before the engine begins
  the loop"). It runs before project files enter the prompt and before any
  session exists, so *decline = no session* falls out naturally. Consequence:
  no `TrustRequest` UiEvent / answer Command (a Spec-feedback item, group 5).
- **Trust prompt is a plain pre-frontend stdin prompt** (like `offer_resume`) —
  it prints before either frontend takes the terminal, so it is identical in
  rich and plain mode; non-tty declines cleanly. Reusing the rich TUI would
  require starting the frontend before the engine, which the paired
  frontend/engine architecture does not support — and would not be reshaped for
  this (guiding principle).
- **Trust store is global-only, 0600**; project `[trust]` is ignored with a
  notice (FR-1: a repo can't self-trust). `trusted_dirs` read only off the
  global config tier.
- **Loop resolution = `ResolveLoop { Resume | Stop | Steer(text) }`** — one
  explicit command rather than overloading `Cancel`/`UserInput`, so the halt
  transcript can record exactly what the user chose (Tech Spec §3.2).
- **Signature normalizes args** (strips the T-9 `explanation`, canonical key
  order) so a caption neither masks a repeat nor fakes progress.
- **The guardrail resets its window on `Resume`** so an explicit "keep going"
  doesn't instantly re-trip on the same signatures.
- **No schema bumps** — `TrustDecision`/`LoopHalt` are additive transcript
  variants (warn-skipped by old readers).
