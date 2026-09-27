# Providers, Models & Keys

```toml
provider = "zai"                # a baked-in profile, or one you define below
model    = "glm-4.6"
```

**Adding a provider is configuration, not code** — a profile names a wire-format
`adapter` (`anthropic` or `openai`), an endpoint, and a key *reference*. The
easiest way in is the **guided setup wizard**: open the model picker
(`/model`) and pick the trailing "+ add new provider…" row — it walks you
through a name, adapter, endpoint, model id, and key (masked as you type),
then writes both files for you. Raw editing below remains the fallback for
anything the wizard's fixed field set doesn't cover (per-model metadata, a
non-standard auth scheme):

```toml
[providers.myserver]
adapter  = "openai"                          # or "anthropic"
base_url = "https://my-endpoint.example/v1"
auth     = { scheme = "bearer", key = "myserver" }   # → MYSERVER_API_KEY

# Optional per-model metadata → live cost estimate, /effort, and image/document input:
[providers.myserver.models."my-model"]
context_window = 128000
max_output     = 8192
pricing        = { input = 1.0, output = 2.0 }   # USD per million tokens
effort         = "medium"        # default reasoning level; enables /effort
effort_levels  = ["low", "medium", "high"]       # optional subset
vision         = true            # model accepts image input (P-11)
documents      = true            # model accepts document/PDF input (P-12)
```

Baked-in profiles can be tweaked the same way — set just the field you want to
change; the rest is kept. **API keys** are read from `<REF>_API_KEY` (e.g.
`ANTHROPIC_API_KEY`, `ZAI_API_KEY`) or from `~/.config/emberly/keys.toml` (a
flat `ref = "secret"` table, mode `0600`). Keys are never read from project
files, so they don't get committed.

**Project instructions:** an `AGENTS.md` (or `CLAUDE.md`) at your project root
is picked up automatically as standing context.
