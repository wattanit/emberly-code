# Editing Config and Prompts From a Session

You don't have to leave Emberly to tune it. **`/config`** opens
`.agents/config.toml` in your `$EDITOR`; **`/prompt [system|compact]`** opens a
prompt file (seeded from the baked-in default). Edits write to the **project
tier** only, and Emberly tells you whether you're editing an existing value or
creating a new override. On save the change is **applied to the running
session**; startup-only settings are named as needing a restart. **`/reload`**
re-reads everything on demand. In plain mode there's no editor handoff — the
commands print the path to edit yourself, and `/reload` applies it.

Adding a provider has its own guided path: from `/model`, the trailing "+ add
new provider…" row opens a step-by-step wizard (name → adapter → endpoint →
model id → key), ending in a summary screen with the key redacted to its last
4 characters before it writes anything. It writes the profile into the same
project `.agents/config.toml` and the key into `~/.config/emberly/keys.toml`,
then reloads exactly like a manual edit would — no separate success message,
no auto-switch (pick the new profile from `/model` afterward). Not available
in plain mode, which falls back to the same print-the-path-and-`/reload`
pattern as `/config`/`/prompt` above (no `$EDITOR` handoff either way in
plain mode).
