# Emberly Code — Design Guideline

**Version:** 0.3 
**Status:** approved 
**Date:** 2026-07-06
**Owner:** Wattanit
**Companion documents:** Requirements Document v0.1 (upstream), Technical
Specification (downstream — this document constrains it)

This document defines how Emberly Code looks, feels, and speaks. It is the
second of three project documents. Where a decision here has technical
consequences (event model, rendering, terminal capabilities), the Technical
Specification must absorb it.

---

## 1. Identity

### 1.1 Name

- **Product name:** Emberly Code — the coding-agent sibling of Emberly
  (the chat client), following the family-name convention.
- **Binary/command name:** `emberly`.
- The install docs suggest (but do not create) a short shell alias `emb`
  for heavy daily use. The harness itself never assumes the alias exists.
- Naming collision check (crates.io, Homebrew, common distro repos, PATH
  conventions) is a release-checklist item in the Tech Spec.
- **Wordmark:** plain styled text — `emberly` in the ember accent
  followed by `code` and the version in dimmed secondary text. No
  figlet/ASCII-art rendering, no logo glyph. Identical in degraded mode
  minus color.

### 1.2 Brand concept: "the ember"

Emberly's brand is the fireplace-side chat: warm, private, comfortable.
Emberly Code translates that to the terminal, where the metaphor is
literal — an ember is a point of warm light on a dark ground, which is
exactly what a terminal is.

Design translation, stated as principles:

- **Warm, not loud.** One warm accent (ember amber/copper range) carries
  identity. Everything else is quiet: dimmed grays for chrome and
  secondary text, high-contrast neutral for primary content.
- **Light, clean, useful.** Personality comes from restraint, spacing,
  and voice — not decoration. No ASCII-art splash, no gratuitous boxes,
  no emoji in chrome.
- **Calm under load.** The interface never gets busier when things go
  wrong. Errors and permission prompts are the calmest, clearest screens
  in the app.

### 1.3 Anti-goals

- The sterile hacker-terminal aesthetic (pure function, zero warmth).
- The over-decorated TUI (heavy box-drawing everywhere, rainbow syntax
  chrome, animation as noise). Motion in small, purposeful doses is
  welcome — see §6.4.
- Any personality expression that costs clarity or a keystroke.

## 2. Color and Typography

- **Palette roles and candidate values.** These are working values —
  expected to be tuned by eye once the real interface exists. All colors
  live in a single theme definition in code (one file, named roles), so
  tuning is a value change, not a refactor.
  - *Background reference* — `#16120F` warm near-black (the terminal's
    own background wins where not drawn over; this value guides drawn
    surfaces). Raised surface (prompts, overlays): `#221C17`.
  - *Ember accent* — `#E8833A`. Dim accent (focus borders, subtle
    marks): `#9C5A2B`. Used for: the wordmark, the active/focus
    indicator, the spinner, key highlights. Used sparingly; if more than
    ~5% of a screen is accent-colored, something is wrong.
  - *Primary text* — `#EDE6DB` warm off-white.
  - *Secondary/chrome* — `#857C6F` dimmed warm gray. Labels, dividers,
    hints, metadata.
  - *Semantic colors* — success/allow `#8FB573`, error/deny `#D96C5F`,
    warning/caution `#DBA94D`, used only for their semantic meaning,
    never decoratively. Diff colors follow universal convention (green
    additions, red deletions) using the success/error values.
- **One theme in v1.** User theming is out of scope for v1; the single
  built-in warm-dark theme ships, with a light-terminal legibility
  fallback. The centralized theme definition is the future-proofing —
  a theme system later means loading a different set of values into the
  same roles, nothing more.
- **Safety styling is reserved.** The visual treatment used for
  outside-project-root permission prompts (see §5) is used for that and
  nothing else. It must never be diluted by reuse.
- **Typography** is whatever monospace the user's terminal provides.
  Hierarchy is achieved through spacing, dimming, and weight (bold), never
  through size (unavailable) or color alone (degradation, §7).
- Box-drawing characters are used minimally: section dividers and the
  sidebar separator. Content is never trapped inside full boxes.

## 3. Layout

### 3.1 Two-pane structure

A main conversation pane and a right sidebar, in the spirit of the best
current harnesses:

- **Main pane** — the conversation: user prompts, assistant text, tool
  activity, diffs, permission prompts. This pane owns scrollback.
- **Right sidebar** (fixed width, collapsible with a keybinding):
  1. Wordmark + version — one line, small. The brand is present, not
     dominant.
  2. Session title (auto-generated from the task, user-renamable).
  3. Project root path (abbreviated with `~`).
  4. Model block: provider + model name, context usage percentage
     (Requirements §8.4), session cost estimate, and sandbox status
     (Requirements §6.7) — one dimmed line when fully confined, warning
     styling with a short reason when degraded. Sandbox status is
     always-visible state, like context percentage, never a silent
     assumption.
  5. **Modified files** — every file the agent has created or changed
     this session, with add/remove line counts. Each entry is
     selectable → opens the cumulative diff for inspection (§4.2).
  6. Extension sections — MCP, LSP, Skills — rendered only when the
     underlying subsystem exists and has content. In v1 these are absent,
     not shown as empty "None" stubs. The layout reserves the pattern,
     not the pixels.
- **Status line** (bottom, one line): current mode (normal /
  auto-accept-edits / auto), context %, and 3–5 contextual keybinding
  hints in dimmed text. Hints change with state (e.g. during a permission
  prompt they show the prompt's keys).

### 3.2 Narrow terminals

Below a width threshold, the sidebar auto-collapses and its critical
information (context %, mode) migrates to the status line. Everything the
sidebar shows must also be reachable via commands (e.g. `/files`,
`/session`) so no information is sidebar-exclusive.

### 3.3 Command palette

A fuzzy-searchable command palette (Ctrl+P convention) listing every
command with its keybinding and a one-line description. All functionality
is reachable three ways: palette, `/command`, or keybinding — the palette
is the discovery mechanism, slash-commands are the muscle-memory
mechanism. New-user help text teaches the palette first.

## 4. Rendering Policy

### 4.1 Markdown subset

Assistant output renders a small, boring subset well:

- Fenced code blocks with syntax highlighting (accent-free scheme;
  highlighting uses the neutral/semantic range so code never competes
  with the ember accent).
- Bold, inline code, bulleted/numbered lists, headings as bold + spacing.
- Everything else (tables, images, links, nested exotica) passes through
  as plain text, unmangled.

### 4.2 Diffs are first-class

Unified diffs, colored, with file path header and line numbers. Shown:
inline in the main pane when an edit executes (and inside edit permission
prompts, §5), and on demand from the sidebar's modified-files list as a
per-file cumulative diff for the session, opened as a **pane overlay**
(scrollable, Esc to dismiss) rather than an `$EDITOR` handoff — staying
in the TUI keeps review friction near zero.

### 4.3 Full-fidelity escape hatch

Any assistant message can be opened in the user's `$EDITOR` as a
read-only temporary file containing the raw markdown (`/view`, also on
the palette). This satisfies "I want the full markdown" without building
a terminal markdown browser. Fallback order: `$VISUAL` → `$EDITOR` →
print raw to the pane with a notice. The same mechanism is reused for
inspecting full untruncated tool outputs referenced by truncation markers
(Requirements §8.1).

## 5. The Permission Prompt

The most important screen in the product. It is where the safety model
meets human attention, and it is designed so that **saying yes always
requires having seen what you are saying yes to.**

- **Full content, always.** The complete command, or the complete diff,
  and all affected paths. Never truncated to fit — long content scrolls
  within the prompt. If the user hasn't scrolled to the end of a long
  diff, the approve hint indicates there is more below.
- **Escalation is visually loud.** A prompt for anything touching outside
  the project root (Requirements HC-4) uses the reserved safety styling
  (§2): distinct color band and an explicit plain-language line — "This
  affects files OUTSIDE your project." It must be impossible to mistake
  for a routine prompt at a glance.
- **Choices:** Deny (safe default) · Allow once · Allow for this session
  (where the rule layer permits persistence, per Requirements §6.6).
  The default keypress (Enter/Esc behavior) maps to *deny*. Approval is
  always a deliberate, distinct key.
- **Forbidden patterns**, written down so they stay forbidden:
  - No timeout-to-approve, ever.
  - No "Enter approves whatever is focused."
  - No batching multiple distinct actions under one approval.
  - No auto-scroll that moves content out from under the user's eyes
    while a prompt is open.
- Every prompt shows *why* it appeared (which rule matched, or "outside
  project root") in one dimmed line — this teaches the permission model
  in situ.

## 6. Voice and Language

### 6.1 Error voice

Every error answers, in order: **what happened → why → what you can do
next.** Calm, specific, no exclamation marks, no blame, no cutesy
apology. One error, one message; never a stack trace as the primary
surface (available via a verbose flag / log file).

Two failure registers, visually and verbally distinct because the user's
correct response differs:

- **Agent-world failures** (tool errors, edit mismatch, command failed):
  rendered as normal tool-result content in the conversation flow — the
  *model* is expected to react, and the user watches it recover.
- **Harness-world failures** (network, provider errors, bugs): rendered
  in the harness's own voice in a clearly-out-of-band style, with the
  next step ("retrying in 5s", "session saved; run `emberly resume`").

### 6.2 General voice

- Second person, present tense, plain words. "Saved session to …" not
  "Session persistence completed successfully."
- Warmth lives in helpfulness and tone, not exclamation points. The
  fireplace feeling is *calm*, and calm text is short text.
- Hints and chrome are terse (2–5 words). Explanations live in `/help`
  and docs, not in the interface chrome.
- **Interface text is English in v1; Thai text is fully supported as
  content.** The user must be able to type Thai in the input box and
  read Thai anywhere it appears — messages, model output, file content,
  diffs, session titles — with correct rendering, wrapping, and cursor
  movement. The technical caveat for the Tech Spec: Thai text layout in
  terminals needs real grapheme-cluster width handling (combining
  vowel/tone marks are zero-width); the layout and input engines must
  not assume one-char-one-column. Interface strings are still kept
  centralized rather than scattered as literals — cheap discipline that
  leaves the door open to interface localization later without
  committing to it now.

### 6.3 Loading and progress

A single small spinner in the ember accent with a dimmed verb phrase
("thinking", "running tests", "reading files"). Verb phrases are dull and
truthful — they describe what is happening, they do not perform
personality. Elapsed time appears after 5 seconds.

### 6.4 Motion

Small amounts of motion keep the workspace feeling alive rather than
like a dark, cold terminal — the ember should visibly glow. The policy
is *few, small, purposeful*:

- **Sanctioned motion:** the spinner (an ember-like pulse rather than a
  generic line spinner is on-brand and costs nothing); a subtle
  brightness pulse on the accent while the model is streaming; brief
  ease-in of overlays (one or two frames of expansion, not a slide
  show); a short settle animation when a modified-file entry lands in
  the sidebar.
- **Rules:** motion is ambient status, never information — nothing is
  communicated *only* by animation. Nothing moves on a screen where the
  user is being asked to decide (permission prompts are perfectly
  still). No looping animation on an idle screen except the prompt
  cursor. Frame budget is trivial (a TUI animating at 10–15fps in a
  handful of cells); animation must never delay input handling or
  streaming output.
- **Off switch:** all motion disabled in degraded mode (§7) and by a
  `motion = false` config key.

The test for any proposed animation: if it were removed, the app loses a
little warmth but zero information. If removing it loses information,
it's misdesigned; if it adds more than a little warmth, it's probably
too much.

## 7. Capability Degradation

The harness was born from a bad experience on a non-standard stack; the
interface honors the same spirit. Degradation is a designed mode, not an
accident:

- `NO_COLOR` and `TERM=dumb` are respected. In degraded mode: no color,
  no box-drawing, no spinner (plain "working…" lines), ASCII-only
  markers. Diff +/- prefixes and the words ALLOW/DENY carry the meaning
  that color otherwise reinforces — **color and unicode are enhancement,
  never the sole carrier of meaning**, everywhere, always.
- The permission prompt in degraded mode keeps its guarantees: full
  content, explicit OUTSIDE-PROJECT-ROOT banner in capital letters,
  deliberate approve key.
- A `--plain` flag forces degraded mode for weird environments, piping,
  and screen readers; degraded mode output is line-oriented and
  append-only (no cursor repositioning), which is also what makes a
  future headless mode trivial.
- Degraded mode is a supported, tested configuration, not a best-effort
  fallback.

## 8. Key Moments

### 8.1 First run and `init`

- First run without config simply works (baked-in defaults, Requirements
  C-1) and prints a two-line orientation: where it's running, and that
  `emberly init` materializes the configuration for inspection.
- `emberly init` prints exactly: what it created, where, and the one
  next command worth knowing. No walls of text. The init experience is
  the first impression and should feel like a considerate colleague, not
  an installer wizard.

### 8.2 Session start

One compact header: wordmark + version, model, project root, sandbox
status, and — when config overrides are active — one dimmed provenance
line per overridden piece (Requirements C-3). Silence about defaults;
speech about deviations. If the sandbox is unavailable or partial, a
one-time plain-language notice explains what that means and what was
tightened (Requirements §6.7) — calm warning styling, not alarm.

### 8.3 Session end / crash

On clean exit: one line — session name, duration, cost, transcript path.
After abnormal exit (Requirements HC-3/S-2), the next launch in that
project offers resume: "Found an interrupted session from 14:02 —
resume? (y/N)". Never auto-resume.

## 9. Design-Driven Requirements Feedback

Decisions in this document that add to or refine the Requirements doc,
recorded so the trace is explicit:

- **Cost estimation** (sidebar model block, §3.1) — extends Requirements
  P-6: per-provider token accounting must also support session cost
  estimates from configurable per-model pricing. Estimates are labeled as
  estimates.
- **Session titles** (§3.1) and rename — a small addition to session
  persistence metadata (Requirements §8.2).
- **Full-output inspection via `$EDITOR`** (§4.3) — realizes the
  "reference where the full output lives" requirement (Requirements
  §8.1) as a concrete UX mechanism.
- **`--plain` / line-oriented degraded mode** (§7) — is the de facto
  contract for the future headless frontend (Requirements A-1).
- **Thai text support (content, not chrome)** (§6.2) — a v1 scope item
  for the Requirements doc: Thai input and display everywhere content
  appears, with grapheme-cluster-aware text layout. Interface
  localization is not a v1 requirement.
- **Single built-in theme, theming deferred** (§2) — a new explicit
  deferral for the Requirements doc's §2.2.

## 10. Open Questions

- Final palette values — candidates are set in §2; tuning happens by eye
  against the built interface. Candidate worth exploring then: harmonize
  with Emberly-the-app's fireplace palette so the family reads as
  related on a shared screen.
- Session-title auto-generation approach (model-generated from the first
  task vs. heuristic; Tech Spec).
