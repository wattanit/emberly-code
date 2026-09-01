//! `McpClient` (FR-11, Tech Spec §5.6): a first-party, newline-delimited
//! JSON-RPC-over-stdio client — the default implementation of
//! [`emberly_tools::McpTransport`] in this version, built entirely on
//! already-locked dependencies (`tokio::process`, `serde_json`). No vendor
//! SDK crate (P-4's own rule, extended here): a remote transport (SSE/HTTP),
//! if ever added, is a second `McpTransport` impl behind the same trait, not
//! a rewrite of this one.
//!
//! Connecting, discovering tools, and registering them into the
//! `ToolRegistry` is composition-root logic (`emberly` binary,
//! `provider_setup::build_tool_registry`) — mirroring exactly how the
//! `web_search` tool is conditionally built there (Tech Spec §5.5). This
//! module owns only the wire client; it holds no config, no trust decision,
//! and no registry.

use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use emberly_tools::{McpError, McpToolSpec, McpTransport};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::Mutex;

/// How long to wait for a response before giving up (initial; tune with use,
/// Requirements §13) — a hung server must not hang the harness (HC-3, the
/// same principle S-4 already states for a child process).
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);

struct ClientIo {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    /// Kept only to hold the child alive; `kill_on_drop` reclaims it if the
    /// client is ever dropped without an explicit shutdown.
    #[allow(dead_code)]
    child: Child,
}

/// A connected MCP server reached over its stdio pipes (Tech Spec §5.6).
/// Calls are serialized behind one lock — correctness over throughput for
/// this initial version (Requirements §13, tune with use); a slow server
/// only slows its own calls, never corrupts another call's response.
pub struct McpClient {
    io: Mutex<ClientIo>,
    next_id: AtomicU64,
}

impl McpClient {
    /// Spawn `command args...` and perform the MCP `initialize` handshake.
    /// A spawn failure (binary not found, not executable) or a handshake
    /// failure both return a structured [`McpError`] — never a panic.
    pub async fn spawn(command: &str, args: &[String]) -> Result<Self, McpError> {
        let mut child = tokio::process::Command::new(command)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| McpError::Connect(format!("spawning '{command}': {e}")))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| McpError::Connect("no stdin pipe on the spawned process".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| McpError::Connect("no stdout pipe on the spawned process".into()))?;

        let client = Self {
            io: Mutex::new(ClientIo {
                stdin,
                stdout: BufReader::new(stdout),
                child,
            }),
            next_id: AtomicU64::new(1),
        };
        client
            .call(
                "initialize",
                json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {"name": "emberly", "version": env!("CARGO_PKG_VERSION")},
                }),
            )
            .await?;
        Ok(client)
    }

    /// `tools/list` — this server's tools, before namespacing (Tech Spec
    /// §5.6). A tool entry missing a `name` is skipped (malformed data from
    /// the server, not a reason to fail the whole discovery).
    pub async fn list_tools(&self) -> Result<Vec<McpToolSpec>, McpError> {
        let result = self.call("tools/list", json!({})).await?;
        let tools = result
            .get("tools")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(tools
            .into_iter()
            .filter_map(|t| {
                let name = t.get("name")?.as_str()?.to_string();
                Some(McpToolSpec {
                    name,
                    description: t
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    input_schema: t
                        .get("inputSchema")
                        .cloned()
                        .unwrap_or_else(|| json!({"type": "object"})),
                })
            })
            .collect())
    }
}

#[async_trait]
impl McpTransport for McpClient {
    async fn call(&self, method: &str, params: Value) -> Result<Value, McpError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let mut line =
            serde_json::to_string(&request).map_err(|e| McpError::Protocol(e.to_string()))?;
        line.push('\n');

        let mut io = self.io.lock().await;
        io.stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| McpError::Connect(e.to_string()))?;
        io.stdin
            .flush()
            .await
            .map_err(|e| McpError::Connect(e.to_string()))?;

        match tokio::time::timeout(RESPONSE_TIMEOUT, read_matching_response(&mut io.stdout, id))
            .await
        {
            Ok(result) => result,
            Err(_) => Err(McpError::Connect("timed out waiting for a response".into())),
        }
    }
}

/// Read lines from `stdout` until one is a JSON-RPC response matching `id` —
/// skipping anything else (a notification, a stale/malformed line) rather
/// than failing on the first thing that isn't our answer.
async fn read_matching_response(
    stdout: &mut BufReader<ChildStdout>,
    id: u64,
) -> Result<Value, McpError> {
    loop {
        let mut buf = String::new();
        let n = stdout
            .read_line(&mut buf)
            .await
            .map_err(|e| McpError::Connect(e.to_string()))?;
        if n == 0 {
            return Err(McpError::Connect("server closed the connection".into()));
        }
        let Ok(value) = serde_json::from_str::<Value>(buf.trim()) else {
            continue;
        };
        if value.get("id").and_then(Value::as_u64) != Some(id) {
            continue;
        }
        if let Some(error) = value.get("error") {
            return Err(McpError::Protocol(error.to_string()));
        }
        return Ok(value.get("result").cloned().unwrap_or(Value::Null));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny script acting as an MCP server: replies to `initialize` and
    /// `tools/list`, echoes anything else back as its own `tools/call`
    /// result — enough to exercise the real client end to end without a
    /// network dependency or an external fixture binary.
    fn fake_server_script() -> &'static str {
        r#"
import sys, json
for line in sys.stdin:
    req = json.loads(line)
    method = req.get("method")
    if method == "initialize":
        result = {"protocolVersion": "2024-11-05"}
    elif method == "tools/list":
        result = {"tools": [{"name": "echo", "description": "Echoes its input", "inputSchema": {"type": "object"}}]}
    elif method == "tools/call":
        result = {"echoed": req.get("params", {}).get("arguments")}
    else:
        result = {}
    sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": req["id"], "result": result}) + "\n")
    sys.stdout.flush()
"#
    }

    fn python_available() -> Option<&'static str> {
        ["python3", "python"].into_iter().find(|candidate| {
            std::process::Command::new(candidate)
                .arg("--version")
                .output()
                .is_ok_and(|o| o.status.success())
        })
    }

    #[tokio::test]
    async fn spawn_discover_and_call_a_real_stdio_server() {
        let Some(python) = python_available() else {
            eprintln!("skipping: no python3/python interpreter available in this environment");
            return;
        };
        let client = match McpClient::spawn(
            python,
            &["-c".to_string(), fake_server_script().to_string()],
        )
        .await
        {
            Ok(c) => c,
            Err(e) => panic!("expected a successful handshake: {e}"),
        };

        let tools = match client.list_tools().await {
            Ok(t) => t,
            Err(e) => panic!("expected tools/list to succeed: {e}"),
        };
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "echo");

        let result = match client
            .call(
                "tools/call",
                json!({"name": "echo", "arguments": {"hello": "world"}}),
            )
            .await
        {
            Ok(r) => r,
            Err(e) => panic!("expected tools/call to succeed: {e}"),
        };
        assert_eq!(result["echoed"]["hello"], "world");
    }

    #[tokio::test]
    async fn spawning_a_nonexistent_command_is_a_structured_connect_error() {
        let err = match McpClient::spawn("emberly-mcp-fixture-that-does-not-exist", &[]).await {
            Ok(_) => panic!("expected a spawn failure"),
            Err(e) => e,
        };
        assert!(matches!(err, McpError::Connect(_)));
    }
}
