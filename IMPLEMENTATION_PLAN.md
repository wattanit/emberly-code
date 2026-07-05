# Emberly Code — Implementation Plan

**Status:** draft
**Date:** 2026-07-06
**Owner:** Wattanit
**Source documents:**
- Requirements Document v0.3 (`docs/emberly-code-requirements-v0.3.md`) — WHAT/WHY
- Design Guideline v0.3 (`docs/emberly-code-design-guideline-v0.3.md`) — UX/voice
- Technical Specification v0.1 (`docs/emberly-code-tech-spec-v0.1.md`) — HOW

This document sequences the build into workable phases (milestones). It
adopts the milestone spine from Tech Spec §15 (M1–M5), prefixed with a
foundational Phase 0 for workspace scaffolding and CI gates that every later
phase depends on. Each phase lists its goal, scope, the requirement/spec IDs
it satisfies, its key deliverables, and an explicit exit criterion ("done
when…"). Phases are ordered so that each proves one coherent slice of the
product and leaves the tree green.

---

## Guiding principles (apply to every phase)

- **Hard constraints are not deferrable.** HC-1 (safe Rust), HC-3 (panic-free
  core), HC-6 (tool failures are data), and HC-7 (audit trail) are wired in
  from Phase 0/1, not bolted on later. Lint gates enforce HC-1/HC-3 in CI
  from the first commit.
- **The seams exist before the second implementation does.** The `Provider`,
  `Tool`, and frontend/engine boundaries are traits/channels from Phase 1,
  even while only one implementation (fake provider, line-mode frontend)
  exists — this is what makes A-1/A-2/A-3, P-1, and T-7 cheap later.
- **Every phase leaves a shippable, testable slice.** No phase ends with a
  broken build or a half-migrated abstraction.
- **Serializable-from-day-one.** All engine events are serializable from
  Phase 1 (A-3), which is what makes the transcript (HC-7) and replay tests
  (A-2) inexpensive in Phase 5.

---

## Phase 0 — Foundation & CI gates

**Goal:** A compiling, lint-gated Cargo workspace with the six-crate skeleton
and the dependency policy enforced. Proves the *guardrails*, not any feature.

**Scope**
- Cargo workspace with the six crates per Tech Spec §1:
  `emberly-core`, `emberly-providers`, `emberly-tools`, `emberly-sandbox`,
  `emberly-tui`, `emberly` (binary). One-way dependency direction wired and
  verified.
- Lint policy (Tech Spec §1): `#![forbid(unsafe_code)]` on every crate;
  `#![deny(clippy::unwrap_used, clippy::expect_used)]` on core/providers/
  tools/sandbox. `anyhow` confined to the binary; `thiserror` in libraries.
- Initial locked dependency set (Tech Spec §12), `rustls`-only, no C deps
  (HC-2). `Cargo.lock` committed.
- CI pipeline (Tech Spec §13): fmt, clippy-as-errors, test, `cargo deny`,
  `cargo vet`, and a `x86_64-unknown-linux-musl` static build check.
- Repository hygiene: `README`, `.gitignore`, workspace `rust-toolchain`,
  license headers.

**Satisfies:** HC-1, HC-2, HC-3 (lint scaffolding), §10 dependency policy,
Tech Spec §13.

**Done when:** `cargo build`, `cargo clippy`, `cargo deny`, and the musl
build all pass in CI on an empty-but-structured workspace; adding an `unwrap`
to a core crate fails CI.

---

## Phase 1 — Core engine (adopts M1)

**Goal:** Prove the core. A working agent loop driven by a scripted fake
provider, with read/bash/edit tools and per-action permission prompts, no OS
sandbox yet, line-mode output. This is the first end-to-end "it does a task"
slice.

**Scope**
- **Event model & channels (A-1, A-3):** `UiEvent` and `TranscriptEvent`
  enums (serializable), the engine as single-owner async task communicating
  over `mpsc` channels only — no shared mutable state (Tech Spec §2, §3).
- **Agent loop (`emberly-core`):** sequential completion → tool-call →
  tool-result iteration, one in-flight completion at a time.
- **`FakeProvider` (A-2):** implements `Provider` from a script of canned
  `StreamEvent`s (text, tool calls, errors, mid-stream drop). All later
  engine testing rides on this.
- **`Tool` trait + four tools:** `read_file`, `write_file`, `edit_file`,
  `bash` (Tech Spec §5.1–§5.2). `ToolOutcome` is always structured data, never
  a harness error (HC-6). Edit failure messages distinguish no-match vs
  N-matches with a fuzzy hint (T-3). `write_file` refuses `.git/` at the tool
  layer (HC-5 tool-level check, independent of the OS sandbox).
- **Truncation at ingestion (§8.1, Tech Spec §5.3):** head/tail elision with
  marker, deterministic, no model call. (Sidecar `full_output_ref` wiring may
  be stubbed until Phase 5's transcript; truncation math itself lands here.)
- **Permission gate (rule-layer only):** per-action ask/allow/deny flow as a
  `Command`/`UiEvent` round trip; denial returns to the model as structured
  data (HC-6, §6.6). No OS confinement and no rule config file yet.
- **Bash execution safety (S-4):** `tokio::process` under a timeout, child in
  its own process group, cancel kills the tree.
- **Line-mode frontend:** minimal append-only output; enough to drive and
  observe the loop. (Not the real TUI — that is Phase 4.)
- **Supervisor skeleton (HC-3):** panic hook + terminal restore + clean exit
  path (transcript persistence fills in at Phase 5).

**Satisfies:** A-1, A-2, A-3, T-1/T-2/T-3/T-4, HC-6, §8.1 (truncation), §6.6
(denials-as-data), S-4, HC-3 (skeleton).

**Done when:** a scripted fake-provider session completes a multi-step task
(read a file, propose an edit, prompt for permission, run a bash command)
end-to-end in line mode, with denials routed back to the model as data, all
under the panic-free lint gate. Detailed task breakdown in `PHASE1_TODO.md`.

---

## Phase 2 — Safety story (adopts M2)

**Goal:** Prove the safety story. Real OS-level containment on Linux, the full
rule engine, the degradation policy, and the escape-test suite that gives
HC-4/HC-5 regression teeth.

**Scope**
- **Landlock confinement (Tech Spec §6.2):** `landlock` crate, ABI probe at
  startup, ruleset applied to every spawned child (project-root read/write,
  `.git/` no-write, system paths read/exec, everything else denied). Harness
  process itself never confined.
- **Rule engine (Tech Spec §6.1):** (tool, matcher) → allow/ask/deny,
  most-specific-first, precedence built-in → global → project
  `.agents/permissions.toml` → session grants. Default bash allowlist.
- **`.git/` protection (HC-5, §6.3, Tech Spec §6.4):** file-tool hard deny;
  genuine-git binary resolution for the `.git/`-writable profile.
- **Project-root confinement (HC-4):** enforced at sandbox layer; approved
  outside-root paths added to a single invocation's ruleset only.
- **Modes (§6.4, Tech Spec §6.6):** normal / auto-accept-edits / auto, as
  type-enforced engine state gated on sandbox status.
- **Degradation policy (§6.7, Tech Spec §6.5):** probe failure → notify,
  suspend allowlist, lock auto modes, keep per-action prompting; policy-level
  labeling; `sandbox.require` opt-in.
- Remaining two tools: `glob`, `grep` (root-confined, `.gitignore`-aware).
- **Escape-test suite (Tech Spec §14.3):** write-outside-root, `.git/` via
  tool and via bash, fake/aliased/in-repo git, genuine git commit, degraded
  environment — all as CI regression tests on a Landlock kernel.

**Satisfies:** HC-4, HC-5, T-5, T-6, §6 (all), S-1 (musl/static sandbox path).

**Done when:** the escape-test suite passes on a Landlock-enabled CI kernel
and produces the exact §6.5 behavior in a Landlock-disabled container.

---

## Phase 3 — Live providers (adopts M3)

**Goal:** Prove P-1..P-6. Two live provider backends behind the abstraction,
streaming, retries, and token/cost accounting.

**Scope**
- **Anthropic Messages API client (Tech Spec §4.2):** content blocks,
  `tool_use`/`tool_result` mapping, SSE streaming. Thin `reqwest` client, no
  SDK (P-4).
- **OpenAI-compatible client:** `tool_calls` mapping, SSE, configurable base
  URL (transitively covers Ollama/vLLM/OpenRouter — P-2).
- **First-party SSE parsing** (Tech Spec §16 open item — decide vs
  `eventsource-stream`, bias first-party).
- **Retries & failure handling (Tech Spec §4.3, S-3):** backoff+jitter on
  connect/429/5xx honoring `Retry-After`, max 3, every retry surfaced;
  mid-stream drop keeps partial text, retries whole turn; post-retry failure
  is a harness-world error, session stays resumable.
- **Token & cost accounting (P-6, Tech Spec §4.4):** authoritative usage when
  offered, chars/4 estimate otherwise; per-model pricing table; cost labeled
  "est."
- **Secrets handling (Tech Spec §8):** env vars first, `keys.toml` with `0600`
  enforcement; auth headers redacted in transcript.

**Satisfies:** P-1, P-2, P-3, P-4, P-5, P-6, S-3.

**Done when:** both live backends complete a one-tool-use round trip in the
manual/nightly smoke suite (Tech Spec §14.4), streaming and retries observable,
context-usage and cost figures driven by real usage data.

---

## Phase 4 — Full TUI (adopts M4)

**Goal:** Prove the Design Guideline. The real `ratatui` interface: panes,
sidebar, palette, diff overlay, Thai text handling, and degraded-mode parity.

**Scope**
- **Layout (Design §3, Tech Spec §9):** main conversation pane + collapsible
  right sidebar (wordmark, session title, root, model block with context %/
  cost/sandbox status, modified-files list) + one-line status bar;
  auto-collapse below 100 columns.
- **Rendering (Design §4):** first-party markdown subset; `syntect`
  (fancy-regex backend) syntax highlighting; first-party unified diffs inline
  and as a pane overlay; `/view` `$EDITOR` escape hatch.
- **The permission prompt (Design §5):** full-content-always, loud reserved
  safety styling for outside-root, deny-as-default, forbidden-pattern
  compliance, in-situ "why" line. **The most important screen** — built with
  care.
- **Thai text (§2.1, Design §6.2, Tech Spec §9):** all width/wrap/cursor math
  via `unicode-segmentation` + `unicode-width`; first-party grapheme-aware
  line editor; test fixtures with stacked Thai vowel/tone marks.
- **Command palette (Design §3.3):** Ctrl+P fuzzy match over a single command
  registry that also serves `/commands` and help.
- **Motion (Design §6.4):** single ~12fps ticker; ember-pulse spinner,
  streaming glow, overlay ease-in, sidebar settle; skipped in degraded mode /
  permission prompts / `motion=false`.
- **Degraded mode (Design §7, Tech Spec §9):** `NO_COLOR`/`TERM=dumb`/
  `--plain` line-oriented output; a tested, supported configuration and the
  contract for a future headless frontend.
- **Centralized theme & string table (Design §2, §6.2):** one theme
  definition, one string module.

**Satisfies:** §2.1 (Thai), A-1 (second frontend consuming the same events),
all of Design Guideline §2–§7.

**Done when:** the TUI drives a full session; Thai-fixture and degraded-mode
tests pass; the permission prompt meets every Design §5 guarantee in both
rich and degraded modes.

---

## Phase 5 — Sessions, config & release (adopts M5)

**Goal:** Prove v1. Durable sessions, resume, manual compaction, init/config
provenance, macOS confinement, and the release pipeline.

**Scope**
- **Transcript persistence (HC-7, §8.2, Tech Spec §3.2):** append-only JSONL,
  one file per session, `v`-versioned events, per-event flush, sidecar files
  for truncated outputs (`full_output_ref`). Nothing ever rewrites a line.
- **Resume (§8.2, Tech Spec §3.3):** replay transcript into conversation view,
  applying compaction events as view transforms; unknown event types warn,
  not crash; offer-on-next-launch flow (Design §8.3).
- **Manual `/compact` (§8.3, Tech Spec §7):** clean-boundary-only,
  purpose-built summarization prompt, pinned content never compacted, failure
  falls back to hard truncation with warning, compaction recorded as event.
- **Context usage indicator (§8.4):** always-visible percentage against
  window − reserved-output budget.
- **Configuration & prompts (§7, Tech Spec §8):** two-tier resolution +
  project instructions (AGENTS.md native, CLAUDE.md compat with precedence
  notice); `emberly init` materialization (C-2, Design §8.1); `emberly config
  show` provenance (C-3); per-model-family prompt variants (P-7, C-4).
- **macOS Seatbelt (§6.3, Tech Spec §16):** confinement profile for children;
  budget spike time (thinly documented API).
- **Supervisor completion (HC-3, S-2, Tech Spec §10):** abnormal-exit path
  persists transcript, appends `abnormal_exit`, restores terminal, prints
  resume hint.
- **CLI surface (Tech Spec §10):** `emberly`, `init`, `resume [id]`,
  `config show`, `--plain`, `--model`, `--provider`, `--version`.
- **Release pipeline (Tech Spec §13):** all v1 targets (musl x86_64/aarch64,
  aarch64-darwin; best-effort x86_64-darwin); release checklist incl.
  name-collision check and `--plain` smoke run.
- **Transcript replay tests (A-2, Tech Spec §14.2):** recorded JSONL fixtures
  + schema-version forward-compat fixtures.

**Satisfies:** HC-3 (complete), HC-7, S-2, §7 (all), §8 (all), C-1/C-2/C-3/C-4,
P-7, §6.3 (macOS), Tech Spec §13.

**Done when:** a session survives an abnormal exit and resumes cleanly;
`/compact` round-trips through the transcript; `init`/`config show` report
correct provenance; macOS confinement passes an escape-test analogue; all
release targets build.

---

## Deferred (designed-for, not built in v1)

Per Requirements §2.2, the seams for these exist but no implementation ships:
MCP client (T-7 keeps `Tool` transport-agnostic), automatic compaction
(one threshold check in the accounting path), headless/server/IDE frontends
(the event boundary from Phase 1), user theming (centralized theme from
Phase 4), and Windows support.

---

## Phase dependency summary

```
Phase 0 (foundation)
   └─> Phase 1 (core engine, fake provider) ── proves the core
          ├─> Phase 2 (sandbox + rules)      ── proves safety
          ├─> Phase 3 (live providers)       ── proves P-1..P-6
          └─> Phase 4 (full TUI)             ── proves design
                 └─> Phase 5 (sessions/config/release) ── proves v1
```

Phases 2, 3, and 4 all depend on Phase 1 but are largely independent of each
other and could be parallelized; Phase 5 depends on all prior phases (resume
needs the transcript, config provenance needs the config system, the release
pipeline needs everything). The plan sequences them 2→3→4→5 as the default
single-track order, matching Tech Spec §15.
