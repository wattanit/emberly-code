//! `bash` (T-4): run a shell command under a timeout, with a scrubbed
//! environment, always asking for permission in Phase 1 (the default
//! allowlist arrives with the rule engine in Phase 2).
//!
//! Safety (S-4): the child runs in its own process group and is killed if it
//! exceeds the timeout (via `kill_on_drop` when the wait future is dropped).
//! Killing the *entire* descendant tree needs a group signal, which requires a
//! syscall dependency vetted alongside the sandbox — that lands in Phase 2.
//! The timeout alone already guarantees the harness never hangs on a child.

use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::ctx::ToolCtx;
use crate::permission::PermissionRequest;
use crate::tool::{Tool, ToolOutcome, ToolSpec};

#[derive(Deserialize)]
struct BashArgs {
    command: String,
    #[serde(default)]
    timeout_secs: Option<u64>,
}

/// The `bash` tool. Timeouts and the environment allowlist are configurable.
pub struct BashTool {
    /// Default per-command timeout when the model does not specify one.
    pub timeout_default: Duration,
    /// Hard ceiling; a model-requested timeout is clamped to this.
    pub timeout_ceiling: Duration,
    /// Environment variables passed through to the child (Tech Spec §5.2);
    /// everything else in the harness's environment is scrubbed.
    pub env_allowlist: Vec<String>,
}

impl Default for BashTool {
    fn default() -> Self {
        Self {
            timeout_default: Duration::from_secs(120),
            timeout_ceiling: Duration::from_secs(600),
            env_allowlist: ["PATH", "HOME", "LANG", "TERM"]
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
        }
    }
}

impl BashTool {
    fn resolve_timeout(&self, requested: Option<u64>) -> Duration {
        requested
            .map(Duration::from_secs)
            .unwrap_or(self.timeout_default)
            .min(self.timeout_ceiling)
    }
}

/// A short one-line summary of a command for the permission prompt header.
fn summarize(command: &str) -> String {
    let first_line = command.lines().next().unwrap_or("").trim();
    let clipped: String = first_line.chars().take(60).collect();
    if clipped.len() < first_line.len() {
        format!("run: {clipped}…")
    } else {
        format!("run: {clipped}")
    }
}

fn render_output(output: &std::process::Output) -> ToolOutcome {
    let code = output.status.code();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    let mut body = match code {
        Some(c) => format!("exit code: {c}\n"),
        None => "exit code: (killed by signal)\n".to_string(),
    };
    if !stdout.is_empty() {
        body.push_str("--- stdout ---\n");
        body.push_str(&stdout);
        if !stdout.ends_with('\n') {
            body.push('\n');
        }
    }
    if !stderr.is_empty() {
        body.push_str("--- stderr ---\n");
        body.push_str(&stderr);
        if !stderr.ends_with('\n') {
            body.push('\n');
        }
    }

    let summary = match code {
        Some(c) => format!("exit {c}"),
        None => "killed".to_string(),
    };
    if matches!(code, Some(0)) {
        ToolOutcome::success(body, summary)
    } else {
        // A non-zero exit is a normal result the model should see (HC-6).
        ToolOutcome::failure(body, summary)
    }
}

#[async_trait]
impl Tool for BashTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "bash".into(),
            description: "Run a shell command from the project root under a timeout.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "Shell command to run via `sh -c`." },
                    "timeout_secs": { "type": "integer", "description": "Optional timeout in seconds (clamped to the configured ceiling)." }
                },
                "required": ["command"]
            }),
        }
    }

    async fn execute(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let args: BashArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolOutcome::failure(format!("invalid arguments: {e}"), "bad args"),
        };
        let timeout = self.resolve_timeout(args.timeout_secs);

        let request = PermissionRequest {
            tool: "bash".into(),
            summary: summarize(&args.command),
            detail: args.command.clone(),
            affected_paths: vec![],
            outside_root: false,
        };
        if !ctx.authorize(request).await.is_allowed() {
            return ToolOutcome::denied("run this command");
        }

        let mut command = tokio::process::Command::new("/bin/sh");
        command.arg("-c").arg(&args.command);
        command.current_dir(ctx.project_root());
        command.env_clear();
        for key in &self.env_allowlist {
            if let Ok(value) = std::env::var(key) {
                command.env(key, value);
            }
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);

        let child = match command.spawn() {
            Ok(child) => child,
            Err(e) => {
                return ToolOutcome::failure(
                    format!("failed to start command: {e}"),
                    "spawn failed",
                )
            }
        };

        match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Ok(Ok(output)) => render_output(&output),
            Ok(Err(e)) => ToolOutcome::failure(format!("command error: {e}"), "run failed"),
            // Timeout: the wait future (owning the child) is dropped here, and
            // kill_on_drop terminates the child leader.
            Err(_elapsed) => ToolOutcome::failure(
                format!(
                    "command timed out after {}s and was killed",
                    timeout.as_secs()
                ),
                "timed out",
            ),
        }
    }
}
