//! Startup confinement probe (Requirements §6.7, Tech Spec §6.2, §6.5).
//!
//! Reports a [`SandboxStatus`] describing what OS-level confinement is in force.
//! It never restricts the calling (harness) process — confinement applies only
//! to spawned children (Requirements §6.7).
//!
//! Platform backends: **Linux** detects the enforced Landlock ABI
//! ([`crate::confine::detect_abi`]) and reports `Confined`/`Partial`;
//! **macOS** confirms Seatbelt works by applying a trivial profile via
//! `/usr/bin/sandbox-exec`. Any other OS reports `Unavailable` honestly. On
//! every platform the probe never restricts the harness and never crashes — an
//! absent or blocked sandbox is reported, so the harness runs the honest
//! degraded path.

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
    #[cfg(target_os = "macos")]
    {
        macos::probe()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        SandboxStatus::Unavailable {
            reason: format!("no OS sandbox backend on {}", std::env::consts::OS),
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::SandboxStatus;
    use crate::confine::seatbelt_available;

    /// macOS confinement via Seatbelt (`/usr/bin/sandbox-exec`, Tech Spec §6.3).
    /// The probe actually applies a trivial profile to a child, so a host where
    /// the binary is present but the sandbox is disabled/blocked reports
    /// `Unavailable` honestly and the harness runs the degraded path — never a
    /// false `Confined` (Requirements §6.7).
    pub(super) fn probe() -> SandboxStatus {
        if seatbelt_available() {
            SandboxStatus::Confined {
                backend: "seatbelt".to_string(),
            }
        } else {
            SandboxStatus::Unavailable {
                reason: "macOS sandbox-exec unavailable or blocked".to_string(),
            }
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
        // ABI v1; a kernel ABI below the target only loses newer, non-core knobs
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
