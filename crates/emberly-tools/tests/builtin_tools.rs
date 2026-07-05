//! Behavior coverage for the built-in tools (Phase 1, group 4): read, write,
//! edit, bash. Each test runs against a fresh temp project directory.
//!
//! No `.unwrap()`/`.expect()`: setup failures `panic!` with context; tools
//! never return `Err` (HC-6), so assertions read fields directly.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use emberly_tools::{
    BashTool, EditFileTool, PermissionGate, PermissionOutcome, PermissionRequest, ReadFileTool,
    Tool, ToolCtx, TruncateConfig, WriteFileTool,
};
use serde_json::json;

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_project() -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("emberly-tools-{}-{}", std::process::id(), n));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        panic!("failed to create temp project dir: {e}");
    }
    dir
}

fn write_file(root: &Path, rel: &str, contents: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            panic!("failed to create dir: {e}");
        }
    }
    if let Err(e) = std::fs::write(&path, contents) {
        panic!("failed to seed file {rel}: {e}");
    }
}

fn read_back(root: &Path, rel: &str) -> String {
    match std::fs::read_to_string(root.join(rel)) {
        Ok(s) => s,
        Err(e) => panic!("failed to read back {rel}: {e}"),
    }
}

struct AllowGate;
#[async_trait]
impl PermissionGate for AllowGate {
    async fn authorize(&self, _request: PermissionRequest) -> PermissionOutcome {
        PermissionOutcome::Allow
    }
}

struct DenyGate;
#[async_trait]
impl PermissionGate for DenyGate {
    async fn authorize(&self, _request: PermissionRequest) -> PermissionOutcome {
        PermissionOutcome::Deny
    }
}

fn ctx(root: &Path, allow: bool) -> ToolCtx {
    let gate: Arc<dyn PermissionGate> = if allow {
        Arc::new(AllowGate)
    } else {
        Arc::new(DenyGate)
    };
    ToolCtx::new(root.to_path_buf(), TruncateConfig::default(), gate)
}

// ---- read ----------------------------------------------------------------

#[tokio::test]
async fn read_returns_file_contents() {
    let root = temp_project();
    write_file(&root, "hello.txt", "สวัสดี\nworld\n");
    let outcome = ReadFileTool
        .execute(json!({ "path": "hello.txt" }), &ctx(&root, true))
        .await;
    assert!(outcome.ok);
    assert_eq!(outcome.content, "สวัสดี\nworld\n");
    assert!(outcome.summary.contains("2 lines"));
}

#[tokio::test]
async fn read_missing_file_is_a_failure_not_a_crash() {
    let root = temp_project();
    let outcome = ReadFileTool
        .execute(json!({ "path": "nope.txt" }), &ctx(&root, true))
        .await;
    assert!(!outcome.ok);
    assert!(outcome.content.contains("nope.txt"));
}

// ---- write ---------------------------------------------------------------

#[tokio::test]
async fn write_creates_file_and_reports_line_delta() {
    let root = temp_project();
    let outcome = WriteFileTool
        .execute(
            json!({ "path": "src/new.rs", "content": "a\nb\nc\n" }),
            &ctx(&root, true),
        )
        .await;
    assert!(outcome.ok);
    assert_eq!(read_back(&root, "src/new.rs"), "a\nb\nc\n");
    let change = outcome.file_change.expect_none_marker();
    assert_eq!(change.adds, 3);
    assert_eq!(change.dels, 0);
    assert_eq!(change.path, "src/new.rs");
}

#[tokio::test]
async fn write_refuses_git_directory() {
    let root = temp_project();
    // Even with an allow-everything gate, .git/ is a hard tool-layer refusal.
    let outcome = WriteFileTool
        .execute(
            json!({ "path": ".git/config", "content": "evil" }),
            &ctx(&root, true),
        )
        .await;
    assert!(!outcome.ok);
    assert!(outcome.content.contains(".git/"));
}

#[tokio::test]
async fn write_denied_by_gate_returns_denied() {
    let root = temp_project();
    let outcome = WriteFileTool
        .execute(
            json!({ "path": "x.txt", "content": "hi" }),
            &ctx(&root, false),
        )
        .await;
    assert!(!outcome.ok);
    assert!(outcome.content.contains("denied"));
    assert!(
        !root.join("x.txt").exists(),
        "denied write must not touch disk"
    );
}

// ---- edit ----------------------------------------------------------------

#[tokio::test]
async fn edit_replaces_unique_match() {
    let root = temp_project();
    write_file(&root, "f.txt", "one\ntwo\nthree\n");
    let outcome = EditFileTool
        .execute(
            json!({ "path": "f.txt", "old_string": "two", "new_string": "TWO" }),
            &ctx(&root, true),
        )
        .await;
    assert!(outcome.ok, "{}", outcome.content);
    assert_eq!(read_back(&root, "f.txt"), "one\nTWO\nthree\n");
}

#[tokio::test]
async fn edit_no_match_reports_closest_line() {
    let root = temp_project();
    write_file(&root, "f.txt", "let value = compute();\n");
    let outcome = EditFileTool
        .execute(
            json!({ "path": "f.txt", "old_string": "let values = compute();", "new_string": "x" }),
            &ctx(&root, true),
        )
        .await;
    assert!(!outcome.ok);
    assert!(outcome.content.contains("no match"));
    assert!(
        outcome.content.contains("closest existing line"),
        "T-3 hint: {}",
        outcome.content
    );
}

#[tokio::test]
async fn edit_multiple_matches_without_replace_all_is_rejected() {
    let root = temp_project();
    write_file(&root, "f.txt", "x\nx\nx\n");
    let outcome = EditFileTool
        .execute(
            json!({ "path": "f.txt", "old_string": "x", "new_string": "y" }),
            &ctx(&root, true),
        )
        .await;
    assert!(!outcome.ok);
    assert!(outcome.content.contains("3 matches"));
    assert_eq!(
        read_back(&root, "f.txt"),
        "x\nx\nx\n",
        "rejected edit is a no-op"
    );
}

#[tokio::test]
async fn edit_replace_all_replaces_every_occurrence() {
    let root = temp_project();
    write_file(&root, "f.txt", "x\nx\nx\n");
    let outcome = EditFileTool
        .execute(
            json!({ "path": "f.txt", "old_string": "x", "new_string": "y", "replace_all": true }),
            &ctx(&root, true),
        )
        .await;
    assert!(outcome.ok);
    assert_eq!(read_back(&root, "f.txt"), "y\ny\ny\n");
}

// ---- bash ----------------------------------------------------------------

#[tokio::test]
async fn bash_echo_succeeds() {
    let root = temp_project();
    let outcome = BashTool::default()
        .execute(json!({ "command": "echo hello" }), &ctx(&root, true))
        .await;
    assert!(outcome.ok);
    assert!(outcome.content.contains("hello"));
    assert!(outcome.content.contains("exit code: 0"));
}

#[tokio::test]
async fn bash_nonzero_exit_is_failure_data() {
    let root = temp_project();
    let outcome = BashTool::default()
        .execute(json!({ "command": "exit 3" }), &ctx(&root, true))
        .await;
    assert!(!outcome.ok);
    assert!(outcome.content.contains("exit code: 3"));
}

#[tokio::test]
async fn bash_times_out_and_is_killed() {
    let root = temp_project();
    let outcome = BashTool::default()
        .execute(
            json!({ "command": "sleep 5", "timeout_secs": 1 }),
            &ctx(&root, true),
        )
        .await;
    assert!(!outcome.ok);
    assert!(outcome.summary.contains("timed out"));
}

#[tokio::test]
async fn bash_environment_is_scrubbed() {
    // A non-allowlisted secret in the harness env must not reach the child.
    std::env::set_var("EMBERLY_SECRET_LEAK_CHECK", "leak");
    let root = temp_project();
    let outcome = BashTool::default()
        .execute(
            json!({ "command": "printf 'PATH=%s SECRET=%s' \"$PATH\" \"$EMBERLY_SECRET_LEAK_CHECK\"" }),
            &ctx(&root, true),
        )
        .await;
    assert!(outcome.ok, "{}", outcome.content);
    assert!(
        !outcome.content.contains("leak"),
        "secret leaked: {}",
        outcome.content
    );
    assert!(
        outcome.content.contains("PATH=/"),
        "PATH should pass through: {}",
        outcome.content
    );
}

#[tokio::test]
async fn bash_denied_by_gate() {
    let root = temp_project();
    let outcome = BashTool::default()
        .execute(json!({ "command": "echo hi" }), &ctx(&root, false))
        .await;
    assert!(!outcome.ok);
    assert!(outcome.content.contains("denied"));
}

/// Tiny helper so a `None` file_change fails loudly without `.unwrap()`.
trait ExpectSome<T> {
    fn expect_none_marker(self) -> T;
}
impl<T> ExpectSome<T> for Option<T> {
    fn expect_none_marker(self) -> T {
        match self {
            Some(v) => v,
            None => panic!("expected a file_change, got None"),
        }
    }
}
