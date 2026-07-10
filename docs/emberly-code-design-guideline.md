# Emberly Code — Design Guideline

**Version:** 0.5 
**Status:** approved 
**Date:** 2026-07-10
**Owner:** Wattanit
**Companion documents:** Requirements Document v0.5 (upstream), Technical
Specification v0.5 (downstream — this document constrains it)

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
- **One theme.** User theming is out of current scope; the single
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
  4. Model block: provider + model name, the reasoning-effort level when
     the model exposes one (Requirements P-9), context usage percentage
     (Requirements §8.4), session cost estimate, and sandbox status
     (Requirements §6.7) — one dimmed line when fully confined, warning
     styling with a short reason when degraded. Sandbox status is
     always-visible state, like context percentage, never a silent
     assumption. The provider/model line and the effort line are
     selectable → each opens a picker (Requirements C-6): the model picker
     lists the configured provider profiles (Requirements P-8); the effort
     picker lists the levels the active model offers. A switch applies to
     the next turn and is announced in the conversation in the harness's
     own voice — never silently.
  5. **Modified files** — every file the agent has created or changed
     this session, with add/remove line counts. Each entry is
     selectable → opens the cumulative diff for inspection (§4.2).
  6. Extension sections — MCP, LSP, Skills — rendered only when the
     underlying subsystem exists and has content. Today these are absent,
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

### 4.4 The reasoning trail

When a provider streams the model's reasoning distinctly from its answer
(Requirements P-10), it renders as its own quiet register, never as the
answer:

- **Collapsed by default**, shown as a single dimmed line — "reasoning
  (12 lines)" — with an expand affordance. The answer is what the user
  came for; the reasoning is available, not imposed. Expanded, it renders
  in secondary/chrome color (§2), one visual step below assistant text, so
  a glance always distinguishes reasoning from conclusion.
- **Live while streaming:** the dimmed reasoning may stream in place so the
  workspace feels alive during long thinks (this is where the accent
  streaming glow, §6.4, lives); when the answer begins, the trail settles
  to its collapsed line unless the user pinned it open.
- A `reasoning = collapsed | expanded | hidden` config key sets the
  default view; **the default is `collapsed`** — reasoning is available at a
  glance's cost, never imposed and never hidden by surprise. `hidden` still
  records the trace to the transcript (Requirements P-10) — hidden is a view
  choice, never a discard.
- Degraded mode (§7): the trail is a plain labeled block
  (`--- reasoning ---`), never color-only, and defaults to collapsed via a
  one-line marker the user can `/view`.

### 4.5 The tool-call explanation line

The model-authored explanation (Requirements T-9) renders as a single
dimmed line directly under the tool call it explains — the call stays the
headline, the explanation is the caption. It appears only when the model
supplied one (non-obvious calls); its absence is normal and never shows a
placeholder. It is never styled as a result or an error, and it never
carries meaning by color alone (§7). **On by default**; a config key
(`ui.tool_explanations`) defeats it entirely for users who do not want the
tokens spent (Requirements T-9).

### 4.6 Editing config and prompts in place

In-app editing (Requirements C-5) offers two paths, chosen by the size of
the edit, both reachable from the command palette (§3.3):

- **Quick edit — a TUI overlay.** A focused, scrollable overlay (the §4.2
  overlay pattern, made editable) for a single config value or a short
  prompt. Before an edit, the overlay shows the value's provenance tier
  (Requirements C-3) in a dimmed line — you always see whether you are
  about to override a baked-in default or an existing project value.
  Saving writes to the project tier (Requirements C-1), never to the
  baked-in defaults, and shows the written path.
- **Full edit — `$EDITOR` handoff.** For a whole prompt file or the full
  config, hand off to `$VISUAL`/`$EDITOR` (the §4.3 fallback order),
  reloading on save. This reuses the user's real editor rather than
  growing a text editor inside the TUI.

Either way, a change that cannot take effect until restart is named as such
at the moment of saving — silence about live changes, speech about the ones
that need a restart.

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

### 5.1 The question prompt (the model asking your opinion)

The ask-user tool (Requirements T-8) produces a prompt that looks and
behaves unlike a permission prompt, because the two ask opposite things: a
permission prompt guards you against an action and defaults to *deny*; a
question prompt invites your input and has no dangerous default.

- **Its own quiet styling — never the reserved safety treatment (§2).**
  Reusing the outside-project-root band here would dilute the one signal
  that must stay rare and loud. The question prompt is calm and neutral: a
  clear question, the model's options as a selectable list when it offered
  them, and a free-text answer always available.
- **No unsafe default.** Unlike the permission prompt, there is no
  "safe default" keypress that answers for you — the model asked because it
  genuinely needs *your* choice, so the prompt waits. It is still
  dismissible (Esc returns "user declined to answer" to the model as a
  structured result, so the model can proceed or stop), but Enter never
  auto-selects an option on the user's behalf.
- **No motion on this screen** (§6.4): like the permission prompt, nothing
  animates while you are being asked to decide.

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
- **Interface text is English in current scope; Thai text is fully supported as
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

### 8.4 Trusting a folder

The workspace-trust gate (Requirements FR-1) is the first thing the user
sees in a folder Emberly has not been trusted in before — it appears
*before* the session starts, before any project file is read into a prompt.

- **Calm, plain, and honest about what it is.** It names the folder, states
  plainly what agreeing means ("Emberly will read, edit, and run commands
  in this folder"), and asks whether the user trusts this code — with a
  one-line nudge to review unfamiliar folders first. It is not styled as an
  alarm; it is a considered question, in keeping with §1.2's "calmest
  screens in the app."
- **The safe default is decline.** The default keypress does not grant
  trust; trusting is the deliberate choice. Declining does not start the
  session (Requirements FR-1) — Emberly says so in one line and exits
  cleanly, never half-starting in a crippled state.
- **It is not the permission prompt and not the reserved safety band
  (§2).** Trust is a once-per-folder gate on *whether* Emberly runs here;
  it never stands in for the per-action prompts that govern *what* it does
  (Requirements FR-1 honesty clause). The prompt says as much in a dimmed
  line: trusting the folder does not switch off later prompts.
- **Asked once per trusted subtree.** Trusting a folder trusts its
  subdirectories too, so the gate does not reappear as the user moves
  within a project they already trusted (Requirements FR-1) — the prompt is
  a rare, considered moment, not a recurring toll.
- Degraded mode (§7): the same content, ASCII-framed, capitalized
  TRUST / DON'T TRUST choices, deliberate key to trust.

### 8.5 When the loop is broken

When the guardrail halts a non-progressing loop (Requirements S-5), the
halt is a harness-world moment (§6.1), rendered in the harness's own
out-of-band voice — not as model output, because the model is precisely
what is not making progress:

- One calm line of what happened and why — "Stopped: the last few steps
  repeated without progress." — then the choices: keep going (resume the
  loop), stop here, or say something (hand a steer back to the model). No
  blame, no alarm styling.
- The user is always the one who decides what happens next; the guardrail
  never quietly resumes or quietly abandons the task. This is the visible
  counterpart to S-5's promise that a runaway loop ends in a user decision.
- It is distinct from the question prompt (§5.1): that is the *model*
  choosing to ask; this is the *harness* stepping in when the model did
  not.

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
- **Thai text support (content, not chrome)** (§6.2) — a scope item
  for the Requirements doc: Thai input and display everywhere content
  appears, with grapheme-cluster-aware text layout. Interface
  localization is not a requirement in current scope.
- **Single built-in theme, theming deferred** (§2) — a new explicit
  deferral for the Requirements doc's §2.2.
- **Reasoning-trail view key `reasoning = collapsed|expanded|hidden`**
  (§4.4) — refines Requirements P-10: `hidden` is a view choice only; the
  trace is still recorded to the transcript. The config key is a Design
  addition the Tech Spec absorbs.
- **Model/effort switch is announced, never silent** (§3.1) — refines
  Requirements C-6: a switch is surfaced in the conversation, consistent
  with "silence about defaults, speech about deviations."
- **Question prompt has no unsafe default** (§5.1) — refines Requirements
  T-8: Esc returns a structured "declined to answer"; Enter never
  auto-answers.
- **Trust gate defaults to decline and exits cleanly on decline** (§8.4) —
  realizes Requirements FR-1 as concrete UX, and reinforces its honesty
  clause (trust ≠ waiver of later prompts) in the prompt text itself.
- **Loop-break offers keep-going / stop / steer** (§8.5) — realizes
  Requirements S-5's "ends in a user decision" as three concrete choices in
  the harness voice.

## 10. Open Questions

- Final palette values — candidates are set in §2; tuning happens by eye
  against the built interface. Candidate worth exploring then: harmonize
  with Emberly-the-app's fireplace palette so the family reads as
  related on a shared screen.
- Session-title auto-generation approach (model-generated from the first
  task vs. heuristic; Tech Spec).
- Whether the model/effort picker and the config/prompt editor get
  dedicated keybindings beyond palette + `/command` reachability (§3.1,
  §4.6). Tune with use once the surfaces exist.

Resolved since v0.4: reasoning-trail default view — `collapsed` (§4.4,
owner); tool-call explanation line — on by default, config-defeatable
(§4.5, owner).
