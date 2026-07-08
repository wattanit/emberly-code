//! The confined-spawn bridge (Tech Spec §5.1, §6.2). Implements the tools-side
//! [`Sandbox`] trait over the OS confinement in `emberly-sandbox`, so the bash
//! tool spawns confined children without `emberly-tools` depending on the
//! security crate. `emberly-core` is the one crate that sees both.

use std::path::{Path, PathBuf};

use emberly_sandbox::confine::shim_invocation;
use emberly_sandbox::git::is_genuine_git;
use emberly_sandbox::SandboxSpec;
use emberly_tools::{BashInvocation, PlainSandbox, Sandbox};

/// Spawns bash commands through the self-exec Landlock shim when confinement is
/// active, and directly (the degraded path) otherwise. `confined` comes from
/// the startup [`SandboxStatus`](emberly_sandbox::SandboxStatus): it is only
/// ever true where the shim actually enforces (Linux with Landlock), so the
/// confined branch never runs where the shim would be a no-op.
///
/// Genuine-git resolution (§6.4): a command earns the `.git/`-writable profile
/// only when it is a simple invocation of the recorded canonical git binary
/// (`git_binary` + `path_env`); everything else runs with `.git/` read-only.
pub struct HostSandbox {
    confined: bool,
    /// The canonical system git binary recorded at session start (`which git`),
    /// or `None` if git was not found — then nothing is `.git/`-writable.
    git_binary: Option<PathBuf>,
    /// The `PATH` the confined child runs with, for re-resolving `git`.
    path_env: String,
}

impl HostSandbox {
    #[must_use]
    pub fn new(confined: bool, git_binary: Option<PathBuf>, path_env: String) -> Self {
        Self {
            confined,
            git_binary,
            path_env,
        }
    }
}

impl Sandbox for HostSandbox {
    fn bash_invocation(&self, command: &str, root: &Path) -> BashInvocation {
        if self.confined {
            let git_writable =
                is_genuine_git(command, self.git_binary.as_deref(), root, &self.path_env);
            let spec = SandboxSpec {
                root: root.to_path_buf(),
                git_writable,
                extra_writable: Vec::new(),
            };
            if let Some((program, args, env)) = shim_invocation(command, &spec) {
                return BashInvocation {
                    program,
                    args,
                    extra_env: vec![env],
                };
            }
        }
        // Degraded, non-Linux, or executable path unknown: run the shell
        // directly — honestly unconfined.
        PlainSandbox.bash_invocation(command, root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The confinement spec JSON carried in the invocation's env.
    fn spec_json(inv: &BashInvocation) -> &str {
        for (key, value) in &inv.extra_env {
            if key == emberly_sandbox::SPEC_ENV {
                return value;
            }
        }
        panic!("confined invocation is missing the spec env var");
    }

    #[test]
    fn genuine_git_earns_the_writable_profile_others_do_not() {
        let path_env = std::env::var("PATH").unwrap_or_default();
        let Some(git) = emberly_sandbox::git::record_git_binary(&path_env) else {
            eprintln!("SKIP: no system git");
            return;
        };
        // A root that does not contain the system git binary.
        let root = std::env::temp_dir();
        let host = HostSandbox::new(true, Some(git), path_env);

        let genuine = host.bash_invocation("git commit -m hi", &root);
        assert!(
            spec_json(&genuine).contains("\"git_writable\":true"),
            "genuine git → writable .git profile: {}",
            spec_json(&genuine)
        );

        for command in ["rm -rf build", "git commit && rm -rf .git", "./git commit"] {
            let inv = host.bash_invocation(command, &root);
            assert!(
                spec_json(&inv).contains("\"git_writable\":false"),
                "`{command}` must NOT earn the writable .git profile: {}",
                spec_json(&inv)
            );
        }
    }

    #[test]
    fn degraded_host_spawns_the_shell_directly() {
        let host = HostSandbox::new(false, None, String::new());
        let inv = host.bash_invocation("echo hi", std::path::Path::new("/tmp"));
        assert_eq!(inv.program, std::path::PathBuf::from("/bin/sh"));
        assert!(
            inv.extra_env.is_empty(),
            "no confinement spec when degraded"
        );
    }
}
