# Interface & Commands

The screen is a conversation timeline — your messages and the agent's, tool
calls, and diffs — with a sidebar showing the model, context usage, cost, the
agent's task list, changed files, and any currently alive subagents. A fresh
session opens with a short pointer to the essentials (the command palette,
`/init`, `/model`, `/compact`, `/help`) instead of a blank pane.

## Input & keybindings

| Key | Action |
|---|---|
| `Enter` | Send your message |
| `Shift+Enter` / `Alt+Enter` | Newline (compose a multi-line message) |
| `Esc` | Cancel the current turn / dismiss an overlay |
| `Ctrl-C` | Cancel the current turn; quits when idle and the input line is empty |
| `↑` `↓` `PgUp` `PgDn` / mouse wheel | Scroll the conversation or an overlay |
| `Ctrl-R` | Toggle the reasoning trail open/closed |
| `Ctrl-P` | Open the command palette |
| `Tab` | Complete a partially typed `/command` name, when unambiguous |
| `Ctrl-L` | Force a full repaint (clears display corruption without resizing) |
| Mouse click | Select an interactive row / affordance (never approves a permission) |

## Commands

Open the palette with **`Ctrl-P`**, or type any `/name`. It's colored live as
you type: recognized once it matches a real command, dim while it's still a
plausible prefix, and flagged if nothing could ever match:

| Command | Key | What it does |
|---|---|---|
| `/new` (`/clear`) | | Start a fresh session (the current one is saved) |
| `/session` | | List saved sessions and switch to one |
| `/compact` | | Summarize older turns to reclaim context space |
| `/model` | | Switch the active provider/model (`/model <profile>` direct) |
| `/mode` | `Shift-Tab` | Pick a permission mode (`Shift-Tab` cycles) |
| `/effort` | | Set reasoning effort (`/effort low\|medium\|high\|max`) |
| `/attach <path>` | | Attach an image to the prompt you're composing |
| `/export <path>` | | Export this session to a self-contained HTML file |
| `/view` | | View the last assistant message in full |
| `/diff` | `Ctrl-O` | Open the latest file's diff |
| `/files` | | List files changed this session |
| `/sidebar` | `Ctrl-B` | Toggle the sidebar |
| `/cancel` | | Cancel the in-flight turn |
| `/init` | | Create `.agents/` (config, prompts, permissions) if missing |
| `/config` | | Edit `.agents/config.toml` in `$EDITOR` |
| `/prompt` | | Edit a prompt file (`/prompt system\|compact`) |
| `/reload` | | Re-read config & prompts from disk and apply them |
| `/memory` | | Inspect, edit, and delete stored memory |
| `/skills` | | List available skills and inspect a skill's instructions |
| `/agents` | | List currently alive subagents and inspect one's activity |
| `/mcp` | | List connected MCP servers and inspect a server's tools |
| `/help` | `Ctrl-P` | List commands and keybindings |
| `/quit` | `Ctrl-D` | Exit |

For the underlying safety model behind permission modes, see
[Safety: Permissions, Sandbox & Trust](./safety.md).
