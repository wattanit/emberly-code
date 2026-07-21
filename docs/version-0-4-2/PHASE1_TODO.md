# Phase 1 — Guided provider/model setup (C-7) — TODO & Progress

**Milestone:** M10 phase 1 (Tech Spec §15) — the 0.4.2 feature set.
**Satisfies:** C-7; C-1, C-3, C-5, C-6; Tech Spec §8, §9, §12, §16; Design
§3.1, §4.6. Pinned to **Req v0.9 / Design v0.9 / Spec v0.10** (all
`approved`).

Written as-built (see `IMPLEMENTATION_PLAN.md`'s "How this plan was written")
— every item below shipped before this checklist existed.

## Group 1 — Prerequisite: `/config` live-reload actually works (C-5)

- [x] Fix `engine.rs`/`factory.rs`/`rules.rs` so a `Command::ReloadConfig`
      after a manual `.agents/config.toml` edit applies to the running
      session in every case it should — surfaced while designing the
      wizard's "no separate success voice, reuses the reload story" design.
      (commit `c6349f5`)

## Group 2 — The writer seam (`emberly-core`, `emberly`)

- [x] `ProviderProfileWriter` trait + `NewProviderProfile` struct in
      `emberly-core/src/provider_writer.rs`, mirroring the `ConfigReloader`
      seam (A-1).
- [x] `ConfigWriter` implementation in `emberly/src/provider_write.rs`:
      name-collision refusal (create-only), `[providers.<name>]` write via
      generic `toml::Table` manipulation (no `toml_edit` dependency, §12),
      `auth.scheme` chosen by adapter matching `builtin_profiles()`'s
      existing convention, `keys.toml` entry written at `0600` from creation.
- [x] Unit tests: fresh write, preserves existing tables/profiles, adapter→
      scheme mapping, key file permissions, overwrite-same-reference.
      (all in `provider_write.rs`)

## Group 3 — The wizard (`emberly-tui`)

- [x] Six-screen state machine (`ProviderWizard`, `WizardStep`): name →
      adapter → endpoint → model id → API key → summary.
- [x] Entry point: trailing "+ add new provider…" row in the model picker,
      shown even with zero profiles configured.
- [x] API key masked per keystroke; summary redacts to last 4 characters;
      never transcripted in full (C-7's never-printed guarantee).
- [x] On write success: fires `Command::ReloadConfig`, no separate "wizard
      complete" voice.
- [x] Model-carryover fix: `wizard_created_models` map so a freshly-created
      profile is tried with the model the wizard collected, not whatever
      model was previously active.
- [x] Tests: happy path writes + fires reload; empty-name / name-collision
      rejection stays on step; Esc steps back without losing the value.

## Group 4 — Foundation docs

- [x] Design Guideline §4.6 touch-up: profile-name screen added, adapter
      wire-format-not-brand explainer added (no version bump — owner's
      call at the time).
- [x] README: guided-setup path documented alongside the raw-TOML fallback.

**Done when:** ✅ all groups above shipped; `cargo build/test/clippy(-D
warnings)/fmt` clean at each commit (verified 2026-07-21).
