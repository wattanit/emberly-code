# Configuration

Everything is optional — Emberly runs on built-in defaults. Settings resolve in
this order, later winning over earlier:

1. Built-in defaults
2. Global config — `~/.config/emberly/config.toml` (or `$XDG_CONFIG_HOME/emberly/`)
3. Project config — `.agents/config.toml` in your project
4. `EMBERLY_*` environment variables
5. `--provider` / `--model` command-line flags

Run **`emberly init`** (from the shell) or **`/init`** (from inside a running
session) to scaffold `.agents/` with a commented `config.toml`, the default
prompts (editable, under `.agents/prompts/`), a `permissions.toml` template,
and a `.gitignore` that keeps session transcripts out of version control.
Existing files are never overwritten. Run **`emberly config show`** to see
the resolved settings and where each value came from.

The rest of this chapter covers:

- [Providers, Models & Keys](./providers.md) — wiring up Anthropic, OpenAI-compatible
  endpoints, or a local model.
- [MCP Servers](./mcp-servers.md) — connecting Model Context Protocol tool servers.
- [Config Keys Reference](./reference.md) — every other config key, with defaults.
- [Editing Config From a Session](./editing-in-session.md) — tuning things without
  leaving the TUI.
