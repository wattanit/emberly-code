//! OS-level child confinement (Requirements §6.2, HC-4/HC-5; Tech Spec §6.2).
//!
//! The **security boundary**: while the rule engine ([`crate::rules`]) decides
//! *when to ask*, this decides *what is possible*. On Linux it applies a
//! Landlock ruleset to spawned children via a **self-exec shim** — the harness
//! re-execs itself under a hidden subcommand that calls `restrict_self()` (safe)
//! and then `exec()`s the real command (safe; Landlock is inherited across
//! `execve`). No `unsafe`, no `pre_exec` (HC-1). The harness process itself is
//! never confined — only children (Requirements §6.7).
//!
//! The ruleset (best-effort ABI): project root read+write; `.git/` under it
//! read+execute only (no write) unless the invocation is genuine git (§6.4);
//! system paths read+execute; explicitly-approved outside-root paths read+write
//! for that one invocation (HC-4); everything else no access.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The hidden subcommand the confined-exec shim runs under. The harness re-execs
/// `current_exe() <SANDBOX_EXEC_ARG> <command>` with [`SPEC_ENV`] set.
pub const SANDBOX_EXEC_ARG: &str = "__sandbox_exec";

/// Env var carrying the JSON-serialized [`SandboxSpec`] to the shim child.
pub const SPEC_ENV: &str = "EMBERLY_SANDBOX_SPEC";

/// The confinement policy for a single child invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxSpec {
    /// Project root: read + write.
    pub root: PathBuf,
    /// When true, `.git/` under the root is writable (genuine git only, §6.4);
    /// otherwise it is read-only (HC-5, safe-closed default).
    pub git_writable: bool,
    /// Outside-root paths the user approved for this one invocation (HC-4):
    /// read + write.
    pub extra_writable: Vec<PathBuf>,
}

impl SandboxSpec {
    /// A default spec confining to `root` with `.git/` read-only and no
    /// approved outside-root paths.
    #[must_use]
    pub fn confined_to(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            git_writable: false,
            extra_writable: Vec::new(),
        }
    }

    /// Serialize for [`SPEC_ENV`]. Infallible in practice (plain data).
    #[must_use]
    pub fn to_env_value(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Parse the spec from [`SPEC_ENV`], if present and valid.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let raw = std::env::var(SPEC_ENV).ok()?;
        serde_json::from_str(&raw).ok()
    }
}

/// The parts to spawn `command` confined by `spec` via the self-exec shim:
/// `(program, args, (env-key, env-value))`. The program is this executable
/// re-invoked under [`SANDBOX_EXEC_ARG`]; the spec rides in [`SPEC_ENV`].
/// `None` if the current executable path can't be determined (fall back to a
/// direct, unconfined shell — the honest degraded behavior).
#[must_use]
pub fn shim_invocation(
    command: &str,
    spec: &SandboxSpec,
) -> Option<(PathBuf, Vec<String>, (String, String))> {
    let exe = std::env::current_exe().ok()?;
    Some((
        exe,
        vec![SANDBOX_EXEC_ARG.to_string(), command.to_string()],
        (SPEC_ENV.to_string(), spec.to_env_value()),
    ))
}

/// System paths children may read + execute (dynamic linker, shell, toolchain).
/// Non-existent entries are silently skipped when the ruleset is built.
fn system_read_exec_paths() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = [
        "/usr", "/bin", "/sbin", "/lib", "/lib64", "/lib32", "/etc", "/opt",
    ]
    .iter()
    .map(PathBuf::from)
    .collect();
    // Rust toolchain (rustup/cargo) so `cargo check`-style commands work confined.
    if let Ok(home) = std::env::var("HOME") {
        let home = PathBuf::from(home);
        paths.push(home.join(".cargo"));
        paths.push(home.join(".rustup"));
    }
    paths
}

/// Character devices children commonly need (read + write).
const DEVICE_PATHS: &[&str] = &[
    "/dev/null",
    "/dev/zero",
    "/dev/full",
    "/dev/urandom",
    "/dev/random",
];

#[cfg(target_os = "linux")]
pub use linux::{detect_abi, exec_confined, restrict, TARGET_ABI};

#[cfg(target_os = "linux")]
mod linux {
    use super::{system_read_exec_paths, SandboxSpec, DEVICE_PATHS, SPEC_ENV};
    use landlock::{
        path_beneath_rules, Access, AccessFs, CompatLevel, Compatible, Ruleset, RulesetAttr,
        RulesetCreatedAttr, RulesetError, RulesetStatus, ABI,
    };
    use std::io;
    use std::os::unix::process::CommandExt;
    use std::path::PathBuf;

    /// The Landlock ABI this ruleset targets. Best-effort compat downgrades to
    /// whatever the running kernel supports; core write/`.git` confinement
    /// exists from ABI v1, so a lower kernel only loses newer, non-core knobs.
    pub const TARGET_ABI: ABI = ABI::V5;

    /// Build and apply the ruleset for `spec`, restricting the **calling**
    /// process (only ever the shim child — never the harness). Returns the
    /// enforcement status; `Err` means the ruleset could not be applied.
    ///
    /// Landlock rules are **additive** (deny-by-default; each rule only *adds*
    /// allowed access — a narrower nested rule cannot subtract what an ancestor
    /// granted). So a read-only `.git/` cannot be carved out of a blanket
    /// read+write root. Instead, for the default (non-git) profile we grant the
    /// root read+execute and read+write **per top-level entry except `.git/`**,
    /// which genuinely denies every `.git/` write (HC-5). The cost is that bash
    /// cannot create brand-new *top-level* entries under this profile (existing
    /// entries and everything inside subdirectories are writable; `write_file`
    /// is unconfined; genuine git gets the full-root profile). Requirements
    /// §6.3.
    pub fn restrict(spec: &SandboxSpec) -> Result<RulesetStatus, RulesetError> {
        let all = AccessFs::from_all(TARGET_ABI);
        let read_exec = AccessFs::from_read(TARGET_ABI);

        let mut created = Ruleset::default()
            .set_compatibility(CompatLevel::BestEffort)
            .handle_access(all)?
            .create()?;

        // System paths: read + execute (skips any that don't exist).
        created = created.add_rules(path_beneath_rules(system_read_exec_paths(), read_exec))?;
        // Common character devices: read + write.
        created = created.add_rules(path_beneath_rules(
            DEVICE_PATHS.iter().map(PathBuf::from),
            all,
        ))?;
        // Approved outside-root paths (HC-4): read + write, this invocation only.
        created = created.add_rules(path_beneath_rules(&spec.extra_writable, all))?;

        if spec.git_writable {
            // Genuine git (§6.4): the whole root, including `.git/`, is writable.
            created = created.add_rules(path_beneath_rules([&spec.root], all))?;
        } else {
            // Root readable + traversable; write granted per top-level entry
            // except `.git/`, so `.git/` stays read-only (HC-5).
            created = created.add_rules(path_beneath_rules([&spec.root], read_exec))?;
            created = created.add_rules(path_beneath_rules(writable_children(&spec.root), all))?;
        }

        Ok(created.restrict_self()?.ruleset)
    }

    /// The root's immediate entries except `.git/` — the writable set for the
    /// non-git profile. Snapshotted at spawn; a command that creates a new
    /// top-level entry and then writes into it will be denied (see `restrict`).
    fn writable_children(root: &std::path::Path) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(root) else {
            return Vec::new();
        };
        entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.file_name().is_none_or(|name| name != ".git"))
            .collect()
    }

    /// The shim body: restrict the current process to `spec`, then `exec`
    /// `/bin/sh -c command`. Never returns on success (the process image is
    /// replaced, carrying the Landlock domain across `execve`). Returns the
    /// failure `io::Error` otherwise — including a **fail-closed** error if the
    /// ruleset could not be applied (we never run a command we promised to
    /// confine without the fence).
    pub fn exec_confined(spec: &SandboxSpec, command: &str) -> io::Error {
        match restrict(spec) {
            Ok(_) => {}
            Err(error) => {
                return io::Error::other(format!("landlock confinement failed: {error}"));
            }
        }
        // Inherit the harness-scrubbed env, minus the spec var so the command
        // can't see it. Landlock is already applied to this thread and carries
        // across the exec.
        std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(command)
            .env_remove(SPEC_ENV)
            .exec()
    }

    /// Detect the highest Landlock ABI the running kernel enforces, without
    /// restricting the caller — a `HardRequirement` ruleset at each ABI (highest
    /// first) whose `create()` succeeds. `None` means no Landlock at all. Cheap:
    /// a few `landlock_create_ruleset` calls, each fd dropped immediately.
    #[must_use]
    pub fn detect_abi() -> Option<i32> {
        for abi in [ABI::V5, ABI::V4, ABI::V3, ABI::V2, ABI::V1] {
            let probe = Ruleset::default()
                .set_compatibility(CompatLevel::HardRequirement)
                .handle_access(AccessFs::from_all(abi))
                .and_then(Ruleset::create);
            if probe.is_ok() {
                return Some(abi as i32);
            }
        }
        None
    }
}
