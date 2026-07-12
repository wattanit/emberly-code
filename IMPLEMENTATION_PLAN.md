# Emberly Code — Implementation Plan (0.4 feature set)

**Status:** 📋 **planned** — not yet started. Six phases expanding Tech Spec
milestone **M8** (the 0.4 capability-parity feature set): the task-list tool
(T-11), multimodal image input (P-11 + T-12), persistent memory (FR-6 + T-13),
the skill system (FR-7 + T-15), the harness-owned web-search backend (T-14),
and TUI mouse support (Design §3.4).
**Date:** 2026-07-12 (planned)
**Owner:** Wattanit
**Source documents** (G-14 as-built pin for the 0.4 release):
- Requirements Document v0.7 (`docs/emberly-code-requirements.md`) — WHAT/WHY
- Design Guideline v0.7 (`docs/emberly-code-design-guideline.md`) — UX/voice
- Technical Specification v0.8 (`docs/emberly-code-tech-spec.md`) — HOW

These three versions are the as-built truth for the 0.4 release, all
`Status: approved` (owner, 2026-07-12). A stale pin is a defect (G-16); when a
foundation document bumps, update this pin. Discoveries that change HOW flow
back into the Technical Specification as version bumps (G-24/G-25) — this plan
never becomes a shadow spec.

This plan expands Tech Spec §15 milestone **M8** into workable phases. It builds
on the shipped 0.3 product (M7 — salient tool-result reduction, the adaptive
context window + `recall`, `/compact` surface + automatic compaction, the
derived resume cache), the 0.2 product beneath it (M6 — endpoint-configurable
provider profiles + Z.ai, in-app editing, reasoning effort/trail, ask-user +
tool-call explanation, workspace trust, loop-breaking guardrail), and the 0.1
product beneath that (M1–M5 — the six crates, the agent loop, both live
providers, the full TUI, sessions / resume / compaction engine, the permission
rule engine, and OS confinement on Linux + macOS). The prior plans are preserved
in `docs/version-0-1/`, `docs/version-0-2/`, and `docs/version-0-3/` as the
earlier as-built records.

Where the 0.3 set was an *economy layer* over existing surface, the 0.4 set is
**new capability surface** — planning, sight, durable memory, extensible skills,
live web reach, and pointer interaction — the parity gap against mature coding
agents (Requirements §2.1, 0.4 feature set). The phases are sliced one per
capability so each ships and tests on its own, ordered so shared machinery is
built before it is reused: the task-list tool establishes the engine-state-tool
+ sidebar-section pattern; memory establishes the progressive-disclosure store
(frontmatter, pinned index, trust-gating, sidebar inspector) that skills then
reuse; mouse comes last so it can wire clicks to every surface the other phases
add.

---

## Guiding principles (apply to every phase)

- **Provider-agnosticism stays real, not nominal (Requirements §1, P-1).** No
  0.4 capability may couple to a single vendor. Image support lives *behind* the
  `Provider` abstraction as a normalized `ContentBlock::Image` with per-adapter
  mapping and a `vision` capability flag (P-11, Tech Spec §4.1/§4.2) — never a
  vendor branch in the tool. Web search is a **harness-owned** first-party
  client reaching a configurable backend (T-14, Tech Spec §5.5), explicitly
  *not* a provider's server-side search — a capability that works only on the
  vendors that offer it is not provider-agnostic (Requirements §2.2). A model
  without a capability gets a structured unsupported result (HC-6), never a
  crash and never a silent drop.
- **The safety model is never widened (Requirements §6, HC-4/HC-5, FR-1).**
  Memory is a **harness-owned, schema-constrained** store — the model supplies
  content, never a path — so it is harness-managed persistence like the
  transcript, not an agent filesystem write, and does not touch HC-4 (FR-6,
  Tech Spec §8.1). A skill's bundled script runs *only* through an ordinary,
  permission- and sandbox-gated `bash` call (FR-7, Tech Spec §8.2); the `skill`
  tool reads instruction text and executes nothing. Web search is
  permission-gated with untrusted results (T-14). Project-scope memory and
  skills load **only** under a trusted root (FR-1, Tech Spec §6.7).
  **`emberly-sandbox` is untouched** by every phase — none of the 0.4
  capabilities is security-critical in the containment sense; they add surface
  *through* the existing rule/permission/trust layers, never around them.
- **Progressive disclosure is the context through-line (Requirements §8).**
  The task list, the memory index, and the skill catalog are pinned context
  (Tech Spec §7) but tiny; entry/skill **bodies** load on demand via their
  tools. Having many memories or skills costs only their index/catalog until one
  is consulted — the same economy the 0.3 set turns on. No 0.4 feature floods
  context: image sends respect `image.max_bytes`, and search results are capped
  (`search.max_results`) and pass the §5.3 size backstop.
- **Pure-Rust dependencies only (HC-2).** The 0.4 set is the first to add
  dependencies since v0.1: `base64` and `imagesize` (Tools, for T-12) and
  `reqwest` into `emberly-tools` (for T-14) — all pure-Rust, all requiring
  `cargo vet` acceptance before merge (Requirements §10, Tech Spec §12). Memory
  and skill manifests use **TOML frontmatter** (reuse the existing `toml` crate)
  with first-party frontmatter splitting — deliberately no YAML crate. Anything
  that appears to need a further crate is a signal to re-check the design.
- **Every phase leaves a shippable, tested slice.** Each ends green with its
  offline, deterministic `FakeProvider`-driven coverage from Tech Spec §14.7,
  and with degraded-mode parity (ASCII markers, no color/motion/mouse reliance)
  wherever it adds or changes a UI surface (Design §7).

---

## Phase 1 — Task-list ("todo") tool (T-11)

**Goal:** Give the model an explicit, ordered, user-visible task list for
multi-step work — model-authored planning made legible — as pure engine state,
touching no filesystem or network.

**Scope**
- **`todo` built-in tool (Tech Spec §5.2).** Args: the *full* list of
  `{text, status: pending|in_progress|done}` items — the model always sends the
  complete list, so the engine replaces rather than merges partial edits. Engine
  state; touches nothing external, so like `recall`/`ask_user` it bypasses the
  sandbox and is **not permission-gated** (Requirements T-11, §6). The harness
  never writes, reorders, or checks off items on the model's behalf.
- **Events (Tech Spec §3.1/§3.2).** Emit `TaskListUpdated{items}` for the TUI;
  record a `task_list` transcript event after each update (additive, older
  readers warn-skip, no `SCHEMA_VERSION` bump — HC-7).
- **Pinned across compaction (Tech Spec §7, Requirements §13 resolved).** The
  current task list joins the pinned set so "what's left" survives compaction
  and windowing. `context.pin_task_list = bool` (**default `true`**, Tech
  Spec §8).
- **Render in two calm places (Design §4.7).** An inline checklist block where
  the model updates it, plus the sidebar **Tasks** section (Design §3.1). Status
  glyphs `○`/`◐`/`✓` with ASCII `[ ]`/`[~]`/`[x]` in degraded mode; the single
  in-progress item lightly ember-accented; a completed list settles to an
  all-`✓` block rather than vanishing. Color is never the sole signal (Design
  §7).

**Satisfies:** T-11; Tech Spec §5.2, §3.1/§3.2, §7, §8; Design §4.7, §3.1, §7;
HC-7.

**Done when:** a `todo` round-trip asserts a full-list replace emits
`TaskListUpdated` and a `task_list` transcript event and stays pinned across a
compaction; the sidebar and inline renders show correct glyphs with an
ASCII-fallback degraded-mode test (Tech Spec §14.7, T-11).

---

## Phase 2 — Multimodal image input (P-11, T-12)

**Goal:** Let the model *see* — read an image file already in the project into
context as an image content block — behind the provider abstraction so it works
across vendors and degrades cleanly on models without vision.

**Scope**
- **Normalized image content block (Tech Spec §4.1).** Add
  `ContentBlock::Image{media_type, data}` (`data` = base64 of the file bytes)
  to the normalized message type alongside text/tool blocks; no wire type
  crosses the boundary (P-1). Add a `vision: bool` capability to `ModelInfo`
  (per-model config flag, **default `false`**), so an image is never sent to a
  model not declared vision-capable (P-11).
- **Per-adapter mapping (Tech Spec §4.2).** The `anthropic` adapter maps to an
  `image` content block (base64 `source` + media type); the `openai` adapter to
  an `image_url` part with a `data:` URI. Validate both against live endpoints
  (M8 open item, Tech Spec §16).
- **`read_image` built-in tool (Tech Spec §5.2).** Path normalized and
  root-checked exactly as `read_file`, governed by the same project-read rules
  (§6.2). Detect format + dimensions header-only (`imagesize`), reject over
  `image.max_bytes` (**default 5 MiB**), base64-encode (`base64`), append a
  `ContentBlock::Image`. Formats: PNG, JPEG, GIF (first frame), WebP. On a
  non-`vision` model, return the structured unsupported-capability result
  (HC-6) instead of sending — the model learns it could not see, rather than
  assuming it saw.
- **Reference-line render, no pixels (Design §4.8).** A `read_image` result
  renders as a labeled reference line (`name · WxH · format`); **no
  sixel/kitty/iTerm image rendering** in scope (Design §4.8/§10). An
  unsupported-vision result renders as a calm tool-result note, not a harness
  error (Design §6.1).
- **Dependencies (Tech Spec §12).** Add `base64` + `imagesize` to
  `emberly-tools` (pure-Rust, `cargo vet` before merge); `imagesize` over the
  full `image` crate deliberately — format + dimensions only, no codec tree.

**Satisfies:** P-11, T-12; Tech Spec §4.1/§4.2, §5.2, §12; Design §4.8, §6.1;
HC-2, HC-6.

**Done when:** an image round-trip asserts `read_image` root-confines its path,
rejects oversize/`.git/` paths, appends a `ContentBlock::Image`, and — on a
`vision:false` model — returns the structured unsupported-capability result
instead of sending; both adapter mappings are exercised via `FakeProvider`
(Tech Spec §14.7, P-11/T-12).

---

## Phase 3 — Persistent memory (FR-6, T-13)

**Goal:** Durable facts that survive across sessions — an index auto-loaded into
context each session, entries written by the model through a schema-constrained
tool that never widens HC-4.

**Scope**
- **Two-scope store, one shape (Tech Spec §8.1).** User-global
  `~/.config/emberly/memory/` (XDG) and project `.agents/memory/`, each a
  directory of `<slug>.md` entry files (TOML frontmatter `name`, `description`,
  `type` + markdown body) plus a `MEMORY.md` one-line-per-entry index. TOML
  frontmatter reuses the existing `toml` crate + first-party frontmatter split —
  no YAML dependency (HC-2).
- **Schema-constrained `memory` tool (Tech Spec §5.2, HC-4 boundary).** Args are
  **content fields, never a path**: `{op: write|update|remove|recall, scope,
  name, description?, type?, body?}`. The engine derives the filename from
  `name` (slugified) within the fixed scope directory and **rejects `..`,
  absolute paths, and separators in `name`**. The engine performs the write —
  harness-managed persistence like the transcript/trust store, so it does not
  widen HC-4 (FR-6 honesty clause). Emits `MemoryStatus{user, project}` (§3.1).
- **Auto-load the index, bodies on demand (Tech Spec §7, §8.1).** Both indexes
  load into pinned context at session start; `recall` fetches a specific entry
  body (progressive disclosure). **Project memory loads only under a trusted
  root** (FR-1, Tech Spec §6.7); user-global always loads.
- **User-inspectable, never hidden state (Design §4.9, Requirements FR-6).**
  Memory files are plain text the user reads/edits/deletes; the sidebar Memory
  inspector (count from `MemoryStatus`, editable entries via the Design §4.6
  in-app path) is the in-TUI surface. Memory/recall tool lines show
  `user`/`project` origin (Design §4.9).
- **Config (Tech Spec §8).** `memory.enabled` (**default `true`**);
  `memory.max_index_entries` (soft warn threshold for index growth — M8 open
  item).

**Satisfies:** FR-6, T-13; Tech Spec §8.1, §5.2, §7, §6.7, §3.1; Design §4.9,
§3.1, §8.4; HC-4 (unwidened), FR-1.

**Done when:** a write/recall round-trip works; a path-escape attempt in `name`
(`..`, absolute, separators) is rejected (the HC-4 boundary made a test); the
index loads as pinned context; and project-scope memory does **not** load under
an untrusted root (Tech Spec §14.7, FR-6/§6.7).

---

## Phase 4 — Skill system (FR-7, T-15)

**Goal:** Named, progressively-disclosed capability folders the model discovers
and invokes on demand — reusing Phase 3's frontmatter / pinned-catalog /
trust-gating machinery.

**Scope**
- **Skill folders, two scopes (Tech Spec §8.2).** A skill is `<name>/` with
  `SKILL.md` (TOML frontmatter `name`, `description` + markdown instruction
  body) and optional bundled resources/scripts. User-global
  `~/.config/emberly/skills/<name>/` and project `.agents/skills/<name>/` (the
  latter loaded only under a trusted root, FR-1, Tech Spec §6.7).
- **Discovery, catalog, precedence (Tech Spec §8.2, §7).** Scan both locations
  at startup; build the **skill catalog** (`SkillMeta{name, description,
  origin}` per skill) loaded into pinned context so the model always knows what
  it can invoke. On a name collision **project overrides user-global** (mirrors
  config precedence), and the shadow is surfaced via `emberly config show`
  (resolves the Requirements §13 / Tech Spec §16 precedence open item — project
  wins, surfaced). Emit `SkillsAvailable{skills}` (§3.1).
- **`skill` invocation tool (Tech Spec §5.2).** Args: skill name. Loads the
  named skill's `SKILL.md` body (and lists bundled resource paths) into the tool
  result — progressive disclosure: only metadata is standing context, the body
  loads on invoke. The tool **executes nothing**.
- **Scripts under the normal safety model (FR-7 honesty clause, Tech Spec
  §8.2).** A skill's script runs only via an ordinary, permission- and
  sandbox-gated `bash` call (§6) — no privileged path. A project skill from an
  untrusted root is neither cataloged nor invocable.
- **Sidebar Skills section (Design §4.9, §3.1).** Lists available skills by
  name, description, and origin; selecting one shows the `SKILL.md` body
  read-only, so "what could this skill tell the model to do" is inspectable
  before it ever runs. Skill invocation renders as quiet tool activity with
  origin on the line.
- **Config (Tech Spec §8).** `skills.enabled` (**default `true`**).

**Satisfies:** FR-7, T-15; Tech Spec §8.2, §5.2, §7, §6.7, §3.1; Design §4.9,
§3.1; FR-1.

**Done when:** discovery builds the catalog; `skill` loads only the body on
invoke (metadata standing); project-over-user precedence resolves with a shadow
notice; an untrusted-root project skill is absent from the catalog; and a
bundled script runs only through the ordinary permission-gated `bash` path (Tech
Spec §14.7, FR-7/§6.7).

---

## Phase 5 — Web search (T-14)

**Goal:** Give the agent live web reach through a harness-owned, configurable,
provider-agnostic search backend — permission-gated, its results treated as
untrusted content.

**Scope**
- **Harness-owned backend (Tech Spec §5.5).** The `web_search` tool talks to a
  configured search service through a thin first-party `reqwest` client (added
  to `emberly-tools` — a new crate→crate edge, not a new external crate; same
  no-default-features + `rustls-tls`/`json`/`stream` set). `[search]` profile:
  `adapter` (response-shape parser: `brave`/`tavily`/`searxng`/`json`),
  `endpoint`, `auth` (bearer/header/query, `key` a reference from env/`keys.toml`
  — never inline), `max_results` (**default 5**). A new search service is a
  profile; only a genuinely new response shape is a new parser (mirrors the §4.5
  provider-profile pattern, P-8).
- **Network egress is harness-process, not a sandboxed child (Tech Spec §5.5,
  §2).** First-party HTTP like a provider call; §6.2/§6.3 child confinement does
  not apply, and the only endpoint reached is the configured one. This is
  precisely why it is governed by the **permission layer** instead.
- **Permission-gated (Requirements T-14, Tech Spec §6.1).** Built-in rule
  `web_search → ask`, allowlistable to `allow` per-session or per-project like a
  bash command. `search.enabled = false` unregisters the tool entirely. The
  Design §5.2 prompt shows the query, the backend name (not the key), and the
  plain line "reaches the internet; results are untrusted" — the ordinary
  permission-prompt treatment, **never** the reserved outside-root safety band.
- **Untrusted results (Requirements T-14, Design §4.10).** Results
  (`{title, url, snippet}`) are tagged so the TUI renders them as fetched web
  content with visible source URLs — never harness or assistant voice; a
  hostile snippet reads visibly as quoted web text. Count capped by
  `max_results`; long snippets pass the §5.3 size backstop, so search cannot
  flood context.
- **Dependency (Tech Spec §12).** `reqwest` into `emberly-tools` (already
  vetted for Providers; confirm the edge in `cargo vet`).

**Satisfies:** T-14; Tech Spec §5.5, §6.1, §12, §8; Design §5.2, §4.10;
Requirements §1 (provider-agnostic), §6.

**Done when:** a profile pointed at a fake search endpoint proves search is
config-only and provider-agnostic; the `web_search → ask` gate fires and an
allowlist grant suppresses it; results are tagged untrusted and capped at
`max_results`; and results render in the untrusted-content style with source
URLs (Tech Spec §14.7, T-14).

---

## Phase 6 — TUI mouse support (Design §3.4)

**Goal:** Pointer interaction as additive convenience — scroll and click-select
across the surfaces the prior phases built — that never becomes the sole path to
a function and never weakens a decision. (Design-owned; no Requirements ID.)

**Scope**
- **Single capture control point (Tech Spec §9, Design §3.4).** `crossterm`
  `EnableMouseCapture` gated on `ui.mouse` (**default `true`**) and rich mode,
  via one control point (like the §6.4 animation ticker) so capture is **off**
  whenever `ui.mouse = false`, degraded mode, or `--plain`.
- **What the pointer does (Design §3.4).** Wheel/trackpad scrolls the focused
  pane or open overlay; a click is a shortcut for "focus + Enter" on interactive
  rows — sidebar entries (modified-files diff, Memory/Skills inspectors, Tasks),
  command-palette rows, model/effort pickers, a collapsed reasoning trail or
  task list to expand. Nothing more.
- **Native selection preserved (Design §3.4).** The terminal's own
  Shift-modified click-drag selection passes through unintercepted so native
  copy still works, documented in `/help`; `ui.mouse = false` releases the mouse
  entirely for users who want the terminal to own it.
- **The pointer never weakens a decision (Design §3.4, §5).** On a permission
  prompt a click may land on Deny/Allow exactly as a keypress would, but every
  §5 guarantee holds: no click "approves whatever is focused," no
  hover-to-approve, no click-through past unseen content, and clicking Allow is
  as deliberate as the approve key.
- **Degraded parity (Design §7).** Mouse capture off in `--plain` /
  `NO_COLOR` / `TERM=dumb` (line-oriented, append-only, no cursor
  repositioning); no feature depends on the pointer to carry meaning.

**Satisfies:** Design §3.4, §5, §7; Tech Spec §9, §8 (`ui.mouse`); Requirements
§2.1 (additive-never-exclusive).

**Done when:** a mouse unit test asserts capture is disabled under
`ui.mouse=false` and in degraded mode, that a wheel event scrolls the focused
pane, that a click selects an interactive row, and that a click never
synthesizes a permission approval (Tech Spec §14.7, Design §3.4/§5).

---

## Phase dependency summary

```
0.3 product (M7, shipped) on 0.2 (M6) on 0.1 (M1–M5)
   ├─> Phase 1  Task-list tool (T-11)            ── establishes engine-state-tool + sidebar-section pattern
   ├─> Phase 2  Multimodal image (P-11, T-12)    ── independent; touches the provider layer + adds deps
   ├─> Phase 3  Persistent memory (FR-6, T-13)   ── establishes progressive-disclosure store machinery
   │      └─> Phase 4  Skill system (FR-7, T-15) ── reuses Phase 3's frontmatter/catalog/trust/inspector machinery
   ├─> Phase 5  Web search (T-14)                ── independent; adds reqwest-into-tools + search profile
   └─> Phase 6  Mouse support (Design §3.4)      ── wires clicks to the surfaces Phases 1/3/4 add
```

The phases are sliced one per capability; each ships and tests on its own. Two
soft dependencies set the order:

- **Phase 4 wants Phase 3 first.** Skills reuse memory's progressive-disclosure
  machinery — TOML frontmatter parsing, a pinned index/catalog, trust-gating of
  the project scope, and the sidebar inspector pattern (Tech Spec §8.1 → §8.2).
  Building memory first means skills add discovery + precedence + the invoke
  tool, not the whole store apparatus again.
- **Phase 6 comes last.** Mouse click-select is most useful once the surfaces it
  targets exist — the Tasks section (Phase 1) and the Memory/Skills sidebar
  inspectors (Phases 3–4). It is mechanism-independent, but last lets it wire to
  everything.

**Phases 1, 2, and 5 are independent of each other and of 3/4** and could be
reordered or parallelized. The default order 1→2→3→4→5→6 builds shared machinery
before its reuse and defers the pure-UI phase to the end.

---

## Not in this plan

- **User-attached images** — deferred (Requirements §2.2); Phase 2 builds the
  P-11 content path so a future attach surface is a frontend affordance, not a
  provider/engine change. Not built here.
- **Provider-native / server-side tools** — web search is harness-owned by
  decision (Requirements §2.2, §1); a provider's server-side search is not used.
  The seam (T-7) is not reshaped.
- **Terminal image protocols** (sixel/kitty/iTerm) — Design §4.8/§10 notes them
  as a possible future nicety behind capability detection; never a default,
  never load-bearing. Not built here.
- **MCP, headless/IDE/server frontends, user theming, Windows** — remain
  deferred per Requirements §2.2; their seams stay unused, not reshaped (A-1,
  T-7).
- **Release-pipeline / prebuilt binaries** — 0.1 shipped compile-from-source
  with the pipeline deferred by owner; 0.2 and 0.3 did not revisit it and
  neither does 0.4. Install-from-source continues unless the owner reopens it.
- **Agent-quality evaluation** (does it code well) — out of scope per Tech
  Spec §14; post-release discipline with separate tooling.

---

## Open items (carried into the phases per G-11; resolvers named)

These are the Tech Spec §16 v0.8 open items the 0.4 phases resolve. Each is
owned by its phase; discoveries that change HOW flow back into the Technical
Specification as version bumps (G-24/G-25).

- **Search adapter & auth coverage (T-14, Phase 5).** `brave`/`tavily`/
  `searxng`/`json` are the initial response-shape parsers and bearer/header/query
  the initial auth set. Resolve: validate against the real services in Phase 5;
  add a parser only for a genuinely new shape (Tech Spec §16).
- **Image formats, caps, and adapter mapping (P-11/T-12, Phase 2).** PNG/JPEG/
  GIF/WebP and a 5 MiB cap are the initial set. Resolve: confirm each maps
  cleanly to the Anthropic `image` block and the OpenAI `image_url` `data:` URI
  against live endpoints, and that `imagesize` covers every accepted format's
  header (Tech Spec §16).
- **Memory index growth (FR-6, Phase 3).** `memory.max_index_entries` is a soft
  warn threshold. Resolve: determine the point at which a large always-pinned
  index itself wants the §7 economy (description truncation or an on-demand
  index tier) (Tech Spec §16, Requirements §13).
- **Skill precedence surfacing (FR-7, Phase 4).** Project-over-user precedence
  is resolved. Resolve: confirm the `config show` shadow notice is clear enough,
  and decide whether a shadowed user skill should ever be invocable by a
  qualified name (Tech Spec §16).
- **Mouse capture / native-selection passthrough (Design §3.4, Phase 6).**
  Shift-passthrough is terminal-dependent. Resolve: verify behavior across the
  target terminals and document those where `ui.mouse = false` is the only way
  to get native selection (Tech Spec §16).
