# Emberly Code — Implementation Plan (0.4.1 feature set)

**Status:** 🚧 **in progress** — Phase 1 (completion gate, S-6) is **done**;
Phase 2 (document/PDF input, P-12 + T-16) is not yet started. Two phases
expanding Tech Spec milestone **M9** (the 0.4.1 cross-project feature set).
**Date:** 2026-07-15 (planned)
**Owner:** Wattanit
**Source documents** (G-14 as-built pin for the 0.4.1 release):
- Requirements Document v0.8 (`docs/emberly-code-requirements.md`) — WHAT/WHY
- Design Guideline v0.8 (`docs/emberly-code-design-guideline.md`) — UX/voice
- Technical Specification v0.9 (`docs/emberly-code-tech-spec.md`) — HOW

These three versions are the as-built truth for the 0.4.1 release, all
`Status: approved` (owner, 2026-07-15). A stale pin is a defect (G-16); when a
foundation document bumps, update this pin. Discoveries that change HOW flow
back into the Technical Specification as version bumps (G-24/G-25) — this plan
never becomes a shadow spec.

This plan expands Tech Spec §15 milestone **M9** into workable phases. It builds
on the shipped 0.4 capability-parity product (M8 — the task-list tool, multimodal
image input, persistent memory, the skill system, the harness-owned web-search
backend, and TUI mouse support), the 0.3 context-economy product beneath it (M7),
the 0.2 product beneath that (M6), and the 0.1 product beneath that (M1–M5 — the
six crates, the agent loop, both live providers, the full TUI, sessions / resume
/ compaction, the permission rule engine, and OS confinement on Linux + macOS).
The prior plans are preserved in `docs/version-0-1/` … `docs/version-0-4/` as the
earlier as-built records.

Unlike every prior feature set, 0.4.1 originates **outside** Emberly: it absorbs
two domain-agnostic requests from TREEGAL Yggdrasil, a separate product that
consumes Emberly as its engine (Requirements §2.1, 0.4.1 feature set; Tech Spec
§16 v0.9). The completion gate is loop-control — "the loop may not declare done
while registered checks fail," which for a coding deployment expresses "tests
must pass before done." Document input is the P-11/T-12 image pattern applied to
PDF. Both earn their place as Emberly capabilities on Emberly's own terms; a
third request (Windows supervised posture, FR-D) was **held, not absorbed** — the
requester targets macOS only for its prototyping stage — and is not in this plan.

---

## Guiding principles (apply to every phase)

- **The safety model is never widened (Requirements §6, HC-4/HC-5).** A
  completion check that runs a command executes through the **sandbox exactly as
  `bash`** (§6.2/§6.3 confinement, the `S-4` timeout) — it has **no privileged
  path around the safety model** (Requirements S-6 honesty clause, Tech Spec §7).
  It is not re-prompted per evaluation only because being registered in
  trust-gated config is the authorization, exactly as a project-config allowlist
  entry is (§6.1); containment still applies. A document read is an ordinary
  project read, path-normalized and root-confined exactly as `read_file`
  (§6.2, T-16). **`emberly-sandbox` is untouched** by both phases — neither
  capability is security-critical in the containment sense; they add surface
  *through* the existing rule/permission/sandbox layers, never around them.
- **Provider-agnosticism stays real, not nominal (Requirements §1, P-1).**
  Document support lives *behind* the `Provider` abstraction as a normalized
  `ContentBlock::Document` with per-adapter mapping and a `documents` capability
  flag (P-12, Tech Spec §4.1/§4.2) — never a vendor branch in the tool, the same
  reasoning that placed image support behind the abstraction (P-11). A model
  without document support gets a structured unsupported-capability result
  (HC-6), never a crash and never a silent drop.
- **The harness does not parse documents (HC-2).** `read_document` forwards the
  document bytes to the provider unparsed — it sniffs a leading `%PDF-` magic
  string (a first-party byte-prefix check, no parser) and base64-encodes; it
  never opens the PDF, so no page count, no extracted text, and **no
  PDF-parsing dependency** (Tech Spec §4.1/§5.2). This is the load-bearing fact
  that makes document input as cheap and as portable as image input.
- **No new dependencies (HC-2, Tech Spec §12).** Document input base64-encodes
  with the already-locked `base64` (from the 0.4 image work) and sniffs by byte
  prefix; the completion gate is engine/config logic running check commands
  through the existing `bash`/sandbox path. The 0.4.1 set adds nothing to the
  tree. Anything that appears to need a crate is a signal to re-check the design.
- **Additive events, no schema bump (HC-7).** The `CompletionGateHalted` UiEvent
  and the `completion_check` / `completion_gate_halt` transcript events, plus the
  `ContentBlock::Document` block, are all additive; older transcript readers
  warn-skip the new event types, so no `SCHEMA_VERSION` bump (Tech Spec
  §3.1/§3.2).
- **Every phase leaves a shippable, tested slice.** Each ends green with its
  offline, deterministic `FakeProvider`-driven coverage from Tech Spec §14.8, and
  with degraded-mode parity (ASCII, no color/motion/mouse reliance) wherever it
  adds or changes a UI surface (Design §7).

---

## Phase 1 — Completion gate (S-6) — ✅ DONE (2026-07-15)

**Goal:** Let a session hold the agent loop to registered pass/fail checks before
it may declare a task done — the model-driven analog of the loop guardrail (S-5):
S-5 stops a loop that re-treads, S-6 stops one that lands early. Inert until
something registers a check, so a session with no checks behaves exactly as today.

**Scope**
- **Check registration (Tech Spec §7).** The engine holds a
  `Vec<CompletionCheck>` populated at startup from config `[[completion.check]]`
  entries — `{name, command, expect_exit}` (pass iff the command exits with
  `expect_exit`). The same internal registration hook is exposed so a frontend or
  a tool may register checks (Requirements S-6); config is the shipped path and
  the one exercised here.
- **Evaluation timing (Tech Spec §7).** The gate evaluates on a *completion
  attempt* — the agent loop reaching a natural stop (an assistant turn with no
  tool calls). With no checks registered the gate is inert and the loop ends as
  today; with checks, each runs and any failure re-opens the loop.
- **Failure re-opens the loop (HC-6).** A failing check's name and structured
  reason — exit status + the §5.3-reduced tail of its output — are appended as a
  tool-result-shaped message the model reads and reacts to (agent-world, Design
  §8.7); the loop continues. A passing gate lets the loop terminate normally.
- **Checks run under §6, un-prompted (Tech Spec §7, §6.1).** A command check
  executes through the sandboxed `bash` path (§6.2/§6.3 confinement, `S-4`
  timeout) — no privileged path around the safety model (Requirements S-6). Not
  re-prompted per evaluation: config registration in a trusted root is the
  authorization, like an allowlist entry; containment still applies.
- **Bounded attempts → halt (Tech Spec §7, §3).** After `completion.max_attempts`
  failed completion attempts (**default `3`**) the engine stops issuing provider
  calls, emits `CompletionGateHalted{failing, attempts}` (UiEvent +
  `completion_gate_halt` transcript event), and awaits a
  `Command::ResolveCompletionGate` — `resume` (try again), `steer(text)` (guide
  the model), `stop`, or `finish`. `finish` is the **user override**: end the task
  as done over a still-failing gate, recorded with `override: true` — the gate
  binds the *model's* claim of done, never the user's authority (Requirements
  S-6). This is the same termination-into-a-decision guarantee as S-5 and prevents
  an S-6/S-5 standoff: a model that cannot satisfy a check cannot spin forever.
- **Two-register surface (Design §8.7).** A failed check is agent-world content
  the model reacts to; only the bounded-attempt halt speaks in the harness voice,
  offering **keep-going / steer / stop / finish-anyway**. Gate status is visible
  in the sidebar **only when checks are registered** (never a "None" stub, like
  Tasks/Memory). "Finish anyway" is labeled as an override, never presented as
  though the checks passed; a green gate is never styled as a guarantee beyond
  what the checks tested. Degraded parity: plain harness-voice lines, ASCII, the
  four choices as capitalized deliberate keys (Design §7).
- **Every evaluation is a `completion_check` transcript event** (name, pass/fail,
  reason — HC-7, Tech Spec §3.2).
- **Config (Tech Spec §8).** `[[completion.check]]` entries; `[completion]` —
  `enabled` (**default `true`**, inert without registered checks) and
  `max_attempts` (**default `3`**).

**Satisfies:** S-6; Tech Spec §7, §3.1/§3.2, §6.1, §8; Design §8.7; HC-6, HC-7;
Requirements §6, §11.

**Done when:** a `FakeProvider` script attempting completion with a registered
check failing asserts the failure re-opens the loop as a tool-result while a
passing check lets it terminate; a check command is proven to run through the
sandboxed `bash` path (contained, not re-prompted); the bounded-attempt halt
fires `CompletionGateHalted` after `max_attempts` with each resolution
(`resume`/`steer`/`stop`/`finish`) behaving correctly and `finish` recording
`override: true`; and an inert gate (no registered checks) leaves loop
termination unchanged (Tech Spec §14.8, S-6).

---

## Phase 2 — Document (PDF) input (P-12, T-16)

**Goal:** Let the model *read* a document — a PDF already in the project — into
context as a document content block, behind the provider abstraction so it works
across vendors and degrades cleanly on models without document support. The
P-11/T-12 image pattern applied to documents, same shape, same fallback, same
permissions.

**Scope**
- **Normalized document content block (Tech Spec §4.1).** Add
  `ContentBlock::Document{media_type, data}` (`data` = base64 of the file bytes)
  to the normalized message type alongside text/image/tool blocks; no wire type
  crosses the boundary (P-1). Add a `documents: bool` capability to `ModelInfo`
  (per-model config flag, **default `false`**), so a document is never sent to a
  model not declared document-capable (P-12). The harness never parses the
  document — it forwards the bytes (HC-2).
- **Per-adapter mapping (Tech Spec §4.2).** The `anthropic` adapter maps to a
  `document` content block (base64 `source` of media type `application/pdf`); the
  `openai` adapter maps to the endpoint's file/document input part where it
  supports one, else the model is declared `documents:false` and `read_document`
  returns the unsupported-capability result rather than sending. Validate both
  against live endpoints (M9 open item, Tech Spec §16).
- **`read_document` built-in tool (Tech Spec §5.2).** Path normalized and
  root-checked exactly as `read_file`, governed by the same project-read rules
  (§6.2). Sniff the type by magic bytes (PDF: a leading `%PDF-` — first-party, no
  parser, HC-2), reject a non-PDF or a file over `document.max_bytes`
  (**default 32 MiB**), base64-encode (`base64`), append a
  `ContentBlock::Document`. Reports file size and format only — **never a page
  count or extracted text** (unparsed). On a non-`documents` model, return the
  structured unsupported-capability result (HC-6) instead of sending — the model
  learns it could not read the document rather than assuming it did.
- **Default rule (Tech Spec §6.1).** `read_document` inside the root → `allow`,
  exactly as `read_file`/`read_image` (§6.2, a project read).
- **Reference-line render, no pixels (Design §4.11).** A `read_document` result
  renders as a labeled reference line (`name · size · format`); **no in-terminal
  document rendering** and no page count. An unsupported-document result renders
  as a calm tool-result note, not a harness error (Design §6.1). Degraded mode:
  the same reference line, ASCII-only.
- **No new dependencies (Tech Spec §12).** `base64` is already locked (0.4 image
  work); the `%PDF-` sniff is a first-party byte-prefix check. `emberly-sandbox`
  and the dependency tree are untouched.
- **PDF only (Requirements §2.3).** Non-PDF word-processor formats (docx) are out
  of scope, and document creation/editing are skills (FR-7), not core — neither
  is built here.

**Satisfies:** P-12, T-16; Tech Spec §4.1/§4.2, §5.2, §6.1, §8, §12; Design
§4.11, §6.1; HC-2, HC-6; Requirements §4, §5.

**Done when:** a document round-trip asserts `read_document` root-confines its
path, rejects an oversize / `.git/` / non-PDF (magic-byte) file, appends a
`ContentBlock::Document`, and — on a `documents:false` model — returns the
structured unsupported-capability result instead of sending; both adapter
mappings are exercised via `FakeProvider` (Tech Spec §14.8, P-12/T-16).

---

## Phase dependency summary

```
0.4 product (M8) on 0.3 (M7) on 0.2 (M6) on 0.1 (M1–M5)
   ├─> Phase 1  Completion gate (S-6)          ── engine/config loop-control; sandbox untouched
   └─> Phase 2  Document input (P-12, T-16)     ── provider layer + read tool; no new deps
```

The two phases are **fully independent** and share no machinery — they may be
reordered or parallelized. Phase 1 is engine/event/config work in
`emberly-core` (the completion gate lives beside the S-5 guardrail in Tech Spec
§7, running check commands through the existing `bash`/sandbox path); Phase 2 is
provider-layer + tool work in `emberly-providers`/`emberly-tools`, mirroring the
0.4 image phase almost exactly. The default order 1→2 leads with the priority-1
loop-control capability; nothing depends on that choice.

---

## Not in this plan

- **Windows supervised posture (FR-D)** — **held, not absorbed** (Requirements
  0.4.1 feature set; the requester targets macOS only for its prototyping stage).
  Tech Spec §13's Windows-deferred posture is unchanged. If revisited it is
  likely a major bump and must touch HC-4/HC-5 (they become structurally
  policy-level-only on a platform with no confinement implementation). Not built
  here.
- **Non-PDF document formats (docx, other word-processor formats)** — out of
  scope (Requirements §2.3); the harness reads PDF as a provider-native
  passthrough and no others. Conversion, if ever wanted, belongs outside the
  harness or in a skill (FR-7). Not built here.
- **Document creation and editing** — a domain capability that ships as a skill
  (FR-7), never harness core (Requirements §2.3). Not built here.
- **Compile-time tool profiles (FR-B)** — withdrawn by the requester before
  absorption (it would have compiled away the command-execution path the skill
  system needs). No mechanism is built.
- **Provider-native / server-side capabilities** — Emberly capabilities stay
  harness-owned and provider-agnostic by decision (Requirements §1, §2.2); the
  document block maps to each provider's native representation but introduces no
  vendor-only path. The tool seam (T-7) is not reshaped.
- **MCP, headless/IDE/server frontends, user theming** — remain deferred per
  Requirements §2.2; their seams stay unused, not reshaped (A-1, T-7).
- **Release-pipeline / prebuilt binaries** — install-from-source continues unless
  the owner reopens it; 0.4.1 does not revisit it.
- **Agent-quality evaluation** (does it code well) — out of scope per Tech Spec
  §14; post-release discipline with separate tooling.

---

## Open items (carried into the phases per G-11; resolvers named)

These are the Tech Spec §16 v0.9 open items the 0.4.1 phases resolve. Each is
owned by its phase; discoveries that change HOW flow back into the Technical
Specification as version bumps (G-24/G-25).

- **Completion-gate defaults and registration (S-6, Phase 1).** `max_attempts = 3`
  is a placeholder. Resolve: tune so the gate stops a premature landing without
  recreating an S-5 spin; confirm the `expect_exit` command-check shape covers the
  common coding checks (test/lint/build); and validate the frontend/tool
  registration hook against a real non-config registrant when one exists (Tech
  Spec §16).
- **Document formats, caps, and adapter mapping (P-12/T-16, Phase 2).** PDF only,
  a 32 MiB byte cap, and the `%PDF-` magic-byte sniff are the initial set.
  Resolve: confirm the `document` block maps cleanly to the Anthropic native
  document block and to each OpenAI-compatible endpoint's document input (or is
  correctly declared `documents:false`) against live endpoints, and that provider
  page/token limits surface as clean unsupported/oversize results rather than
  crashes (Tech Spec §16).
