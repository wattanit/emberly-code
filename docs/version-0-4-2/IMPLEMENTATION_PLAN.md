# Emberly Code — Implementation Plan (0.4.2 feature set)

**Status:** ✅ **done** — Phase 1 (guided provider/model setup, C-7) and Phase 2
(post-ship hardening) are both **done**. Written as-built, after the fact —
see the note under "How this plan was written" below.
**Date:** 2026-07-21 (Phase 1); 2026-07-21–2026-07-22 (Phase 2)
**Owner:** Wattanit
**Source documents** (G-14 as-built pin for the 0.4.2 release):
- Requirements Document v0.9 (`docs/emberly-code-requirements.md`) — WHAT/WHY
- Design Guideline v0.9 (`docs/emberly-code-design-guideline.md`) — UX/voice
- Technical Specification v0.10 (`docs/emberly-code-tech-spec.md`) — HOW

These three versions are the as-built truth for the 0.4.2 release, all
`Status: approved` (owner). A stale pin is a defect (G-16); when a foundation
document bumps, update this pin. Discoveries that change HOW flow back into
the Technical Specification as version bumps (G-24/G-25) — this plan never
becomes a shadow spec.

This plan expands Tech Spec §15 milestone **M10** into workable phases. It
builds on the shipped 0.4.1 product (M9 — the completion gate, document/PDF
input), the 0.4 capability-parity product beneath it (M8), the 0.3
context-economy product beneath that (M7), the 0.2 product beneath that (M6),
and the 0.1 product beneath that (M1–M5). The prior plans are preserved in
`docs/version-0-1/` … `docs/version-0-4-1/` as the earlier as-built records.

### How this plan was written

Unlike every prior version's plan, this one was **not** written before
implementation — the guided-setup wizard and the Phase 2 fixes below were
already built and shipped when this document was drafted, as an owner-directed
documentation cleanup pass. It records what was actually built and why, in the
same structure the prospective plans use, rather than a task list checked off
after the fact. Phase 2 in particular originated as a live bug-report list
from dogfooding the 0.4.2 branch, not a pre-planned scope — each item below
cites the Requirements/HC id(s) its fix restores, since none of them are new
capability.

---

## Guiding principles (apply to both phases)

- **The seams already exist — use them, don't reshape them.** The wizard
  writes through a new frontend-injected trait (`ProviderProfileWriter`,
  mirroring the existing `ConfigReloader` seam, A-1) and reuses the existing
  `Command::ReloadConfig` apply path (C-5) — no new transcript event, no
  `SCHEMA_VERSION` bump.
- **No new dependencies (Tech Spec §12).** The wizard's `config.toml` write is
  generic `toml::Table` manipulation with the already-locked `toml` crate
  (parse-mutate-reserialize; comments/formatting elsewhere in the file are not
  guaranteed to survive — an accepted tradeoff, §16). Every Phase 2 fix is
  engine/tool/TUI logic over the existing crate set; none added a dependency.
- **Hard constraints hold, unchanged.** HC-5 (`.git/` protection) and HC-6
  (tool failures are data) are exactly what two Phase 2 fixes restore after
  they'd drifted (the quote-aware genuine-git check; the parallel-tool-call id
  collision) — neither constraint's *meaning* changed, only broken
  implementations of them were corrected.
- **Every fix ships with a regression test that would have caught it.** Phase
  2's fixes are bug corrections, not features with acceptance criteria decided
  up front — each one's "done when" is the specific regression test added
  alongside it, run against the exact failure shape reported.

---

## Phase 1 — Guided provider/model setup (C-7) — ✅ DONE (2026-07-21)

**Goal:** Let a user add a new `[providers.<name>]` profile — name, adapter,
endpoint, model id, API key — from inside a running session via a guided
step-by-step wizard, instead of hand-editing `.agents/config.toml` +
`~/.config/emberly/keys.toml`. Indistinguishable from a hand-placed profile:
project-tier (C-1), provenance-visible (C-3), live-reloads via the existing
`Command::ReloadConfig` path (C-5), immediately selectable via the existing
model picker (C-6). Never prints or transcripts the key, matching a
hand-placed `keys.toml` entry's own guarantee.

**Scope**
- **Prerequisite: `/config` in-app edits actually live-reload (C-5, commit
  `c6349f5`).** Building the wizard surfaced that a manual `$EDITOR` edit to
  `.agents/config.toml`, on save, did not actually apply to the running
  session in every case — fixed in `engine.rs`/`factory.rs`/`rules.rs`
  first, since the wizard's whole "no separate success voice" design (below)
  depends on `Command::ReloadConfig` being trustworthy.
- **`ProviderProfileWriter` trait (`emberly-core`).** A frontend-injected seam
  (mirrors `ConfigReloader`) so `emberly-tui` can create a `[providers.<name>]`
  profile + its `keys.toml` entry without owning either file's TOML schema
  (A-1). Implemented by the `emberly` binary composition root
  (`ConfigWriter` in `provider_write.rs`): a create-only write (a name
  collision refuses, pointing at raw editing for changes), `auth.scheme`
  chosen by adapter (`x-api-key` for `anthropic`, `bearer` otherwise, matching
  `builtin_profiles()`'s existing convention), and the key written to
  `keys.toml` at `0600` from creation (no write-then-chmod window).
- **Six-screen wizard (Design §4.6):** profile name → adapter (with a wire-
  format-not-brand explainer) → endpoint → model id → API key (masked,
  `•` per keystroke) → summary (key redacted to its last 4 characters).
  Reachable as a trailing "+ add new provider…" row in the existing
  model/provider picker (§3.1, C-6) — not a new top-level command.
- **Model-carryover fix.** The model picker previously reused whatever model
  was last active for *any* newly-selected profile. The wizard writes its
  model id as profile metadata and the picker now looks it up per-profile,
  so a freshly-created profile is tried with the model the user actually
  configured, not a leftover one.
- **Design Guideline touch-up (§4.6, no version bump — owner's call at the
  time):** added the profile-name screen and the adapter wire-format
  explainer that the plan above surfaced as missing from the original
  4-screen sketch.

**Satisfies:** C-7; C-1, C-3, C-5, C-6; Tech Spec §8, §9, §12, §16; Design
§3.1, §4.6.

**Done when:** a fresh profile added via the wizard (create-only; a name
collision refused) writes `[providers.<name>]` + the matching `keys.toml`
entry at `0600`, fires the same reload story as a manual edit (no separate
success voice, no auto-switch), and is immediately selectable from `/model`
with the model id the wizard collected — all covered by unit tests in
`provider_write.rs` and `app.rs`, plus a full `cargo test --workspace` /
clippy (`-D warnings`) / fmt pass.

---

## Phase 2 — Post-ship hardening — ✅ DONE (2026-07-22)

**Goal:** Fix the concrete bugs the owner hit dogfooding the 0.4.2 branch —
each restores an existing guarantee rather than adding scope. Grouped here as
one phase because they share no machinery and were found/fixed independently,
one bug-report at a time, in the order below.

**Scope**
- **Compaction could split a `(tool_use, tool_result)` pair (FR-3, FR-4;
  commit `e7ad2bd`).** `Engine::compact` chose its summarize/keep cut point by
  raw message-array index rather than turn boundary, which could leave a
  `Role::Tool` message with no preceding `tool_calls` — an HTTP 400 on the
  next request. Fixed by reusing `group_turn_starts`, the primitive
  `windowed_messages` already used correctly. Same commit gave plain/line
  mode `/compact` parity with the rich TUI (it had no command for it and
  silently dropped `CompactionStatus`).
- **Command palette had no deliberate order (commit `ee87feb`).** Regrouped
  by frequency of use (owner's call): session start/switch first, then the
  runtime switches used constantly mid-session, then inspection/control, then
  setup/tuning, with `help`/`quit` last.
- **`Esc`/`Ctrl+C` didn't cancel an in-flight turn in the rich TUI (commit
  `84ccd82`).** `Command::Cancel`'s own doc comment and the README both
  promised this; neither key was actually wired to it — `/cancel` typed out
  was the only working path. Fixed in `app.rs`'s top-level key match.
- **`write_file`'s permission overlay didn't show new-file content (HC-6;
  commit `6df1af0`).** New/empty-file writes collapsed to a one-line `Create
  foo (N lines)` summary instead of the actual content, unconditionally —
  `edit_file` never had this gap. `write_file` now always builds a full
  unified diff, matching `edit_file`.
- **Parallel tool calls could corrupt each other's content on OpenAI-
  compatible backends (HC-6; commit `68af736`).** Some backends omit
  `tool_calls[].id` on every chunk of a parallel call; the mapper's
  empty-string fallback let two such calls collide on the same id, silently
  merging their argument buffers (garbling embedded `\n` escapes). Fixed by
  falling back to the wire's own `index`, which is unique per parallel call
  by construction.
- **Sending a message with no provider configured replayed a stale Phase 1
  placeholder reply (commit `cecdbe2`).** The engine now refuses before ever
  calling the placeholder `Provider`, with a `Notice` pointing at `/model`,
  rather than round-tripping a v0.1 relic string as if it were a real turn.
- **The genuine-git shell-metacharacter check was quote-blind (HC-5; commit
  `61ea285`).** `is_genuine_git` scanned the whole raw command string for
  shell metacharacters with no awareness of quoting, so a `git commit -m
  "..."` whose message safely contained e.g. `(`, `;`, or `&` inside the
  quoted argument lost the `.git/`-writable Seatbelt grant — `git` then
  failed to create `.git/index.lock` with "Operation not permitted" (not
  "File exists," confirming it was never a real stale lock). Fixed with a
  quote-aware scanner: single-quoted text is fully literal, double-quoted
  text neutralizes chaining/redirection/subshell punctuation but still flags
  command/parameter substitution (which the shell still expands there).
- **The model had no way to know chained git commands
  (`git add . && git commit ...`) lose the `.git/` grant (commit
  `e6adee6`).** This is intended, documented sandbox behavior (§6.5) — not a
  bug — but the model was observed retrying the same chained idiom
  repeatedly until the loop guard intervened. Added a system-prompt
  (`prompts::VERSION` bumped to 5) boundary telling the model to run one git
  command per `bash` call and that the failure is not transient.
- **Stale "Phase N" doc-comment relics across the workspace (commit
  `82e1181`, plus this documentation pass).** Several module/item doc
  comments still described already-shipped features (the rule engine, live
  providers, glob/grep, retries, transcript persistence, session-scoped
  permission grants) as future work from Phase 2/3/5 — years stale. Worst
  instance was user-facing: `emberly init`'s generated `permissions.toml`
  told every new project "the rule engine lands in a later phase." Reworded
  throughout; removed one dead constant (`SESSION_ENDED`) whose comment made
  the same claim. This pass also corrected two Design Guideline
  claims about plain-mode config editing falling back to `$EDITOR` (it never
  does — it prints the path and relies on `/reload`) and removed a "Quick
  edit — TUI overlay" bullet describing a feature that was never built (only
  the `$EDITOR` handoff exists), fixed a stale "planned — not yet started"
  status on the completed 0.4 plan, and repaired two broken `docs/phase1/`
  cross-references (renamed to `docs/version-0-1/`) — all in-place, no
  version bump, per owner decision.

**Satisfies:** FR-3, FR-4, HC-5, HC-6, C-6; no new IDs (all are corrections
to existing guarantees).

**Done when:** each fix above ships with the regression test named in its
own commit, `cargo test --workspace` is green, and `cargo clippy --workspace
--all-targets --all-features` (`RUSTFLAGS="-D warnings"`) plus `cargo fmt
--check` are clean — verified after every commit in this phase, not just at
the end.

---

## Phase dependency summary

```
0.4.1 product (M9) on 0.4 (M8) on 0.3 (M7) on 0.2 (M6) on 0.1 (M1–M5)
   ├─> Phase 1  Guided provider/model setup (C-7)   ── TUI + binary logic; no new deps
   └─> Phase 2  Post-ship hardening (bug fixes)      ── engine/provider/tool/TUI corrections
```

Phase 2 does not depend on Phase 1's code — it touches compaction, the
command palette, cancellation, `write_file`, the OpenAI streaming mapper, the
placeholder provider, and the Seatbelt git check, none of which the wizard
introduced. It is grouped here because all of it shipped on the same branch,
found and fixed in the order a real dogfooding session surfaced it.

---

## Not in this plan

- **Chained git commands (`git a && git b`) earning the `.git/`-writable
  grant** — considered and explicitly declined (owner decision): would
  require verifying every chain segment independently resolves to genuine
  git, a real (bounded) extension to the sandbox's core write-grant logic,
  not a quick patch. Each git command continues to run as its own `bash`
  call; the model is now told why (Phase 2, prompt v5).
- **MCP, headless/IDE/server frontends, user theming** — remain deferred per
  Requirements §2.2; their seams stay unused, not reshaped (A-1, T-7).
- **Release-pipeline / prebuilt binaries** — install-from-source continues
  unless the owner reopens it.
- **Agent-quality evaluation** (does it code well) — out of scope per Tech
  Spec §14; post-release discipline with separate tooling.

---

## Open items

Tech Spec §16 v0.10's open items (config.toml write fidelity, adapter default
endpoint validation against live third-party endpoints) are unchanged by this
plan — Phase 2 did not touch the wizard's write path. No new open items were
introduced; Phase 2's fixes closed gaps rather than opening new ones.
