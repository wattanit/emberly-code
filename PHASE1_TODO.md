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
| 5. In-session switching (`Command::SwitchModel`) | [ ] | C-6 model half |
| 6. Sidebar model picker | [ ] | Design §3.1 |
| 7. Tests (offline + live smoke) | [ ] | Tech Spec §14.5, §14.4 |
| 8. Docs, provenance & exit criterion | [ ] | |

**Overall Phase 1: NOT STARTED.**

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

- [ ] `Command::SwitchModel{ profile }` in `emberly-core`; the engine swaps
      the active `Arc<dyn Provider>` for **subsequent** turns only, at a clean
      message boundary (never mid-turn). Prior turns are untouched.
- [ ] Emit `UiEvent::ModelChanged{ provider, model }` (Tech Spec §3.1) and
      write a `model_switch` `TranscriptEvent` (Tech Spec §3.2) — additive,
      forward-compatible (Tech Spec §3.3), so v0.1 transcripts still resume.
- [ ] Announce the switch in the conversation in the harness's own voice —
      never silently (Design §3.1, "speech about deviations").
- [ ] `/model` command routes to the same `SwitchModel` command (palette +
      slash + keybinding parity, Design §3.3).

## 6. Sidebar model picker  *(Design §3.1)*

- [ ] Make the sidebar model line selectable → opens a picker overlay listing
      the configured `[providers.*]` profiles (reuse the existing overlay
      machinery in `emberly-tui`). Selecting one issues `SwitchModel`.
- [ ] The picker is the surface Phase 3's effort picker will reuse — keep it
      general (a labeled-choice overlay), not model-specific.
- [ ] Degraded-mode parity: the picker is reachable and operable with no
      color / ASCII markers (`--plain`).

## 7. Tests  *(Tech Spec §14.5 offline; §14.4 live smoke)*

- [ ] **Config-only path (P-8 proof):** a `FakeProvider`-backed profile
      pointed at a fake endpoint is selected and drives a round trip with no
      change under `emberly-providers/` — assert the config-only property.
- [ ] **Switch mid-session:** `SwitchModel` applies to the next turn, emits
      `ModelChanged`, writes `model_switch`, and leaves prior turns/transcript
      lines unchanged.
- [ ] **Auth schemes:** unit tests from group 1 (each variant → header).
- [ ] **Baked-in profiles:** `emberly init` materializes the anthropic /
      openai / zai / local profiles and they parse back cleanly.
- [ ] **Live Z.ai smoke** (manual/nightly, Tech Spec §14.4): one real
      one-tool-use round trip against the Z.ai profile. Never in the merge
      path; needs a key.

## 8. Docs, provenance & exit criterion

- [ ] `emberly config show` reports the active profile and each piece's
      provenance tier (C-3), including which profile a value came from.
- [ ] README/config docs: the `[providers.<name>]` schema, auth schemes, the
      Z.ai example, and the legacy-key mapping.
- [ ] **Exit criterion (done when):** a Z.ai profile added purely in config
      completes a live one-tool-use round trip; the offline config-only test
      proves no `emberly-providers` change was needed; switching
      provider/model mid-session applies to the next turn, is announced and
      recorded, and leaves prior turns untouched; all lint gates and the
      offline suite stay green.

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
