//! `emberly-sandbox` — the two-layer safety model: the **rule engine**
//! ([`rules`], convenience-tier allow/ask/deny) and **OS-level confinement**
//! ([`probe`] + Landlock on Linux; Seatbelt on macOS is Phase 5) enforcing HC-4
//! and HC-5 (Requirements §6, Tech Spec §6).
//!
//! This is the security-critical crate: kept small, dependency-minimal, and
//! separately auditable (Tech Spec §1). Phase 2 lands the rule engine, the
//! startup probe + [`SandboxStatus`], the confinement-gated [`Mode`], and (on
//! Linux) the Landlock child-confinement path.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]

pub mod mode;
pub mod probe;
pub mod rules;
pub mod status;

pub use mode::{Mode, ModeUnavailable};
pub use rules::{
    bash_session_grant, parse_rules, tool_session_grant, Decision, Matcher, Outcome,
    PermissionsFile, Query, Rule, RuleEngine, RuleSource, RuleSpec, ToolSelector,
    DEFAULT_BASH_ALLOWLIST,
};
pub use status::SandboxStatus;
