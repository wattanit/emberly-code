//! `emberly-sandbox` — the two-layer safety model: the rule engine
//! (convenience-tier allow/ask/deny) and OS-level confinement (Landlock on
//! Linux, Seatbelt on macOS) enforcing HC-4 and HC-5 (Requirements §6,
//! Tech Spec §6).
//!
//! This is the security-critical crate: kept small, dependency-minimal, and
//! separately auditable (Tech Spec §1). Phase 0: crate skeleton only. The
//! rule engine and confinement land in Phases 2 (Landlock) and 5 (Seatbelt).
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]
