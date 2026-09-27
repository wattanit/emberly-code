# Quick Start

1. **Pick a provider and give it a key.** Emberly ships four baked-in
   profiles, ready to use as soon as they have a key (the `local` one needs
   none). Pick the one you want — you only need one of these blocks:

   **Anthropic:**

   ```sh
   export EMBERLY_PROVIDER=anthropic
   export EMBERLY_MODEL=claude-sonnet-5
   export ANTHROPIC_API_KEY=sk-...
   ```

   **OpenAI:**

   ```sh
   export EMBERLY_PROVIDER=openai
   export EMBERLY_MODEL=gpt-5.1
   export OPENAI_API_KEY=sk-...
   ```

   **Z.ai** (the Z.ai coding plan, OpenAI-compatible):

   ```sh
   export EMBERLY_PROVIDER=zai
   export EMBERLY_MODEL=glm-4.6
   export ZAI_API_KEY=...
   ```

   **Local** (Ollama, vLLM, or anything else OpenAI-compatible at
   `http://localhost:11434/v1` — keyless):

   ```sh
   export EMBERLY_PROVIDER=local
   export EMBERLY_MODEL=llama3
   ```

2. **Run it in your project directory:**

   ```sh
   cd ~/my-project
   emberly
   ```

3. Type your task and press **Enter**. Emberly asks before running commands or
   editing files; you approve or deny each action.

The first time you run Emberly in a folder it will ask whether you **trust** the
code there (the safe default is *decline*). Without any provider configured,
Emberly still starts in an **offline placeholder mode** so you can explore the
interface. To keep settings with a project instead of in your shell, run
`emberly init`.

For everything you can set — providers, models, keys, MCP servers, and every
config key — see [Configuration](./configuration/index.md).

## Terminal font, if you work in Thai/CJK/etc.

Emberly's own text handling is fully Unicode-correct — grapheme clusters,
column widths, and cursor motion are right for Thai combining marks and wide
CJK glyphs alike (Requirements §2.1) — but glyph *rendering* is entirely your
terminal's and its font's job, not this app's. Most programming monospace
fonts (JetBrains Mono, Fira Code, Hack, Cascadia Code, Iosevka) don't include
Thai glyphs at all; your terminal falls back to a system font for those
codepoints, which is normal and not a bug. If that fallback looks wrong
(misaligned marks, inconsistent spacing), make sure font fallback is enabled
in your terminal — iTerm2, Kitty, WezTerm, and Windows Terminal all do this by
default — and that a proper Thai font is actually installed (e.g. **Noto Sans
Thai**, **Sarabun**, or **Consolas** on Windows).
