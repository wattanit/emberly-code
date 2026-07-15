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
| 1. `ContentBlock::Document` + `DocumentContent` + `ModelInfo.documents` (`providers`/`tools`) | [ ] | mirror `Image`/`vision`; serde tag `"document"` |
| 2. `read_document` tool (mirror `read_image`, `%PDF-` sniff, no `imagesize`) + registration | [ ] | capability gate `!ctx.documents()`; summary = reference line (size · PDF) |
| 3. `ToolCtx` `documents`/`document_max_bytes` + engine append + token-estimate arms | [ ] | mirror `with_vision`/`with_image_max_bytes`; `ContentBlock::Document` arms |
| 4. Per-adapter mapping (Anthropic `document`, OpenAI `file` part + guard) | [ ] | Anthropic base64 `application/pdf`; OpenAI `file`/`file_data`; extend `has_image` guard |
| 5. Config: `document.max_bytes` (32 MiB) + per-model `documents` flag + provenance | [ ] | mirror the six `image.max_bytes` sites + `ModelFile.vision` |
| 6. Permission rule: `read_document` → `Allow` in-root (named rule) | [ ] | Spec §6.1; model on `read_file` rule; flag `read_image` gap in log |
| 7. Tests (offline, deterministic — §14.8) + exit criterion | [ ] | round-trip, non-doc-model unsupported, transcript-path-not-bytes, both adapter mappings |

**Overall Phase 2: NOT STARTED.**

---

## 1. `ContentBlock::Document` + `DocumentContent` + capability  *(P-12; Tech Spec §4.1)*

- [ ] `message.rs:60` (beside `Image`): add `Document { media_type: String, data:
      String }` to `ContentBlock` (`data` = base64 of the file bytes). serde tag
      `"document"`. No wire type crosses the boundary (P-1).
- [ ] `model.rs:99-100` (beside `vision`): add `#[serde(default)] pub documents:
      bool` to `ModelInfo`. Update the `FakeProvider` default (`fake.rs:147`) with
      `documents: false`, and add a `documents`-capable path for tests.
- [ ] `tool.rs` (beside `ImageContent` `:45-49` and `image` `:75`): add `struct
      DocumentContent { media_type: String, data: String }`, `pub document:
      Option<DocumentContent>` on `ToolOutcome`, init `None` in both ctors
      (`:93`/`:107`), and `with_document(...)` (`:119-124` analog).
- [ ] Engine append (`engine.rs:2189-2196`, beside the `outcome.image` arm): `if
      let Some(doc) = outcome.document { … push ContentBlock::Document { media_type,
      data } }` onto the turn.
- [ ] Token-estimate + summarization: add a `ContentBlock::Document` arm to each
      match that currently handles `ContentBlock::Image` (`engine.rs:2854`,
      `:2898-2900`, `:2945-2946`); define a `DOCUMENT_TOKEN_ESTIMATE` beside
      `IMAGE_TOKEN_ESTIMATE` (`:2966`) — a coarse constant is fine (trigger-grade,
      P-6), refined by provider-reported usage where available (P-12).

## 2. `read_document` tool  *(T-16; Tech Spec §5.2)*

Copy `read_image.rs` structure; swap the format-detection block for a `%PDF-`
check and author the document reference-line summary.

- [ ] New `crates/emberly-tools/src/builtin/read_document.rs` — `ReadDocumentArgs`
      + `ReadDocumentTool` (mirror `:22-28`); `spec()` (name `"read_document"`,
      description, `{path}` schema — mirror `:32-51`).
- [ ] `execute()`: capability gate **first** — `if !ctx.documents() { return
      unsupported-capability failure }` (mirror `:65-73`, HC-6) so an incapable
      model learns it could not read the document. Then path resolve + root check
      (mirror `:75-96`), byte read (`:98-107`), size cap
      `ctx.document_max_bytes()` (mirror `:109-121`).
- [ ] **Type sniff (replaces `imagesize` `:123-159`):** verify the bytes begin
      with the `%PDF-` magic prefix; reject a non-PDF with a structured failure
      (HC-6). **No parsing, no page count** (HC-2, Design §4.11).
- [ ] base64-encode the bytes (`base64`, already locked) and
      `.with_document(DocumentContent{ media_type: "application/pdf", data })`.
      Summary string = the Design §4.11 reference line: `read document {rel} ·
      {N KB} · PDF` (size + format, **no page count**). This summary is what both
      frontends render (see Key fact 1) — no frontend change needed.
- [ ] Register in `builtin/mod.rs`: `mod read_document;` (`:15` neighbourhood),
      re-export (`:29`), `registry.register(Arc::new(ReadDocumentTool));` beside
      `ReadImageTool` (`:41`). Advertised to the provider via `registry.specs()`
      automatically.
- [ ] Tool description is behavior-critical config (C-4), overridable per C-1 —
      keep the schema/description in the tool-descriptions source.

## 3. `ToolCtx` accessors + engine wiring  *(P-12/T-16; Tech Spec §4.1, §5.2)*

- [ ] `ctx.rs`: `DOCUMENT_MAX_BYTES_DEFAULT` (32 MiB) beside `:45`; fields
      `documents` + `document_max_bytes` (mirror `:66-70`); ctor defaults
      (`:95-96`); builders `with_documents` / `with_document_max_bytes` (mirror
      `:143-152`); getters `documents()` / `document_max_bytes()` (mirror
      `:217-224`).
- [ ] Engine `ToolCtx` wiring (`engine.rs:2500-2501`): add
      `.with_documents(model.documents).with_document_max_bytes(self.document_max_bytes)`
      beside the vision/image wiring. Engine field `document_max_bytes` beside
      `image_max_bytes` (`:606`/init `:749`/config `:230-232`).

## 4. Per-adapter mapping  *(P-12; Tech Spec §4.2)*

- [ ] **Anthropic** (`anthropic.rs:227-236`, beside the `Image` arm): map
      `ContentBlock::Document { media_type, data }` →
      `{"type":"document","source":{"type":"base64","media_type":"application/pdf","data":…}}`.
      Applies in user content and nested `tool_result` arrays (same mapper).
- [ ] **OpenAI** (`openai.rs:194-217`): extend the `has_image` guard (`:195-198`)
      so a `Document` block also triggers the structured-content path (rename or
      generalize the guard); add a `Document` arm mapping to OpenAI's file part —
      `{"type":"file","file":{"filename":…,"file_data":"data:application/pdf;base64,…"}}`
      (mirror the `image_url` arm `:207-212`). Tool-role messages still can't carry
      it (comment `:193`). **An OpenAI-compatible endpoint without document input
      is declared `documents:false`** so the tool returns unsupported rather than
      sending — validate against live endpoints (M9 open item, Tech Spec §16).

## 5. Config: `document.max_bytes` + per-model flag + provenance  *(P-12/T-16; Tech Spec §8)*

Mirror the six `image.max_bytes` sites and the `ModelFile.vision` flag — all in
`crates/emberly/src/config.rs` unless noted.

- [ ] `ModelFile` (`:226-228`): add `pub documents: Option<bool>` beside `vision`;
      build into `ModelInfo` at `provider_setup.rs:107`
      (`documents: meta.and_then(|m| m.documents).unwrap_or(false)`).
- [ ] `struct DocumentConfigFile { max_bytes: Option<usize> }` (mirror
      `ImageConfigFile` `:122-128`); top-level `pub document: DocumentConfigFile`
      field (`:52-54` analog, `[document]` comment).
- [ ] Merge (`:399-402` analog); resolve `document_max_bytes:
      merged.document.max_bytes.unwrap_or(32*1024*1024)` (`:987` analog); resolved
      struct field `pub document_max_bytes: usize` (`:479-481` analog).
- [ ] Provenance `record("document.max_bytes", …)` on deviation (mirror `:764-780`).

## 6. Permission rule: `read_document` → allow in-root  *(T-16; Tech Spec §6.1, Requirements §6.2)*

- [ ] `rules.rs` `builtin_defaults()` (`:343`): add a
      `ToolSelector::Named("read_document")` → `Allow` rule modeled on the
      `read_file` block (`:345-350`), so a project-root document read does not
      prompt (Spec §6.1). The outside-root hard gate (`:213`/`:552`) is untouched —
      a document outside the root still asks per-action (HC-4), same as any read.

## 7. Tests + exit criterion  *(Tech Spec §14.8 offline, deterministic)*

Mirror the image tests (engine-loop, tool-unit, adapter-unit); inject a
`documents`-capable model via `FakeProvider::with_model_info`.

- [ ] **Engine round-trip** (mirror `engine_loop.rs:3638-3672`): a `read_document`
      call on a `documents:true` model root-confines its path, appends a
      `ContentBlock::Document`, and the summary reads as the reference line.
- [ ] **Unsupported on incapable model** (mirror `:3675-3712`): on a
      `documents:false` model, `read_document` returns the structured
      unsupported-capability result (HC-6) instead of sending.
- [ ] **Transcript records path, not bytes** (mirror `:3715-3764`, HC-7): the
      `tool_call` records the project-relative path; the base64 bytes never land in
      the JSONL (re-derived from the file when building the request).
- [ ] **Tool-unit cases** (mirror `builtin_tools.rs:573-720`): oversize reject
      (> `document.max_bytes`), non-PDF (magic-byte) reject, `.git/` / outside-root
      permission path, missing-file failure.
- [ ] **Adapter mapping** (mirror `anthropic.rs:594-616`, `openai.rs:425-454`): a
      `Document` block maps to the Anthropic base64 `document` source and to the
      OpenAI `file`/`file_data` part; the OpenAI guard triggers on `Document`.
- [ ] **Degraded parity:** the plain frontend renders the reference line (ASCII,
      no ANSI) via the generic `ToolFinished` path — extend the existing
      degraded-output test alongside the image case.
- [ ] **Exit criterion (Phase 2 done when):** `read_document` root-confines,
      rejects oversize/`.git/`/non-PDF, appends a `ContentBlock::Document`, and on a
      `documents:false` model returns unsupported instead of sending; both adapter
      mappings are exercised via `FakeProvider` (Tech Spec §14.8, P-12/T-16).
      Workspace clippy-clean under the §1 lint policy; **`cargo deny`/`vet` confirm
      no new dependency entered the tree** (HC-2, Tech Spec §12); offline suite
      green.

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
