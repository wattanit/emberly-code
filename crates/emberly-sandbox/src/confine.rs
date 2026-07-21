//! OS-level child confinement (Requirements §6.2/§6.3, HC-4/HC-5; Tech Spec §6.2/§6.3).
//!
//! The **security boundary**: while the rule engine ([`crate::rules`]) decides
//! *when to ask*, this decides *what is possible*. Two backends, one policy
//! shape (Tech Spec §6.3 — "same policy shape as §6.2"):
//!
//! - **Linux (Landlock).** A Landlock ruleset applied to children via a
//!   **self-exec shim** — the harness re-execs itself under a hidden subcommand
//!   that calls `restrict_self()` (safe) and then `exec()`s the real command
//!   (safe; Landlock is inherited across `execve`). No `unsafe`, no `pre_exec`
//!   (HC-1). The ruleset (best-effort ABI): project root read+write; `.git/`
//!   under it read+execute only unless the invocation is genuine git (§6.4);
//!   system paths read+execute; approved outside-root paths read+write for that
//!   one invocation (HC-4); everything else no access.
//! - **macOS (Seatbelt).** Children run under `/usr/bin/sandbox-exec -p <profile>`
//!   — Apple's supported road to the kernel sandbox with no C FFI (HC-1). The
//!   SBPL profile enforces the same *write* lines: reads broad, writes confined
//!   to the project root, `.git/` carved back out (HC-5) unless genuine git,
//!   plus the approved outside-root paths (HC-4). See [`macos::seatbelt_profile`].
//!
//! In both cases the harness process itself is **never** confined — only
//! spawned children (Requirements §6.7). [`confined_invocation`] is the single
//! entry point the spawn bridge calls; it dispatches to the platform backend.

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

/// A confined-spawn invocation as the spawn bridge needs it: the `program` to
/// exec, its `args`, and any `extra_env` to set on the child. Built by
/// [`confined_invocation`] per platform.
pub type ConfinedInvocation = (PathBuf, Vec<String>, Vec<(String, String)>);

/// The platform's confined-spawn invocation for `command` under `spec`:
/// `(program, args, extra_env)`. On Linux this is the self-exec Landlock shim
/// (the spec rides in one env var); on macOS it is
/// `/usr/bin/sandbox-exec -p <profile> /bin/sh -c <command>` (the policy rides
/// in the profile arg — no extra env). `None` when confinement cannot be
/// expressed on this platform or `sandbox-exec` is missing — the caller then
/// falls back to a direct, honestly-unconfined shell (the degraded behavior).
#[must_use]
pub fn confined_invocation(command: &str, spec: &SandboxSpec) -> Option<ConfinedInvocation> {
    #[cfg(target_os = "linux")]
    {
        shim_invocation(command, spec).map(|(program, args, env)| (program, args, vec![env]))
    }
    #[cfg(target_os = "macos")]
    {
        macos::seatbelt_invocation(command, spec)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (command, spec);
        None
    }
}

/// System paths children may read + execute (dynamic linker, shell, toolchain).
/// Non-existent entries are silently skipped when the ruleset is built.
#[cfg(target_os = "linux")]
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
#[cfg(target_os = "linux")]
const DEVICE_PATHS: &[&str] = &[
    "/dev/null",
    "/dev/zero",
    "/dev/full",
    "/dev/urandom",
    "/dev/random",
];

#[cfg(target_os = "linux")]
pub use linux::{detect_abi, exec_confined, restrict, TARGET_ABI};

#[cfg(target_os = "macos")]
pub use macos::{seatbelt_available, seatbelt_profile, SANDBOX_EXEC};

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

#[cfg(target_os = "macos")]
mod macos {
    use super::SandboxSpec;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};

    /// The system Seatbelt front-end. Present on every supported macOS and the
    /// only supported road to the kernel sandbox without C FFI (HC-1, Tech Spec
    /// §6.3). Apple has deprecated it, but it is still shipped and functional;
    /// the risk is noted in `docs/version-0-1/PHASE5_TODO.md` group 8.
    pub const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

    /// Resolve `p` to its real path (following symlinks), falling back to the
    /// input. Seatbelt matches rules against the **resolved** path — on macOS
    /// `/var`, `/tmp`, and `$TMPDIR` are symlinks into `/private`, so a profile
    /// written with the unresolved path would silently never match. Falling back
    /// to the raw path is safe-closed: a wrong path over-denies, never
    /// over-allows.
    fn real(p: &Path) -> PathBuf {
        std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
    }

    /// Escape a path for embedding inside an SBPL double-quoted string literal
    /// (`\` and `"`). Paths are passed as a single argv element, never through a
    /// shell, so this is only about SBPL's own string syntax.
    fn quote(p: &Path) -> String {
        let raw = p.to_string_lossy();
        let mut out = String::with_capacity(raw.len() + 2);
        for ch in raw.chars() {
            if ch == '\\' || ch == '"' {
                out.push('\\');
            }
            out.push(ch);
        }
        out
    }

    /// Build the Seatbelt profile (SBPL) enforcing the §6.2 policy shape:
    /// **reads broad, writes confined to the project root**, with `.git/` carved
    /// back out (HC-5) unless this invocation is genuine git (§6.4), plus the
    /// approved outside-root paths for this one invocation (HC-4) and the
    /// character devices commands need.
    ///
    /// SBPL is **last-match-wins**, so order encodes specificity: `(allow
    /// default)` opens everything, a blanket write-deny closes all writes, then
    /// narrower allows re-open the root and devices, and the `.git` deny closes
    /// it again. Unlike Landlock (additive-only, so it needs a per-top-level-entry
    /// workaround), Seatbelt can genuinely *subtract* `.git/` from a writable
    /// root — so the macOS profile is both simpler and lets bash create new
    /// top-level entries under the root.
    ///
    /// Scope note: the hard lines enforced here are the **write** lines (no write
    /// outside root, no `.git/` write). Reads stay broad — read-confinement is a
    /// documented hardening follow-up; Landlock, by contrast, also denies reads
    /// outside its grants.
    #[must_use]
    pub fn seatbelt_profile(spec: &SandboxSpec) -> String {
        let root = real(&spec.root);
        let mut p = String::new();
        p.push_str("(version 1)\n");
        p.push_str("(allow default)\n");
        // Close every write, then re-open under the project root.
        p.push_str("(deny file-write* (subpath \"/\"))\n");
        p.push_str(&format!(
            "(allow file-write* (subpath \"{}\"))\n",
            quote(&root)
        ));
        if !spec.git_writable {
            // Genuine subtraction: `.git/` stays read-only inside a writable root.
            p.push_str(&format!(
                "(deny file-write* (subpath \"{}\"))\n",
                quote(&root.join(".git"))
            ));
        }
        // Approved outside-root paths, this invocation only (HC-4).
        for extra in &spec.extra_writable {
            p.push_str(&format!(
                "(allow file-write* (subpath \"{}\"))\n",
                quote(&real(extra))
            ));
        }
        // Character devices commands commonly write (/dev/null, /dev/tty, …).
        // Raw disk devices under /dev are root-owned, so this cannot escalate.
        p.push_str("(allow file-write* (subpath \"/dev\"))\n");
        p
    }

    /// The confined-spawn invocation on macOS:
    /// `/usr/bin/sandbox-exec -p <profile> /bin/sh -c <command>`. `None` when
    /// `sandbox-exec` is absent (the caller falls back to an unconfined shell).
    #[must_use]
    pub fn seatbelt_invocation(
        command: &str,
        spec: &SandboxSpec,
    ) -> Option<super::ConfinedInvocation> {
        if !Path::new(SANDBOX_EXEC).exists() {
            return None;
        }
        Some((
            PathBuf::from(SANDBOX_EXEC),
            vec![
                "-p".to_string(),
                seatbelt_profile(spec),
                "/bin/sh".to_string(),
                "-c".to_string(),
                command.to_string(),
            ],
            Vec::new(),
        ))
    }

    /// Whether Seatbelt confinement actually works on this host: `sandbox-exec`
    /// is present **and** successfully applies a trivial profile to a child.
    /// Running the real front-end (not a mere existence check) is the honest
    /// probe (Requirements §6.7) — it catches a host where the binary exists but
    /// the sandbox is disabled or blocked. Cheap: one short-lived `/usr/bin/true`.
    #[must_use]
    pub fn seatbelt_available() -> bool {
        if !Path::new(SANDBOX_EXEC).exists() {
            return false;
        }
        Command::new(SANDBOX_EXEC)
            .arg("-p")
            .arg("(version 1)(allow default)")
            .arg("/usr/bin/true")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn default_profile_confines_writes_and_denies_git() {
            let spec = SandboxSpec::confined_to("/private/tmp/proj");
            let profile = seatbelt_profile(&spec);
            assert!(profile.contains("(allow default)"));
            assert!(profile.contains("(deny file-write* (subpath \"/\"))"));
            assert!(profile.contains("(allow file-write* (subpath \"/private/tmp/proj\"))"));
            assert!(
                profile.contains("(deny file-write* (subpath \"/private/tmp/proj/.git\"))"),
                "default profile must deny .git writes: {profile}"
            );
        }

        #[test]
        fn git_writable_profile_omits_the_git_deny() {
            let spec = SandboxSpec {
                git_writable: true,
                ..SandboxSpec::confined_to("/private/tmp/proj")
            };
            let profile = seatbelt_profile(&spec);
            assert!(
                !profile.contains("/.git\""),
                "genuine-git profile must not deny .git: {profile}"
            );
        }

        #[test]
        fn approved_outside_paths_are_granted() {
            let spec = SandboxSpec {
                extra_writable: vec![PathBuf::from("/private/tmp/allowed")],
                ..SandboxSpec::confined_to("/private/tmp/proj")
            };
            let profile = seatbelt_profile(&spec);
            assert!(profile.contains("(allow file-write* (subpath \"/private/tmp/allowed\"))"));
        }
    }
}
