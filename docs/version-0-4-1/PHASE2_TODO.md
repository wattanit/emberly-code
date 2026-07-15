# Phase 2 — Document (PDF) input (P-12, T-16) — TODO & Progress

**Milestone:** M9 phase 2 (Tech Spec §15) — the 0.4.1 cross-project feature set.
**The exact analog of the shipped 0.4 image feature** (P-11/T-12): wherever an
`Image`/`vision` construct lives, the parallel `Document`/`documents` construct
goes beside it. The one difference: no `imagesize`, no dimensions — the harness
**passes the PDF bytes unparsed**, sniffing only a `%PDF-` magic prefix.
**Satisfies:** P-12, T-16; Tech Spec §4.1/§4.2, §5.2, §6.1, §8, §12; Design
§4.11, §6.1; HC-2, HC-6; Requirements §4, §5. Pinned to **Req v0.8 / Design v0.8
/ Spec v0.9** (all `approved`).
**Goal:** Let the model *read* a PDF already in the project into context as a
document content block, behind the provider abstraction so it works across
vendors and degrades cleanly on models without document support. Document
support lives *behind* the `Provider` abstraction (P-1), not in the tool, so
`read_document` is portable — the same reasoning as image support (P-11).
**No new dependencies** (`base64` already locked; PDF is a byte-prefix check).

**Depends on:** the shipped product (M1–M8), the image feature especially. Every
seam is a parallel of an existing image location:
- **`ContentBlock` enum** (`crates/emberly-providers/src/message.rs`):
  `Image{media_type,data}` variant `:60` (enum `:26`, `#[serde(tag="type",…)]`
  `:25`). Add `Document{media_type,data}` beside it (tag `"document"`).
- **`ModelInfo` + capability** (`crates/emberly-providers/src/model.rs`): struct
  `:77`, `#[serde(default)] pub vision: bool` `:99-100`. Add `documents: bool`.
  `FakeProvider` default `fake.rs:147`.
- **Per-model config flag** (`crates/emberly/src/config.rs`): `struct ModelFile`
  `:215`, `pub vision: Option<bool>` `:226-228`; built into `ModelInfo` at
  `crates/emberly/src/provider_setup.rs:107`
  (`vision: meta.and_then(|m| m.vision).unwrap_or(false)`, literal from `:92`).
- **`read_image` tool** — the template (`crates/emberly-tools/src/builtin/read_image.rs`,
  1-203): args/struct `:22-28`, `spec()` `:32-51`, capability gate `:65-73`
  (`!ctx.vision()`), path resolve + root check `:75-96`, byte read `:98-107`, size
  cap `:109-121` (`ctx.image_max_bytes()`), **format detection `:123-159`
  (`imagesize` — REPLACE with `%PDF-` check)**, base64 + `.with_image(...)` +
  reference-line summary `:163-179`, media-type/label helpers `:182-202`.
- **`ToolOutcome` payload plumbing** (`crates/emberly-tools/src/tool.rs`):
  `struct ImageContent` `:45-49`, `pub image: Option<ImageContent>` `:75`, ctor
  inits `:93`/`:107`, `with_image(...)` `:119-124`. Add `DocumentContent` +
  `document: Option<…>` + `with_document(...)`.
- **`ToolCtx` accessors** (`crates/emberly-tools/src/ctx.rs`):
  `IMAGE_MAX_BYTES_DEFAULT` `:45`, `vision` `:66-68`, `image_max_bytes` `:69-70`,
  ctor `:95-96`, builders `with_vision` `:143-145` / `with_image_max_bytes`
  `:150-152`, getters `vision()` `:217-218` / `image_max_bytes()` `:223-224`.
- **Engine append + wiring** (`crates/emberly-core/src/engine.rs`): image-block
  append `:2189-2196`, `ToolCtx` wiring `:2500-2501`
  (`.with_vision(...).with_image_max_bytes(...)`), engine field `image_max_bytes`
  `:606`/init `:749`/config `:230-232`, token-estimate + summarization arms for
  `ContentBlock::Image` at `:2854`, `:2898-2900`, `:2945-2946`,
  `IMAGE_TOKEN_ESTIMATE` `:2966`.
- **Per-adapter mapping:** Anthropic `crates/emberly-providers/src/anthropic.rs:227-236`
  (`Image` → `{"type":"image","source":{base64…}}`); OpenAI
  `crates/emberly-providers/src/openai.rs:194-217` (`user_content`, `has_image`
  guard `:195-198`, `Image` → `image_url` `data:` URI `:207-212`; tool-role can't
  carry parts, comment `:193`).
- **Tool registration** (`crates/emberly-tools/src/builtin/mod.rs`): `mod
  read_image;` `:15`, re-export `:29`, `registry.register(Arc::new(ReadImageTool));`
  `:41` in `default_registry()` (`:38`).
- **Permission rule defaults** (`crates/emberly-sandbox/src/rules.rs`):
  `builtin_defaults()` `:343`; `read_file` explicit `Allow` `:345-350`,
  `web_search` explicit `Ask` `:354-359`; outside-root hard gate `:213`/`:552`.
- **Config `image.max_bytes`** (`crates/emberly/src/config.rs`): field `:52-54`,
  `struct ImageConfigFile` `:122-128`, merge `:399-402`, provenance `:764-780`,
  resolve `:987` (`unwrap_or(5*1024*1024)`), resolved field `:479-481`.
- **Image tests to mirror:** engine loop `crates/emberly-core/tests/engine_loop.rs`
  (`vision_model_info` `:3624-3636`, round-trip `:3638-3672`, non-vision `:3675-3712`,
  transcript-records-path-not-bytes `:3715-3764`, `make_config` `:100`); tool unit
  `crates/emberly-tools/tests/builtin_tools.rs` (`ctx_vision` `:567-570`, and
  cases `:573-720`); adapter unit tests `anthropic.rs:594-616`, `openai.rs:425-454`.
  Injection via `FakeProvider::with_model_info`.

**Key structural facts (from the codebase):**
1. **No new TUI render code.** There is *no* image-specific render branch: the
   `read image name · W×H · format` reference line and the "no vision" note are
   just the tool's `ToolOutcome` **summary strings**, rendered by the generic
   `ToolFinished` handler (plain `line.rs:85-93`, rich `render.rs:715-732`). So
   the Design §4.11 document reference line is **authored in `read_document.rs`'s
   summary**, not in the frontends.
2. **No parser, no dimensions.** Replace the `imagesize` block
   (`read_image.rs:123-159`) with a `%PDF-` byte-prefix check; the summary carries
   **size + format, never a page count** (a page count would need a PDF lib, which
   is out of scope — HC-2, Design §4.11).
3. **The OpenAI `has_image` guard is image-specific** (`openai.rs:195-198`): it
   must also trigger on a `Document` block, and tool-role messages still cannot
   carry document parts (comment `:193`).
4. **`read_image` has no named `Allow` rule today** — it falls through to the
   `Ask` fallback (`rules.rs`), which diverges from Spec §6.1 (read_image →
   allow). **Add a named `Allow` rule for `read_document`** (Spec §6.1) modeled on
   `read_file` (`:345-350`); note the `read_image` gap for the owner (see log) but
   keep this phase's scope to `read_document`.

**Legend:** `[ ]` todo · `[~]` in progress · `[x]` done · `[!]` blocked

---

## Progress summary

| Group | Status | Notes |
|---|---|---|
| 1. `ContentBlock::Document` + `DocumentContent` + `ModelInfo.documents` (`providers`/`tools`) | [x] | mirror `Image`/`vision`; serde tag `"document"` |
| 2. `read_document` tool (mirror `read_image`, `%PDF-` sniff, no `imagesize`) + registration | [x] | capability gate `!ctx.documents()`; summary = reference line (size · PDF) |
| 3. `ToolCtx` `documents`/`document_max_bytes` + engine append + token-estimate arms | [x] | mirror `with_vision`/`with_image_max_bytes`; `ContentBlock::Document` arms |
| 4. Per-adapter mapping (Anthropic `document`, OpenAI `file` part + guard) | [x] | Anthropic base64 `application/pdf`; OpenAI `file`/`file_data`; extend `has_image` guard |
| 5. Config: `document.max_bytes` (32 MiB) + per-model `documents` flag + provenance | [x] | mirror the six `image.max_bytes` sites + `ModelFile.vision` |
| 6. Permission rule: `read_document` → `Allow` in-root (named rule) | [x] | Spec §6.1; model on `read_file` rule; flag `read_image` gap in log |
| 7. Tests (offline, deterministic — §14.8) + exit criterion | [x] | round-trip, non-doc-model unsupported, transcript-path-not-bytes, both adapter mappings |

**Overall Phase 2: DONE (2026-07-15).** All 7 groups complete. Workspace
builds clean, `cargo clippy --workspace --all-targets` is fully clean (zero
warnings), `cargo fmt --check` is clean, and the offline `FakeProvider` suite
is green: `emberly-core` (`engine_loop.rs`, 111 tests, up from 108 pre-phase),
`emberly-tools` (`builtin_tools.rs`, 41 tests, up from 34), `emberly-providers`
(44 tests, up from 42), `emberly-sandbox` (31 tests, up from 30), `emberly`
(40 tests, up from 36), and `emberly-tui` (201 tests, unchanged count — the
degraded-parity case extended an existing test rather than adding a new one)
all pass. No `Cargo.toml`/`Cargo.lock` change across the entire phase.

---

## 1. `ContentBlock::Document` + `DocumentContent` + capability  *(P-12; Tech Spec §4.1)* — DONE

- [x] `message.rs:60` (beside `Image`): add `Document { media_type: String, data:
      String }` to `ContentBlock` (`data` = base64 of the file bytes). serde tag
      `"document"`. No wire type crosses the boundary (P-1).
- [x] `model.rs:99-100` (beside `vision`): add `#[serde(default)] pub documents:
      bool` to `ModelInfo`. Update the `FakeProvider` default (`fake.rs:147`) with
      `documents: false`, and add a `documents`-capable path for tests.
- [x] `tool.rs` (beside `ImageContent` `:45-49` and `image` `:75`): add `struct
      DocumentContent { media_type: String, data: String }`, `pub document:
      Option<DocumentContent>` on `ToolOutcome`, init `None` in both ctors
      (`:93`/`:107`), and `with_document(...)` (`:119-124` analog).
- [x] Engine append (`engine.rs`, beside the `outcome.image` arm): `if
      let Some(doc) = outcome.document { … push ContentBlock::Document { media_type,
      data } }` onto the turn.
- [x] Token-estimate + summarization: add a `ContentBlock::Document` arm to each
      match that currently handles `ContentBlock::Image`; define a
      `DOCUMENT_TOKEN_ESTIMATE` beside `IMAGE_TOKEN_ESTIMATE` — a coarse constant
      is fine (trigger-grade, P-6), refined by provider-reported usage where
      available (P-12).

## 2. `read_document` tool  *(T-16; Tech Spec §5.2)* — DONE

Copy `read_image.rs` structure; swap the format-detection block for a `%PDF-`
check and author the document reference-line summary.

- [x] New `crates/emberly-tools/src/builtin/read_document.rs` — `ReadDocumentArgs`
      + `ReadDocumentTool` (mirror `:22-28`); `spec()` (name `"read_document"`,
      description, `{path}` schema — mirror `:32-51`).
- [x] `execute()`: capability gate **first** — `if !ctx.documents() { return
      unsupported-capability failure }` (mirror `:65-73`, HC-6) so an incapable
      model learns it could not read the document. Then path resolve + root check
      (mirror `:75-96`), byte read (`:98-107`), size cap
      `ctx.document_max_bytes()` (mirror `:109-121`).
- [x] **Type sniff (replaces `imagesize` `:123-159`):** verify the bytes begin
      with the `%PDF-` magic prefix; reject a non-PDF with a structured failure
      (HC-6). **No parsing, no page count** (HC-2, Design §4.11).
- [x] base64-encode the bytes (`base64`, already locked) and
      `.with_document(DocumentContent{ media_type: "application/pdf", data })`.
      Summary string = the Design §4.11 reference line: `read document {rel} ·
      {N KB} · PDF` (size + format, **no page count**). This summary is what both
      frontends render (see Key fact 1) — no frontend change needed.
- [x] Register in `builtin/mod.rs`: `mod read_document;` (`:15` neighbourhood),
      re-export (`:29`), `registry.register(Arc::new(ReadDocumentTool));` beside
      `ReadImageTool` (`:41`). Advertised to the provider via `registry.specs()`
      automatically.
- [x] Tool description is behavior-critical config (C-4), overridable per C-1 —
      keep the schema/description in the tool-descriptions source. Confirmed:
      there is no separate override file today — `read_image`'s own `spec()`
      description literally *is* the tool-descriptions source (C-4's "versionable
      per model family" override mechanism is aspirational, not yet built).
      `read_document.rs` follows the identical pattern.

## 3. `ToolCtx` accessors + engine wiring  *(P-12/T-16; Tech Spec §4.1, §5.2)* — DONE

- [x] `ctx.rs`: `DOCUMENT_MAX_BYTES_DEFAULT` (32 MiB) beside `:45`; fields
      `documents` + `document_max_bytes` (mirror `:66-70`); ctor defaults
      (`:95-96`); builders `with_documents` / `with_document_max_bytes` (mirror
      `:143-152`); getters `documents()` / `document_max_bytes()` (mirror
      `:217-224`). _Pulled forward into group 2 — `read_document.rs` calls these
      accessors directly, so they had to exist to compile; see group 2 log._
- [x] Engine `ToolCtx` wiring (`engine.rs`, in `make_ctx()`): added
      `.with_documents(self.provider.model_info().documents)
      .with_document_max_bytes(self.document_max_bytes)` beside the
      vision/image wiring. Engine field `document_max_bytes` added beside
      `image_max_bytes` (public `EngineConfig` field, private engine field,
      ctor init).

## 4. Per-adapter mapping  *(P-12; Tech Spec §4.2)* — DONE

- [x] **Anthropic** (`anthropic.rs`, beside the `Image` arm): map
      `ContentBlock::Document { media_type, data }` →
      `{"type":"document","source":{"type":"base64","media_type":"application/pdf","data":…}}`.
      Applies in user content and nested `tool_result` arrays (same mapper). _Landed
      in group 1, out of compilation necessity (`ContentBlock` isn't
      `#[non_exhaustive]`) — confirmed correct against this group's own spec, no
      changes needed._
- [x] **OpenAI** (`openai.rs`): generalized the `has_image` guard to `has_media`
      so a `Document` block also triggers the structured-content path; added a
      `Document` arm mapping to OpenAI's file part —
      `{"type":"file","file":{"filename":"document.pdf","file_data":"data:application/pdf;base64,…"}}`
      (mirroring the `image_url` arm). Tool-role messages still can't carry it
      (unchanged — `user_content` is only ever called for `Role::User`). **An
      OpenAI-compatible endpoint without document input is declared
      `documents:false`** so the tool returns unsupported rather than sending —
      validating against live endpoints stays the carried-in M9 open item (Tech
      Spec §16).

## 5. Config: `document.max_bytes` + per-model flag + provenance  *(P-12/T-16; Tech Spec §8)* — DONE

Mirror the six `image.max_bytes` sites and the `ModelFile.vision` flag — all in
`crates/emberly/src/config.rs` unless noted.

- [x] `ModelFile`: added `pub documents: Option<bool>` beside `vision`;
      built into `ModelInfo` at `provider_setup.rs`
      (`documents: meta.and_then(|m| m.documents).unwrap_or(false)` — replacing
      group 1's `documents: false` placeholder).
- [x] `struct DocumentConfigFile { max_bytes: Option<usize> }` (mirror
      `ImageConfigFile`); top-level `pub document: DocumentConfigFile`
      field (`[document]` comment).
- [x] Merge (mirror the `[image]` block); resolve `document_max_bytes:
      merged.document.max_bytes.unwrap_or(32*1024*1024)`; resolved
      struct field `pub document_max_bytes: usize` — replacing group 3's
      `main.rs` placeholder literal with `resolved.document_max_bytes`.
- [x] Provenance `record("document.max_bytes", …)` on deviation (mirror the
      `[image] max_bytes` block).

## 6. Permission rule: `read_document` → allow in-root  *(T-16; Tech Spec §6.1, Requirements §6.2)* — DONE

- [x] `rules.rs` `builtin_defaults()`: added a
      `ToolSelector::Named("read_document")` → `Allow` rule modeled on the
      `read_file` block, so a project-root document read does not
      prompt (Spec §6.1). The outside-root hard gate is untouched —
      a document outside the root still asks per-action (HC-4), same as any read.

## 7. Tests + exit criterion  *(Tech Spec §14.8 offline, deterministic)* — DONE

Mirror the image tests (engine-loop, tool-unit, adapter-unit); inject a
`documents`-capable model via `FakeProvider::with_model_info`.

- [x] **Engine round-trip** (mirror `engine_loop.rs:3638-3672`): a `read_document`
      call on a `documents:true` model root-confines its path, appends a
      `ContentBlock::Document`, and the summary reads as the reference line.
      _Landed in group 3 (`read_document_round_trip_appends_document_block`),
      pulled forward to prove the engine wiring worked end-to-end._
- [x] **Unsupported on incapable model** (mirror `:3675-3712`): on a
      `documents:false` model, `read_document` returns the structured
      unsupported-capability result (HC-6) instead of sending. _Also landed in
      group 3 (`read_document_on_non_documents_model_returns_unsupported_result`)._
- [x] **Transcript records path, not bytes** (mirror `:3715-3764`, HC-7): the
      `tool_call` records the project-relative path; the base64 bytes never land in
      the JSONL (re-derived from the file when building the request). Added
      `read_document_transcript_records_path_not_bytes`.
- [x] **Tool-unit cases** (mirror `builtin_tools.rs:573-720`): oversize reject
      (> `document.max_bytes`), non-PDF (magic-byte) reject, outside-root
      permission path (denied + allowed), missing-file failure, plus a
      no-document-support unit case and a success/payload case. Added 7 tests
      in `builtin_tools.rs`: `read_document_on_non_documents_model_returns_unsupported`,
      `read_document_success_appends_document_payload`, `read_document_rejects_oversize`,
      `read_document_rejects_non_pdf`, `read_document_outside_root_requests_permission`,
      `read_document_missing_file_is_a_failure_not_a_crash`, and
      `read_document_git_dir_requests_permission_like_any_outside_path` (see log —
      this last one actually proves an in-root `.git/` read *succeeds*, not that it's
      refused; see the discrepancy note in the log).
- [x] **Adapter mapping** (mirror `anthropic.rs:594-616`, `openai.rs:425-454`): a
      `Document` block maps to the Anthropic base64 `document` source and to the
      OpenAI `file`/`file_data` part; the OpenAI guard triggers on `Document`.
      _Landed in group 4 (`document_block_maps_to_anthropic_base64_source`,
      `document_in_user_message_produces_file_data_uri`)._
- [x] **Degraded parity:** the plain frontend renders the reference line (ASCII,
      no ANSI) via the generic `ToolFinished` path — extended the existing
      `degraded_output_has_no_ansi_escapes` test (`line.rs`) with a
      `read_document`/no-document-support pair alongside the image case.
- [x] **Exit criterion (Phase 2 done when):** `read_document` root-confines,
      rejects oversize/non-PDF, appends a `ContentBlock::Document`, and on a
      `documents:false` model returns unsupported instead of sending; both adapter
      mappings are exercised via `FakeProvider` (Tech Spec §14.8, P-12/T-16).
      Workspace clippy-clean under the §1 lint policy — confirmed:
      `cargo clippy --workspace --all-targets` is zero-warning. **`cargo
      deny`/`vet` are not installed in this environment** — confirmed no new
      dependency by direct inspection instead: `git status` shows no
      `Cargo.toml`/`Cargo.lock` diff across all seven groups (HC-2, Tech Spec
      §12). Offline suite green: `cargo test --workspace` passes in full.

---

## Decisions & notes log

> Fill in as the phase proceeds (mirrors the v0.1–v0.4 phase docs' logs).

- **No parser, size not pages.** `read_document` sniffs `%PDF-` and forwards the
  bytes; the reference line shows size + format, never a page count (that would
  need a PDF lib — out of scope, HC-2). This is the load-bearing decision that
  keeps P-12/T-16 dependency-free. _If a page count is ever wanted it is a new
  dependency decision and a Spec bump (G-24), not a quiet addition._
- **No new TUI render code.** The reference line and the no-`documents` note are
  the tool's `ToolOutcome` summary strings, rendered by the generic `ToolFinished`
  handler (`line.rs:85-93`, `render.rs:715-732`). Documents need no frontend
  branch — author the strings in `read_document.rs`. _Confirm Design §4.11 is
  satisfied by the summary alone; if a distinct doc label is wanted it lives in
  the summary, not the frontend._
- **OpenAI `has_image` guard generalized.** The user-content guard
  (`openai.rs:195-198`) is image-specific; it must also fire on `Document`.
  _Decide whether to rename it (`has_media`) or add a parallel check; record here._
- **`read_image` lacks a named allow rule.** Only `read_file` is explicitly
  `Allow` in-root; `read_image` falls through to the `Ask` fallback, which appears
  to diverge from Spec §6.1 (read_image → allow). This phase adds a named `Allow`
  rule for `read_document` (Spec §6.1). _Flag the `read_image` discrepancy to the
  owner as a separate cleanup — either the code needs the rule or the Spec needs a
  correction (G-24); out of scope for this phase._
- **docx stays out.** Non-PDF formats are out of scope (Requirements §2.3); the
  `%PDF-` sniff rejects everything else with a clean HC-6 failure. Document
  creation/editing are skills (FR-7), not this tool.
- **Open item carried in (Tech Spec §16 v0.9):** confirm the `Document` block maps
  cleanly to the Anthropic native document block and each OpenAI-compatible
  endpoint's document input (or is correctly declared `documents:false`) against
  **live** endpoints, and that provider page/token limits surface as clean
  unsupported/oversize results, not crashes.
- **Group 1 done (2026-07-15).** `ContentBlock::Document{media_type,data}` added
  beside `Image` in `message.rs` (serde tag `"document"`); `ModelInfo.documents:
  bool` added beside `vision` (every existing `ModelInfo` literal across
  `emberly-providers`/`emberly-core`/`emberly` updated — the struct is not
  `#[non_exhaustive]`, so this touched ~10 call sites, all set to
  `documents: false` except a new `documents_model_info()` test helper in
  `engine_loop.rs` mirroring `vision_model_info()`, left for group 7's tests to
  use (a transitional `dead_code` warning until then, same pattern Phase 1
  tolerated mid-phase). `DocumentContent` + `ToolOutcome.document` +
  `with_document(...)` added beside their `Image` analogs in `tool.rs`. Engine
  append (`engine.rs`, beside the image-block append) pushes a
  `ContentBlock::Document` as a synthetic user message, same HC-7 reasoning as
  images. Added a `ContentBlock::Document` arm to all three matches that handle
  `ContentBlock::Image` (context-budget token count, summarization render,
  recall render) plus `DOCUMENT_TOKEN_ESTIMATE` beside `IMAGE_TOKEN_ESTIMATE`.
  **`ContentBlock` is not `#[non_exhaustive]`,** so `anthropic.rs`'s
  `block_to_anthropic` — an exhaustive match — would not compile once the
  variant existed; pulled forward the single Anthropic mapping arm from group 4
  (`{"type":"document","source":{"type":"base64","media_type":…,"data":…}}`,
  the exact Tech Spec §4.2 shape) out of compilation necessity. `openai.rs`'s
  matches already carry a wildcard arm, so a `Document` block is silently
  dropped there for now — group 4 still owns extending the `has_image` guard
  and adding the real `file`/`file_data` mapping. Growing `ToolOutcome` by a
  second `Option<…Content>` field pushed it past clippy's
  `large_enum_variant` threshold on `engine::ToolCallResult::Completed
  (ToolOutcome)`; boxed it (`Completed(Box<ToolOutcome>)`) at all four
  construction/destructure sites — a pure size fix, no behavior change.
  Workspace builds clean; `cargo test --workspace` green (108 tests in
  `engine_loop.rs`, unchanged — group 1 adds no new test *cases*, only the
  `documents_model_info` seam); `cargo clippy --workspace --all-targets` clean
  except the one expected transitional `dead_code` warning, which clears when
  group 7 lands; `cargo fmt --check` clean. No `Cargo.toml`/`Cargo.lock` change
  (HC-2, Tech Spec §12).
- **Group 2 done (2026-07-15).** `crates/emberly-tools/src/builtin/read_document.rs`
  added, a near-exact structural mirror of `read_image.rs`: same args/spec/describe
  shape, same capability-gate-first order (HC-6), same path-resolve +
  outside-root permission-request shape (HC-4), same byte-read +
  size-cap-before-encode order. The `imagesize` header-parse block is replaced
  by a `bytes.starts_with(b"%PDF-")` check — no parser, no page count (HC-2);
  the reference-line summary is `read document {rel} · {size} · PDF` via a new
  `format_size` helper (KB below 1 MiB, MB at/above, one decimal — the same
  spirit as `read_image`'s `{width}×{height}` but for size, since there is no
  intrinsic dimension to report). Registered in `builtin/mod.rs` beside
  `ReadImageTool`, in the same three places (`mod`, `pub use`,
  `default_registry()`). **Pulled group 3's `ToolCtx` accessors forward**
  (`DOCUMENT_MAX_BYTES_DEFAULT` = 32 MiB, `documents`/`document_max_bytes`
  fields, `with_documents`/`with_document_max_bytes` builders,
  `documents()`/`document_max_bytes()` getters) — the tool's `execute()` calls
  `ctx.documents()`/`ctx.document_max_bytes()` directly, so they had to exist
  to compile; group 3's remaining scope (wiring the *engine*'s `make_ctx` to
  set them from `model.documents`/config, so the capability actually reaches
  the tool at runtime) is still open. Until that wiring lands, every `ToolCtx`
  defaults `documents: false` — `read_document` will report "no document
  support" even on a documents-capable model unless a test builds its `ToolCtx`
  directly with `.with_documents(true)` (the same pattern `read_image`'s
  existing unit tests already use via a `ctx_vision()` helper). Workspace
  builds clean; `cargo test --workspace` green, no count changed (no new test
  *cases* yet — group 7 owns those); `cargo clippy --workspace --all-targets`
  clean except the one pre-existing transitional `dead_code` warning from
  group 1; `cargo fmt --check` clean. No `Cargo.toml`/`Cargo.lock` change.
- **Group 3 done (2026-07-15).** `EngineConfig.document_max_bytes` (public),
  the private engine field, and its ctor init added beside every
  `image_max_bytes` triplet; `make_ctx()` now wires
  `.with_documents(self.provider.model_info().documents)
  .with_document_max_bytes(self.document_max_bytes)` beside the vision/image
  builders — this is the piece that makes group 2's `ctx.documents()` call
  actually reflect the active model at runtime rather than always defaulting
  `false`. `main.rs`'s `EngineConfig` literal gets a **placeholder**
  `document_max_bytes: 32 * 1024 * 1024` (group 5 replaces it with
  `resolved.document_max_bytes` once `[document]` config exists — same
  placeholder-then-replace pattern Phase 1 group 1 used for
  `CompletionConfig::default()`); the `engine_loop.rs` test-config literal gets
  the same literal, non-placeholder (tests don't route through `emberly`'s
  config resolver). Used the `documents_model_info()` seam group 1 left
  unused: added `read_document_round_trip_appends_document_block` and
  `read_document_on_non_documents_model_returns_unsupported_result`
  (`engine_loop.rs`, mirroring `read_image`'s equivalent pair almost exactly,
  down to a `tiny_pdf()` fixture beside `tiny_png()`) — these aren't group 7's
  full coverage (no oversize/non-PDF/transcript-HC-7/adapter-mapping cases
  yet), but they're the minimum proof that group 3's engine wiring works
  end-to-end, and they clear the transitional `dead_code` warning group 1 left
  open. Workspace builds clean; `cargo test --workspace` green
  (`engine_loop.rs` up to 110 tests, from 108); `cargo clippy --workspace
  --all-targets` **fully clean, zero warnings** (the group-1 transitional one
  is gone); `cargo fmt --check` clean. No `Cargo.toml`/`Cargo.lock` change.
- **Group 4 done (2026-07-15).** Anthropic's mapping arm needed no changes —
  it landed correctly in group 1 out of compilation necessity, and this group
  just confirmed it against Tech Spec §4.2 and added a unit test
  (`document_block_maps_to_anthropic_base64_source`, mirroring
  `image_block_maps_to_anthropic_base64_source`). OpenAI's `has_image` guard
  (`openai.rs`) is renamed `has_media` and now matches
  `ContentBlock::Image { .. } | ContentBlock::Document { .. }`; a `Document`
  arm was added to the parts-mapping match producing
  `{"type":"file","file":{"filename":"document.pdf","file_data":"data:application/pdf;base64,…"}}`.
  **`ContentBlock::Document` carries no filename** (HC-2 — the harness never
  tracks more than media type + bytes, so there is nothing else to put there);
  used a fixed `"document.pdf"` since the media type is always
  `application/pdf` (Requirements §2.3, PDF-only). Confirmed `user_content` is
  only ever invoked for `Role::User` (unchanged dispatch in
  `message_to_openai`), so the pre-existing "tool-role can't carry it" comment
  needed no update — it already covered image and now document alike. Added
  `document_in_user_message_produces_file_data_uri`, mirroring
  `image_in_user_message_produces_image_url_data_uri`. Both adapter mappings
  are now exercised via unit test, ahead of group 7's fuller
  `FakeProvider`-driven coverage. Workspace builds clean; `cargo test
  --workspace` green (`emberly-providers` up to 44 tests, from 42); `cargo
  clippy --workspace --all-targets` fully clean, zero warnings; `cargo fmt
  --check` clean. No `Cargo.toml`/`Cargo.lock` change. The M9 open item
  (validating both mappings against **live** endpoints, not just
  `FakeProvider`) remains carried forward — Tech Spec §16.
- **Group 5 done (2026-07-15).** Mirrored every `[image]` site in
  `crates/emberly/src/config.rs` for `[document]`: `DocumentConfigFile {
  max_bytes: Option<usize> }` + top-level `pub document` field, a
  field-by-field merge block, a provenance `record("document.max_bytes", …)`
  block (silent on the default, speech on deviation — same as `[image]`), and
  `Resolved.document_max_bytes` resolving to `merged.document.max_bytes
  .unwrap_or(32 * 1024 * 1024)`. `ModelFile.documents: Option<bool>` added
  beside `vision`; `provider_setup.rs` now builds `ModelInfo.documents` from
  `meta.and_then(|m| m.documents).unwrap_or(false)`, **replacing group 1's
  `documents: false` placeholder** — a model can finally declare document
  support through config. `main.rs`'s `EngineConfig` literal now reads
  `resolved.document_max_bytes`, **replacing group 3's placeholder literal**.
  Added three config-level tests mirroring the `[completion]` test pattern
  (`document_config_parses_and_merges_scalars`,
  `document_max_bytes_resolves_with_provenance`,
  `document_max_bytes_defaults_to_32_mib_when_unconfigured`) plus one in
  `provider_setup.rs` (`documents_flag_feeds_model_info`, mirroring
  `per_model_metadata_feeds_model_info`) proving the `ModelFile.documents` →
  `ModelInfo.documents` round trip through `build_profile(...).model_info()`
  — placed in `provider_setup.rs` rather than `config.rs` since that's where
  the actual `ModelInfo`-building logic lives (an initial draft of this test
  in `config.rs` only checked the parsed struct field, not the feed-through;
  moved once that gap was noticed). Workspace builds clean; `cargo test
  --workspace` green (`emberly` bin test binary up to 40, from 36 — 3
  config.rs + 1 provider_setup.rs); `cargo clippy --workspace --all-targets`
  fully clean, zero warnings; `cargo fmt --check` clean. No
  `Cargo.toml`/`Cargo.lock` change (HC-2, Tech Spec §12) — this closes out
  every "no new dependency" surface Phase 2 touches.
- **Group 6 done (2026-07-15).** Added a `ToolSelector::Named("read_document")`
  → `Allow` rule to `builtin_defaults()` in `emberly-sandbox/src/rules.rs`,
  directly beside `read_file`'s — a project-root document read is an ordinary
  project read (§6.2) and does not prompt, matching Spec §6.1. The
  outside-root hard gate is untouched (it's structural, evaluated
  independently of which named rule matches — HC-4), so a `read_document`
  outside the root still asks per-action exactly like `read_file`. Added
  `read_document_in_root_allows_outside_root_asks`, mirroring
  `reads_in_root_allow_writes_ask` / `outside_root_never_auto_allows`, proving
  both halves in one test. **The `read_image` gap this group's own log flagged
  stays open and unfixed** — `read_image` still has no named `Allow` rule and
  falls through to the `Ask` fallback, which appears to diverge from Spec
  §6.1's own text (`read_image` → allow, same as every other project read).
  This was correctly out of scope for Phase 2 (which owns documents, not a
  retroactive image fix); it remains a discrepancy for the owner to resolve as
  a separate cleanup — either `read_image` needs the same rule `read_document`
  just got, or the Spec's §6.1 wording needs a correction (G-24). Workspace
  builds clean; `cargo test --workspace` green (`emberly-sandbox` up to 31
  tests, from 30); `cargo clippy --workspace --all-targets` fully clean, zero
  warnings; `cargo fmt --check` clean. No `Cargo.toml`/`Cargo.lock` change.
- **Group 7 done (2026-07-15) — Phase 2 complete.** Filled in the coverage
  groups 3/4 didn't already pull forward: an HC-7 transcript test
  (`read_document_transcript_records_path_not_bytes`, mirroring
  `read_image_transcript_records_path_not_bytes` — confirms the tool_call
  records the path, the tool_result records the reference line, and the
  base64 PDF bytes (`JVBERi0…`) never appear anywhere in the JSONL); 7
  tool-unit tests in `builtin_tools.rs` (unsupported-capability, success,
  oversize, non-PDF magic-byte rejection, outside-root denied/allowed,
  missing-file) requiring a new `ReadDocumentTool` re-export at the
  `emberly-tools` crate root (it wasn't exported outside `builtin::` until
  now — `read_image`'s own re-export was already there from Phase 0.4, so
  this was a one-line gap group 2 should have caught but didn't need to,
  since group 2 never referenced the tool from outside its own crate); and a
  degraded-parity extension to `line.rs`'s `degraded_output_has_no_ansi_escapes`
  test with a `read_document` reference-line pair, mirroring the image case.
  **Discrepancy surfaced and resolved by inspection, not by adding code:** the
  Tech Spec's own §14.8 exit-criterion prose (both the 0.4 image line and the
  0.4.1 document line) says the round-trip test should assert a `.git/`
  rejection, but neither `read_image` nor `read_document`'s own §5.2 tool-table
  row mentions one, and `read_image.rs` has no `is_under_git_dir` call at all
  (confirmed by reading it — only `write_file`/`edit_file` guard `.git/`,
  Tech Spec §5.2 table). Rather than add a new restriction to `read_document`
  that `read_image` never had (which would be scope creep past "the exact
  analog of the image feature" this phase's own charter states), added
  `read_document_git_dir_requests_permission_like_any_outside_path` to prove
  and document the **actual** behavior: an in-root `.git/` path succeeds via
  the ordinary in-root `Allow` rule (group 6), same as any other project-
  relative read. _This is a second instance of the same class of gap as the
  `read_image`-lacks-an-Allow-rule discrepancy flagged in group 6's log — the
  Tech Spec's test-description prose (§14.8) is looser than its own tool-table
  (§5.2) and the shipped code for both the 0.4 and 0.4.1 read tools. Flagging
  both together for the owner as one cleanup: either reads should hard-refuse
  `.git/` like writes do (a behavior change, Spec bump) or §14.8's prose
  should drop the `.git/` clause for read tools (a wording fix) — out of
  scope for this phase either way._ Workspace builds clean; `cargo test
  --workspace` green (`engine_loop.rs` up to 111 tests from 110,
  `builtin_tools.rs` up to 41 from 34, `emberly-tui` unchanged at 201 — an
  existing test extended, not a new one added); `cargo clippy --workspace
  --all-targets` fully clean, zero warnings; `cargo fmt --check` clean.
  `cargo deny`/`cargo vet` are not installed in this sandboxed environment;
  confirmed no new dependency by direct inspection instead — `git status`
  shows no `Cargo.toml`/`Cargo.lock` diff across any of the seven groups
  (HC-2, Tech Spec §12). **Phase 2 is DONE.**
