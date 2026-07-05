//! `emberly-core` — the engine. Owns the agent loop, the event model
//! (`UiEvent` / `TranscriptEvent`), session state, and context management
//! (Requirements §8, §9; Tech Spec §2, §3, §7).
//!
//! The engine is the single owner of all mutable session state and
//! communicates exclusively over channels (no shared mutable state), which is
//! what makes the frontend/engine separation (A-1), testability (A-2), and
//! the serializable event model (A-3) hold. Phase 0: crate skeleton only.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]
