# Using Connected MCP Servers

Once a `[mcp.servers.*]` is configured (see
[MCP Servers](../configuration/mcp-servers.md)), it connects at startup — a
quiet, one-line confirmation, never ceremony — and its tools appear in the
sidebar's **MCP** section (present only while at least one server is
connected). **`/mcp`** opens a read-only inspector: pick a server to see its
full discovered tool list, no round trip needed since it was all learned at
connect time. A tool call still asks permission exactly like a built-in
tool's, with the prompt naming the originating server; a connection that
fails is reported plainly and never blocks the rest of the session.
