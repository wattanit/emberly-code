# Phase 2 — Multimodal image input (P-11, T-12) — TODO & Progress

**Milestone:** M8 phase 2 (Tech Spec §15) — the 0.4 capability-parity set. Second
phase; the first to add dependencies since v0.1 and the first to touch the
provider layer since M6. It establishes the *normalized content block → per-adapter
wire mapping → capability-gated tool → reference-line render* pattern.
**Satisfies:** P-11, T-12; Tech Spec §4.1/§4.2, §5.2, §12; Design §4.8, §6.1;
HC-2, HC-6. Pinned to **Req v0.7 / Design v0.7 / Spec v0.8** (all `approved`).
**Goal:** Let the model *see* — read an image file already in the project into
context as an image content block — **behind the Provider abstraction** (P-1) so it
works across vendors and **degrades cleanly** on models without vision (P-11): a
`vision:false` model gets the structured unsupported-capability result (HC-6), never
a silent drop and never a crash. No pixels are painted in the terminal (Design §4.8);
the value is the *model* seeing the image, surfaced to the user as a labeled
reference line.

**Depends on:** the shipped product (M1–M7) and Phase 1 only for the established
tool-registration seam (not for machinery — this phase touches the provider layer
Phase 1 did not). The seams this phase plugs into all exist:
- Normalized message type — `ContentBlock` enum (`crates/emberly-providers/src/message.rs:26`),
  re-exported at `.../lib.rs:31`; `Message`/`Message::tool_result` (`message.rs:66`, :90).
- Model capability struct — `ModelInfo` (`crates/emberly-providers/src/model.rs:77`);
  `Provider::model_info()` (`.../provider.rs:21`).
- Adapter message→wire seams — anthropic `message_to_anthropic` / `block_to_anthropic`
  (`crates/emberly-providers/src/anthropic.rs:180`, :190); openai `message_to_openai` /
  `joined_text` (`crates/emberly-providers/src/openai.rs:140`, :178).
- The tool to mirror for path/root confinement — `ReadFileTool` (`crates/emberly-tools/src/builtin/read.rs`);
  `resolve_in_root` (`crates/emberly-tools/src/path.rs:67`), `ResolvedPath.outside_root`.
- Tool trait / `ToolOutcome` / registry — `tool.rs:53`/:110, `builtin/mod.rs:30`
  (`default_registry`), `registry.rs`.
- Tool context — `ToolCtx` (`crates/emberly-tools/src/ctx.rs:51`), built per call in
  `Engine::make_ctx` (`crates/emberly-core/src/engine.rs:2212`); it already carries a
  Copy `truncate` config (:53) — the precedent for threading a `vision` capability.
- Tool-result ingestion into the conversation — `ingest_tool_result`
  (`crates/emberly-core/src/engine.rs:1906`, message push :1946).
- Config — `ConfigFile` (`crates/emberly/src/config.rs:20`), `TruncateConfigFile`
  (:105, the `max_bytes` precedent), `ModelFile` (:148), merge (:267), resolve (:657),
  provenance (:494–601).
- Provider profiles → runtime `ModelInfo` — `provider_setup.rs:90`.
- TUI render — `ConvItem::Tool` render (`crates/emberly-tui/src/render.rs:415`), plain
  frontend tool-finish (`crates/emberly-tui/src/line.rs:84`).
- Deps table — workspace `[workspace.dependencies]` (`Cargo.toml:25`), `emberly-tools`
  (`crates/emberly-tools/Cargo.toml:10`).
- Offline tests — `FakeProvider` (`crates/emberly-providers/src/fake.rs:123`, `model_info`
  :197); provider unit tests (`.../tests/fake_provider.rs`); tool unit tests
  (`crates/emberly-tools/tests/builtin_tools.rs`); engine round-trips
  (`crates/emberly-core/tests/engine_loop.rs`).

**Key structural facts (from the codebase):**
1. **`ToolOutcome` is string-only today** (`tool.rs:53` — `content: String`), and
   `ingest_tool_result` pushes exactly one `Message::tool_result(id, string, is_error)`
   (`engine.rs:1946`). There is **no path for a tool to add a non-text content block**
   to the conversation. Getting a `ContentBlock::Image` into the sent context is the
   central new mechanism this phase adds — resolve the shape in group 4 / the notes log.
2. **The vision gate is a capability check, not a new enum.** HC-6 is already just
   `ToolOutcome{ok:false, content, summary}` (`tool.rs:80`, `failure`); there is no
   `UnsupportedCapability` type and none is needed. "This model can't see" is an
   ordinary `ToolOutcome::failure` with a precise message. Mirror `read.rs:114` (range
   error) / `write.rs` (`.git/` refusal), never a panic (HC-6, `execute` "never Err").
3. **The tool produces the unsupported result** (Tech Spec §5.2: "on a non-`vision`
   model … returns the structured unsupported-capability result … rather than sending"),
   so `read_image` must *know* the active model's vision capability. `ModelInfo` lives in
   providers and is read via `self.provider.model_info()` in the engine; the tool reaches
   it through a new `ToolCtx` field, threaded in `make_ctx` — exactly as `truncate` is
   threaded (group 4).
4. **`read_image` mirrors `read_file`, not `write`/`edit`.** It uses `resolve_in_root`
   + the outside-root permission gate (`read.rs:81`, :86) and is governed by the same
   §6.2 project-read rules — it does **not** hard-refuse `.git/` the way write/edit do.

**Legend:** `[ ]` todo · `[~]` in progress · `[x]` done · `[!]` blocked

---

## Progress summary

| Group | Status | Notes |
|---|---|---|
| 1. `ContentBlock::Image` + `ModelInfo.vision` (`emberly-providers`) | [ ] | additive enum variant + capability flag (default `false`); update in-crate match arms |
| 2. Per-adapter wire mapping: anthropic `image` block + openai `image_url` (`emberly-providers`) | [ ] | openai user-content must become an array; both validated against live endpoints (§16) |
| 3. `read_image` tool + `base64`/`imagesize` deps (`emberly-tools`) | [ ] | path/root-check like `read_file`; header-only format+dims; `image.max_bytes`; vision-aware |
| 4. Engine: image block into the conversation + `vision` in `ToolCtx` + transcript (`emberly-core`) | [ ] | extend `ToolOutcome` with an optional image payload; thread `vision`; transcript = path only |
| 5. Config: `[image] max_bytes` + per-model `vision` + resolve/provenance (`emberly`) | [ ] | mirror `[truncate]` + `ModelFile.effort`; `config show` provenance |
| 6. TUI: reference-line render + no-vision calm note + degraded parity (`emberly-tui`) | [ ] | `name · WxH · format`, no pixels; ASCII in degraded mode |
| 7. Tests (offline, deterministic — §14.7) + exit criterion | [ ] | root-confine, oversize/`.git`, block appended, `vision:false` unsupported, both adapters |

**Overall Phase 2: NOT STARTED.**

---

## 1. `ContentBlock::Image` + `ModelInfo.vision`  *(P-11; Tech Spec §4.1)*

The normalized types are the provider-agnostic vocabulary (P-1); both additions land
in `emberly-providers` and are `serde`-derived so they serialize into the resume cache
and transcript for free (A-3).

- [ ] Add `Image { media_type: String, data: String }` to the `ContentBlock` enum
      (`message.rs:26`). `data` is base64 of the file bytes (P-11); `media_type` is the
      MIME string (`image/png`, `image/jpeg`, `image/gif`, `image/webp`). The enum is
      `#[serde(tag = "type", rename_all = "snake_case")]` (:25) so the new variant
      serializes as `{"type":"image", ...}` — additive, no other variant changes.
      Consider a small `ImageSource`/newtype if it keeps the adapters clean; decide in
      group 2 (**notes log**).
- [ ] Sweep the in-crate `match block` sites so the new arm is handled, not defaulted:
      `block_to_anthropic` (`anthropic.rs:190`) and the openai `joined_text` /
      `message_to_openai` filters (`openai.rs:178`, :140) — group 2 fills these in;
      here just make the match exhaustive so the crate compiles.
- [ ] Sweep the **core** match arms that walk `ContentBlock` so an image is counted and
      rendered, never panicked on: token counting (`engine.rs:2379`), summary/compaction
      rendering (`engine.rs:2424`, :2447), resume rebuild (`resume.rs:78`, :142). An
      image block's token weight is not chars/4 of its base64 — count it as a
      provider-reported/estimated fixed image cost or `0` pending authoritative usage
      (P-6); **record the chosen estimate in the notes log** (initial; tune with use,
      Requirements §13).
- [ ] Add `vision: bool` to `ModelInfo` (`model.rs:77`), `#[serde(default)]` so it reads
      as `false` when absent (P-11: "default `false`, so an image is never sent to a model
      not declared vision-capable"). Update every `ModelInfo { .. }` literal: the fake
      (`fake.rs:138`) and the resolver (`provider_setup.rs:91`, group 5).

## 2. Per-adapter wire mapping  *(P-11; Tech Spec §4.2)*

Each adapter maps `ContentBlock::Image` to its provider's native shape; **no wire type
crosses the boundary** (P-1). This is where the "portable across providers" promise of
P-11 is actually paid.

- [ ] **Anthropic** (`block_to_anthropic`, `anthropic.rs:190`): map `Image{media_type,
      data}` to `{"type":"image","source":{"type":"base64","media_type":..,"data":..}}`
      — the base64 `source` shape Tech Spec §4.2 names. An image block appears in a
      `user`-role message and (per Anthropic) may also be nested inside a `tool_result`
      content array; which one the engine emits is group 4's decision — this arm renders
      the block wherever it sits.
- [ ] **OpenAI-compat** (`openai.rs:140`): map `Image` to an `image_url` content part
      with a `data:` URI — `{"type":"image_url","image_url":{"url":"data:<mt>;base64,<data>"}}`
      (Tech Spec §4.2). **Structural change:** `joined_text` (`openai.rs:178`) collapses
      a message to a single text string, and user/system messages currently send flat
      `content: "<text>"` (:173–174). A user message carrying an image needs
      `content: [ {type:text,..}, {type:image_url,..} ]`. Add a `user_content(message)`
      helper that returns a string when text-only (unchanged wire) and an array when any
      image block is present (back-compat for every existing text turn). **OpenAI tool-role
      messages cannot carry image parts** — if group 4 routes images through a tool_result,
      the openai path must instead attach them to a following `user` message; settle this
      with group 4 (**notes log**, G-24 candidate if it forces a Spec §4.2 clarification).
- [ ] Mirror the existing "unsupported control is a silent no-op, never an error" adapter
      convention (effort omission at `anthropic.rs:160` / `openai.rs:131`): an adapter
      never *decides* vision — the engine/tool already gated it (group 3/4), so by the time
      a block reaches the adapter it is meant to be sent.
- [ ] **Open item (Tech Spec §16, this phase's resolver).** Validate both mappings against
      the live Anthropic `image` block and OpenAI `image_url` `data:` URI, and confirm
      `imagesize` covers every accepted format's header; record results in the notes log.

## 3. `read_image` tool + dependencies  *(T-12; Tech Spec §5.2, §12)*

New `crates/emberly-tools/src/builtin/read_image.rs`, modeled on `read.rs`. The tool
detects and encodes; it never talks to a provider.

- [ ] **Dependencies (HC-2, pure-Rust, `cargo vet` before merge — Requirements §10, Tech
      Spec §12).** Add `base64` and `imagesize` to `[workspace.dependencies]`
      (`Cargo.toml:25`, beside `similar`/`ignore`) then reference them from
      `emberly-tools/Cargo.toml` (`:10`). `imagesize` deliberately over the full `image`
      crate — header-only format + dimensions, no codec tree (plan §Phase 2). Neither crate
      exists in the workspace yet.
- [ ] `struct ReadImageTool` implementing `Tool` (`tool.rs:110`). `spec()`: name
      `"read_image"`, description (behavior-critical config, C-4 — keep it overridable per
      C-1), schema `{path: string}` required. `describe()` → `format!("read image {path}")`.
- [ ] `execute()` path handling — **copy `read.rs:81`–:101 exactly**: `resolve_in_root`
      (map `Err` → `ToolOutcome::failure`), and on `resolved.outside_root` request the
      HC-4 permission through `ctx.authorize(...)`, returning `ToolOutcome::denied` on
      refusal. Same §6.2 read rules as `read_file`; **no `.git/` hard-refuse** (that is a
      write/edit rule).
- [ ] Read the bytes (`tokio::fs::read`, not `read_to_string` — images are not UTF-8).
      Enforce `ctx.image_max_bytes()` (group 4/5): reject over the cap with a precise
      `ToolOutcome::failure` naming the size and limit (HC-6). Detect format + dimensions
      header-only via `imagesize`; reject an unsupported/undetectable format with a clear
      failure. Accept **PNG, JPEG, GIF (first frame), WebP** (Tech Spec §5.2). Derive
      `media_type` from the detected format, not the file extension.
- [ ] **Vision gate (Tech Spec §5.2, HC-6).** If `ctx.vision()` is `false`, return the
      structured unsupported result — `ToolOutcome::failure("This model has no vision
      support; the image was not sent. Switch to a vision-capable model or describe the
      image.", "no vision")` — **before** encoding, so the model learns it could not see
      rather than assuming it saw (P-11). This is data, not a harness error.
- [ ] On success: base64-encode the bytes and hand the image block to the engine. Because
      `ToolOutcome` is string-only today (structural fact 1), this needs group 4's
      `ToolOutcome` extension — carry `{media_type, data}` on the outcome and set a text
      `content`/`summary` of the reference line (`read image {rel} · {w}×{h} · {FMT}`,
      Design §4.8). The tool stays deterministic; no network.

## 4. Engine: image block into the conversation + `vision` threading + transcript  *(P-11, T-12; Tech Spec §4.1, §5.2, §3.2)*

The engine turns the tool's image payload into a `ContentBlock::Image` in the sent
context, and tells the tool whether the model can see. All in `crates/emberly-core/src/`
plus the `ToolOutcome`/`ToolCtx` shape in `emberly-tools`.

- [ ] **Extend `ToolOutcome`** (`tool.rs:53`) with an optional image payload, e.g.
      `image: Option<ImageContent{media_type, data}>`, `#[serde(default, skip_serializing_if
      = "Option::is_none")]` — additive, mirroring the existing optional `file_change`
      (:61) and its `with_file_change` builder (:91). Add a `with_image` builder. Every
      existing `ToolOutcome::success/failure` stays `None` — no other tool changes.
- [ ] **Ingest the image** in `ingest_tool_result` (`engine.rs:1906`). When
      `outcome.image` is `Some`, in addition to the text `tool_result` push, append the
      `ContentBlock::Image` so it reaches the model (P-11). **Decide the exact placement
      (notes log, G-24 candidate):** nest it in the `tool_result` content array (clean for
      Anthropic) vs. append a following synthetic `user` message carrying the image (works
      for both, required for OpenAI whose tool-role cannot hold images — structural fact in
      group 2). The image bytes are **not** written to the transcript.
- [ ] **Transcript records the path, not the bytes (Tech Spec §3.2).** The `tool_call`
      already records the args (the project-relative path); the image bytes are re-derived
      from that file when the provider request is next built (§4.1), so no new transcript
      type and no byte duplication (HC-7). Confirm the existing `tool_result` line
      (`engine.rs:1938`) records only the reference-line text; **no `SCHEMA_VERSION` bump**.
      Note the resume consequence in the log: a resumed session re-reads the image file
      from disk when it rebuilds the turn — if the file is gone, the block is absent (the
      transcript honestly references what it no longer holds).
- [ ] **Thread `vision` into `ToolCtx`.** Add a `vision: bool` field to `ToolCtx`
      (`ctx.rs:51`, beside `truncate` :53) with a `vision()` accessor and set it in
      `ToolCtx::new` (or a `with_*` builder to keep existing test callers untouched — mirror
      `with_ask_gate` :87). Populate it in `Engine::make_ctx` (`engine.rs:2212`) from
      `self.provider.model_info().vision`. Same pattern for `image_max_bytes` (from config,
      group 5) — either a `ToolCtx` field or fold the cap into the existing truncate/config
      surface; **decide in the notes log** (leaning to a dedicated `ToolCtx` field so the
      cap and vision travel together as tool-facing capability facts).
- [ ] **Token/cost accounting (P-6).** Feed provider-reported image token usage where the
      response offers it (`engine.rs:1263` pricing path, :2320 usage); the group-1 estimate
      is the between-response fallback. Label estimates "est." as elsewhere (Design §3.1).

## 5. Config: `[image] max_bytes` + per-model `vision` + provenance  *(T-12, P-11; Tech Spec §8)*

All in `crates/emberly/src/config.rs`, mirroring `[truncate]` (the value cap) and
`ModelFile.effort` (the per-model capability flag).

- [ ] Add `pub image: ImageConfigFile` to `ConfigFile` (`config.rs:20`, beside `truncate`
      :48) with `struct ImageConfigFile { max_bytes: Option<usize> }` (mirror
      `TruncateConfigFile` :105). Default **5 MiB** (plan §Phase 2, Tech Spec §5.2) applied
      at resolve, not in the file struct (files are all-optional).
- [ ] Add `vision: Option<bool>` to `ModelFile` (`config.rs:148`, beside `effort` :157) —
      the per-model capability flag (Tech Spec §4.2: "§8 pricing/model config gains an
      optional `vision` flag; default `false`"). Wire it through the runtime `ModelInfo` in
      `provider_setup.rs:91`: `vision: meta.and_then(|m| m.vision).unwrap_or(false)`.
- [ ] Merge (project over global): add an `image.max_bytes` clause at `config.rs:303`
      (beside the `truncate.max_bytes` merge); `ModelFile.vision` rides the existing
      `models.extend` profile merge (`config.rs:179`) — no new clause.
- [ ] Resolve `image.max_bytes` into the tool-facing config at `config.rs:657` (beside the
      `truncate` resolve), routed to `ToolCtx`/engine per group 4's decision.
- [ ] Provenance / `config show` (C-3): add an `image.max_bytes` block to the provenance
      machinery (`config.rs:494`–:601, mirroring the `truncate`/`context` blocks) — speech
      about deviations, silent on the default (Tech Spec §5, handbook §5). Per-model
      `vision` surfaces with the model metadata already shown.

## 6. TUI: reference-line render + no-vision note + degraded parity  *(T-12; Design §4.8, §6.1, §7)*

No pixels are painted (Design §4.8/§10). The value is the model seeing the image; the
user sees an honest reference line.

- [ ] The reference line rides the existing tool-activity path with **no new UiEvent**
      (Tech Spec §3.1: "image reads … flow through the existing `ToolStarted`/`ToolFinished`
      events"). The tool's `summary` (`read image mockup.png · 1200×800 · PNG`) flows to
      `UiEvent::ToolFinished.summary` and renders as the result field in `ConvItem::Tool`
      (`render.rs:415`, result at :434) — verify the `·`-separated form matches Design §4.8
      (`name · WxH · format`). **Leave `preview` empty** so no base64/pixel bytes render.
- [ ] Plain/degraded frontend (`line.rs:84`): the same reference line as ASCII-only tool
      finish, no ANSI — Design §4.8 "Degraded mode: the same reference line, ASCII-only".
      Keep `degraded_output_has_no_ansi_escapes` (`line.rs`) green.
- [ ] No-vision result renders as a **calm tool-result note, not a harness error**
      (Design §4.8, §6.1): because it is a `ToolOutcome::failure` (group 3), it already
      renders on the ordinary failed-tool path (`render.rs` `[mark]` :423 / `line.rs`
      `[FAILED]` :90). Confirm the copy reads like Design §4.8 ("this model can't read
      images; switch model or describe it"), not an alarm.

## 7. Tests + exit criterion  *(Tech Spec §14.7 offline, deterministic)*

- [ ] **Path confinement (mirror `read_file` tests, `builtin_tools.rs`):** a `read_image`
      call root-confines its path; an outside-root path triggers the HC-4 permission gate;
      an oversize file (> `image.max_bytes`) returns a precise `ToolOutcome::failure`.
- [ ] **Block appended:** on a `vision:true` model, `read_image` on a small fixture image
      appends a `ContentBlock::Image{media_type, data}` to the sent context with correct
      detected `media_type` and base64 `data` (assert via the engine round-trip /
      `FakeProvider.last_request()`, `fake.rs:166`).
- [ ] **Unsupported-vision (HC-6):** on a `vision:false` model the tool returns the
      structured unsupported result and **no image block is sent** — the model learns it
      could not see (P-11). Assert the outcome is `ok:false` with the informative message,
      not a panic and not a silent drop.
- [ ] **Both adapter mappings:** unit-test `block_to_anthropic` produces the base64
      `source` `image` block and `message_to_openai` produces the `image_url` `data:` URI
      part inside an array `content` (mirror the effort-mapping adapter tests at
      `anthropic.rs:431` / `openai.rs:351`); confirm a text-only message's openai wire is
      unchanged (back-compat).
- [ ] **Format coverage:** `imagesize` detects PNG/JPEG/GIF/WebP fixture headers and the
      right dimensions; an undetectable/unsupported file is a clean failure.
- [ ] **HC-7 / transcript:** the `tool_result` line records the reference-line text and the
      **path in the `tool_call`**, never the image bytes; resume re-derives the block from
      the file (and honestly omits it when the file is gone).
- [ ] **Degraded parity:** the reference line and the no-vision note render ASCII-only with
      no ANSI in the plain frontend (Design §7).
- [ ] **Exit criterion (Phase 2 done when):** an image round-trip asserts `read_image`
      root-confines its path, rejects oversize/`.git/` paths, appends a
      `ContentBlock::Image`, and — on a `vision:false` model — returns the structured
      unsupported-capability result instead of sending; both adapter mappings are exercised
      via `FakeProvider` (Tech Spec §14.7, P-11/T-12). Workspace clippy-clean under the §1
      lint policy; `base64`/`imagesize` `cargo vet`-accepted (HC-2); offline suite green.

---

## Decisions & notes log

> Fill in as the phase proceeds (mirrors the v0.1–v0.3 and Phase 1 logs).

- **How an image enters the conversation (group 4).** `ToolOutcome` is string-only
  (`tool.rs:53`) and `ingest_tool_result` pushes one text `tool_result` (`engine.rs:1946`),
  so a new carrier is required. Chosen approach: extend `ToolOutcome` with
  `image: Option<ImageContent>` (additive, like `file_change`) and have the engine append a
  `ContentBlock::Image`. _Placement — inside the `tool_result` content array vs. a following
  synthetic `user` message — decide during group 4; OpenAI tool-role messages cannot carry
  images, which pushes toward the user-message form for the openai adapter. If this forces
  a clarification of Tech Spec §4.1/§4.2 wording (which message the image rides), it flows
  back as a version bump per G-24, not an edit here._
- **The tool, not the engine, produces the unsupported result.** Per Tech Spec §5.2 the
  `read_image` tool returns the HC-6 result on a non-vision model, so the tool must read the
  model's `vision` capability via a new `ToolCtx.vision` field threaded in `make_ctx` from
  `provider.model_info().vision` — the same way `truncate` is threaded. _Confirm on
  implementation; record if the Spec should absorb the `ToolCtx` capability shape (G-24)._
- **Image token accounting (group 1/4).** Base64 length is not the model's image token
  cost. Initial estimate: prefer provider-reported usage (P-6); between responses use a
  fixed per-image estimate (or `0`) rather than chars/4 of the base64. _Pick the constant
  in group 1 and tune with use (Requirements §13, Tech Spec §16)._
- **Transcript references the file, does not copy the image.** The `tool_call` records the
  path; the bytes are re-read from disk when the request is rebuilt (Tech Spec §3.2/§4.1).
  Consequence: a resumed session whose image file was deleted honestly shows no block. _This
  is the intended §3.2 behavior; record if it proves surprising in use._
- **`imagesize` over the `image` crate.** Header-only format + dimensions, no codec/decoder
  tree — smaller dep surface, HC-2 clean. _Revisit only if a supported format's header is
  not covered (Tech Spec §16 open item, this phase's resolver)._
- **Open item carried in (Tech Spec §16 v0.8):** image formats, the 5 MiB cap, and both
  adapter mappings are the initial set — **validated against live Anthropic/OpenAI endpoints
  in this phase** (group 2). Discoveries that change HOW flow back to the Tech Spec (G-24/G-25).
