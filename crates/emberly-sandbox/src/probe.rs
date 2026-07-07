//! Startup confinement probe (Requirements §6.7, Tech Spec §6.2, §6.5).
//!
//! Reports a [`SandboxStatus`] describing what OS-level confinement is in force.
//! It never restricts the calling (harness) process — confinement applies only
//! to spawned children (Requirements §6.7).
//!
//! Platform state (Phase 2, foundation checkpoint): the non-Linux branch
//! reports `Unavailable` honestly (Seatbelt is Phase 5). On Linux the
//! `landlock` crate deliberately does not expose runtime ABI detection — it
//! steers callers toward building a best-effort ruleset and reading the
//! `RestrictionStatus` produced by `restrict_self()` in the confined **child**
//! (the self-exec shim). That child path is Phase 2 group 3; until it lands the
//! Linux branch reports `Unavailable` with a reason, so the harness runs the
//! honest degraded path everywhere. Wiring group 3 flips this to the real
//! granted-vs-requested status without touching the degradation policy below.

use crate::status::SandboxStatus;

/// Probe OS-level confinement availability at startup. Never fails: an absent
/// or partial sandbox is reported honestly, never as an error (Requirements
/// §6.7 — never crash, never pretend).
#[must_use]
pub fn probe() -> SandboxStatus {
    #[cfg(target_os = "linux")]
    {
        linux::probe()
    }
    #[cfg(not(target_os = "linux"))]
    {
        SandboxStatus::Unavailable {
            reason: format!("no OS sandbox backend on {}", std::env::consts::OS),
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::SandboxStatus;
    use landlock::ABI;

    /// The Landlock ABI this build's child ruleset (group 3) targets. Recorded
    /// here so the status detail and the group-3 ruleset agree on one number.
    /// Core write/`.git` confinement exists from ABI v1, so a lower kernel ABI
    /// is only ever a *non-core* capability gap → `Partial`, never a downgrade
    /// of the hard lines (Tech Spec §6.5).
    const TARGET_ABI: ABI = ABI::V5;

    pub(super) fn probe() -> SandboxStatus {
        // Group 3 replaces this with detection via the confined child's
        // `RestrictionStatus`. Until then we do not confine children, so
        // reporting anything but `Unavailable` would be dishonest.
        let _ = TARGET_ABI;
        SandboxStatus::Unavailable {
            reason: "Landlock child confinement not yet wired (Phase 2 group 3)".to_string(),
        }
    }
}
