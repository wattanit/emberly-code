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
    AskUserGate, AskUserOutcome, AskUserTool, BashTool, EditFileTool, GlobTool, GrepTool,
    PermissionGate, PermissionOutcome, PermissionRequest, PlainSandbox, ReadDocumentTool,
    ReadFileTool, ReadImageTool, Sandbox, Tool, ToolCtx, TruncateConfig, WriteFileTool,
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

fn write_bytes(root: &Path, rel: &str, data: &[u8]) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            panic!("failed to create dir: {e}");
        }
    }
    if let Err(e) = std::fs::write(&path, data) {
        panic!("failed to seed file {rel}: {e}");
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

/// Allows every request and records the last one, so a test can inspect what
/// the tool actually put in `detail` (Design §5: never truncated to fit).
struct CapturingGate {
    last: std::sync::Mutex<Option<PermissionRequest>>,
}

impl CapturingGate {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            last: std::sync::Mutex::new(None),
        })
    }

    fn last_detail(&self) -> String {
        match self.last.lock() {
            Ok(guard) => guard
                .as_ref()
                .map_or_else(String::new, |r| r.detail.clone()),
            Err(e) => panic!("capturing gate mutex poisoned: {e}"),
        }
    }
}

#[async_trait]
impl PermissionGate for CapturingGate {
    async fn authorize(&self, request: PermissionRequest) -> PermissionOutcome {
        match self.last.lock() {
            Ok(mut guard) => *guard = Some(request),
            Err(e) => panic!("capturing gate mutex poisoned: {e}"),
        }
        PermissionOutcome::Allow
    }
}

fn capturing_ctx(root: &Path, gate: Arc<CapturingGate>) -> ToolCtx {
    let sandbox: Arc<dyn Sandbox> = Arc::new(PlainSandbox);
    ToolCtx::new(root.to_path_buf(), TruncateConfig::default(), gate, sandbox)
}

fn ctx(root: &Path, allow: bool) -> ToolCtx {
    let gate: Arc<dyn PermissionGate> = if allow {
        Arc::new(AllowGate)
    } else {
        Arc::new(DenyGate)
    };
    let sandbox: Arc<dyn Sandbox> = Arc::new(PlainSandbox);
    ToolCtx::new(root.to_path_buf(), TruncateConfig::default(), gate, sandbox)
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

#[tokio::test]
async fn read_line_range_returns_numbered_slice() {
    let root = temp_project();
    write_file(&root, "many.txt", "one\ntwo\nthree\nfour\nfive\n");
    let outcome = ReadFileTool
        .execute(
            json!({ "path": "many.txt", "start_line": 2, "end_line": 4 }),
            &ctx(&root, true),
        )
        .await;
    assert!(outcome.ok);
    // 1-based inclusive, line-numbered — the prompt-free `sed -n` equivalent.
    assert_eq!(
        outcome.content,
        "     2\ttwo\n     3\tthree\n     4\tfour\n"
    );
    assert!(outcome.summary.contains("lines 2-4 of 5"));
}

#[tokio::test]
async fn read_line_range_start_only_reads_to_eof() {
    let root = temp_project();
    write_file(&root, "many.txt", "one\ntwo\nthree\n");
    let outcome = ReadFileTool
        .execute(
            json!({ "path": "many.txt", "start_line": 2 }),
            &ctx(&root, true),
        )
        .await;
    assert!(outcome.ok);
    assert_eq!(outcome.content, "     2\ttwo\n     3\tthree\n");
}

#[tokio::test]
async fn read_line_range_past_end_is_a_failure() {
    let root = temp_project();
    write_file(&root, "short.txt", "only\n");
    let outcome = ReadFileTool
        .execute(
            json!({ "path": "short.txt", "start_line": 50 }),
            &ctx(&root, true),
        )
        .await;
    assert!(!outcome.ok);
    assert!(outcome.content.contains("past the end"));
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
async fn write_permission_detail_is_a_full_diff_for_a_new_file() {
    // Regression: the permission overlay used to collapse a new file's
    // content to a one-line "Create foo (N lines)" summary instead of
    // showing it — the diff/edit tool never had this gap. `detail` must
    // carry the actual content (as added lines) so the user isn't asked to
    // approve content they can't see (Design §5: never truncated to fit).
    let root = temp_project();
    let gate = CapturingGate::new();
    let outcome = WriteFileTool
        .execute(
            json!({ "path": "src/new.rs", "content": "a\nb\nc\n" }),
            &capturing_ctx(&root, gate.clone()),
        )
        .await;
    assert!(outcome.ok);
    let detail = gate.last_detail();
    assert!(
        !detail.contains("lines)"),
        "no collapsed summary: {detail:?}"
    );
    assert!(detail.contains("+a"), "added content visible: {detail:?}");
    assert!(detail.contains("+b"), "added content visible: {detail:?}");
    assert!(detail.contains("+c"), "added content visible: {detail:?}");
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

/// S-4: a timeout must kill the *whole* process group, not just the `sh` leader.
/// The command backgrounds a long `sleep` (a grandchild) and waits, so the tool
/// times out with the sleep alive; after the group kill it must be gone. Uses
/// `/proc` to check liveness without signaling, so it is Linux-gated.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn bash_timeout_kills_the_whole_process_group() {
    let root = temp_project();
    let pidfile = root.join("grandchild.pid");
    let command = format!("sleep 30 & echo $! > {}; wait", pidfile.display());
    let outcome = BashTool::default()
        .execute(
            json!({ "command": command, "timeout_secs": 1 }),
            &ctx(&root, true),
        )
        .await;
    assert!(!outcome.ok, "the command timed out");

    // Let the group-kill and reaping settle, then confirm the grandchild is gone.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let pid = std::fs::read_to_string(&pidfile).unwrap_or_else(|e| panic!("read pidfile: {e}"));
    let pid = pid.trim();
    assert!(!pid.is_empty(), "the grandchild recorded its pid");
    assert!(
        !std::path::Path::new(&format!("/proc/{pid}")).exists(),
        "backgrounded grandchild (pid {pid}) must die with the group, not survive the leader"
    );
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

// ---- glob (T-5) -----------------------------------------------------------

#[tokio::test]
async fn glob_finds_matching_files_skipping_git_and_gitignored() {
    let root = temp_project();
    write_file(&root, "a.rs", "fn a() {}\n");
    write_file(&root, "src/b.rs", "fn b() {}\n");
    write_file(&root, "notes.txt", "hello\n");
    write_file(&root, "target/gen.rs", "fn gen() {}\n");
    write_file(&root, ".git/hooks/pre.rs", "fn hook() {}\n");
    write_file(&root, ".gitignore", "target/\n");

    let outcome = GlobTool
        .execute(json!({ "pattern": "**/*.rs" }), &ctx(&root, true))
        .await;
    assert!(outcome.ok, "glob succeeds: {}", outcome.content);
    let lines: Vec<&str> = outcome.content.lines().collect();
    assert!(lines.contains(&"a.rs"), "top-level match: {lines:?}");
    assert!(lines.contains(&"src/b.rs"), "nested match: {lines:?}");
    assert!(
        !lines.iter().any(|l| l.contains(".git/")),
        ".git/ is skipped: {lines:?}"
    );
    assert!(
        !lines.iter().any(|l| l.contains("target/")),
        "gitignored target/ is skipped: {lines:?}"
    );
}

#[tokio::test]
async fn glob_refuses_outside_the_root() {
    let root = temp_project();
    let outcome = GlobTool
        .execute(
            json!({ "pattern": "*", "path": "../.." }),
            &ctx(&root, true),
        )
        .await;
    assert!(!outcome.ok);
    assert!(outcome.summary.contains("outside root"));
}

#[tokio::test]
async fn glob_reports_no_matches_cleanly() {
    let root = temp_project();
    write_file(&root, "a.txt", "x\n");
    let outcome = GlobTool
        .execute(json!({ "pattern": "**/*.rs" }), &ctx(&root, true))
        .await;
    assert!(outcome.ok);
    assert!(outcome.content.contains("no files match"));
}

// ---- grep (T-6) -----------------------------------------------------------

#[tokio::test]
async fn grep_finds_matches_with_path_and_line() {
    let root = temp_project();
    write_file(
        &root,
        "src/lib.rs",
        "fn one() {}\nlet x = 1;\nfn two() {}\n",
    );
    write_file(&root, "readme.md", "no functions here\n");
    write_file(&root, ".git/config.rs", "fn secret() {}\n");

    let outcome = GrepTool
        .execute(json!({ "pattern": r"fn \w+" }), &ctx(&root, true))
        .await;
    assert!(outcome.ok, "grep succeeds: {}", outcome.content);
    assert!(
        outcome.content.contains("src/lib.rs:1:fn one() {}"),
        "path:line:text format: {}",
        outcome.content
    );
    assert!(
        outcome.content.contains("src/lib.rs:3:fn two() {}"),
        "second match: {}",
        outcome.content
    );
    assert!(
        !outcome.content.contains(".git/"),
        ".git/ is skipped: {}",
        outcome.content
    );
}

#[tokio::test]
async fn grep_reports_no_matches_cleanly() {
    let root = temp_project();
    write_file(&root, "a.txt", "nothing interesting\n");
    let outcome = GrepTool
        .execute(json!({ "pattern": "zzz-not-present" }), &ctx(&root, true))
        .await;
    assert!(outcome.ok);
    assert!(outcome.content.contains("no matches"));
}

#[tokio::test]
async fn grep_invalid_regex_is_a_failure_not_a_crash() {
    let root = temp_project();
    let outcome = GrepTool
        .execute(json!({ "pattern": "(unclosed" }), &ctx(&root, true))
        .await;
    assert!(!outcome.ok);
    assert!(outcome.summary.contains("bad pattern"));
}

// ---- ask_user (T-8) ------------------------------------------------------

/// A gate that echoes a fixed answer, capturing what it was asked.
struct FixedAskGate {
    answer: AskUserOutcome,
    seen: std::sync::Mutex<Option<(String, Vec<String>)>>,
}
#[async_trait]
impl AskUserGate for FixedAskGate {
    async fn ask(&self, question: String, options: Vec<String>) -> AskUserOutcome {
        if let Ok(mut g) = self.seen.lock() {
            *g = Some((question, options));
        }
        self.answer.clone()
    }
}

#[tokio::test]
async fn ask_user_returns_the_answer_as_data() {
    let root = temp_project();
    let gate = Arc::new(FixedAskGate {
        answer: AskUserOutcome::Answered("dev".into()),
        seen: std::sync::Mutex::new(None),
    });
    let ctx = ToolCtx::new(
        root,
        TruncateConfig::default(),
        Arc::new(AllowGate),
        Arc::new(PlainSandbox) as Arc<dyn Sandbox>,
    )
    .with_ask_gate(gate.clone());

    let outcome = AskUserTool
        .execute(
            json!({ "question": "which env?", "options": ["dev", "prod"] }),
            &ctx,
        )
        .await;
    assert!(outcome.ok);
    assert!(outcome.content.contains("dev"));
    // The tool passed the question and options through to the gate.
    let seen = gate.seen.lock().ok().and_then(|g| g.clone());
    assert_eq!(
        seen,
        Some(("which env?".to_string(), vec!["dev".into(), "prod".into()]))
    );
}

#[tokio::test]
async fn ask_user_default_gate_declines() {
    let root = temp_project();
    // A ctx built without `with_ask_gate` declines by default (safe no-op).
    let ctx = ctx(&root, true);
    let outcome = AskUserTool
        .execute(json!({ "question": "proceed?" }), &ctx)
        .await;
    // A decline is structured data, not a failure (HC-6).
    assert!(outcome.ok);
    assert!(outcome.content.contains("declined"));
}

// ---- read_image (T-12, P-11) -----------------------------------------------

/// A minimal 1×1 red PNG for fixture use.
fn tiny_png_bytes() -> Vec<u8> {
    use base64::{engine::general_purpose, Engine as _};
    general_purpose::STANDARD
        .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==")
        .unwrap_or_default()
}

/// A ctx with vision enabled (P-11).
fn ctx_vision(root: &Path, allow: bool) -> ToolCtx {
    ctx(root, allow).with_vision(true)
}

#[tokio::test]
async fn read_image_on_non_vision_model_returns_unsupported() {
    // HC-6: the model learns it could not see, rather than assuming it saw.
    let root = temp_project();
    write_bytes(&root, "img.png", &tiny_png_bytes());
    let outcome = ReadImageTool
        .execute(json!({ "path": "img.png" }), &ctx(&root, true)) // vision=false
        .await;
    assert!(!outcome.ok);
    assert!(outcome.content.contains("no vision") || outcome.content.contains("vision"));
    assert!(outcome.image.is_none());
}

#[tokio::test]
async fn read_image_success_appends_image_payload() {
    let root = temp_project();
    write_bytes(&root, "img.png", &tiny_png_bytes());
    let outcome = ReadImageTool
        .execute(json!({ "path": "img.png" }), &ctx_vision(&root, true))
        .await;
    assert!(outcome.ok);
    assert!(outcome.image.is_some());
    let image = outcome.image.as_ref().expect("image payload");
    assert_eq!(image.media_type, "image/png");
    assert!(!image.data.is_empty());
    // Summary matches the Design §4.8 reference-line form.
    assert!(outcome.summary.contains("·"));
    assert!(outcome.summary.contains("PNG"));
    assert!(outcome.summary.contains("1×1"));
}

#[tokio::test]
async fn read_image_rejects_oversize() {
    let root = temp_project();
    let png = tiny_png_bytes();
    write_bytes(&root, "img.png", &png);
    let ctx_small = ctx_vision(&root, true).with_image_max_bytes(1);
    let outcome = ReadImageTool
        .execute(json!({ "path": "img.png" }), &ctx_small)
        .await;
    assert!(!outcome.ok);
    assert!(outcome.content.contains("exceeds") || outcome.content.contains("limit"));
}

#[tokio::test]
async fn read_image_rejects_unknown_format() {
    let root = temp_project();
    write_file(&root, "data.bin", "this is not an image");
    let outcome = ReadImageTool
        .execute(json!({ "path": "data.bin" }), &ctx_vision(&root, true))
        .await;
    assert!(!outcome.ok);
    assert!(outcome.content.contains("not") || outcome.content.contains("format"));
}

#[tokio::test]
async fn read_image_outside_root_requests_permission() {
    let root = temp_project();
    let png = tiny_png_bytes();
    // Write a file in a sibling dir (outside the project root).
    let outside = std::env::temp_dir().join(format!(
        "emberly-outside-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::create_dir_all(&outside);
    let _ = std::fs::write(outside.join("ext.png"), &png);
    let abs = outside.join("ext.png").display().to_string();

    // Denied → the tool returns a denied outcome.
    let outcome = ReadImageTool
        .execute(json!({ "path": abs }), &ctx_vision(&root, false))
        .await;
    assert!(!outcome.ok);
    assert!(outcome.content.contains("denied"));

    // Allow → the image loads.
    let outcome2 = ReadImageTool
        .execute(json!({ "path": abs }), &ctx_vision(&root, true))
        .await;
    assert!(outcome2.ok);
    assert!(outcome2.image.is_some());
}

#[tokio::test]
async fn read_image_missing_file_is_a_failure_not_a_crash() {
    let root = temp_project();
    let outcome = ReadImageTool
        .execute(json!({ "path": "nope.png" }), &ctx_vision(&root, true))
        .await;
    assert!(!outcome.ok);
    assert!(outcome.content.contains("nope.png"));
}

// ---- read_image format coverage (Tech Spec §5.2: PNG, JPEG, GIF, WebP) -----

fn decode_b64(s: &str) -> Vec<u8> {
    use base64::{engine::general_purpose, Engine as _};
    general_purpose::STANDARD.decode(s).unwrap_or_default()
}

fn tiny_jpeg_bytes() -> Vec<u8> {
    decode_b64("/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsLDBkSEw8UHRofHh0aHBwgJC4nICIsIxwcKDcpLDAxNDQ0Hyc5PTgyPC4zNDL/wAALCAABAAEBAREA/8QAHwAAAQUBAQEBAQEAAAAAAAAAAAECAwQFBgcICQoL/8QAtRAAAgEDAwIEAwUFBAQAAAF9AQIDAAQRBRIhMUEGE1FhByJxFDKBkaEII0KxwRVS0fAkM2JyggkKFhcYGRolJicoKSo0NTY3ODk6Q0RFRkdISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWGh4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uHi4+Tl5ufo6erx8vP09fb3+Pn6/9oACAEBAAA/APvSiiig/9k=")
}

fn tiny_gif_bytes() -> Vec<u8> {
    decode_b64("R0lGODlhAQABAIAAAP///yH5BAAAAAAALAAAAAABAAEAAAICRAEAOw==")
}

fn tiny_webp_bytes() -> Vec<u8> {
    decode_b64("UklGRhoAAABXRUJQVlA4TA0AAAAvAAAAAACvACEBHwE=")
}

#[tokio::test]
async fn read_image_detects_jpeg_format() {
    let root = temp_project();
    write_bytes(&root, "img.jpg", &tiny_jpeg_bytes());
    let outcome = ReadImageTool
        .execute(json!({ "path": "img.jpg" }), &ctx_vision(&root, true))
        .await;
    assert!(outcome.ok, "JPEG should succeed: {}", outcome.content);
    let image = outcome.image.as_ref().expect("image payload");
    assert_eq!(image.media_type, "image/jpeg");
    assert!(outcome.summary.contains("JPEG"));
}

#[tokio::test]
async fn read_image_detects_gif_format() {
    let root = temp_project();
    write_bytes(&root, "img.gif", &tiny_gif_bytes());
    let outcome = ReadImageTool
        .execute(json!({ "path": "img.gif" }), &ctx_vision(&root, true))
        .await;
    assert!(outcome.ok, "GIF should succeed: {}", outcome.content);
    let image = outcome.image.as_ref().expect("image payload");
    assert_eq!(image.media_type, "image/gif");
    assert!(outcome.summary.contains("GIF"));
}

#[tokio::test]
async fn read_image_detects_webp_format() {
    let root = temp_project();
    write_bytes(&root, "img.webp", &tiny_webp_bytes());
    let outcome = ReadImageTool
        .execute(json!({ "path": "img.webp" }), &ctx_vision(&root, true))
        .await;
    assert!(outcome.ok, "WebP should succeed: {}", outcome.content);
    let image = outcome.image.as_ref().expect("image payload");
    assert_eq!(image.media_type, "image/webp");
    assert!(outcome.summary.contains("WebP"));
}

// ---- read_document (T-16, P-12) --------------------------------------------

/// A minimal, valid-enough PDF: just the `%PDF-` magic prefix `read_document`
/// sniffs on (P-12) — the harness never parses past it (HC-2).
fn tiny_pdf_bytes() -> Vec<u8> {
    b"%PDF-1.4\n%%EOF".to_vec()
}

/// A ctx with document input enabled (P-12).
fn ctx_documents(root: &Path, allow: bool) -> ToolCtx {
    ctx(root, allow).with_documents(true)
}

#[tokio::test]
async fn read_document_on_non_documents_model_returns_unsupported() {
    // HC-6: the model learns it could not read the document, rather than
    // assuming it did.
    let root = temp_project();
    write_bytes(&root, "doc.pdf", &tiny_pdf_bytes());
    let outcome = ReadDocumentTool
        .execute(json!({ "path": "doc.pdf" }), &ctx(&root, true)) // documents=false
        .await;
    assert!(!outcome.ok);
    assert!(
        outcome.content.contains("no document support") || outcome.content.contains("document")
    );
    assert!(outcome.document.is_none());
}

#[tokio::test]
async fn read_document_success_appends_document_payload() {
    let root = temp_project();
    write_bytes(&root, "doc.pdf", &tiny_pdf_bytes());
    let outcome = ReadDocumentTool
        .execute(json!({ "path": "doc.pdf" }), &ctx_documents(&root, true))
        .await;
    assert!(outcome.ok);
    assert!(outcome.document.is_some());
    let document = outcome.document.as_ref().expect("document payload");
    assert_eq!(document.media_type, "application/pdf");
    assert!(!document.data.is_empty());
    // Summary matches the Design §4.11 reference-line form: no page count.
    assert!(outcome.summary.contains("·"));
    assert!(outcome.summary.contains("PDF"));
    assert!(!outcome.summary.to_lowercase().contains("page"));
}

#[tokio::test]
async fn read_document_rejects_oversize() {
    let root = temp_project();
    let pdf = tiny_pdf_bytes();
    write_bytes(&root, "doc.pdf", &pdf);
    let ctx_small = ctx_documents(&root, true).with_document_max_bytes(1);
    let outcome = ReadDocumentTool
        .execute(json!({ "path": "doc.pdf" }), &ctx_small)
        .await;
    assert!(!outcome.ok);
    assert!(outcome.content.contains("exceeds") || outcome.content.contains("limit"));
}

#[tokio::test]
async fn read_document_rejects_non_pdf() {
    let root = temp_project();
    write_file(&root, "data.bin", "this is not a pdf");
    let outcome = ReadDocumentTool
        .execute(json!({ "path": "data.bin" }), &ctx_documents(&root, true))
        .await;
    assert!(!outcome.ok);
    assert!(outcome.content.contains("not") || outcome.content.contains("PDF"));
}

#[tokio::test]
async fn read_document_outside_root_requests_permission() {
    let root = temp_project();
    let pdf = tiny_pdf_bytes();
    // Write a file in a sibling dir (outside the project root).
    let outside = std::env::temp_dir().join(format!(
        "emberly-outside-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::create_dir_all(&outside);
    let _ = std::fs::write(outside.join("ext.pdf"), &pdf);
    let abs = outside.join("ext.pdf").display().to_string();

    // Denied → the tool returns a denied outcome.
    let outcome = ReadDocumentTool
        .execute(json!({ "path": abs.clone() }), &ctx_documents(&root, false))
        .await;
    assert!(!outcome.ok);
    assert!(outcome.content.contains("denied"));

    // Allow → the document loads.
    let outcome2 = ReadDocumentTool
        .execute(json!({ "path": abs }), &ctx_documents(&root, true))
        .await;
    assert!(outcome2.ok);
    assert!(outcome2.document.is_some());
}

#[tokio::test]
async fn read_document_missing_file_is_a_failure_not_a_crash() {
    let root = temp_project();
    let outcome = ReadDocumentTool
        .execute(json!({ "path": "nope.pdf" }), &ctx_documents(&root, true))
        .await;
    assert!(!outcome.ok);
    assert!(outcome.content.contains("nope.pdf"));
}

#[tokio::test]
async fn read_document_git_dir_requests_permission_like_any_outside_path() {
    // `.git/` is inside the project tree, not outside-root, so this proves
    // the tool has no special-case for it — the ordinary in-root Allow rule
    // (group 6) covers it exactly like any other project-relative path.
    let root = temp_project();
    write_bytes(&root, ".git/doc.pdf", &tiny_pdf_bytes());
    let outcome = ReadDocumentTool
        .execute(
            json!({ "path": ".git/doc.pdf" }),
            &ctx_documents(&root, true),
        )
        .await;
    assert!(
        outcome.ok,
        "in-root .git/ path should succeed: {}",
        outcome.content
    );
    assert!(outcome.document.is_some());
}
