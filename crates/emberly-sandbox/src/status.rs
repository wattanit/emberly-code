//! [`SandboxStatus`] — OS-level confinement status, probed at startup and kept
//! as always-visible engine state (Requirements §6.7, Tech Spec §6.5). Emitted
//! as a `UiEvent` and written into the transcript `session_start` record.
//!
//! The probe *produces* this type, so it lives with the sandbox and core
//! re-exports it (mirroring the `ToolCallId`/`TokenUsage` pattern). The startup
//! probe lives in [`crate::probe`].

use serde::{Deserialize, Serialize};

/// OS-level confinement status.
///
/// Populated for real by [`crate::probe::probe`] (Landlock on Linux, Seatbelt
/// on macOS). The event and transcript schemas depend on this shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SandboxStatus {
    /// Full confinement granted by the OS.
    Confined { backend: String },
    /// Confinement active but missing a requested capability (e.g. an older
    /// Landlock ABI); the granted-vs-requested delta is in `missing`.
    Partial { backend: String, missing: String },
    /// No kernel confinement available; the harness runs in the honest
    /// degraded mode (allowlist suspended, auto modes locked).
    Unavailable { reason: String },
}

impl SandboxStatus {
    /// Whether auto-accept modes may be offered (Requirements §6.7): only when
    /// core write/`.git` confinement is actually enforced. A [`Partial`] status
    /// keeps auto modes *only* because the probe records a `Partial` solely for
    /// non-core capability gaps (Tech Spec §6.5) — core confinement being
    /// present is the probe's invariant for choosing `Partial` over
    /// `Unavailable`.
    ///
    /// [`Partial`]: SandboxStatus::Partial
    #[must_use]
    pub fn allows_auto_modes(&self) -> bool {
        matches!(self, Self::Confined { .. } | Self::Partial { .. })
    }

    /// Whether OS-level confinement is active at all. When `false`, the harness
    /// runs the degraded path (Requirements §6.7): the bash allowlist is
    /// suspended and the hard lines (HC-4/HC-5) are enforced at the policy
    /// (tool) layer rather than the kernel.
    #[must_use]
    pub fn is_confined(&self) -> bool {
        self.allows_auto_modes()
    }

    /// Whether the default bash allowlist may auto-allow commands. Suspended
    /// when confinement is unavailable (Requirements §6.7 item 2): without a
    /// kernel fence, "harmless" convenience must revert to per-action asking.
    #[must_use]
    pub fn bash_allowlist_active(&self) -> bool {
        self.is_confined()
    }

    /// A short, plain-language label for the kind of protection in force, used
    /// where the UI must be honest that degraded protection is *policy-level*,
    /// not kernel-level (Requirements §6.7 item 3).
    #[must_use]
    pub fn protection_level(&self) -> &'static str {
        if self.is_confined() {
            "kernel-level"
        } else {
            "policy-level"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confined_and_partial_enable_convenience() {
        let confined = SandboxStatus::Confined {
            backend: "landlock".into(),
        };
        let partial = SandboxStatus::Partial {
            backend: "landlock".into(),
            missing: "abi gap".into(),
        };
        for status in [&confined, &partial] {
            assert!(status.allows_auto_modes());
            assert!(status.bash_allowlist_active());
            assert_eq!(status.protection_level(), "kernel-level");
        }
    }

    #[test]
    fn unavailable_forces_the_degraded_path() {
        let s = SandboxStatus::Unavailable {
            reason: "no landlock".into(),
        };
        assert!(!s.allows_auto_modes());
        assert!(!s.bash_allowlist_active());
        assert!(!s.is_confined());
        assert_eq!(s.protection_level(), "policy-level");
    }
}
