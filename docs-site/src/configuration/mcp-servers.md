# MCP Servers

Emberly can connect to [MCP](https://modelcontextprotocol.io) servers over
stdio and register each one's tools alongside the built-ins — a first-party
JSON-RPC client, no vendor SDK:

```toml
[mcp.servers.myserver]
command = "npx"
args    = ["-y", "@my/mcp-server"]
```

Each discovered tool registers as `mcp__myserver__<tool>`, so the connected
server is always legible in the tool-activity line and the permission
prompt. A server declared in **project** config is trust-gated exactly like
a project skill — it is never even connected in an untrusted folder; a
server declared in your **global** config connects unconditionally. Connect
outcomes reconnect on `/reload`, and every MCP-sourced result is labeled
untrusted external content, the same as a web search result.

See [Using MCP Servers](../guide/mcp-usage.md) for how connected servers show
up once you're running a session.
