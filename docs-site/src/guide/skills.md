# Skills

A skill is a named, self-contained capability folder the model can discover
and invoke on demand — instructions for doing some task well, optionally
bundled with supporting files or scripts. Skills are how you extend what
Emberly's agent knows how to do, without changing any code.

## Progressive disclosure

Every available skill's **name and description are always in context**, so
the model always knows what it can reach — but a skill's full instruction
body only loads when the model actually invokes it. This keeps the cost of
carrying many skills near zero until one is used, the same economy principle
behind [persistent memory](./memory.md).

## Where skills live

Two scopes, resolved at startup:

- **User-global** — `~/.config/emberly/skills/<name>/`. Always loaded,
  available in every project.
- **Project** — `.agents/skills/<name>/`. Loaded **only when the project
  root is trusted** (see [Safety: Permissions, Sandbox & Trust](./safety.md),
  workspace trust) — a skill from an untrusted folder is neither surfaced nor
  invocable.

If a user-global and a project skill share a name, **the project skill
wins** — the same precedence rule project config takes over global config.
The shadowed user-global skill is reported by `emberly config show`, so the
override is never a silent surprise.

## Using a skill

You don't invoke a skill directly — the model does, when it decides one is
relevant, via its own `skill` tool call. Your part is making skills available
and reviewing what they say:

- **`/skills`** opens a read-only inspector: `↑`/`↓` moves through the
  catalog, **Enter** fetches and shows a skill's full instructions plus its
  bundled resource paths, and `Esc`/`q` closes it. There's no edit or delete
  from here — a skill is an externally-authored folder; edit its files
  directly to change it.
- Reading a skill's instructions never runs anything. If a skill bundles a
  script, the model runs it as an ordinary **`bash`** call — fully gated by
  the normal permission rules and OS sandbox, exactly like any other command.
  A skill has no privileged path around that safety model.

## Writing your own skill

Create a folder named after your skill, containing a `SKILL.md`:

```
~/.config/emberly/skills/commit-messages/
└── SKILL.md
```

`SKILL.md` is TOML frontmatter (fenced with `+++`) giving a `name` and a
one-line `description`, followed by the markdown instruction body:

```markdown
+++
name = "commit-messages"
description = "Write a commit message in this project's house style."
+++

Write commit messages as:

- A single-line summary in the imperative mood, 50 characters or fewer.
- A blank line, then body paragraphs wrapping at 72 columns explaining
  *why*, not just what changed.
- No trailing period on the summary line.
```

`description` is what the model sees standing in context for every skill, so
make it specific enough to tell the model *when* to reach for this skill, not
just what it's called.

Any other file you put in the same folder — a template, a reference doc, a
helper script — is a **bundled resource**. It isn't loaded into context
automatically; it's listed by path alongside the instruction body when the
skill is invoked, so the model can `read_file` or `bash` it if the
instructions call for it.

### If your skill doesn't show up

A folder without a readable `SKILL.md`, or with frontmatter that isn't valid
TOML, is **silently skipped** — not cataloged, not invocable, and with no
warning shown anywhere. If a skill you wrote isn't appearing in `/skills`,
check first that:

- `SKILL.md` exists directly inside the skill's folder (not a subfolder).
- The frontmatter fence is exactly `+++` on its own line, opening and
  closing.
- `name` and `description` are both present and are valid TOML strings
  (quoted).

## Reusing a Claude / Anthropic Skill

Anthropic's public Agent Skills format is conceptually the same idea — a
folder, a name/description manifest, an instruction body, optional bundled
resources — but it isn't a drop-in match for Emberly's. The manifest syntax
differs: Anthropic's uses **YAML frontmatter fenced with `---`**, while
Emberly only understands **TOML frontmatter fenced with `+++`**. This is a
deliberate choice (avoiding a YAML dependency, and avoiding `---`'s ambiguity
with a markdown thematic break), not an oversight — but it does mean an
unmodified Claude Skill folder won't be recognized at all if you just copy it
in; it will hit the "silently skipped" case above.

Porting one over is usually a small, mechanical edit — convert the fence and
the field syntax, keep the body as-is:

```yaml
---
name: pdf-fill
description: Fill PDF forms with the given data.
license: MIT
---
```

becomes:

```toml
+++
name = "pdf-fill"
description = "Fill PDF forms with the given data."
+++
```

Fields Emberly's manifest doesn't read (like `license` above) are harmlessly
ignored if you leave them in valid TOML form — only `name` and `description`
are required. The instruction body and any bundled files underneath need no
changes.

## Config

`[skills] enabled` (default `true`) turns the whole skill system off — see
the [Config Keys Reference](../configuration/reference.md).
