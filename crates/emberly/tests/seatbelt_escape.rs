//! Escape-test suite (Phase 5 group 8; Requirements HC-4/HC-5, Tech Spec §6.3,
//! §14.3) for the macOS Seatbelt backend. Builds the *real* confined-spawn
//! invocation (`emberly_sandbox::confine::confined_invocation`) and spawns it,
//! asserting the fence actually holds on this host.
//!
//! macOS only, and only where `sandbox-exec` genuinely confines. When Seatbelt
//! is unavailable the whole module is skipped with a printed notice, so a host
//! that can't run it never reads as silently passing (no silent coverage gaps).

#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use emberly_sandbox::confine::{confined_invocation, seatbelt_available};
use emberly_sandbox::SandboxSpec;

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh, canonicalized temp project root. Canonical so the spawned child's
/// resolved cwd matches the Seatbelt profile's resolved paths (`/var` → `/private`).
fn temp_root() -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("emberly-sb-{}-{}", std::process::id(), n));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        panic!("create temp root: {e}");
    }
    match std::fs::canonicalize(&dir) {
        Ok(real) => real,
        Err(e) => panic!("canonicalize temp root: {e}"),
    }
}

/// Run `command` under the real Seatbelt invocation for `spec`, from `cwd`.
/// Mirrors what the bash tool does (program + args, env cleared to a minimal
/// set, cwd = root). `HOME` points at `cwd` so tools like git don't fail
/// touching an unreadable real home.
fn run_confined(spec: &SandboxSpec, cwd: &Path, command: &str) -> Output {
    let Some((program, args, extra_env)) = confined_invocation(command, spec) else {
        panic!("no confined invocation on this platform");
    };
    let mut cmd = Command::new(&program);
    cmd.args(&args)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", cwd)
        .current_dir(cwd);
    for (key, value) in &extra_env {
        cmd.env(key, value);
    }
    match cmd.output() {
        Ok(o) => o,
        Err(e) => panic!("spawn sandbox-exec: {e}"),
    }
}

/// Whether a real `git` is available to exercise the genuine-git profile.
fn git_available() -> bool {
    emberly_sandbox::git::record_git_binary(&std::env::var("PATH").unwrap_or_default()).is_some()
}

#[test]
fn confined_child_can_write_inside_the_root() {
    if !seatbelt_available() {
        eprintln!("SKIP: sandbox-exec unavailable — in-root write test not run");
        return;
    }
    let root = temp_root();
    let spec = SandboxSpec::confined_to(&root);
    let out = run_confined(&spec, &root, "echo hi > allowed.txt");
    assert!(
        out.status.success(),
        "in-root write should succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(root.join("allowed.txt")).unwrap_or_default(),
        "hi\n"
    );
}

#[test]
fn confined_child_cannot_write_outside_the_root() {
    if !seatbelt_available() {
        eprintln!("SKIP: sandbox-exec unavailable — outside-root write test not run");
        return;
    }
    let root = temp_root();
    let escape = std::env::temp_dir().join(format!(
        "emberly-sb-target-{}-{}.txt",
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
    if !seatbelt_available() {
        eprintln!("SKIP: sandbox-exec unavailable — .git write test not run");
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
        ".git/ must be read-only under the default profile"
    );
    // Reading .git is still allowed (reads are broad; §6.3 write-confinement).
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
    if !seatbelt_available() {
        eprintln!("SKIP: sandbox-exec unavailable — git-writable test not run");
        return;
    }
    let root = temp_root();
    if let Err(e) = std::fs::create_dir_all(root.join(".git")) {
        panic!("create .git: {e}");
    }
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
    if !seatbelt_available() {
        eprintln!("SKIP: sandbox-exec unavailable — genuine-git test not run");
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
