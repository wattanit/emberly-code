//! The confined-spawn seam as seen by tools (Tech Spec §5.1, §6.2).
//!
//! The bash tool must be able to run a command under OS confinement without
//! `emberly-tools` depending on `emberly-sandbox` (which would couple the tool
//! suite to the security crate). So it asks a [`Sandbox`] — injected into
//! [`ToolCtx`](crate::ctx::ToolCtx), the same pattern as the permission gate —
//! *how* to spawn: which program and arguments, and any extra environment. The
//! confining implementation (in `emberly-core`, over `emberly-sandbox`) returns
//! the self-exec shim; the [`PlainSandbox`] default runs the shell directly.

use std::path::{Path, PathBuf};

/// How to spawn a bash command: the program to run, its arguments, and extra
/// environment variables to set on the child (e.g. the confinement spec the
/// shim reads). The tool still owns env scrubbing, stdio, the process group,
/// the timeout, and the tree-kill guard — this only decides *what* to exec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BashInvocation {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub extra_env: Vec<(String, String)>,
}

/// The confined-spawn capability a tool spawns through. Implemented in
/// `emberly-core` over the OS confinement; tools depend only on this trait.
pub trait Sandbox: Send + Sync {
    /// How to spawn `command` (run via a shell) for a session rooted at `root`.
    /// A confining implementation returns the self-exec shim invocation; a
    /// degraded/non-Linux one returns the shell directly.
    fn bash_invocation(&self, command: &str, root: &Path) -> BashInvocation;
}

/// No OS confinement: run `/bin/sh -c <command>` directly. The default when the
/// sandbox is unavailable (degraded path) or on platforms without a backend,
/// and the convenient default for tests.
pub struct PlainSandbox;

impl Sandbox for PlainSandbox {
    fn bash_invocation(&self, command: &str, _root: &Path) -> BashInvocation {
        BashInvocation {
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".to_string(), command.to_string()],
            extra_env: Vec::new(),
        }
    }
}
