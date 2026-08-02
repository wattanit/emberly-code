//! Genuine-git resolution (Requirements §6.3, §6.4; Tech Spec §6.4).
//!
//! Only an invocation of the *real* git binary earns the `.git/`-writable
//! confinement profile (HC-5). Everything else — a `./git` in the repo, a
//! PATH-shadowing git inside the project, an `sh -c "git …"` wrapper, or a git
//! call chained/redirected/substituted with other commands — runs under the
//! read-only-`.git` profile (safe-closed). Shell semantics are **not** parsed
//! (Requirements §6.5): only a *simple* `git …` command is recognized, and
//! anything carrying shell metacharacters that could smuggle another writer in
//! while `.git/` is open is refused.

use std::path::{Path, PathBuf};

/// Record the canonical system git binary at session start: resolve `git`
/// against `path_env` and canonicalize it (following symlinks). `None` if no
/// executable `git` is found. Compared against later to accept only this exact
/// binary (Tech Spec §6.4).
#[must_use]
pub fn record_git_binary(path_env: &str) -> Option<PathBuf> {
    resolve_on_path("git", path_env)
}

/// Whether `command` qualifies for the `.git/`-writable profile:
///
/// - a canonical `git` binary was recorded at session start, **and**
/// - the command has no shell metacharacters that could chain, redirect, or
///   substitute another command (safe-closed; §6.5), **and**
/// - its first token is exactly `git` (not `./git`, `/path/git`, `mygit`, or a
///   `sh`/`bash` wrapper), **and**
/// - resolving `git` on `path_env` yields the recorded binary, which lies
///   **outside** the project root (so an in-repo or PATH-shadowing git fails).
#[must_use]
pub fn is_genuine_git(command: &str, recorded: Option<&Path>, root: &Path, path_env: &str) -> bool {
    let Some(recorded) = recorded else {
        return false;
    };
    let command = command.trim();
    if command.is_empty() || has_dangerous_shell_syntax(command) {
        return false;
    }
    // First whitespace-delimited token must be the bare word `git`.
    if command.split_whitespace().next() != Some("git") {
        return false;
    }
    match resolve_on_path("git", path_env) {
        Some(resolved) => resolved == recorded && !resolved.starts_with(root),
        None => false,
    }
}

/// Shell syntax that could let a command *other than git* run (and thus write
/// `.git/`) while the profile is open: chaining, pipes, redirection, subshells,
/// command substitution, backgrounding, and line breaks. Quotes, globs (`*`,
/// `?`), brace/tilde expansion, and bare `$VAR` are allowed — they cannot
/// themselves write to `.git/` (a bare `$` without `$(`/`${` only expands to an
/// argument). This is conservative on purpose (§6.5): borderline commands run
/// without `.git/` write rather than risk it.
///
/// Quote-aware: a metacharacter is only dangerous where the shell would
/// actually treat it as one. Single-quoted text is entirely literal in POSIX
/// shell (not even `$(`/backslash are special there); double-quoted text
/// neutralizes chaining/redirection/subshell punctuation but *not* command or
/// parameter substitution, which the shell still expands inside `"..."`. A
/// commit message like `-m "fix (#123); see <a@b.com>"` must keep the
/// `.git/`-writable grant — those characters never reach the shell unquoted.
/// An unterminated quote is itself suspicious and stays fail-closed.
fn has_dangerous_shell_syntax(command: &str) -> bool {
    #[derive(PartialEq)]
    enum Quote {
        None,
        Single,
        Double,
    }
    let chars: Vec<char> = command.chars().collect();
    let mut quote = Quote::None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match quote {
            Quote::Single => {
                if c == '\'' {
                    quote = Quote::None;
                }
                i += 1;
            }
            Quote::Double => match c {
                '"' => {
                    quote = Quote::None;
                    i += 1;
                }
                // Backslash escapes the next character inside double quotes
                // (e.g. `\"` for a literal quote) — skip both without
                // reinterpreting it, so an escaped `"` never closes the string.
                '\\' => i += 2,
                '`' => return true,
                '$' if matches!(chars.get(i + 1), Some('(' | '{')) => return true,
                _ => i += 1,
            },
            Quote::None => match c {
                '\'' => {
                    quote = Quote::Single;
                    i += 1;
                }
                '"' => {
                    quote = Quote::Double;
                    i += 1;
                }
                ';' | '&' | '|' | '<' | '>' | '(' | ')' | '`' | '\n' | '\r' | '\\' => return true,
                '$' if matches!(chars.get(i + 1), Some('(' | '{')) => return true,
                _ => i += 1,
            },
        }
    }
    quote != Quote::None
}

/// Resolve `program` against a `PATH`-style string: the first entry holding an
/// executable file, canonicalized. Mirrors `which`.
fn resolve_on_path(program: &str, path_env: &str) -> Option<PathBuf> {
    for dir in path_env.split(':') {
        if dir.is_empty() {
            continue;
        }
        let candidate = Path::new(dir).join(program);
        if is_executable_file(&candidate) {
            if let Ok(canonical) = std::fs::canonicalize(&candidate) {
                return Some(canonical);
            }
        }
    }
    None
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static N: AtomicU64 = AtomicU64::new(0);

    fn tmp() -> PathBuf {
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("emberly-git-{}-{}", std::process::id(), n));
        if let Err(e) = std::fs::create_dir_all(&dir) {
            panic!("mkdir: {e}");
        }
        dir
    }

    /// The real system git, if present — most assertions need a canonical git.
    fn system_git() -> Option<(PathBuf, String)> {
        let path_env = std::env::var("PATH").unwrap_or_default();
        record_git_binary(&path_env).map(|g| (g, path_env))
    }

    #[test]
    fn bare_git_command_qualifies() {
        let Some((git, path_env)) = system_git() else {
            eprintln!("SKIP: no system git");
            return;
        };
        let root = tmp();
        assert!(is_genuine_git(
            "git commit -m hi",
            Some(&git),
            &root,
            &path_env
        ));
        // A quoted message with a bare `$` is still fine (no substitution).
        assert!(is_genuine_git(
            "git commit -m \"cost $5\"",
            Some(&git),
            &root,
            &path_env
        ));
    }

    #[test]
    fn no_recorded_binary_never_qualifies() {
        let root = tmp();
        assert!(!is_genuine_git("git commit", None, &root, "/usr/bin"));
    }

    #[test]
    fn wrappers_lookalikes_and_paths_do_not_qualify() {
        let Some((git, path_env)) = system_git() else {
            eprintln!("SKIP: no system git");
            return;
        };
        let root = tmp();
        for command in [
            "./git commit",         // in-repo git
            "/usr/bin/git commit",  // explicit path, not the bare word
            "mygit commit",         // lookalike
            "sh -c \"git commit\"", // wrapper
            "GIT_DIR=x git commit", // env-prefixed (first token not `git`)
        ] {
            assert!(
                !is_genuine_git(command, Some(&git), &root, &path_env),
                "must not qualify: {command}"
            );
        }
    }

    #[test]
    fn shell_metacharacters_disqualify() {
        let Some((git, path_env)) = system_git() else {
            eprintln!("SKIP: no system git");
            return;
        };
        let root = tmp();
        for command in [
            "git commit && rm -rf .git",
            "git log > .git/escape",
            "git status | tee .git/x",
            "git commit -m \"$(touch .git/x)\"",
            "git commit -m \"${x}\"",
            "git commit; echo hi > .git/x",
            "(git commit)",
            "git commit `id`",
        ] {
            assert!(
                !is_genuine_git(command, Some(&git), &root, &path_env),
                "must be safe-closed: {command}"
            );
        }
    }

    #[test]
    fn quoted_metacharacters_in_a_commit_message_still_qualify() {
        // Regression: a commit message containing shell metacharacters that
        // are safely quoted (never reaching the shell unescaped) must not
        // lose the .git/-writable grant — this was denying real `git commit`
        // calls with "Operation not permitted" on .git/index.lock whenever
        // the message happened to contain e.g. `(`, `<`, `;`, or `&`.
        let Some((git, path_env)) = system_git() else {
            eprintln!("SKIP: no system git");
            return;
        };
        let root = tmp();
        for command in [
            r#"git commit -m "fix (#123): retry & polish""#,
            r#"git commit -m "Reviewed-by: A <a@b.com>""#,
            r#"git commit -m "note: a; b | c > d < e""#,
            r#"git commit -m "she said \"hi\"""#,
            "git commit -m 'literal $(not substituted) here'",
        ] {
            assert!(
                is_genuine_git(command, Some(&git), &root, &path_env),
                "quoted metacharacters must still qualify: {command}"
            );
        }
    }

    #[test]
    fn unterminated_quote_stays_safe_closed() {
        let Some((git, path_env)) = system_git() else {
            eprintln!("SKIP: no system git");
            return;
        };
        let root = tmp();
        assert!(!is_genuine_git(
            r#"git commit -m "unterminated"#,
            Some(&git),
            &root,
            &path_env
        ));
    }

    #[test]
    fn a_git_shadowing_inside_the_root_does_not_qualify() {
        // A fake `git` inside the project, first on PATH, resolves inside the
        // root → not the recorded system binary and not outside-root → refused.
        let root = tmp();
        let bin = root.join("bin");
        if let Err(e) = std::fs::create_dir_all(&bin) {
            panic!("mkdir bin: {e}");
        }
        let fake = bin.join("git");
        if let Err(e) = std::fs::write(&fake, "#!/bin/sh\n") {
            panic!("write fake git: {e}");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Err(e) = std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755))
            {
                panic!("chmod: {e}");
            }
        }
        // Record the *system* git as canonical, but put the fake first on PATH.
        let Some(system) = record_git_binary(&std::env::var("PATH").unwrap_or_default()) else {
            eprintln!("SKIP: no system git");
            return;
        };
        let shadowed_path = format!(
            "{}:{}",
            bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        assert!(
            !is_genuine_git("git commit", Some(&system), &root, &shadowed_path),
            "a git shadowing inside the root must not earn .git write"
        );
    }
}
