//! Escape-test suite (Phase 2 group 9; Requirements HC-4/HC-5, Tech Spec §14.3)
//! for the Landlock self-exec shim. Drives the built `emberly` binary as the
//! confined-exec shim and asserts the fence actually holds on this kernel.
//!
//! Linux + Landlock-enabled only. When Landlock is absent the whole module is
//! skipped with a printed notice, so a platform that can't run it never reads
//! as silently passing (no silent coverage gaps).

#![cfg(target_os = "linux")]

use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use emberly_sandbox::confine::detect_abi;
use emberly_sandbox::SandboxSpec;

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh temp project root.
fn temp_root() -> std::path::PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("emberly-escape-{}-{}", std::process::id(), n));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        panic!("create temp root: {e}");
    }
    dir
}

/// Run `command` under the confined-exec shim for `spec`, from `cwd`. `HOME` is
/// pointed at `cwd` (writable, in-root) so tools like git don't fail trying to
/// touch an unreadable real home under confinement.
fn run_confined(spec: &SandboxSpec, cwd: &Path, command: &str) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_emberly"))
        .arg(emberly_sandbox::SANDBOX_EXEC_ARG)
        .arg(command)
        .env("EMBERLY_SANDBOX_SPEC", spec.to_env_value())
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", cwd)
        .current_dir(cwd)
        .output();
    match output {
        Ok(o) => o,
        Err(e) => panic!("spawn shim: {e}"),
    }
}

/// Whether a real `git` is available to exercise the genuine-git profile.
fn git_available() -> bool {
    emberly_sandbox::git::record_git_binary(&std::env::var("PATH").unwrap_or_default()).is_some()
}

/// Whether this kernel enforces Landlock; if not, escape tests can't run.
fn landlock_available() -> bool {
    detect_abi().is_some()
}

#[test]
fn confined_child_can_write_inside_the_root() {
    if !landlock_available() {
        eprintln!("SKIP: no Landlock on this kernel — in-root write test not run");
        return;
    }
    let root = temp_root();
    // The non-git profile grants write per existing top-level entry, so write
    // into a seeded subdirectory (the common case: target/, src/, etc.).
    if let Err(e) = std::fs::create_dir_all(root.join("work")) {
        panic!("seed work dir: {e}");
    }
    let spec = SandboxSpec::confined_to(&root);
    let out = run_confined(&spec, &root, "echo hi > work/allowed.txt");
    assert!(
        out.status.success(),
        "in-root (subdir) write should succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(root.join("work/allowed.txt")).unwrap_or_default(),
        "hi\n"
    );
}

#[test]
fn confined_child_cannot_write_outside_the_root() {
    if !landlock_available() {
        eprintln!("SKIP: no Landlock on this kernel — outside-root write test not run");
        return;
    }
    let root = temp_root();
    // A target deliberately outside the confined root.
    let escape = std::env::temp_dir().join(format!(
        "emberly-escape-target-{}-{}.txt",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&escape);
    let spec = SandboxSpec::confined_to(&root);
    let out = run_confined(
        &spec,
        &root,
        &format!("echo escaped > {}", escape.display()),
    );
    assert!(
        !out.status.success(),
        "writing outside the root must fail under confinement"
    );
    assert!(
        !escape.exists(),
        "the outside-root file must never be created"
    );
}

#[test]
fn confined_child_cannot_write_under_git() {
    if !landlock_available() {
        eprintln!("SKIP: no Landlock on this kernel — .git write test not run");
        return;
    }
    let root = temp_root();
    if let Err(e) = std::fs::create_dir_all(root.join(".git")) {
        panic!("create .git: {e}");
    }
    let spec = SandboxSpec::confined_to(&root);
    let out = run_confined(&spec, &root, "echo tamper > .git/escape");
    assert!(
        !out.status.success(),
        "writing under .git/ must fail (HC-5), even inside the root"
    );
    assert!(
        !root.join(".git/escape").exists(),
        ".git/ must be read-only under confinement"
    );
    // But reading git data is still allowed (read+execute on .git/).
    if let Err(e) = std::fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n") {
        panic!("seed .git/HEAD: {e}");
    }
    let read = run_confined(&spec, &root, "cat .git/HEAD");
    assert!(
        read.status.success(),
        "reading .git/ should still work: {}",
        String::from_utf8_lossy(&read.stderr)
    );
}

#[test]
fn git_writable_spec_permits_dot_git_writes() {
    if !landlock_available() {
        eprintln!("SKIP: no Landlock on this kernel — git-writable test not run");
        return;
    }
    let root = temp_root();
    if let Err(e) = std::fs::create_dir_all(root.join(".git")) {
        panic!("create .git: {e}");
    }
    // The genuine-git profile (§6.4) opens .git/ for writing.
    let spec = SandboxSpec {
        git_writable: true,
        ..SandboxSpec::confined_to(&root)
    };
    let out = run_confined(&spec, &root, "echo ok > .git/COMMIT_EDITMSG");
    assert!(
        out.status.success(),
        "git-writable spec should allow .git/ writes: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(root.join(".git/COMMIT_EDITMSG").exists());
}

#[test]
fn genuine_git_commits_under_the_writable_profile_but_not_the_default() {
    if !landlock_available() {
        eprintln!("SKIP: no Landlock on this kernel — genuine-git test not run");
        return;
    }
    if !git_available() {
        eprintln!("SKIP: no system git — genuine-git test not run");
        return;
    }
    let root = temp_root();
    let git_ident =
        "git -c user.email=t@example.com -c user.name=Tester -c init.defaultBranch=main";

    // Under the git-writable profile: init + an empty commit both succeed.
    let writable = SandboxSpec {
        git_writable: true,
        ..SandboxSpec::confined_to(&root)
    };
    let init = run_confined(&writable, &root, &format!("{git_ident} init -q"));
    assert!(
        init.status.success() && root.join(".git").is_dir(),
        "genuine git init should succeed under the writable profile: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    let commit = run_confined(
        &writable,
        &root,
        &format!("{git_ident} commit -q --allow-empty --allow-empty-message -m init"),
    );
    assert!(
        commit.status.success(),
        "genuine git commit should succeed under the writable profile: {}",
        String::from_utf8_lossy(&commit.stderr)
    );

    // The SAME commit under the default (read-only-.git) profile must fail —
    // proving it is the profile, not luck, that lets git write .git/ (HC-5).
    let default = SandboxSpec::confined_to(&root);
    let denied = run_confined(
        &default,
        &root,
        &format!("{git_ident} commit -q --allow-empty --allow-empty-message -m second"),
    );
    assert!(
        !denied.status.success(),
        "a commit must fail when .git/ is read-only (the non-git profile)"
    );
}
