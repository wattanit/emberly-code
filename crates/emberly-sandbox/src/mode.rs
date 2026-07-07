//! [`Mode`] — the auto-accept escalation tier (Requirements §6.4, Tech Spec
//! §6.6), and its confinement-gated transition.
//!
//! The auto tiers are only *reachable* while OS confinement is active. That
//! invariant is enforced here, next to [`SandboxStatus`](crate::SandboxStatus):
//! the only way to move to an auto mode is [`Mode::resolve`], which takes the
//! sandbox status and refuses to escalate when it is degraded. Relocated from
//! `emberly-core` in Phase 2 and re-exported from core.

use serde::{Deserialize, Serialize};

use crate::status::SandboxStatus;

/// Auto-accept escalation tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Prompt per the rule layer (Requirements §6.2). The always-safe default.
    #[default]
    Normal,
    /// File writes/edits inside the project root auto-allow; bash still asks
    /// per the rules.
    AutoAcceptEdits,
    /// All tools auto-run inside the project root — the OS sandbox (required for
    /// this tier) enforces the hard lines the rule layer stops asking about
    /// (root confinement; `.git` writes only via genuine git). Anything
    /// outside the root still asks (HC-4), and `Deny` rules still deny.
    Auto,
}

/// Why a requested auto mode could not be entered: confinement is not active,
/// so the safety invariant behind auto modes (a kernel fence beneath the
/// convenience layer) does not hold (Requirements §6.4, §6.7).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("auto-accept modes are unavailable without OS confinement ({reason})")]
pub struct ModeUnavailable {
    /// The sandbox reason, for a plain-language explanation in the UI.
    pub reason: String,
}

impl Mode {
    /// Whether this tier requires OS confinement to be active.
    #[must_use]
    pub fn is_auto(self) -> bool {
        matches!(self, Self::AutoAcceptEdits | Self::Auto)
    }

    /// Resolve a *requested* mode against the current confinement, the single
    /// gate for entering an auto tier (Tech Spec §6.6). [`Normal`] always
    /// resolves; an auto tier resolves only when [`SandboxStatus::allows_auto_modes`]
    /// holds, otherwise this returns [`ModeUnavailable`] and the caller stays in
    /// its current mode.
    ///
    /// [`Normal`]: Mode::Normal
    pub fn resolve(requested: Mode, sandbox: &SandboxStatus) -> Result<Mode, ModeUnavailable> {
        if requested.is_auto() && !sandbox.allows_auto_modes() {
            return Err(ModeUnavailable {
                reason: match sandbox {
                    SandboxStatus::Unavailable { reason } => reason.clone(),
                    // Unreachable while the invariant above holds, but keep an
                    // honest message rather than panicking.
                    other => format!("{other:?}"),
                },
            });
        }
        Ok(requested)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn confined() -> SandboxStatus {
        SandboxStatus::Confined {
            backend: "landlock".into(),
        }
    }
    fn partial() -> SandboxStatus {
        SandboxStatus::Partial {
            backend: "landlock".into(),
            missing: "abi v3 truncate control".into(),
        }
    }
    fn unavailable() -> SandboxStatus {
        SandboxStatus::Unavailable {
            reason: "no landlock".into(),
        }
    }

    #[test]
    fn normal_always_resolves() {
        assert_eq!(Mode::resolve(Mode::Normal, &unavailable()), Ok(Mode::Normal));
        assert_eq!(Mode::resolve(Mode::Normal, &confined()), Ok(Mode::Normal));
    }

    #[test]
    fn auto_modes_require_confinement() {
        assert_eq!(
            Mode::resolve(Mode::Auto, &confined()),
            Ok(Mode::Auto),
            "confined allows Auto"
        );
        assert_eq!(
            Mode::resolve(Mode::AutoAcceptEdits, &partial()),
            Ok(Mode::AutoAcceptEdits),
            "partial (core confinement present) still allows auto tiers"
        );
        let Err(err) = Mode::resolve(Mode::Auto, &unavailable()) else {
            panic!("degraded must block Auto");
        };
        assert!(err.reason.contains("no landlock"));
        assert!(Mode::resolve(Mode::AutoAcceptEdits, &unavailable()).is_err());
    }
}
