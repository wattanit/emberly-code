# Emberly Code — Implementation Plan (0.2 feature set)

**Status:** ✅ **complete (2026-07-11)** — all five phases implemented and
merged to `version0.2`. The whole 0.2 feature set
(M6) ships: P-8 provider profiles + Z.ai, C-5 in-app editing, P-9/P-10 effort +
thinking trail, T-8/T-9 ask-user + tool-call explanation, FR-1 workspace trust,
S-5 loop-breaking guardrail.
**Date:** 2026-07-10 (planned); completed 2026-07-11
**Owner:** Wattanit
**Source documents** (G-14 as-built pin for the 0.2 release):
- Requirements Document v0.5 (`docs/emberly-code-requirements.md`) — WHAT/WHY
- Design Guideline v0.5 (`docs/emberly-code-design-guideline.md`) — UX/voice
- Technical Specification v0.6 (`docs/emberly-code-tech-spec.md`) — HOW
  (bumped v0.3 → v0.6 across implementation: v0.4 endpoint-configurable
  adapters, v0.5 effort/reasoning refinements, v0.6 pre-engine trust gate)

These three versions are the as-built truth for the 0.2 release. Post-merge
polish shipped under the *same* version numbers (deliberately not bumped — minor,
additive, part of the 0.2 ship): `grep` added to the bash allowlist (Req, Spec
§6.1), `read_file` `start_line`/`end_line` ranged read (Spec tool table §6.1),
and the trust prompt accepting a bare `y` (Design §8.4). Prompt set is at v4.

This plan expands Tech Spec §15 milestone **M6** (the 0.2 feature set) into
workable phases. It builds on the shipped v0.1 product (all six crates, the
agent loop, both live providers, the full TUI, sessions/resume/`/compact`,
the permission rule engine, and OS confinement on Linux + macOS); the v0.1
plan is preserved in `docs/phase1/` as the prior as-built record.

Unlike v0.1 — where phases were subsystem milestones built from nothing —
the 0.2 features are cross-cutting additions to a working product. Phases
are therefore sliced **by feature, end-to-end**: each phase adds one
coherent, shippable capability across the provider/engine/TUI seams it
touches, leaves the tree green, and can be released on its own. They are
ordered by owner priority (provider extensibility first, then in-app
editing), not by layer.

---

## Guiding principles (apply to every phase)

- **The seams already exist — use them, don't reshape them.** `Provider`,
  `Tool`, the engine/frontend channels, and the transcript are all in place
  from v0.1. Every 0.2 feature is added *through* those boundaries (a new
  `StreamEvent` variant, a new `Tool`, new `UiEvent`/`TranscriptEvent`
  cases, new `Command`s), never by leaking a new concept past them (P-1,
  T-7, A-1, A-3).
- **No new dependencies (Tech Spec §12).** The whole 0.2 set is engine,
  config, and TUI logic over the existing crate set. Anything that appears
  to need a new crate is a signal to re-check the design first; a genuine
  addition takes the §12 `cargo vet` route and, for `emberly-sandbox`,
  owner sign-off — but none is expected.
- **Hard constraints hold, unchanged.** HC-1 (safe Rust), HC-2 (pure Rust /
  rustls, static musl), HC-3 (panic-free core), HC-6 (tool failures are
  data), HC-7 (audit trail) apply to all new code from the first commit.
  New provider endpoints reach the network through the existing
  `reqwest`+`rustls` client — no C, no new TLS stack (HC-2).
- **Transcript schema is append-only and forward-compatible.** New event
  types and fields are additive; resume of a v0.1 transcript must still
  work, and an older Emberly meeting a newer transcript warns rather than
  crashes (Tech Spec §3.3). The tool-call `explanation` field rides in
  existing `args`, so it needs **no** transcript schema-version bump
  (Tech Spec §5.4).
- **Every phase leaves a shippable, tested slice.** Each ends green, with
  the offline `FakeProvider`-driven tests of Tech Spec §14.5 for its
  feature, and with degraded-mode parity where it adds a UI surface.

---

## Phase 1 — Provider profiles & Z.ai (P-8) · *the priority*

**Goal:** Make a new model provider reachable by **configuration alone**.
Prove it by bringing up the Z.ai coding plan with zero provider-specific
code, and let the user switch provider/model mid-session.

**Scope**
- **Adapters become endpoint-configurable (Tech Spec §4.2, §4.5).** Lift
  the two existing wire-format clients so base URL and authentication are
  data, not constants. Auth schemes: `bearer`, `x-api-key`, arbitrary
  `header`; key values resolved from env / `keys.toml` (Tech Spec §8),
  never inline.
- **Provider profiles (Tech Spec §4.5).** `[providers.<name>]` config —
  `adapter`, `base_url`, `auth`, `models`, optional `pricing` — resolved
  into an active `Provider` at session start. Adding a service that speaks
  an existing wire format is a profile and no code (P-8).
- **Z.ai profile** as the acceptance proof: a `[providers.zai]` profile
  selecting whichever wire format it speaks, its base URL, and its auth —
  no Z.ai-specific code anywhere in the tree.
- **In-session model/provider switching (C-6, partial).**
  `Command::SwitchModel{profile}`; engine swaps the active `Provider` for
  subsequent turns; `model_switch` transcript event; never rewrites prior
  turns. The sidebar model line becomes a selectable picker over the
  configured profiles (Design §3.1). (Effort switching lands in Phase 3
  with P-9.)
- **`--provider` / `--model` CLI** continue to work, now selecting among
  profiles.

**Satisfies:** P-8; C-6 (model/provider half); Tech Spec §4.2, §4.5, §8
(provider keys); Design §3.1 (model picker).

**Done when:** a Z.ai profile added purely in config completes a live
one-tool-use round trip (nightly smoke, Tech Spec §14.4); a `FakeProvider`
profile pointed at a fake endpoint proves the config-only path in offline
tests (Tech Spec §14.5); switching provider/model mid-session applies to
the next turn and is recorded, with prior turns untouched.

---

## Phase 2 — In-app config & prompt editing (C-5)

**Goal:** Close the gap between "the harness is configurable" and "I can
change it right here," without dropping to a shell.

**Scope**
- **Quick edit — editable overlay (Design §4.6).** Reuse the §4.2 overlay
  machinery, made editable, for a single config value or short prompt.
  Show the value's provenance tier (C-3) before editing; writes land in the
  **project tier** (C-1), never in baked-in defaults; show the written
  path.
- **Full edit — `$EDITOR` handoff.** For a whole prompt file or the full
  config, hand off to `$VISUAL`/`$EDITOR` (the §4.3 fallback order),
  reload on save.
- **Live vs restart (Tech Spec §8).** Prompts and most config live-reload on
  save; each config key carries a `reload: Live | RestartRequired` flag so
  the editor names restart-only changes at save time — not a guess.
- **Reachable three ways** (Design §3.3): palette, `/command`, keybinding.

**Satisfies:** C-5; Tech Spec §8; Design §3.3, §4.6.

**Done when:** editing a config value via the overlay writes to the project
tier with provenance shown and takes effect on the running session (or is
named restart-only); a prompt file edited via `$EDITOR` reloads on save;
tier resolution and `config show` provenance remain correct after edits.

---

## Phase 3 — Reasoning effort & the thinking trail (P-9, P-10)

**Goal:** Let the user own the latency/cost/quality trade per task, and make
the model's reasoning visible without imposing it.

**Scope**
- **Reasoning effort (Tech Spec §4.6).** `CompletionRequest.effort:
  Option<Effort>` (`Low|Medium|High|Max`); each adapter maps it to the
  provider's native control or drops it (no-op, never an error — P-9).
  `ModelInfo` declares the levels a model offers + its default.
- **Effort as engine state (C-6, completion).** `Command::SetEffort`,
  `effort_change` transcript event; per-session, switchable in-session; the
  effort picker + sidebar effort line (Design §3.1), populated from
  `ModelInfo` and reusing the picker surface Phase 1 introduced. Config
  default per model (Tech Spec §8).
- **Reasoning trace (Tech Spec §4.7).** Adapters translate provider-native
  thinking into the normalized `ReasoningDelta` `StreamEvent`; the engine
  records it as a **distinct** field on `assistant_message` (never merged
  into the answer — P-10). Where a provider requires reasoning-block
  signatures echoed back on tool-use turns, the adapter preserves and
  replays them inside the boundary (no wire detail leaks — P-1).
- **The thinking trail UI (Design §4.4).** Collapsed dim line with expand
  affordance, streaming in place while thinking and settling on answer;
  `reasoning = collapsed|expanded|hidden` view key, **default `collapsed`**;
  `hidden` still records to the transcript. Degraded-mode plain block.

**Satisfies:** P-9, P-10; C-6 (effort half); Tech Spec §4.6, §4.7, §3.1/§3.2
(events), §9; Design §3.1, §4.4.

**Done when:** an effort round-trip is observable per live provider that
supports one and a no-op on one that doesn't; a `FakeProvider` emitting
`ReasoningDelta` renders as a collapsed, expandable trail and is recorded
distinctly in the transcript; the `hidden` view still writes the trace.

---

## Phase 4 — Interaction: ask-user & tool-call explanation (T-8, T-9)

**Goal:** Give the model an explicit "I need your input" channel, and a
quiet, honest caption for what a non-obvious tool call is doing.

**Scope**
- **`ask_user` tool (Tech Spec §5.2).** A built-in `Tool` that presents a
  question + optional discrete options and blocks the loop until answered;
  returns the typed answer or a structured `{declined:true}` on dismiss.
  Implemented as an `AskUserRequest` `UiEvent` / answer `Command` pair;
  touches no filesystem or network (bypasses the sandbox, still flows
  through the `Tool` trait). `ask_user` transcript event (question +
  answer/decline).
- **The question prompt UI (Design §5.1).** Neutral styling — **never** the
  reserved outside-root safety band; selectable options + free-text answer;
  **no unsafe default** (Enter never auto-answers; Esc returns "declined");
  no motion while deciding. Full degraded-mode parity.
- **Tool-call explanation (Tech Spec §5.4).** Inject an optional
  `explanation` string into every tool's `input_schema` at the single
  `ToolSpec → provider` point; bump the prompt version to instruct the
  model to fill it briefly and only for non-obvious calls. Rides in `args`
  (tools ignore it; no `deny_unknown_fields`) — **no transcript
  schema-version bump**.
- **Explanation UI (Design §4.5).** Dim caption line under the call from
  `ToolStarted.explanation`; absent when the model gave none (no
  placeholder); never styled as result/error; never color-only.
  `ui.tool_explanations`, **default `true`**; when off, the schema property
  is omitted entirely so no tokens are spent.

**Satisfies:** T-8, T-9; Tech Spec §5.2, §5.4, §3.1/§3.2; Design §4.5, §5.1.

**Done when:** an `ask_user` round trip blocks the loop and resumes with the
answer (and a dismiss returns a structured decline), offline via
`FakeProvider`; a scripted non-obvious call renders its dim explanation and
an obvious one renders none; toggling `ui.tool_explanations=false` removes
the schema property and the prompt instruction.

---

## Phase 5 — Safety guardrails: workspace trust & loop-breaking (FR-1, S-5)

**Goal:** Gate the agent on a conscious trust decision before it runs in an
unfamiliar folder, and guarantee a runaway loop always ends in a user
decision rather than silent unbounded spend.

**Scope**
- **Workspace trust (Tech Spec §6.7, Design §8.4).** Startup check in the
  binary before the engine loop: canonicalize the root, test membership
  against the global store ∪ pre-trust allowlist. Miss → the trust gate
  (safe default = decline; decline exits cleanly, no session); accept →
  write to the store. **Subtree-trusted** (an ancestor match suffices),
  matching Claude Code + VS Code. Store: `~/.config/emberly/trust.toml`,
  `0600`, **global only**; optional `trust.trusted_dirs` allowlist in global
  config; neither is ever a project key (FR-1). `trust_decision` transcript
  event.
- **Trust CLI (Tech Spec §10).** `emberly trust list` / `emberly trust
  revoke <path>` — explicit management, so revoking is never hand-editing a
  file.
- **Trust is a gate, not containment (FR-1 honesty clause).** Lives outside
  `emberly-sandbox`; changes no ruleset; never widens HC-4/HC-5 or relaxes a
  prompt. The prompt says so in a dimmed line.
- **Loop-breaking guardrail (Tech Spec §7, Design §8.5).** Engine keeps a
  rolling signature per turn — `(tool_name, normalized-args)` multiset +
  tool-result hash + modified-file set. Trip when the last
  `loop.repeat_window` turns (default 3) repeat signatures **and** produce
  no new modified files and no new distinct results. On trip: stop provider
  calls, emit `LoopHalted{reason}` (`UiEvent` + transcript event), await a
  user `Command` (resume / stop / steer). Harness-voice surface (Design
  §8.5), distinct from the question prompt. `[loop] enabled, repeat_window,
  max_no_progress_turns` config. Never trips while progress continues.

**Satisfies:** FR-1, S-5; Tech Spec §6.7, §7, §10, §3.1/§3.2; Design §8.4,
§8.5.

**Done when:** an untrusted root raises the gate and decline exits with no
session created; a project-local trust key is ignored; a trusted parent
suppresses re-prompting in a subfolder; `trust revoke` re-arms the gate; a
scripted re-treading loop trips `LoopHalted` while a progressing loop does
not, and the user's choice (resume/stop/steer) is honored and recorded.

---

## Phase dependency summary

```
v0.1 product (shipped)
   ├─> Phase 1  Provider profiles & Z.ai (P-8)             ── the priority
   │      └─> Phase 3  Effort & thinking trail (P-9,P-10)   ── completes the C-6 picker Phase 1 starts
   ├─> Phase 2  In-app config/prompt editing (C-5)
   ├─> Phase 4  ask-user & explanation (T-8,T-9)
   └─> Phase 5  Trust & loop guardrail (FR-1,S-5)
```

The only hard dependency is Phase 3 on Phase 1: effort switching completes
the C-6 picker/switch surface Phase 1 introduces, so it wants Phase 1 done
first. Phases 2, 4, and 5 are independent of everything else and could be
reordered or parallelized. The default single-track order 1→2→3→4→5 leads
with the owner's priorities — Z.ai, then in-app editing — then the
provider-layer effort/trail work, then the interaction and safety slices.
Each phase ships on its own.

---

## Not in this plan

- **Auto-compaction, MCP, headless/IDE frontends, user theming, Windows** —
  still deferred per Requirements §2.2; their seams remain unused, not
  reshaped.
- **Release-pipeline / prebuilt binaries** — v0.1 shipped compile-from-
  source with the pipeline deferred by owner (see `docs/phase1/`); this plan
  does not revisit that decision. 0.2 continues to install from source
  unless the owner reopens it.
