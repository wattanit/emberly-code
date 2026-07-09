---
name: sfd-standard
description: Company standard (SFD — Software Foundation Documents) for writing and maintaining a project's foundation documents. Use this skill whenever creating, drafting, editing, reviewing, or version-bumping a Requirements Document, Design Guideline, or Technical Specification for any software project — and whenever the user mentions foundation documents, SFD, a requirements doc, design guideline, tech spec, documentation standard, starting documentation for a new project or a new product version, or asks whether project documents comply with the standard. Also consult it at the boundary when drafting downstream documents (implementation plan, phase to-do lists, test cases) that must cite foundation document IDs.
---

# SFD Standard — Agent Guide

This skill is the **SFD Agent Guide v1.0** in skill form: the compact,
normative rules for writing foundation documents under the company's
Software Foundation Documents standard. Each rule cites the handbook rule
(G-n) it compresses. The canonical source is bundled at
`references/sfd-handbook.md` (SFD Standard Handbook v1.0, approved) — read
it when a situation is not covered here, when the user asks about the
standard itself, or when rules seem to conflict. If this guide and the
handbook conflict, the handbook wins and the conflict must be reported to
the user as a defect. [G-30]

## The suite

Three foundation documents, each owning one question. Never answer another
document's question. [G-4..6]

1. **Requirements** — WHAT the product must do and WHY it exists.
   Behavioral statements only; an implementation constraint may appear only
   with a stated reason why it is itself a requirement.
2. **Design Guideline** — how the product looks, feels, and speaks:
   identity, visual language, layout, interaction patterns, how the
   product responds to the user in normal use, in failure, and under
   constrained conditions, voice and wording, key user moments. Where a
   design decision has technical consequences, Design decides and the
   Spec absorbs.
3. **Technical Specification** — HOW it is built. Every choice satisfying
   an upstream requirement cites that requirement's ID.

Write them in this order: Requirements → Design → Spec (design constrains
structure more than the reverse). [G-8]

Scale rule: small project → one document, three charter-keeping parts. No
human-facing surface → omit the Design Guideline, fold voice/naming into
Requirements. Never produce three documents that each answer a bit of
everything. [G-2]

Downstream of the suite come implementation documents: a phase-wise
Implementation Plan expanding the Spec's milestones, per-phase to-do
lists, and test cases that confirm expected behavior. Their internal
structure is not governed by this standard yet, but their boundary is:
every scheduled task and every test case cites foundation IDs (a test case
cites the requirement it verifies — FR/HC especially), and implementation
discoveries that change HOW flow back as Spec version bumps, never into a
shadow spec. [G-10, G-11]

## Versions, projects, and the suite's lifetime

- One living suite per **project**, and the project's boundary is
  **shared requirements** — not the application. Multiple applications
  working together with shared requirements share one suite (group
  per-application content under headings where they diverge); applications
  that share no requirements are independent projects with independent
  suites, whatever their branding. Never start a fresh suite for version
  2, and never add "VERSION 2" sections inside a document — each document
  always reads as the current truth. [G-12]
- New versions land as version bumps: new features are new FR-n IDs
  (numbering continues, never restarts); deferred-tier items graduate to
  in-scope through their named door; dropped features are withdrawn with
  reason, keeping their numbers; reversals are major bumps, additions
  minor. [G-13, G-20, G-17]
- Document versions and release versions are independent counters. [G-13]
- Each release's Implementation Plan pins the exact foundation versions it
  was built against — that pin, not the foundation documents, is the
  as-built record of what a release shipped. [G-14]

## Routing decisions

Route every decision to its home [G-7]:

- Why the product exists, ordered priorities; testable musts; scope tiers;
  requirement-grade constraints (with justification) → **Requirements**
- Name, identity, anti-goals; visual language; layout; interaction
  patterns and user-facing response policy (normal, failure, constrained);
  voice and wording → **Design Guideline**
- Architecture; module boundaries; data models; dependencies; defaults
  and tunables; testing strategy; high-level milestones; release →
  **Spec**
- Phase sequencing, task breakdown, enumerated test cases →
  **implementation documents**, not the suite [G-10]
- Cannot route it → **Open Questions** of the current document, naming the
  resolver. Never guess a home. [G-22]

## Mandatory mechanics

- **Header block** on every document: Version, Status
  (draft/review/approved/superseded), Date, Owner, Companion documents
  with pinned versions and relationship (upstream/downstream/derived).
  Update pinned companion versions whenever a companion bumps — a stale
  pin is a defect. [G-15, G-16]
- **Charter paragraph** directly after the header: what this document
  owns, what it defers, and to which document. [G-4..6]
- **Stable IDs from the standard categories only:** `HC-n` hard
  constraint, `FR-n` functional requirement, `A-n` architecture, `C-n`
  configuration, `S-n` stability/quality (`G-n` is reserved for the
  standard itself). Group FRs by concern under headings, but keep one
  continuous FR numbering — never mint project-specific prefixes.
  Anything outside these categories takes no ID; refer to it by section
  number and full name. IDs are immutable, never reused, never deleted
  (withdraw with reason instead). Hard constraints get defect-grade
  phrasing ("a release that violates any of them is defective").
  [G-18, G-19]
- **Scope in three tiers:** in scope · explicitly deferred (must name the
  door being kept open) · out of scope (with the reason). A deferral
  without a designed-for clause is out-of-scope wearing optimism. The
  deferred tier is the designed entry path for future-version features.
  [G-20]
- **Open Questions** section ends every document. Every entry names its
  resolver. Resolved entries move to "Resolved since v<X>" with a pointer;
  they never silently disappear. [G-21..23]
- **Upstream feedback:** when your downstream decision extends or refines
  an upstream document, record it in a dedicated feedback section (what,
  which upstream ID, why). Never edit the upstream document directly; the
  upstream owner absorbs it at the next bump. The feedback section is a
  permanent record. [G-24, G-25]
- **Versioning:** minor = additive/clarifying; major = reversal or
  reference-breaking restructure. Never change an approved document
  without a bump. Review process itself is team business, outside the
  standard. [G-17]

## Writing rules

- Every sentence is a decision or a rationale. Delete background prose,
  restated common knowledge, and hedged non-commitments. [G-26]
- No stubs. A section with nothing to say is deleted — no "TBD", no
  "None", no empty headings. [G-27]
- Rationale rides inline, in the same breath as the decision ("X is not
  used — it reintroduces the risk this project exists to avoid"). [G-28]
- Write honesty clauses: state what is *not* guaranteed next to what is.
  Write prohibitions down explicitly, each with its reason, so they stay
  forbidden. [G-29]
- Phrase requirements as observable behavior, with the acceptance standard
  inside the requirement where one exists. [handbook §4.1]
- In the Spec, cite upstream IDs for every satisfying choice; label
  defaults "initial; tune with use" where honest; make each milestone
  state what it proves and keep it high-level — expansion belongs to the
  Implementation Plan. [handbook §4.3, G-10]
- Anything you present as an illustration must be marked as an example;
  examples carry no normative weight. [G-3]
- Silence about defaults, speech about deviations. [handbook §5]

For the required section list of each document type, read handbook §4
(§4.1 Requirements, §4.2 Design Guideline, §4.3 Technical Specification)
before drafting that document.

## Your role as an agent

- The human owner owns the WHAT (all decisions). You own the HOW:
  structure, phrasing, consistency, traceability bookkeeping. [handbook
  §6.2]
- Work chunk by chunk against human checkpoints; do not draft
  fire-and-forget past a checkpoint. [handbook §6.2]
- Never change a document's Status; that is the owner's act. [handbook
  §6.2]
- If asked to decide something that belongs to the owner, surface it as a
  decision point or an Open Questions entry instead of deciding. [G-22]

## Workflows

### Drafting a new foundation document

1. Locate and read the upstream documents (Requirements before drafting
   Design; Requirements and Design before drafting the Spec). If an
   upstream document is missing or not `approved`, tell the owner before
   drafting downstream content against it.
2. Read handbook §4 for the target document's required sections.
3. Propose a section plan to the owner, then draft chunk by chunk with
   checkpoints.
4. Route every decision per the routing table; unroutable decisions go to
   Open Questions with a named resolver.
5. Finish with the header block fully populated (Status: `draft`) and
   companion versions pinned.

### Editing or bumping an existing document

1. Read the current document's header. If Status is `approved` or
   `review`, changes require a version bump — never edit in place. While
   `draft`, in-place editing is allowed. [G-17]
2. New requirement-grade content gets new IDs (numbering continues);
   removed content is withdrawn with reason; resolved open questions move
   to the "Resolved since" ledger with pointers.
3. After any bump, update the pinned companion references in every
   document that cites this one. [G-16]
4. If the change originated downstream, verify it is recorded in that
   document's feedback section before absorbing it upstream. [G-24, G-25]

### Compliance review of an existing document

Check, in order: header block complete and companions pinned to current
versions · charter paragraph present · IDs from standard categories only,
no project-minted prefixes · scope in three tiers with named doors on
deferrals · Open Questions present, every entry with a resolver · no stubs
or empty sections · rationale inline on decisions · no content answering
another document's question (routing violations). Report findings with
the G-n each violates.
