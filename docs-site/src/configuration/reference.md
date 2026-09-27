# Config Keys Reference

Every key below is the exact TOML name to set — no abbreviations — so you
never have to go spelunking in source to find one (e.g. context tuning:
`[context] keep_recent_turns`). Provider/model keys (`[providers.<name>]` and
friends, see [Providers, Models & Keys](./providers.md)) and per-server MCP
keys (see [MCP Servers](./mcp-servers.md)) are introduced with worked examples
in their own pages; this table is the complete reference for everything else.

| Key | Default | What it controls |
|---|---|---|
| `reasoning` | `collapsed` | Thinking-trail view: `collapsed` / `expanded` / `hidden` |
| `[ui] tool_explanations` | `true` | One-line captions under non-obvious tool calls |
| `[ui] mouse` | `true` | Wheel-scroll + click-to-select in the rich TUI |
| `[trust] trusted_dirs` | `[]` | Folders pre-approved for the trust gate (**global config only**) |
| `[loop] enabled` | `true` | Loop-breaking guardrail on/off |
| `[loop] repeat_window` | `3` | Consecutive turns repeating the *same* tool-call signature before it trips |
| `[loop] max_no_progress_turns` | `6` | Consecutive no-progress turns before it trips, even if the calls vary |
| `[completion] enabled` | `true` | Completion gate on/off — inert until a check is registered |
| `[completion] max_attempts` | `3` | Failed completion attempts allowed before halting to you |
| `[[completion.check]]` | — | One registered pass/fail check: `name`, `command`, `expect_exit` (default `0`) |
| `[context] window_turns` | `40` | Trailing turns sent to the provider; older ones are elided behind one marker |
| `[context] keep_recent_turns` | `6` | Trailing turns `/compact` (manual or automatic) keeps verbatim |
| `[context] auto_compact` | `true` | Automatic compaction on/off |
| `[context] auto_compact_threshold` | `0.85` | Context-usage fraction that triggers auto-compaction; must be in `(0.0, 1.0]` |
| `[context] pin_task_list` | `true` | Keep the agent's task list pinned in the sent context |
| `[truncate] reduce` | `true` | Salient reduction of tool output on/off |
| `[truncate] max_lines` | `400` | Truncate tool output once it exceeds this many lines |
| `[truncate] max_bytes` | `65536` (64 KiB) | …or this many bytes |
| `[truncate] head_lines` | `150` | Lines of head kept when truncating |
| `[truncate] tail_lines` | `100` | Lines of tail kept when truncating |
| `[image] max_bytes` | `5242880` (5 MiB) | Max size for an image read into the conversation |
| `[image] max_attachments` | `4` | Max images attachable to a single prompt via `/attach` |
| `[document] max_bytes` | `33554432` (32 MiB) | Max size for a PDF read into the conversation |
| `[memory] enabled` | `true` | Persistent cross-session memory on/off |
| `[memory] max_index_entries` | `50` | Soft warn threshold for memory index growth (does not truncate) |
| `[skills] enabled` | `true` | The skill system on/off |
| `[agents] enabled` | `true` | Multi-agent delegation on/off |
| `[agents] max_concurrent` | `3` | Ceiling on subagents alive at once per session |
| `[agents] spawn_timeout_secs` | `600` | Seconds `spawn_agents`/`message_agent` wait before reporting "still running" |
| `[agents] idle_timeout_secs` | `1800` | Seconds a subagent may go without a message before it's reclaimed as idle |
| `[mcp] enabled` | `true` | The MCP subsystem overall — a kill switch above each server's own `enabled` |
| `[mcp.servers.<name>] transport` | `"stdio"` | Wire mechanism (only `"stdio"` is supported this version) |
| `[mcp.servers.<name>] command` / `args` | — | Launch command and arguments for a stdio server |
| `[mcp.servers.<name>] enabled` | `true` | This specific server on/off |
| `[search] enabled` | `true` | Register the `web_search` tool |
| `[search] adapter` | — | Response-shape parser: `brave` / `tavily` / `searxng` / `json` |
| `[search] endpoint` | — | The search service endpoint URL |
| `[search] auth` | — | `{ scheme, key }`, the same shape as a provider's `auth` |
| `[search] max_results` | `5` | Results sent to the model per search |
| `[stream] first_chunk_secs` | `300` | Seconds to wait for a completion stream's first chunk — raise it for a slow backend |
| `[stream] idle_secs` | `90` | Seconds allowed between later chunks before the stream is considered dead |
| `[sandbox] require` | `false` | Refuse to start without active OS confinement |

**Environment variables:** `EMBERLY_PROVIDER`, `EMBERLY_MODEL`. For display,
`NO_COLOR` or `TERM=dumb` force plain mode and `EMBERLY_MOTION=0` disables
animation.
