//! `bash` (T-4): run a shell command under a timeout, with a scrubbed
//! environment, always asking for permission in Phase 1 (the default
//! allowlist arrives with the rule engine in Phase 2).
//!
//! Safety (S-4): the child runs in its own process group. On a timeout or a
//! cancel (the engine drops this future) a [`GroupKillGuard`] SIGKILLs the whole
//! process group — leader *and* the grandchildren an `sh -c` spawns — via
//! `rustix` (pure-Rust `linux_raw`, no libc). `kill_on_drop` remains as a
//! belt-and-braces leader kill and the non-unix fallback. The timeout alone
//! already guarantees the harness never hangs on a child.

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

/// SIGKILLs a child's entire process group on drop — leader plus the
/// grandchildren an `sh -c` spawns — reaping the descendants that
/// `kill_on_drop` (leader-only) would leave behind (S-4). It fires on both a
/// timeout (the wait future is dropped, then this is) and a cancel (the engine
/// drops the whole tool future). Disarmed after a clean wait so a since-reused
/// pgid is never signaled.
struct GroupKillGuard {
    /// The child's process-group id (equal to the leader pid, since the child
    /// leads a fresh group). `None` once disarmed. Only read on unix; kept
    /// cross-platform so the call sites need no `cfg`.
    #[cfg_attr(not(unix), allow(dead_code))]
    pgid: Option<i32>,
}

impl GroupKillGuard {
    fn arm(child_pid: Option<u32>) -> Self {
        Self {
            pgid: child_pid.and_then(|p| i32::try_from(p).ok()),
        }
    }

    fn disarm(&mut self) {
        self.pgid = None;
    }
}

impl Drop for GroupKillGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pgid) = self.pgid {
            if let Some(pid) = rustix::process::Pid::from_raw(pgid) {
                // Best-effort: an already-exited group yields ESRCH, ignored.
                let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
            }
        }
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
            description: "Run a shell command from the project root under a timeout. Most \
                          commands require a permission prompt; prefer the first-party tools \
                          (`read_file`, `grep`, `glob`) for reading and searching, which run \
                          without prompting."
                .into(),
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

    fn describe(&self, args: &Value) -> Option<String> {
        serde_json::from_value::<BashArgs>(args.clone())
            .ok()
            .map(|a| summarize(&a.command))
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

        // Ask the sandbox how to spawn: `/bin/sh -c …` directly when degraded,
        // or the self-exec confinement shim when the OS sandbox is active
        // (Tech Spec §6.2). The tool still owns everything else below.
        let invocation = ctx
            .sandbox()
            .bash_invocation(&args.command, ctx.project_root());
        let mut command = tokio::process::Command::new(&invocation.program);
        command.args(&invocation.args);
        command.current_dir(ctx.project_root());
        command.env_clear();
        for key in &self.env_allowlist {
            if let Ok(value) = std::env::var(key) {
                command.env(key, value);
            }
        }
        for (key, value) in &invocation.extra_env {
            command.env(key, value);
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

        // Arm the group-kill guard with the child's pid (== its pgid). It fires
        // on drop — on timeout below, or on cancel when the engine drops this
        // whole future — unless a clean wait disarms it.
        let mut group_kill = GroupKillGuard::arm(child.id());

        match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Ok(Ok(output)) => {
                group_kill.disarm();
                render_output(&output)
            }
            Ok(Err(e)) => {
                group_kill.disarm();
                ToolOutcome::failure(format!("command error: {e}"), "run failed")
            }
            // Timeout: leaving the guard armed, its drop SIGKILLs the entire
            // process group (leader + any `sh -c` grandchildren), not just the
            // leader that `kill_on_drop` reaches.
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
