# Phase 1 — Provider profiles & Z.ai — TODO & Progress

**Milestone:** M6 group 1 (Tech Spec §15) — *the 0.2 priority.* Make a new
model provider reachable by **configuration alone**; prove it with the Z.ai
coding plan (zero provider-specific code) and add in-session model/provider
switching (C-6 model half).
**Satisfies:** P-8; C-6 (model/provider half); Tech Spec §4.2, §4.5, §8;
Design §3.1. Pinned to Req v0.5 / Design v0.5 / Spec v0.3.
**Goal:** A `[providers.zai]` profile added purely in config completes a
round trip, and the user can switch provider/model mid-session.

**Depends on:** v0.1 product (shipped) — the `Provider` trait, both wire
adapters, config/keys system, engine channels, and TUI sidebar all exist.

**Legend:** `[ ]` todo · `[~]` in progress · `[x]` done · `[!]` blocked

---

## Progress summary

| Group | Status | Notes |
|---|---|---|
| 1. Auth-scheme abstraction in adapters | [x] | Done 2026-07-10; `Auth` enum + 5 unit tests; workspace green |
| 2. `[providers.<name>]` profile config | [x] | Done 2026-07-10 (with group 3) |
| 3. Profile resolution in the composition root | [x] | Done 2026-07-10; live smoke of all 3 paths |
| 4. Z.ai profile (baked-in + init + docs) | [x] | Done 2026-07-10; baked-in zai (openai @ paas/v4), grep-gate test, README |
| 5. In-session switching (`Command::SwitchModel`) | [x] | Done 2026-07-10; engine + factory + `/model`; 5 tests |
| 6. Model picker overlay | [x] | Done 2026-07-10; generic Choices overlay; line-mode `/model` too |
| 7. Tests (offline + live smoke) | [x] | Done 2026-07-10; config-only proof, switch, auth, schema |
| 8. Docs, provenance & exit criterion | [x] | Done 2026-07-10; exit criterion met |

**Overall Phase 1: COMPLETE (2026-07-10).** All 8 groups done. Provider
profiles + Z.ai, endpoint-configurable adapters, in-session model switching,
and the picker all working and tested; workspace clippy clean, 21 test
binaries green. Live Z.ai round trip remains the manual/nightly step (needs a
key). Next: Phase 2 — in-app config & prompt editing (C-5).

---

## 1. Auth-scheme abstraction in the adapters  *(P-8; Tech Spec §4.5)*

Today auth is hardcoded per adapter (`anthropic.rs:81` `x-api-key` +
`anthropic-version`; `openai.rs:67` `bearer_auth`). Lift it to data so any
endpoint speaking a wire format we parse is reachable.

- [x] Defined `Auth` in new `emberly-providers/src/auth.rs`: `None`,
      `Bearer`, `XApiKey`, `Header{ name, value }` — plain data type, key
      carried inline, `apply()` is `pub(crate)`; no wire logic leaks (P-1).
      Exported from `lib.rs`.
- [x] `AnthropicProvider::new`/`with_default_url` take an `Auth`; still
      always send `anthropic-version`. `provider_setup.rs` passes
      `Auth::XApiKey` → byte-identical default behavior.
- [x] `OpenAiProvider::new` takes an `Auth`; `provider_setup.rs` passes
      `Auth::Bearer`, and an empty `Bearer` key sends no header (local
      Ollama/vLLM path preserved).
- [x] 5 unit tests in `auth.rs` assert each variant's header on a built
      `reqwest::Request`, incl. empty-Bearer (no header) and empty-XApiKey
      (header still sent). Live-client tests updated to the new signatures.

## 2. `[providers.<name>]` profile config  *(P-8; C-1/C-2; Tech Spec §4.5, §8)*

**Profile-only — no legacy back-compat** (owner decision 2026-07-10,
pre-release, no real users). The flat `provider`/`base_url`/`context_window`/
`max_output`/`pricing` fields in `config.rs` are **replaced** by profile
tables, not augmented.

- [x] New config model in `config.rs`: `ProfileFile`/`AuthFile`/`ModelFile`;
      `[providers.<name>]` with `adapter`, `base_url`, `auth = {scheme,
      header?, key}`, and `[providers.<name>.models."id"]` metadata
      (`context_window`/`max_output`/`pricing`). Field-merge per profile
      across tiers; flat provider fields removed (profile-only).
- [x] `auth.key` is a **reference**; `api_key(ref)` resolves
      `<REF>_API_KEY` then `keys.toml` (now a flat `ref = "secret"` map,
      `0600`). Missing key for a configured ref → clear early error.
- [x] Active-profile selection is explicit: top-level `provider` /
      `--provider` / `EMBERLY_PROVIDER` names the profile; `model` / `--model`
      the model. No profile → offline placeholder (unchanged).
- [x] Baked-in profiles (C-1) injected at the lowest tier: `anthropic`,
      `openai`, `local` (Ollama default). `emberly init` template shows the
      profile schema incl. a commented `zai` example. Concrete ready-to-use
      `zai` baked-in → group 4 (needs the real endpoint).

## 3. Profile resolution in the composition root  *(P-8; Tech Spec §4.5)*

Replace the hardcoded `match kind.as_str()` in
`emberly/src/provider_setup.rs::build()` with profile-driven construction.

- [x] `provider_setup::build` resolves the active profile → `ModelInfo` from
      the profile's model entry (or defaults) → dispatch on `adapter` to
      construct `AnthropicProvider`/`OpenAiProvider` with the profile's
      `base_url` + `Auth`. `resolve_auth` maps scheme+key-ref → `Auth`.
- [x] Clear errors: unknown profile (lists configured), missing `adapter`,
      unknown adapter, unknown auth scheme, unresolved key ref — all verified
      live via the binary.
- [x] Offline-placeholder fallback (no active profile) preserved.
- [x] P-8 property demonstrated live: a `zai` profile (adapter `anthropic`)
      builds and starts a session with **zero** change under
      `crates/emberly-providers/` — Z.ai is config, not code.

## 4. Z.ai profile — the acceptance driver  *(P-8; C-1/C-2; Tech Spec §4.5)*

- [x] Baked-in `zai` profile: **OpenAI-compatible** (`adapter = "openai"`,
      `base_url = "https://api.z.ai/api/paas/v4"`, `bearer`, key ref `zai`) —
      confirmed against docs.z.ai. Works with just `EMBERLY_PROVIDER=zai` +
      `EMBERLY_MODEL` + `ZAI_API_KEY`, no config file (C-1). `init` template
      lists it.
- [x] README "adding a provider" section rewritten to the profile schema;
      stale flat-config docs (`EMBERLY_BASE_URL`, `[pricing]`, …) removed.
- [x] Grep-gate test `tests/no_vendor_code_in_providers.rs`: fails if
      `zai`/`z.ai`/`glm` appears in `emberly-providers/src`. Green — Z.ai is
      data, not code (P-8 proof).

## 5. In-session model/provider switching  *(C-6 model half; Tech Spec §8, §3)*

- [x] `Command::SwitchModel{ profile, model }` in `emberly-core`; engine swaps
      the active provider at the idle boundary (frontend gates on `busy`).
      Provider construction stays in the binary via a new injected
      `ProviderFactory` trait (`factory.rs`) — the engine calls it, keeping
      the engine/frontend boundary clean. Binary impl: `ConfiguredProviders`
      wrapping the resolved profiles (shares `build_profile` with group 3).
- [x] Emit `UiEvent::ModelChanged{ provider, model }` + write a `ModelSwitch`
      `TranscriptEvent` — additive, warn-skipped by old readers (Tech Spec
      §3.3), no schema bump (audit-only, skipped on rebuild).
- [x] Switch announced via a harness-voice `Notice` — never silent
      (Design §3.1). Failure (unknown profile / missing key) → `HarnessError`,
      current model kept.
- [x] `/model <profile> [model]` slash command → `SwitchModel`; `model`
      omitted keeps the current model. Listed in `/help`. Sidebar updates on
      `ModelChanged`. (Palette picker is group 6.)
- [x] Tests: engine round-trip (swap + emit + record; unknown-profile error
      without switching) and TUI parsing (args, usage, `modelx` non-match,
      sidebar update) — 5 tests, all green.

## 6. Model picker overlay  *(Design §3.1)*

- [x] Model picker as a **generic `OverlayContent::Choices`** overlay
      (`ChoiceKind::Model`) listing the configured profiles with the active
      one marked; ↑/↓ move, Enter → `SwitchModel` (keeps current model),
      Esc closes. Reachable via the palette (`model`), `/model` with no args,
      and `/model <profile>` for a direct switch. Profile names are threaded
      to the TUI (`App::new` → `tui::run` → `frontend::run` ← `main.rs`).
- [x] Kept general so Phase 3's effort picker reuses `Choices`/`ChoiceRow`
      (only the `ChoiceKind` and Enter mapping differ).
- [x] Degraded-mode parity: line mode has no overlay, but `/model <profile>
      [model]` switches there too (verified live); the picker overlay renders
      with ASCII `▶`/`(current)` markers, no color-only meaning.
- [x] Tests: open-marks-current, navigate+Enter→SwitchModel, no-args-opens,
      empty-profiles notice, sidebar update on `ModelChanged`.

## 7. Tests  *(Tech Spec §14.5 offline; §14.4 live smoke)*

- [x] **Config-only path (P-8 proof):** `provider_setup` unit tests —
      `build_profile` selects the adapter purely from config (asserting
      `provider.id()` + `model_info`), per-model metadata feeds `ModelInfo`,
      and every error path is clear. Plus the `no_vendor_code_in_providers`
      grep gate. No change under `emberly-providers/`.
- [x] **Switch mid-session:** engine round-trip tests — `SwitchModel` emits
      `ModelChanged`, writes `ModelSwitch`, announces via Notice; unknown
      profile errors without switching. (`ModelSwitch` is a separate audit
      record; the conversation view is untouched by design.)
- [x] **Auth schemes:** 5 unit tests in `auth.rs` (each variant → header;
      empty-Bearer / empty-XApiKey edge cases).
- [x] **Config parses back:** `builtin_profiles_present_and_field_merge` and
      `parses_provider_profile_with_model_metadata` cover the profile schema
      and baked-ins; `config show` renders them (verified live).
- [~] **Live Z.ai smoke** (manual/nightly, Tech Spec §14.4): needs a real
      key, so it stays out of the merge path. Stood in for by the offline
      config-only test + the live plain-mode switch smoke run this session.

## 8. Docs, provenance & exit criterion

- [x] `emberly config show` reports the active profile + model, lists every
      profile (adapter, endpoint, key status), and shows the non-default
      provenance tiers (C-3). Verified live.
- [x] README/config docs rewritten to the profile schema: baked-in profiles,
      auth schemes, "adding a provider", `keys.toml` as a flat ref map. (No
      legacy-key mapping — config is profile-only.)
- [x] **Exit criterion met:** the offline config-only test proves no
      `emberly-providers` change is needed to add a provider; the Z.ai profile
      is baked in and builds/starts a session (live smoke); mid-session
      switching applies to the next turn, is announced + recorded, and leaves
      prior turns untouched; all lint gates and the offline suite are green
      (21 test binaries). Live Z.ai round trip is the manual/nightly step
      (needs a key).

---

## Decisions & notes log

> Fill in as the phase proceeds (mirrors the v0.1 phase docs' decisions log).

- Config is **profile-only**; no legacy flat-key back-compat (owner decision
  2026-07-10, pre-release, no real users). Flat `provider`/`base_url`/
  `context_window`/`max_output`/`pricing` are replaced by profile tables;
  the top-level `provider` key survives only as the active-profile selector.
- Adapter values are plain `anthropic` / `openai` (owner decision
  2026-07-10; the `-wire` naming was dropped as jargon). Matches the existing
  `provider_setup.rs` kind strings, so minimal churn. Aligned in Spec v0.4.
- Whether a profile with one model needs the `models` table or can inline a
  single model entry: _TBD at implementation._
