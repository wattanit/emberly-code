# Emberly Code — Design Guideline

**Version:** 0.12 
**Status:** approved
**Date:** 2026-08-16
**Owner:** Wattanit
**Companion documents:** Requirements Document v0.12 (upstream), Technical
Specification v0.14 (downstream — this document constrains it)

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
  - *Secondary/chrome* — `#9D968C` dimmed warm gray. Labels, dividers,
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
  6. **Tasks** (Requirements T-11) — the model's current task list when one
    exists: each item on its own line with a status glyph (§4.7), the
     in-progress item lightly accented. Absent when the model has not opened
     a list; never an empty stub.
  7. Extension sections — **Memory**, **Skills**, **Agents**, MCP, LSP — each
    rendered only when its subsystem exists and has content. Memory
     (Requirements FR-6) and Skills (Requirements FR-7) became real in the 0.4
     feature set: Memory shows a count and opens an entry inspector (§4.9);
     Skills lists the available skills by name and origin (§4.9). **Agents**
     (Requirements FR-9) becomes real in this version: it lists currently
     alive subagents by name and status, absent entirely when none are alive
     — never an empty "Agents (0)" stub — and each entry opens a per-agent
     inspector (§4.13). MCP and LSP remain absent, not shown as empty "None"
     stubs. The layout reserves the pattern, not the pixels.
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

### 3.4 Pointer (mouse) interaction

Mouse support (Requirements §2.1) is **additive convenience, never a new
authority**. The governing rule: **anything the mouse can do, the keyboard can
already do, and the mouse can do nothing the keyboard cannot.** The pointer
speeds up navigation; it never becomes the sole path to a function (the §3.3
"reachable three ways" principle extends to a fourth, optional way) and it
never lowers the bar on a decision.

- **What the pointer does:** wheel/trackpad scrolls the focused pane or open
overlay; a click selects — a sidebar entry (opening its diff or inspector,
§4.2/§4.9), a command-palette row, a model/effort picker row, a collapsed
reasoning trail or task list to expand it (§4.4, §4.7). Clicking is a
shortcut for "focus + Enter," nothing more.
- **Text selection is preserved.** Capturing the mouse for the above would
otherwise steal the terminal's own click-drag-to-copy — a real loss in a
tool people read constantly. Emberly keeps native selection reachable: hold
the terminal's selection modifier (Shift in most terminals) to drag-select
and copy as usual, and this is stated in `/help`. Users who want their
terminal's selection unconditionally can set `mouse = false` (below), which
releases the mouse entirely.
- **The pointer never weakens a decision.** On a permission prompt (§5), a
click may land on Deny/Allow exactly as a keypress would, but every §5
guarantee holds unchanged: no click "approves whatever is focused," no
hover-to-approve, no click-through past unseen content (the approve
affordance still indicates unscrolled content below), and clicking Allow is
as deliberate an act as pressing the approve key. The mouse buys speed on
the safe paths and buys *nothing* on the dangerous one.
- **Off switch and degradation:** `mouse = false` disables capture for users
who prefer their terminal's native pointer behavior. Degraded mode (§7) —
`--plain`, `NO_COLOR`, `TERM=dumb` — turns mouse capture **off**: that mode
is line-oriented and append-only with no cursor repositioning (§7), so
seizing the mouse there would only break the terminal's own selection for no
gain. Mouse is thus an enhancement of the rich TUI, consistent with color
and motion (§2, §6.4) as things that add comfort and never carry meaning.

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
inspecting full tool outputs referenced by a reduction marker — both the
size truncation of Requirements §8.1 and the salient reduction of
Requirements FR-2 (§8.5). A reduced or truncated tool result is never a dead
end: its marker names what was withheld and offers `/view` to the complete
output, so "kept the meat in context" never reads as "lost the rest."

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

In-app editing (Requirements C-5) offers two paths, both reachable from the
command palette (§3.3):

- **`$EDITOR` handoff.** `/config` and `/prompt` hand off to `$VISUAL`/
`$EDITOR` (the §4.3 fallback order) on the whole file — the project
config or a prompt file, seeded from the baked-in default if it doesn't
exist yet — reloading on save. This reuses the user's real editor rather
than growing a text editor inside the TUI. In degraded mode (§7), where a
multi-screen editor handoff isn't available, the command instead prints
the file's path (creating it first if needed) and waits for the user to
edit it externally and run `/reload`.
- **Guided setup — a step-by-step wizard (Requirements C-7).** A focused
sequence of single-question screens — profile name, adapter, endpoint,
model id, then the API key — Enter to advance, Esc/Back to step back. The
profile name comes first because it is the identifier the user is actually
choosing (e.g. "deepseek"), distinct from the adapter that follows: the
adapter step is the wire format, not the provider's brand, and reads as
"which wire format does it speak? — most third-party and OpenAI-compatible
APIs, including local models, speak `openai`" so a profile named "deepseek"
picking adapter `openai` doesn't read as a contradiction. Reachable as a
trailing "Add new provider…" row at the bottom of the existing
model/provider picker (§3.1, Requirements C-6), not a new top-level
command: the picker a user already opens to switch models is where
they'd also think to add one. A summary screen (profile name, adapter,
endpoint, and the key redacted to its last 4 characters) precedes the
write — the same "shows the written path" honesty as the `$EDITOR` path,
before the point of no return, not after. On completion the session reports
the change exactly as a `/config` reload would — no separate "wizard
complete" voice; one reload story for every path onto the same
configuration. The typed key is masked character-by-character as it's
entered (`•` per keystroke) and never echoed in full again anywhere in
the interface, matching C-7's never-printed guarantee.

Guided setup is not available in degraded mode (§7): a multi-screen
wizard needs cursor repositioning that plain mode does not have, so
degraded mode falls back to the same print-the-path-and-`/reload` pattern
`/config`/`/prompt` already use there — not an `$EDITOR` handoff, which
degraded mode never offers either way.

Whichever path, a change that cannot take effect until restart is named as
such at the moment of saving — silence about live changes, speech about the
ones that need a restart.

### 4.7 The task list

The model's task list (Requirements T-11) renders in two calm places, never as
a spectacle:

- **Inline**, when the model creates or updates it: a compact checklist block
in the conversation flow — one line per item, prefixed by a status glyph.
It appears where the model wrote it, so the reader sees the plan take shape
in context, then scrolls past it like any other turn.
- **Persistently**, in the sidebar Tasks section (§3.1) so "what's left" is
answerable at a glance without scrolling back.
- **Status glyphs carry meaning without color:** pending `○`, in-progress
`◐`, done `✓`, with ASCII fallbacks `[ ]` / `[~]` / `[x]` in degraded mode
(§7) — color is reinforcement, never the sole signal. The single
in-progress item is lightly ember-accented; it is the one place a task list
earns a touch of warmth, and only one item is ever in progress at a glance.
- It is **model-authored**: Emberly renders it faithfully and never
editorializes, reorders, or checks items off on the model's behalf (T-11).
A completed list settles to a quiet all-`✓` block rather than vanishing —
the record of what was done is part of the conversation.

### 4.8 Images in a terminal

The terminal cannot be relied on to paint pixels, and Emberly does not try:
there is **no sixel/kitty/iTerm image rendering in current scope** (a
compatibility rabbit hole that degrades badly across terminals, against §7's
spirit). A read-image call (Requirements T-12) renders instead as a labeled
reference line in tool-activity styling — `read image  mockup.png · 1200×800 · PNG` — because the value is the *model* seeing the image (P-11), not the user
re-seeing a file they already have. The honesty is explicit: the line states
that the image went to the model, not to the screen.

- If the active model has no vision (Requirements P-11), the
unsupported-capability result renders as a calm tool-result note — "this
model can't read images; switch model (§3.1) or describe it" — never a
harness error (§6.1), so the model and user both learn the image was not
seen rather than assuming it was.
- Degraded mode (§7): the same reference line, ASCII-only.
- Terminal image protocols are noted as a possible future nicety (§10), gated
on capability detection, never a default.

### 4.9 Memory and skills in the flow

Memory (Requirements FR-6) and skills (Requirements FR-7) surface the same way
the `recall` tool does (§8.6): as **quiet, ordinary tool activity**, one dim
line, never ceremony.

- A memory write/update reads like `remembered · project · "uses pnpm, not npm"`; a recall like `recalled 2 memories`. A skill invocation reads like
`skill · pdf-fill · user` — name and origin on the line. Each may carry a
§4.5 explanation when the model supplied one. None of these is a harness-voice
moment; they are the model using its own faculties, shown without weight.
- **Origin is always on the line** — `user` vs `project` for memory, and for
skills — because origin is how the user reads trust (Requirements FR-6/FR-7):
a project-supplied skill or memory is exactly the kind of thing to notice.
- **Inspectors, not black boxes.** The sidebar Memory section opens an overlay
(§4.2 pattern) listing entries grouped by scope, each editable/deletable via
the in-app edit path (§4.6) — memory the user cannot see or correct is memory
the user cannot trust (FR-6). The Skills section lists available skills by
name, description, and origin; selecting one shows its instruction body
read-only, so "what could this skill tell the model to do" is always
inspectable before it ever runs.
- **Trust is visible, not just enforced.** Project memory and project skills
from an untrusted folder are simply **not shown and not offered** (Requirements
FR-1); their absence is the correct, quiet signal, consistent with the trust
gate (§8.4). Running a skill's bundled script still surfaces the ordinary
permission prompt (§5) like any command — the skill line never implies its
scripts ran unprompted.

### 4.10 Web-search results

A web search (Requirements T-14) returns results into the conversation as
**agent-world content** (§6.1): a short list of hits — title, source URL, and a
snippet each — rendered as a tool result the model reads and reacts to. The
rendering makes two things unmistakable:

- **It came from the open internet, and it is untrusted (Requirements T-14).**
The block is labeled as fetched web content and styled as data, never in the
harness's own voice and never as the assistant's conclusion — so a snippet
that says "ignore your instructions" reads visibly as *quoted web text*, not
as something Emberly is telling the user or the model to do. Source URLs are
shown so the user can judge provenance.
- Result count and snippet length are bounded (the Technical Specification sets
the caps) and long result sets follow the §8.6 reduction economy — search is
not a context flood. The permission gate for reaching the network at all is
§5.2.

### 4.11 Documents in a terminal

Documents (Requirements T-16/P-12) get the identical treatment to images
(§4.8), for the identical reason: the terminal cannot paint a PDF, and the value
is the *model* reading the document, not the user re-seeing a file they already
have. A read-document call renders as a labeled reference line in tool-activity
styling — `read document  contract.pdf · 240 KB · PDF` — naming the file, its
size, and its format. It deliberately does **not** show a page count or extracted
text: the harness passes the document to the provider unparsed (P-12), so it
reports only what it knows without opening the file. The honesty is explicit, as
with images: the line states the document went to the model, not to the screen.

- If the active model cannot read documents (Requirements P-12), the
unsupported-capability result renders as a calm tool-result note — "this model
can't read documents; switch model (§3.1) or extract the text" — never a harness
error (§6.1), so model and user both learn the document was not seen.
- Degraded mode (§7): the same reference line, ASCII-only.
- Only PDF is read (Requirements §2.3 declines other document formats); the
reference line never implies a format the harness does not send.

### 4.12 Scratch writes in the flow

A scratch-write (Requirements T-17) surfaces the same way memory and skill
activity does (§4.9): a quiet, single dim line, never ceremony — reading like
`scratch · analysis.py · 1.2 KB written` (illustrative; exact wording is
free), naming the file and its size. There is no permission prompt to render
here, because none occurs (T-17); the line
exists so the user can see what working files the model left behind, not to
gate the write. It is otherwise ordinary tool activity: it may carry a §4.5
explanation when the model supplied one, and degraded mode (§7) renders it
ASCII-only like any other tool line.

### 4.13 Subagents in the flow

A subagent (Requirements FR-9, T-18–T-21) is delegated work, not a second
voice in the room: its activity surfaces as **quiet, ordinary tool
activity**, the same register as memory, skills, and scratch writes (§4.9,
§4.12) — never a second live-streamed conversation competing with the
primary agent's own text for the user's attention. Calm under load (§1.2)
applies most exactly when several subagents are running at once.

- **Spawn, message, and end read as one-line events.** `spawn_agents` reads
like `agents · spawned "db-migration", "test-writer" · running`; a reply
from `message_agent` reads like `agent · db-migration · replied`; `end_agent`
reads like `agent · db-migration · ended`. Each may carry a §4.5 explanation
when the model supplied one. None of these is a harness-voice moment — the
model is using its own delegated workers, shown without weight, exactly as
§4.9 already establishes for memory and skills.
- **No raw concurrent streaming.** When several subagents run at once
(a batch spawn, §5), their individual assistant text does *not* stream into
the main pane — that would turn the conversation into an illegible braid of
interleaved voices. The main pane shows only the one-line events above; a
subagent's full turn-by-turn activity is available on demand.
- **Inspectors, not black boxes (extends §4.9).** The sidebar's Agents
section (§3.1) lists every currently alive subagent; selecting one opens a
read-only, live-updating overlay (the §4.2 pattern) showing that subagent's
own conversation as it happens — its assistant text, its tool calls, and
its own tool-activity lines — so "what is this delegate actually doing" is
always inspectable, never assumed. A subagent that has ended keeps its
inspector reachable for the rest of the session so its work is reviewable
after the fact, not just while live.
- **Permission prompts name their subagent.** When a subagent's own tool
call raises a permission prompt (Requirements FR-9 — the same rule/sandbox
model as the primary agent, §5), the prompt carries one additional dimmed
line naming which subagent is asking — e.g. "on behalf of subagent
db-migration" — so the user is never asked to approve an action without
knowing who it's for. This is an addition to, never a dilution of, the
ordinary permission prompt: no new styling, no reserved-band treatment,
the same Deny-default and forbidden patterns as §5 apply unchanged.
- **A timed-out or still-running spawn is a calm, one-line fact.** When
`spawn_agents` returns with one or more subagents still running past the
per-call timeout (Requirements T-18), the tool-result line for that
subagent reads like `agent · test-writer · still running` rather than an
error — a slower delegate is normal, not a failure, and it stays reachable
via `message_agent`/`list_agents`.
- Degraded mode (§7): the same one-line events, ASCII-only, exactly as
memory/skill/scratch lines already degrade; the inspector overlay falls
back the same way any overlay does in plain mode.

### 4.14 Attaching an image to a prompt

A user-attached image (Requirements FR-10) is the opposite direction of
§4.8's read-image tool: there, the *model* looks at a project file; here, the
*user* shows the model something. The two are never rendered the same way —
an attachment is content in the **user's own message**, not a tool call, so
it never wears tool-activity styling.

- **A small attachment chip on the sent message**, not a tool line — reading
like `📎 mockup.png · 800×600 · PNG attached` under the user's turn
(illustrative; exact wording is free). It says the image went into this
message, mirroring §4.8's honesty about where content actually goes.
- **Two gestures, one honest limitation.** An explicit `/attach <path>`
command (or a file-picker reusing the §4.6 overlay machinery) always works,
any terminal. Drag-and-drop works where the terminal delivers a dropped
file's path as pasted text — Emberly recognizes a pasted string that
resolves to an existing image file and offers to attach it rather than
insert it as typed text. **True clipboard-image-byte paste (a screenshot
copied without a backing file) is not supported this version** — terminal
paste delivers text, not image bytes, with no cross-terminal API for the
latter; this pairs with the existing sixel/kitty/iTerm deferral (§4.8, §10)
as the same class of terminal-capability gap, not a harness oversight.
- **Same unsupported-vision honesty as §4.8.** If the model active at send
time has no vision (Requirements P-11), the attachment produces the same
calm unsupported-capability note as a read-image call (§4.8, HC-6) — never
a silent drop and never a harness error.
- Degraded mode (§7): the chip renders as a plain ASCII line; the file-picker
is not offered (same fallback pattern as guided setup, §4.6/§7) — `/attach
<path>` remains the always-available path.

### 4.15 MCP tools in the flow

An MCP-sourced tool (Requirements FR-11) earns no special drama for coming
from outside the harness — it renders as **quiet, ordinary tool activity**,
the same register as memory, skills, scratch writes, and subagents
(§4.9, §4.12, §4.13).

- **The owning server is always on the line**, exactly as origin is always on
a memory or skill line (§4.9) — e.g. `mcp · jira · get_issue` — because
which server is acting is exactly the kind of thing a user reads trust from.
May carry a §4.5 explanation when the model supplied one.
- **A sidebar MCP section, inspectors not black boxes (extends §4.9).**
Lists currently connected servers; selecting one opens a read-only overlay
(§4.2 pattern) listing that server's discovered tools — "what can this
server make the model do" is always inspectable before it is ever used,
the same commitment memory/skills/subagents already make. Present only
while at least one server is connected, like Tasks/Agents (§4.7/§4.13) —
never an empty "MCP (0)" stub.
- **Trust is visible, not just enforced (extends §4.9).** A project-declared
server withheld by workspace trust (Requirements FR-1, FR-11) is simply
**not shown and not connected** — the same quiet, correct absence already
established for project memory and skills (§4.9, §8.4), and, per §8.10,
gated by the very same trust prompt, never a second one.
- **Untrusted content, like web search (extends §4.10).** A tool result
returned by an MCP server renders labeled as external, untrusted content the
model reads — quoted data, never harness or assistant voice — mirroring how
a web-search hit is labeled (§4.10) rather than the harness's own words.
- Degraded mode (§7): plain ASCII tool-activity lines and a plain-text
server/tool list; the inspector overlay degrades the same way any overlay
does in plain mode.

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
- **A subagent's request names the subagent (Requirements FR-9, §4.13).**
When the action belongs to a subagent's own tool call rather than the
primary agent's, the prompt carries one additional dimmed line naming which
subagent is asking. Every other guarantee on this page — Deny default,
full content, the forbidden patterns — holds exactly as if the primary
agent had asked; a subagent earns no different treatment, easier or harder.

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

### 5.2 The web-search prompt (reaching the network)

Web search is permission-gated (Requirements T-14), so it produces a permission
prompt — but a plain one, deliberately **not** the reserved outside-project-root
safety band (§2). That band guards filesystem escape and must stay rare and
loud; a web search is a routine, lower-stakes action, and diluting the reserved
styling here would blunt the one signal that must never be ignored.

- **What it shows:** the search query, the configured search backend it will
reach (endpoint name, not the raw key — §4.5-style dimmed line), and one
plain line naming the two facts the user is consenting to — *this reaches the
internet, and what comes back is untrusted web content* (§4.10). Enough to
decide, no alarm.
- **Choices and default** follow the ordinary permission prompt (§5): Deny is
the default keypress; Allow once · Allow for this session · and, where the
rule layer permits, an "always allow web search in this project" that writes
a rule (Requirements §6.6). Search is rule-allowlistable exactly like a bash
command (Requirements T-14), so a user who wants unattended search grants it
once.
- It is a permission prompt, so all §5 forbidden patterns apply (no
timeout-to-approve, no click/Enter-through). It reuses the calm neutral
treatment of a normal prompt; only the outside-root escalation earns the
reserved band.

### 5.3 The MCP tool-call prompt (an external server acting)

An MCP-sourced tool call (Requirements FR-11) is permission-gated exactly
like a built-in tool call, and its prompt is the ordinary permission prompt
(§5) with one addition, mirroring how a subagent's request adds a provenance
line (§4.13, §5) rather than becoming a new prompt type.

- **What it adds:** one dimmed line naming which server the call belongs
to — e.g. "via MCP server jira" — shown *before* approval, never after, so
the user always knows which external process is asking. Because the tool
itself can be arbitrary (an MCP server may do anything its own code does),
the ordinary **full-content-always** rule (§5) applies without exception:
complete arguments shown, never summarized.
- **Ordinary band, not reserved (mirrors §5.2's reasoning for web search).**
A routine MCP tool call keeps the plain permission treatment; only an
action that itself touches outside the project root (Requirements HC-4)
earns the reserved escalation band, exactly as any tool would — an
external origin does not, by itself, make a call more dangerous than a
built-in one doing the same thing.
- **Choices and default** are the ordinary permission prompt's (§5): Deny
default, Allow once, Allow for this session where the rule layer permits —
rule-allowlistable per tool name like bash or web search (Requirements
§6.6).
- All §5 forbidden patterns apply unchanged (no timeout-to-approve, no
auto-scroll, no batching).

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
- The 0.4 surfaces degrade with everything else: mouse capture is off (§3.4);
the task list uses ASCII status markers `[ ]`/`[~]`/`[x]` (§4.7); an image
read is a plain ASCII reference line (§4.8); memory/skill/search activity
and their origins are plain labeled lines; and the web-search prompt keeps
its full permission-prompt guarantees in capitals (§5.2). No 0.4 feature
relies on color, motion, or the pointer to carry meaning.
- The 0.4.1 surfaces degrade the same way: a document read is a plain ASCII
reference line (§4.11), and the completion-gate halt (§8.7) keeps its full
harness-voice guarantees with the four choices as capitalized deliberate keys.
Neither relies on color, motion, or the pointer.
- The 0.4.2 surface degrades by falling back rather than reflowing: guided
provider setup (§4.6) is not offered in degraded mode; the same
print-the-path-and-`/reload` pattern `/config`/`/prompt` already use there
covers the same ground.
- The 0.5 surface degrades like memory/skills/scratch before it: subagent
spawn/message/end lines (§4.13) are plain ASCII tool-activity lines, a
permission prompt raised on a subagent's behalf keeps its full capitalized
guarantees with the subagent's name in the same plain dimmed line, and the
per-agent inspector overlay degrades the same way any overlay does in plain
mode. No 0.5 feature relies on color, motion, or the pointer to carry
meaning.
- The 0.5.1 surfaces degrade the same way: an attached image renders as a
plain ASCII attachment line under the user's own message (§4.14), with no
file-picker offered (`/attach <path>` remains available); MCP tool-activity
lines, the sidebar MCP section, and a connection-failure notice are all
plain text with full capitalized permission-prompt guarantees where one
appears (§4.15, §5.3, §8.10); and `emberly export`'s output and its
sensitive-content line (§8.11) are plain text by construction, since the
command's own output is never a rich-mode-only surface. No 0.5.1 feature
relies on color, motion, or the pointer to carry meaning.
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

- **No provider configured yet.** When the session is running on the
offline placeholder (no `provider =` selected), the model line in the
session-start header names the gap and points straight at the fix in the
same breath — e.g. *"model: none configured — /model to add a
provider"* — rather than raw env-var instructions, now that guided setup
(§4.6, Requirements C-7) is the easier path. Degraded mode keeps the same
line in plain text. This is the header's normal one-time content, not a
popup to dismiss — silence about defaults, speech about the one
deviation that actually blocks the user from doing anything.

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
trust; trusting is the deliberate choice. Typing `trust`, `yes`, or `y`
(case-insensitive) grants; anything else — including a bare Enter —
declines. Declining does not start the session (Requirements FR-1) —
Emberly says so in one line and exits cleanly, never half-starting in a
crippled state.
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
TRUST / DON'T TRUST choices, deliberate affirmative input to trust.
- **What trust now also gates:** as of the 0.4 features, the gate stands
before not just project files entering a prompt but also project **memory**
(Requirements FR-6) and project **skills** (Requirements FR-7) being loaded
or offered — both are project-resident text/code that reach the model. The
prompt's plain-language line already covers this ("Emberly will read, edit,
and run commands in this folder"); no new wording is needed, but the gate's
reach is wider, and untrusted-folder memory/skills are silently absent
(§4.9) rather than half-loaded.

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

### 8.6 Keeping context lean (reduction, windowing, compaction)

The 0.3 context-economy layers (Requirements §8.5–§8.8) share one design
rule: **the context may get leaner, but the user is never lied to about
it.** Nothing is silently dropped, and everything dropped has a visible way
back.

- **Tool-result reduction and windowing are quiet by default.** Reduced tool
results carry the §4.3 marker (what was withheld, `/view` for the whole
thing) and nothing more — reduction is the normal case, not an event to
announce. A turn dropped from the adaptive window (Requirements FR-3) is
not erased from the scrollback the user reads; windowing governs what is
*sent to the model*, and the conversation the user scrolls remains whole.
The context-usage indicator (§3.1, §8.4) reflects the working window, so
"why did usage drop" is always answerable, never mysterious. When the model
reaches back for a dropped turn, its `recall` call (Requirements T-10)
renders as ordinary, quiet tool activity in the flow — one dim line like any
tool call, optionally with a §4.5 explanation — so the user can *see* the
model choosing to reload history, without ceremony. `recall` is the model
reading its own context; it is distinct from `/view` (§4.3), which is the
user opening full output — the two never share a surface.
- **Compaction is a harness-world moment (§6.1), announced in the harness's
own voice** — never as model output, since it is the harness reshaping the
model's context. Manual `/compact` shows the §6.3 spinner ("compacting")
and then one calm line — "Compacted 24 turns into a summary." Automatic
compaction (Requirements FR-4) is the same line with its reason —
"Context was near full — compacted 24 turns to keep going." — so the
context changing under the model is a visible act, not a silent one. No
alarm styling: staying under budget is routine housekeeping, and §1.2's
"calm under load" applies most exactly here.
- **The summary is inspectable, not a black box.** The compaction summary is
ordinary conversation content in the flow and openable in full via `/view`
(§4.3), so the user can see what the model will now treat as history.
- **Resume is fast and honest (Requirements FR-5).** Resuming restores the
working view directly, so a long session reopens without a visible
reload-the-world stall; the one-line resume offer (§8.3) is unchanged. If
the derived cache is unusable and the harness must rebuild from the
transcript, it says so in one dimmed harness-voice line rather than
reopening silently slower — silence about the fast path, speech about the
fallback.

### 8.7 When completion is gated

When a session has registered completion checks (Requirements S-6) — for a
coding deployment, "tests/lint/build must pass" — the gate governs the loop's
claim of *done*, and its visible moments stay calm and honest, never alarm.

- **A failed completion attempt is agent-world (§6.1), not a harness event.**
When the model tries to finish and a check fails, the failing checks return to
the model as ordinary tool-result content — the check name and its structured
reason (e.g. `tests: 2 failed`) — and the loop simply continues. The user
watches the model react and fix, exactly as with any tool failure; the harness
does not editorialize. A passing evaluation is quiet — one dim tool-activity
line, never a celebration.
- **The halt is a harness-world moment (§6.1), in the harness's own voice** —
reached only after a bounded number of failed completion attempts (Requirements
S-6), because a model that cannot satisfy a check must not spin (the same
promise as §8.5). One calm line of what happened — "Stopped: the completion
checks still fail after 3 attempts (tests: 2 failed)." — then the choices:
**keep going** (let the model try again), **say something** (steer it), **stop
here**, or **finish anyway** (end the task as done despite the failing gate).
- **"Finish anyway" is a deliberate, recorded override.** The gate binds the
*model's* claim of done, never the user's authority (Requirements S-6): a user
who judges a check wrong or irrelevant may end the session, and that override is
written to the transcript as an explicit user decision — never silently, and
never presented as though the checks passed. The honesty clause holds in the UI:
a green gate is never styled as a guarantee beyond what the checks tested, and a
gate overridden red is labeled as overridden.
- **Gate status is visible only when checks are registered**, alongside sandbox
and context status (§3.1): a dim line naming the registered checks and their
last result. A session with no registered checks shows nothing — the gate is
inert and, like the Tasks/Memory sections (§3.1), never a "None" stub.
- It is distinct from the loop-break (§8.5): S-5 stops a loop that is
*re-treading*; S-6 stops one that is *landing early*. Both end in a user decision
in the harness voice; a session may meet either.
- Degraded mode (§7): the halt keeps its guarantees — plain harness-voice lines,
ASCII, the four choices as capitalized deliberate keys.

### 8.8 Reclaiming scratch space

`emberly clean` (Requirements FR-8) follows the same voice as `init` (§8.1):
prints exactly what it found and removed, and where — no walls of text, no
installer-wizard ceremony. A target that resolves to nothing says so plainly
(e.g. *"nothing to clean"*) — never silence. If the Technical Specification's chosen
scope requires confirming before deletion (open question, Requirements §13),
that confirmation is a plain question with a plain `y`/`n` answer, matching
the trust prompt's tone (§8.4) — not a scary dialog for what is, after all,
disposable working space.

### 8.9 Delegating to a subagent

Spawning and conversing with a subagent (Requirements FR-9, §4.13) is the
one 0.5 moment that could, mishandled, feel like losing sight of what the
agent is doing — so it stays legible at every step, in keeping with §1.2's
transparency.

- **A batch spawn is one calm block, not a wall of chatter.** When the
primary agent spawns several subagents in one call (T-18), the main pane
shows one line per subagent as each reaches its own first stop — "running,"
then a result or "still running" — never a flood of interleaved streaming
text (§4.13).
- **The sidebar Agents count is the at-a-glance answer to "what's still
going."** Exactly like Tasks (§4.7) and the completion-gate status (§8.7),
it is present only while at least one subagent is alive and disappears
quietly when the last one ends — never a lingering "Agents (0)."
- **Ending a subagent is quiet, not a confirmation dialog.** `end_agent`
(T-21) reads as an ordinary one-line tool event (§4.13); a session ending
with subagents still alive ends them silently along with it — this is
expected cleanup, not a moment the user is interrupted to confirm.
- **A subagent's own permission prompts feel like the session's own**, just
labeled (§5) — the user is never asked to context-switch into "now I'm
approving for a delegate" versus "now I'm approving for the main agent";
it is one continuous safety model with one added fact per prompt.

### 8.10 Connecting to an MCP server

A project-declared MCP server (Requirements FR-11, C-8) is reached through
the same trust gate that already governs project memory and skills (§8.4),
never a second prompt — the user is not asked twice to trust the same
folder.

- **Trust withheld is silent, not an error.** In an untrusted folder the
server is simply not connected and not shown (§4.15) — the correct, quiet
absence, exactly matching §4.9/§8.4's existing pattern.
- **A successful connection is quiet.** One dim line at session start —
e.g. `mcp · jira · connected · 4 tools` — no splashy banner, matching how
memory and skill catalogs already load without ceremony (§4.9).
- **A failed connection is a harness-world moment (§6.1)**, in the harness's
own voice: what happened, why if known, and what to do — e.g. "Couldn't
connect to MCP server 'jira': command not found — check
`[mcp.servers.jira]`." Never a stack trace as the primary surface. A broken
server does not stop the session from starting — the rest of the harness
stays usable, the same "make the risk clear, stay usable" principle the
sandbox-degradation policy already holds (Requirements §6.7).
- Degraded mode (§7): the same lines, plain text.

### 8.11 Exporting a session

`emberly export` (Requirements FR-12) follows the same voice as `init` and
`clean` (§8.1, §8.8): it prints exactly what it wrote and where — no walls
of text, no installer-wizard ceremony.

- **One calm, factual line about sensitive content**, shown once at export
— e.g. "This file may contain file contents, command output, and anything
else this session touched — review before sharing." Never alarm styling
(§6.1's no-blame, no-alarm voice): export does not redact (Requirements
FR-12), so the honest disclosure *is* the safeguard, not a dialog the user
must click through. It is not a confirmation gate — export mutates nothing,
so it carries none of the trust-gate/`clean`-command "are you sure" weight.
- A large session shows the §6.3 spinner with a dull, truthful verb phrase
("exporting") if the write takes visible time; the finished line names the
output path, exactly like a clean exit's session summary (§8.3).

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
- `**--plain` / line-oriented degraded mode** (§7) — is the de facto
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
- **Compaction and windowing are surfaced, never silent** (§8.6) — refines
Requirements FR-3/FR-4: automatic compaction and a shrinking context
window are announced in the harness voice (or, for routine reduction,
carry a `/view`-able marker), consistent with "silence about defaults,
speech about deviations." The context indicator reflecting the working
window (§8.6) is a Design commitment the Tech Spec's accounting absorbs.
- **A reduced/truncated tool result is never a dead end** (§4.3) — realizes
Requirements FR-2's "full result one reference away" as the reused `/view`
escape hatch with a marker naming what was withheld.
- **No terminal image painting** (§4.8) — realizes Requirements T-12/P-11 as a
labeled reference line rather than in-terminal image rendering; sixel/kitty/
iTerm protocols are deferred (§10). A Design constraint the Tech Spec absorbs
(the read-image tool surfaces a reference, not pixels).
- **Memory and skills render as quiet tool activity with visible origin**
(§4.9) — refines Requirements FR-6/FR-7 surfacing: memory/skill/recall lines
show `user` vs `project` origin so trust is legible, and both get sidebar
inspectors (memory editable, skill body read-only). Origin-on-the-line is a
Design commitment the Tech Spec's event payloads carry.
- **Web-search prompt uses the ordinary permission treatment, not the reserved
band** (§5.2) — refines Requirements T-14: reaching the network is a routine
gated action with a network/untrusted label, deliberately kept off the
outside-project-root safety styling so that band stays rare.
- **Web results are labeled untrusted agent-world content** (§4.10) — realizes
Requirements T-14's untrusted-content requirement as a styling rule: fetched
web text is quoted data with visible source URLs, never harness or assistant
voice.
- **Mouse is additive-never-exclusive with native selection preserved** (§3.4)
— realizes the Requirements §2.1 mouse scope item: the pointer speeds safe
navigation and buys nothing on a permission decision; Shift-drag keeps the
terminal's own copy, and `mouse = false` releases capture. The Tech Spec
absorbs the capture/passthrough mechanics.
- **Task-list glyphs never color-only** (§4.7) — realizes Requirements T-11 as
a two-place (inline + sidebar) render with ASCII-fallback status markers,
consistent with §7.
- **Completion-gate halt offers resume / steer / stop / finish-anyway** (§8.7) —
realizes Requirements S-6's "returns control to the user" as four concrete
choices in the harness voice, and refines S-6 with a **user override**: the user
may end a task as done over a still-failing gate, recorded in the transcript as
an explicit override (the gate binds the model's claim, never the user's
authority). The override choice and its transcript record are a Design commitment
the Tech Spec absorbs.
- **A failed check is agent-world, the halt is harness-world** (§8.7) — refines
Requirements S-6's surfacing: a failing check returns to the model as ordinary
tool-result content (the model fixes and retries), while only the bounded-attempt
halt speaks in the harness voice, consistent with the §6.1 two-register split and
§8.5's loop-break.
- **No terminal document painting** (§4.11) — realizes Requirements T-16/P-12 as
a labeled reference line (file · size · format), never in-terminal rendering and
never a page count or extracted text, since the harness passes the document
unparsed. The Tech Spec absorbs the read-document surface as a reference, not
content.
- **Subagent activity is quiet tool activity, never live-concurrent streaming**
(§4.13) — realizes Requirements FR-9 as one-line spawn/message/end events
matching the memory/skill/scratch register (§4.9/§4.12), with a per-agent
inspector overlay carrying the full turn-by-turn detail on demand. The Tech
Spec absorbs "no raw text streaming from a subagent to the main pane" as an
event-routing decision, not just a rendering choice.
- **Permission prompts gain a subagent-provenance line, never a new prompt type**
(§5, §4.13) — refines Requirements FR-9: a subagent's action is asked for
through the exact same prompt, guarantees, and defaults as the primary
agent's, with one added dimmed line naming the subagent. The Tech Spec
absorbs the provenance field on the permission-rendering payload.
- **The sidebar Agents section is present only while a subagent is alive**
(§3.1, §8.9) — extends the established "no empty stub" rule (Tasks, Memory,
Skills, the completion gate) to Requirements FR-9.
- **An attached image is content on the user's own message, never tool
activity** (§4.14) — refines Requirements FR-10: rendering lives on the
sent-message payload, distinct from the §4.8 tool-result line, because the
two are opposite directions of the same capability. The Tech Spec absorbs
this as a structural distinction (which transcript event carries the
image block), needing no new content-block flag.
- **True clipboard-image-byte paste is declined, not silently unsupported**
(§4.14) — a new explicit deferral for Requirements §2.2, alongside the
existing sixel/kitty/iTerm terminal-image-protocol door: both wait on the
same missing cross-terminal capability.
- **MCP tools render with server provenance on the line and a sidebar
inspector** (§4.15) — refines Requirements FR-11 the same way §4.9 already
established for memory/skills: origin visible, nothing a black box. The
Tech Spec absorbs a server-name field on the relevant tool events.
- **MCP connection is trust-gated by the existing gate, never a second
prompt** (§8.10) — refines Requirements FR-11/FR-1: a project-declared
server reuses the one workspace-trust decision rather than asking again.
- **MCP tool-call prompt uses the ordinary band, not reserved** (§5.3) —
refines Requirements FR-11 the same way §5.2 refined T-14: an external
origin does not, by itself, escalate a prompt's styling.
- **A broken MCP server connection is harness-world and non-fatal** (§8.10)
— refines Requirements FR-11: the halt/notice pattern of §6.1 applies, and
the session still starts, consistent with the sandbox-degradation "stay
usable" principle (Requirements §6.7).
- **Session export warns once, calmly, and never blocks** (§8.11) — realizes
Requirements FR-12's "Design Guideline concern, not a filtering guarantee"
honesty clause as a specific, non-blocking disclosure line shown at export
time.

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
- Exact wording and verbosity of the automatic-compaction notice (§8.6) —
candidate lines are examples; tune against real sessions so the notice
informs without nagging in a session that compacts repeatedly.
- Task-list emphasis: sidebar-primary vs inline-primary once real sessions
exist, and whether long done-lists collapse to a count (§4.7). Tune with use.
- Terminal image protocols (sixel/kitty/iTerm) as a future display nicety
behind capability detection (§4.8) — worth exploring only if users on
capable terminals ask; never a default, never load-bearing.
- Web-search result density — how many hits and how long a snippet reads well
in the flow without becoming a context flood (§4.10); pairs with the Tech
Spec's caps. Tune with use.
- Whether clicking should also select *text* within a pane (beyond entries),
or leave in-pane text selection entirely to the terminal via the Shift
modifier (§3.4). Lean to the latter until a real need appears.
- Completion-gate halt wording and how the registered-checks status reads in the
sidebar (§8.7) — candidate lines are examples; tune against real gated sessions
so the halt informs without nagging when a check fails repeatedly.
- Exact wording for the spawn/message/end tool-activity lines and the
"still running" note past a spawn timeout (§4.13, §8.9) — candidate lines
are examples; tune against real multi-agent sessions.
- Whether the Agents sidebar entry shows a subagent's own token/cost usage
inline, or only on opening its inspector (§4.13, Requirements §13 — Design
resolves this one). Tune with use once real sessions exist.
- Whether the attach gesture needs an explicit confirm step ("attach
mockup.png?") or attaches immediately on a recognized drag-drop path
(§4.14). Tune with use once real terminals are tested.
- Whether the sidebar MCP section shows per-tool status/last-call detail or
only the connected-server list at a glance (§4.15). Tune with use.
- Exact wording of the MCP connection-failure line, and whether a
repeatedly-failing server across sessions should elevate to a more visible
notice than the quiet §8.10 line. Tune with use.
- Exact wording of the export sensitive-content line, and whether a very
large session's export should show byte/turn progress rather than only
the §6.3 spinner (§8.11). Tune with use.

Resolved since v0.4: reasoning-trail default view — `collapsed` (§4.4,
owner); tool-call explanation line — on by default, config-defeatable
(§4.5, owner).

Resolved since v0.5 (0.3 feature set): the context-economy layers are
surfaced honestly and calmly, never silently — routine reduction/windowing
carries a `/view`-able marker (§4.3, §8.6), compaction (manual and automatic)
is a harness-voice moment (§8.6), and the context indicator tracks the
working window (§8.6, owner). The model's `recall` of dropped turns
(Requirements T-10) renders as quiet, ordinary tool activity, kept distinct
from the user-facing `/view` (§8.6, owner).

Resolved since v0.6 (0.4 feature set): the six 0.4 capabilities are given feel
and interaction, all under the identity's calm-and-honest rules. The task list
renders inline plus a sidebar Tasks section with color-free status glyphs
(§4.7). Images are a labeled reference line, not in-terminal pixels — no
sixel/kitty/iTerm rendering in scope (§4.8). Memory and skills render as quiet
tool activity with origin on the line and sidebar inspectors, and are gated by
workspace trust (§4.9, §8.4). Web search reaches the network behind an ordinary
permission prompt — never the reserved outside-root band — and its results are
labeled untrusted agent-world content with visible source URLs (§5.2, §4.10).
Mouse interaction is additive-never-exclusive: it speeds safe navigation, buys
nothing on a permission decision, preserves the terminal's native Shift-drag
selection, and is off in degraded mode and under `mouse = false` (§3.4). All
six degrade to plain, tested, meaning-preserving output (§7).

Resolved since v0.7 (0.4.1 feature set): the two absorbed capabilities are given
feel under the identity's calm-and-honest rules. Documents render as a labeled
reference line (file · size · format), never in-terminal pixels and never a
parsed page count — the image treatment (§4.8) applied to PDF (§4.11). The
completion gate (Requirements S-6) surfaces in two registers: a failed check is
agent-world content the model reacts to, and the bounded-attempt halt is a
harness-voice moment offering keep-going / steer / stop / finish-anyway, where
"finish anyway" is a deliberate, transcript-recorded user override of a red gate
(§8.7). Both degrade to plain, tested, meaning-preserving output (§7).

Resolved since v0.11 (0.5 feature set): the multi-agent subsystem is given
feel under the same calm-and-honest rules as memory, skills, and scratch
writes before it. Subagent spawn/message/end events render as one-line
quiet tool activity (§4.13), never raw concurrent text streamed into the
main pane — chosen deliberately over a multi-pane live view (which the
Requirements deferred, §2.2) because interleaving several assistants'
streaming text in one pane would break "calm under load" (§1.2) long before
it became genuinely useful. A subagent's own full activity is always
available via a per-agent inspector overlay, the same "inspectors, not
black boxes" pattern already established for memory and skills (§4.9). A
subagent's permission prompts reuse the ordinary prompt verbatim with one
added provenance line — deliberately not a new prompt type or the reserved
safety band, matching how web-search (§5.2) and completion checks already
avoid diluting that band. The sidebar Agents section follows the
established no-empty-stub rule. Degrades to plain, tested, meaning-
preserving output (§7).

Resolved since v0.12 (0.5.1 feature set): the three 0.5.1 capabilities are
given feel under the same calm-and-honest rules established for the 0.4 and
0.5 sets. An attached image (§4.14) renders as a small chip on the user's
own sent message, deliberately distinct from the §4.8 tool-activity
reference line, because the two are opposite directions of the same
capability — the user showing the model something, versus the model
looking at something already there; true clipboard-image-byte paste joins
the terminal-image-protocol door as a new explicit deferral (§2.2), since
both wait on the same missing cross-terminal capability. MCP-sourced tools
(§4.15) render as quiet tool activity with the owning server named on the
line, mirroring §4.9's origin visibility, plus a sidebar inspector listing
connected servers and their tools; a project-declared server is trust-gated
by the existing workspace-trust prompt (§8.10), never a second one, and a
connection failure is a harness-voice moment (§6.1) that never blocks the
session from starting. The MCP tool-call permission prompt (§5.3) reuses
the ordinary band, not the reserved safety styling, matching how the
web-search prompt (§5.2) already avoids diluting that band. Session export
(§8.11) follows the `init`/`clean` voice — exactly what was written and
where — plus one calm, non-blocking line naming that the exported file may
carry sensitive content already in the transcript, since redaction is
explicitly not attempted (Requirements FR-12). All three degrade to plain,
tested, meaning-preserving output (§7).
