//! `emberly-providers` — the `Provider` trait and first-party provider
//! clients (Anthropic Messages API, OpenAI-compatible), behind a single
//! normalized abstraction (Requirements P-1, Tech Spec §4).
//!
//! Phase 0: crate skeleton only. The `Provider` trait, `FakeProvider`, and
//! live clients land in Phases 1 and 3.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]
