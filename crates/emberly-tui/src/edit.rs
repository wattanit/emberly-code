//! Handing a file to the user's editor (Design §4.3, §4.6, C-5). Resolves
//! `$VISUAL` → `$EDITOR`, runs it as a foreground child on the file, and waits.
//! The caller suspends and restores the terminal around this
//! ([`TerminalGuard::suspend`](crate::terminal::TerminalGuard::suspend)).

use std::path::Path;
use std::process::Command;

/// Outcome of an editor handoff — structured so the frontend surfaces it as a
/// calm notice, never a crash (HC-6-style: a missing editor is data, not an
/// error).
#[derive(Debug)]
pub enum EditStatus {
    /// The editor ran and exited successfully.
    Edited,
    /// No editor is configured (`$VISUAL` / `$EDITOR` both unset/empty).
    NoEditor,
    /// The editor could not be launched, or exited non-zero.
    Failed(String),
}

/// Resolve the editor command: `$VISUAL`, else `$EDITOR`, else `None`. Pure
/// over its inputs so it is unit-testable without touching the environment.
#[must_use]
pub fn resolve_editor(visual: Option<&str>, editor: Option<&str>) -> Option<String> {
    [visual, editor]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|s| !s.is_empty())
        .map(str::to_string)
}

/// The configured editor from the process environment.
fn editor_program() -> Option<String> {
    let visual = std::env::var("VISUAL").ok();
    let editor = std::env::var("EDITOR").ok();
    resolve_editor(visual.as_deref(), editor.as_deref())
}

/// Open `path` in the user's editor and wait. The caller must suspend the TUI
/// first. Never returns an `Err`: a missing or failing editor is an
/// [`EditStatus`] the frontend turns into a notice.
#[must_use]
pub fn run_editor(path: &Path) -> EditStatus {
    match editor_program() {
        Some(program) => run_program(&program, path),
        None => EditStatus::NoEditor,
    }
}

/// Run `program` (which may carry arguments, e.g. `code --wait`) on `path`,
/// inheriting the terminal's stdio, and wait for it to exit.
#[must_use]
pub fn run_program(program: &str, path: &Path) -> EditStatus {
    let mut parts = program.split_whitespace();
    let Some(bin) = parts.next() else {
        return EditStatus::NoEditor;
    };
    let mut command = Command::new(bin);
    command.args(parts).arg(path);
    match command.status() {
        Ok(status) if status.success() => EditStatus::Edited,
        Ok(status) => EditStatus::Failed(format!("editor exited with {status}")),
        Err(error) => EditStatus::Failed(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_prefers_visual_then_editor() {
        assert_eq!(
            resolve_editor(Some("vim"), Some("nano")).as_deref(),
            Some("vim")
        );
        assert_eq!(resolve_editor(None, Some("nano")).as_deref(), Some("nano"));
        // Empty/whitespace is treated as unset, falling through.
        assert_eq!(
            resolve_editor(Some(""), Some("nano")).as_deref(),
            Some("nano")
        );
        assert_eq!(resolve_editor(Some("  "), None), None);
        assert_eq!(resolve_editor(None, None), None);
    }

    #[cfg(unix)]
    #[test]
    fn run_program_runs_the_editor_on_the_file() {
        use std::os::unix::fs::PermissionsExt;
        // A fake editor: a shell script that appends to its file argument.
        let dir = std::env::temp_dir().join(format!("emberly-edit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let script = dir.join("fake-editor.sh");
        std::fs::write(&script, "#!/bin/sh\nprintf EDITED >> \"$1\"\n").expect("write script");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");

        let target = dir.join("f.txt");
        std::fs::write(&target, "before\n").expect("seed");
        let status = run_program(script.to_str().expect("utf8 path"), &target);

        assert!(matches!(status, EditStatus::Edited), "status: {status:?}");
        let contents = std::fs::read_to_string(&target).expect("read back");
        assert!(
            contents.contains("EDITED"),
            "editor ran on the file: {contents:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn run_program_reports_a_failing_editor() {
        // `false` exits non-zero → Failed, not a crash.
        assert!(matches!(
            run_program("false", Path::new("/tmp/whatever")),
            EditStatus::Failed(_)
        ));
    }
}
