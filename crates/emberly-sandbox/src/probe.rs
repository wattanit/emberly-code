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
    use crate::confine::{detect_abi, TARGET_ABI};

    pub(super) fn probe() -> SandboxStatus {
        // Detect the highest ABI the kernel enforces without confining the
        // harness (Requirements §6.7). Core write/`.git` confinement exists from
        // ABI v1; a kernel ABI below our target only loses newer, non-core knobs
        // → `Partial`, never a weakening of the hard lines (Tech Spec §6.5).
        let target = TARGET_ABI as i32;
        match detect_abi() {
            None => SandboxStatus::Unavailable {
                reason: "kernel has no Landlock support (need >= 5.13 with the LSM enabled)"
                    .to_string(),
            },
            Some(abi) if abi >= target => SandboxStatus::Confined {
                backend: format!("landlock (ABI v{abi})"),
            },
            Some(abi) => SandboxStatus::Partial {
                backend: format!("landlock (ABI v{abi})"),
                missing: format!(
                    "kernel ABI v{abi} < target v{target}; core confinement active, newer knobs unavailable"
                ),
            },
        }
    }
}
