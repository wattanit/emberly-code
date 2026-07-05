//! `emberly-tools` — the `Tool` trait and the built-in tool suite:
//! read, write, edit, bash, glob, grep (Requirements §5, Tech Spec §5).
//!
//! Transport-agnostic by design so a future MCP adapter implements `Tool`
//! without engine changes (T-7). Phase 0: crate skeleton only. The trait and
//! tools land in Phases 1 (read/write/edit/bash) and 2 (glob/grep).
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]
