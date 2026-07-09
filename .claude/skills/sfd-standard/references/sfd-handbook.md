# Software Foundation Documents (SFD) — Standard Handbook

**Version:** 1.0
**Status:** approved
**Date:** 2026-07-09
**Owner:** Wattanit
**Companion documents:** SFD Agent Guide v1.0 (derived from this handbook —
see G-30; the two documents version together)

This handbook defines how a software project's **foundation documents** are
written and maintained across the product's lifetime: the three documents
that define a project before implementation begins, what question each one
owns, the conventions that keep them consistent across authors — human or
AI — and how they evolve across the project's versions and lifetime
(§2.5).
"Foundation" is meant literally: everything downstream — implementation
plans, to-do lists, test cases, code — is built on these documents and
cites into them (§2.4). This handbook is the canonical statement of the
standard; the Agent Guide is a derived, compact form of the same rules for
direct use in AI agent context. Rules in this handbook carry stable
identifiers (**G-n**) so they can be cited, audited, and traced into the
Agent Guide.

The standard was extracted from a working document suite (the Emberly Code
project's Requirements v0.3, Design Guideline v0.3, and Technical
Specification v0.1), which serves throughout as the **reference example**.
Passages marked *Example (reference project)* are illustrations, never
rules; the standard itself is project-agnostic.

---

## 1. Purpose and Applicability

### 1.1 What problem this solves

Project documentation fails in two familiar ways: documents that overlap
until nobody knows where a decision lives, and documents that are filled in
as ritual until nobody reads them. This standard prevents both with one
central idea: **every document owns a question, and every decision has
exactly one home.** Requirements own WHAT and WHY. The Design Guideline
owns how the product looks, feels, and speaks. The Technical Specification
owns HOW. The standard itself is the WHERE — the routing that assigns each
decision to its home.

### 1.2 When the standard applies — and the scale rule

- **G-1.** The full three-document suite applies to any project expected to
  outlive its first implementation sprint or to be built by more than one
  person or agent.
- **G-2 — Scale rule.** The suite scales down, never dilutes. A small
  project may collapse the three documents into one document with three
  clearly separated parts, each keeping its charter (§2.1). A project with
  no human-facing surface (a library, a batch job) may omit the Design
  Guideline entirely, folding any voice/naming decisions into Requirements.
  What is never acceptable is three documents that each answer a little of
  everything.
- **G-3.** The standard is domain-agnostic. Nothing in it assumes a
  particular product type, technology, or company. Illustrations are
  always marked as examples (*Example (reference project):* …) and carry
  no normative weight; if an example does not fit a project, the rule
  still applies and the example is ignored.

## 2. The Document Suite

### 2.1 The three documents and their charters

Each document opens with a **charter paragraph** immediately after its
header block: one short paragraph stating what the document owns, what it
defers, and to which document it defers it. The charter is the contract
that makes the routing table (§2.2) enforceable.

- **G-4 — Requirements Document.** Owns WHAT the product must do and WHY it
  exists. Contains: purpose and motivation (with priorities in order),
  scope (§3.4), hard constraints, functional requirements with stable IDs,
  and the document plan for the rest of the suite. It states *behavioral*
  requirements only; where it constrains implementation (e.g. a dependency
  policy), it must state why that constraint is itself a requirement and
  not an implementation preference.
- **G-5 — Design Guideline.** Owns how the product looks, feels, and
  speaks: identity and naming, visual language, layout, interaction
  patterns, how the product responds to the user — in normal use, in
  failure, and under constrained conditions — voice and wording, and the
  key user moments. Where a design decision has technical consequences,
  the Design Guideline decides and the Technical Specification absorbs.
- **G-6 — Technical Specification.** Owns HOW the product is built:
  architecture and module layout, data models, dependencies, algorithms,
  defaults and tunables, build/release, testing strategy, and milestones.
  Every choice that satisfies an upstream requirement cites that
  requirement's ID, so traceability is mechanical, not archaeological.

### 2.2 Decision routing table

- **G-7.** When writing, route each decision using this table. If a
  decision does not fit a row, the ambiguity itself goes to the Open
  Questions ledger (§3.5) of the document currently being written, with
  the resolving document named.

| Decision type | Home | Notes |
|---|---|---|
| Why the product exists; priority order of its reasons | Requirements | Priorities are ordered, not listed flat |
| What must be true, phrased testably | Requirements | See G-18 |
| Scope: in / deferred / out | Requirements | Three tiers, see G-20 |
| Implementation constraint that is itself a requirement | Requirements | Must state why (G-4) |
| Product name, brand, identity, anti-goals | Design Guideline | |
| Visual language, layout, key screens/moments | Design Guideline | |
| Interaction patterns; user-facing responses in normal use, failure, and constrained conditions | Design Guideline | The *policy*; the mechanism goes to the Spec |
| Voice, wording, message structure | Design Guideline | |
| Design decision with technical consequences | Design Guideline decides | Spec absorbs; if it changes WHAT, use the feedback protocol (G-24) |
| Architecture, module boundaries, data models | Technical Specification | |
| Dependency list and acceptance decisions | Technical Specification | Under any policy set in Requirements |
| Concrete defaults and tunables | Technical Specification | Flag as "initial; tune with use" where true |
| Testing strategy, high-level milestones, release targets | Technical Specification | Milestones say what each proves; phase detail and enumerated test cases belong downstream (§2.4) |
| Phase sequencing, per-phase task breakdown, enumerated test cases | Implementation documents (§2.4) | Downstream of the suite; governed only at the boundary (G-11) |
| A question no current document can answer | Open Questions ledger | Name the resolving document (G-23) |

### 2.3 Writing order and the reason for it

- **G-8.** The documents are written in order: Requirements → Design
  Guideline → Technical Specification. The reason is directional
  constraint: design decisions (interaction, presentation, response
  behavior) constrain technical structure far more often than the reverse.
  The order exists so that each document is written against a stable
  upstream, and the reason is recorded here so the order is a rule, not a
  habit.
- **G-9.** Writing order is not reading order and not immutability:
  downstream documents feed changes upstream through the feedback protocol
  (G-24), and upstream documents version forward to absorb them.

### 2.4 Downstream of the suite: implementation documents

- **G-10.** The foundation documents are not the end of the paper trail;
  they are its upstream. Once the suite is approved, work proceeds through
  downstream documents, standard divide-and-conquer:
  1. **Implementation Plan** — phase-wise sequencing of the work,
     expanding the Technical Specification's milestone list into ordered
     phases with entry/exit conditions.
  2. **Per-phase to-do lists** — the execution detail for one phase at a
     time, written close to the work.
  3. **Test cases** — the enumerated checks that confirm expected
     behavior. The Specification owns the testing *strategy*; test-case
     documents enumerate the concrete cases, and each case cites the
     requirement it verifies — `FR`/`HC` items especially, since a test
     case is the most literal consumer of a testably phrased requirement
     (G-18).
  The division of labor mirrors the division of documents: the suite
  captures lead-level decisions, the Implementation Plan is senior-level
  breakdown, to-do lists and test cases drive hands-on execution and
  verification.
- **G-11.** The internal structure of implementation documents is
  **explicitly deferred** from this version of the standard (per its own
  G-20, the door kept open is this): implementation documents must cite
  foundation IDs and section references for every piece of work they
  schedule and every behavior they test, so that "why is this task here"
  and "what does this test confirm" are always answerable, and they may
  adopt the shared conventions of §3 (header block, open questions) as-is.
  Discoveries during implementation that change HOW flow into the
  Technical Specification as version bumps via the feedback protocol
  (G-24/G-25) — the Implementation Plan never becomes a shadow spec.

### 2.5 Versions, projects, and the suite's lifetime

- **G-12 — One living suite per project.** The suite's unit is the
  **project**, and the project's boundary is **shared requirements** — not
  the application. A project that ships multiple applications working
  together keeps one suite: the shared requirements are precisely what
  make it one project, and documents may group per-application content
  under headings where the applications diverge. Applications that do not
  share requirements are independent projects with independent suites,
  however related their branding or family name. Within its project, the
  suite lives for the project's whole life, evolving by version bumps —
  never a fresh suite per release version, and never "VERSION 2" sections
  layered inside a document. Each document always reads as the current
  truth of the project; a reader must never reconstruct the present by
  mentally merging historical layers. History is carried by the machinery
  that already exists for it: document version control, superseded
  versions kept on record (G-17), withdrawn-with-reason IDs (G-18), and
  the resolved-questions ledger (G-23).
- **G-13 — How a new version enters the suite.** A new development cycle
  (version 2 and beyond) re-enters the project flow at its start —
  idea dump and bouncing (§6.1) — and lands in the suite as version bumps
  under the existing rules: new features are new `FR-n` IDs (numbering
  continues, never restarts); features promoted from the **explicitly
  deferred** tier graduate to in-scope through the door G-20 required to
  be named; dropped features are withdrawn with reason, keeping their
  numbers; reversals of approved decisions are major bumps, additions are
  minor bumps (G-17). Document versions and release versions are
  independent counters — a document does not bump to match a release
  number, and correlation between the two is coincidence, not a rule.
- **G-14 — Release pinning.** Every release records, in its Implementation
  Plan, the exact foundation document versions it was built against
  (*"Release 1.0 implements Requirements v1.2, Design Guideline v1.0,
  Technical Specification v1.4"*). This pin is the as-built record: it
  answers "what exactly did version N ship?" without the foundation
  documents carrying per-release sections, and it is what makes G-12's
  always-current documents safe — the historical question has a designated
  home downstream.

## 3. Shared Conventions

### 3.1 Header block

- **G-15.** Every document begins with this header block, followed by the
  charter paragraph (G-4..6):

  ```
  # <Project> — <Document Name>

  **Version:** <major.minor>
  **Status:** approved | review | approved | superseded
  **Date:** <date of this version>
  **Owner:** <the person accountable for the WHAT of this document>
  **Companion documents:** <each companion, with its pinned version and
  its relationship: upstream / downstream / derived>
  ```

- **G-16 — Pinned companions.** Companion references pin an exact version
  and must be updated whenever a companion's version bumps. A stale
  companion reference is a defect of the referencing document. (*Example
  (reference project):* the failure this rule exists to prevent was
  observed there — a design document citing "Requirements v0.1 (upstream)"
  three versions after the requirements reached v0.3.)

### 3.2 Versioning and status lifecycle

- **G-17.** Versions are `major.minor`. Minor bumps: clarifications,
  additive decisions, absorbed feedback that does not reverse an approved
  decision. Major bumps: a reversal of an approved decision, or a
  restructuring that breaks section references. Any approved change bumps
  the version; documents are never edited in place without a bump once
  they have left `draft`. Status vocabulary: `draft` (owner still
  deciding), `review` (circulated), `approved` (normative; downstream work
  may rely on it), `superseded` (kept for the record; header names the
  successor). How review is conducted — who must see a document and what
  constitutes an objection — is team process, out of this standard's
  scope.

### 3.3 Identifiers

- **G-18.** Normative items that other documents will cite carry stable
  identifiers: `<CATEGORY>-<n>`. Three invariants: identifiers are
  immutable, never reused, and never deleted — a withdrawn item is marked
  withdrawn with a reason, keeping its number. Hard constraints get the
  strongest phrasing available: testable, and framed so that violation
  equals defect ("a release that violates any of them is defective by
  definition").
- **G-19 — Standard categories only.** The category set is standardized
  across all projects so that a prefix always means the same thing in
  every human's head:

  | Prefix | Meaning | Lives in |
  |---|---|---|
  | `HC-n` | Hard constraint | Requirements |
  | `FR-n` | Functional requirement | Requirements |
  | `A-n` | Architecture requirement | Requirements |
  | `C-n` | Configuration requirement | Requirements |
  | `S-n` | Stability / quality requirement | Requirements |
  | `G-n` | Rule of this standard | This handbook only |

  Functional requirements may be *grouped* into sections by concern while
  keeping one continuous `FR` numbering — the grouping is headings, not
  prefixes. Anything that does not fit a standard category takes **no
  identifier** and is referenced by section number and full name instead;
  ad-hoc project prefixes are not minted. The category set is reviewed
  against real use and extended only by a version bump of this standard.
  (Trade accepted knowingly — *Example (reference project):* its domain
  prefixes `P-n`/`T-n` were mnemonic within one project; cross-project
  consistency is worth more.)

### 3.4 Scope tiers

- **G-20.** Scope has three tiers, and the middle one is the load-bearing
  one: **in scope**, **explicitly deferred** (designed-for, not built —
  the current version must leave the named door open, and the requirement
  says which door; *Example (reference project):* "the tool abstraction
  must permit a future MCP adapter"), and **out of scope** (no door
  promised; ideally with the reason the line is drawn there). Deferrals
  without a designed-for clause are just out-of-scope items wearing
  optimism. The deferred tier is also the front door for future product
  versions: graduation from deferred to in-scope is the designed path for
  v2 features (G-13).

### 3.5 Open Questions ledger

- **G-21.** Every document ends with an Open Questions section listing what
  the document knowingly leaves unresolved.
- **G-22.** No orphan questions: every entry names the document (or
  process, e.g. "tune with real use") that will resolve it.
- **G-23.** Questions never evaporate. When a question is resolved, it
  moves to a "Resolved since v<X>" note in the same section, with a pointer
  to where the answer now lives.

### 3.6 Upstream feedback protocol

- **G-24.** When a downstream document makes a decision that adds to or
  refines an upstream document, it records that decision in a dedicated
  **feedback section**: what the decision is, which upstream item it
  extends, and why. The section is a permanent record, not a temporary
  inbox — it stays after absorption so the trace remains explicit.
  (*Example (reference project):* its Design Guideline's "Design-Driven
  Requirements Feedback" section.)
- **G-25.** The upstream owner absorbs feedback in the upstream document's
  next version bump, citing back to the downstream section. Downstream
  documents do not silently edit upstream ones, and upstream documents do
  not silently ignore feedback: absorption or rejection is recorded either
  way.

## 4. Per-Document Requirements

Each document type has required sections. Required means: required *when
the content exists*. A section with nothing to say is deleted, not stubbed
(G-27) — "required" governs where content goes, never manufactures content.

### 4.1 Requirements Document

Sections, in order: Purpose and Motivation (reasons in priority order) ·
Scope (three tiers, G-20) · Hard Constraints (`HC-n`) · functional
requirement sections grouped by concern (`FR-n`, continuous numbering per
G-19) · architecture / configuration / stability requirements (`A-n`,
`C-n`, `S-n`) as applicable · policy sections that are themselves
requirements (with justification, G-4) · Deliverables and Document Plan
(names the rest of the suite and the writing-order rationale, G-8) · Open
Questions.

Do: phrase requirements as observable behavior, and state the acceptance
standard inside the requirement where one exists (*Example (reference
project):* "approximate counting is acceptable; the requirement is a
reliable trigger, not exact counts"). Don't: specify mechanisms (library
names, file formats, screen layouts) unless the mechanism is the
requirement — and then say why.

### 4.2 Design Guideline

Sections, in order: Identity (name, brand concept, **anti-goals**) ·
visual language · layout · interaction patterns and response policy —
including how the product behaves in failure and under constrained
conditions, when the product's medium makes that a design question ·
the product's critical moment(s), each given its own section when it
deserves one (*Example (reference project):* the permission prompt — "the
most important screen in the product") · Voice and Language (including how
errors speak) · Key Moments (first run, start, end/failure) · the feedback
section (G-24) · Open Questions.

Do: state anti-goals — what the design must *not* become is as normative
as what it must be. Write prohibitions down explicitly "so they stay
forbidden," each with its reason. Give working values (colors, thresholds)
while labeling them as tunable, and centralize them so tuning is a value
change, not a refactor. Don't: let personality cost clarity; decorate;
write voice rules you don't follow in the document's own prose.

### 4.3 Technical Specification

Sections, in order: structure/layout of the system · concurrency or
execution model · data models · one section per major subsystem ·
configuration · error handling policy · dependencies (a locked initial
set, plus the acceptance policy inherited from Requirements) · build and
release · testing strategy · milestones · Open Items.

Do: cite the upstream ID for every choice that satisfies a requirement
("traceability is mechanical"). State defaults as initial values with a
tuning note where honest. Make milestones prove something, and say what
(*Example (reference project):* "M1 … *proves the core*") — the milestone
list is the seed the Implementation Plan (§2.4) expands, so it stays
high-level here. Don't: restate upstream content — cite it; leave a
technical consequence of a design decision unabsorbed; present a choice
without the requirement or reason behind it.

## 5. Voice and Density Rules

These rules apply to all documents in the suite, including this one.

- **G-26 — Every sentence is a decision or a rationale.** Documents record
  decisions and the reasons for them. Background prose, restated common
  knowledge, and hedged non-commitments are deleted in review.
- **G-27 — No stubs.** Sections exist only when they have content. Empty
  sections, "TBD" placeholders without an Open Questions entry, and "None"
  stubs are deleted.
- **G-28 — Rationale rides inline.** The reason for a decision appears in
  the same breath as the decision (*Example (reference project):* "vendor
  SDK crates are not used — they churn and reintroduce the
  third-party-dependence risk this project exists to avoid"). Separate
  "rationale" appendices decay; inline rationale survives editing.
- **G-29 — Honesty clauses.** Documents state what the product does *not*
  guarantee, in the same place they state what it does (*Example
  (reference project):* "this guarantees 'only git touches git's data,'
  not immortal history"). A limitation discovered during writing is
  written down, not smoothed over; where users could over-trust a
  mechanism, the document mandates honest labeling in the product itself.
- Style defaults: plain words over ceremony; short sentences carry
  decisions best; "silence about defaults, speech about deviations" — a
  document spends its reader's attention on what differs from the
  expected, not on what matches it.

## 6. Process

### 6.1 The project flow

The suite exists inside a working rhythm: (1) a broad idea and pain-point
dump, (2) idea bouncing and exploration, (3) the foundation documents in
the G-8 order, (4) implementation planning — the Implementation Plan,
per-phase to-do lists, and test cases of §2.4, (5) implementation against
them. Phases 1–2 are deliberately unstructured; this standard governs
phase 3 fully and phase 4 only at its boundary (G-11), so that phase 5 can
be delegated safely. Later versions re-enter this flow at phase 1
and land in the suite per G-13.

### 6.2 Drafting with AI agents

- Documents are drafted chunk by chunk with human checkpoints, not
  fire-and-forget. The human owner owns the WHAT of every document (the
  decisions); the agent may own the HOW (structure, phrasing, consistency,
  traceability bookkeeping).
- An agent drafting under this standard is given the **SFD Agent Guide**
  (the derived compact form) as working context, plus the upstream
  documents of whatever it is writing. Agents must route decisions per
  G-7 and must place decisions they cannot route into Open Questions
  rather than guessing a home.
- Status transitions (`draft → review → approved`) are made by the owner,
  never by an agent.

### 6.3 Maintaining the standard itself

- This handbook follows its own rules: header block, charter, stable IDs,
  Open Questions, versioning.
- **G-30 — Handbook/Agent Guide lockstep.** The Agent Guide is derived
  from this handbook and carries the same version number. Every normative
  line in the Agent Guide cites the G-n rule it compresses. A change to
  any G-n rule requires regenerating the Agent Guide in the same change;
  publishing one without the other is a defect. Drift is auditable
  mechanically: a G-n cited in one document and absent or contradicted in
  the other.
- Proposed changes to the standard are collected against the current
  version and absorbed in explicit version bumps, exactly as G-24/G-25
  prescribe for project documents.

## 7. Open Questions

- Whether and when the standard grows a part governing the internal
  structure of implementation documents (Implementation Plan, per-phase
  to-do lists, test cases), beyond the boundary rules of G-11. Resolve:
  after the practice is exercised end-to-end on the reference project
  (foundation docs → plan → phases → tests → code), then a version bump of
  this handbook.
- Whether the standard category table (G-19) needs additional categories.
  Resolve: periodic review against real projects; extensions land as
  minor bumps of this handbook.

Resolved since v0.1: identifier category conventions — standardized
company-wide (G-19); non-standard concepts take no identifier. Review
protocol — declared out of the standard's scope (§3.2). Implementation
feedback — the downstream pipeline is defined at its boundary (§2.4,
G-10/G-11).

Resolved since v0.2: multi-version lifecycle — one living suite per
project, where the project's boundary is shared requirements; bump-based
evolution; release pinning downstream (§2.5, G-12/G-13/G-14). Test cases —
added to the downstream pipeline as its third document type (G-10).
