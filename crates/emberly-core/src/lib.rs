//! `emberly-core` — the engine. Owns the agent loop, the event model
//! (`UiEvent` / `TranscriptEvent`), session state, and context management
//! (Requirements §8, §9; Tech Spec §2, §3, §7).
//!
//! The engine is the single owner of all mutable session state and
//! communicates exclusively over channels (no shared mutable state), which is
//! what makes the frontend/engine separation (A-1), testability (A-2), and
//! the serializable event model (A-3) hold.
//!
//! Phase 1, group 1 (current): the event model and channel boundary — the
//! [`UiEvent`] stream out, the [`Command`] stream in, the durable
//! [`TranscriptEvent`] record, and the [`channels`] that connect the engine to
//! a frontend. The agent loop and tools land in later Phase 1 groups.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]

pub mod channels;
pub mod command;
pub mod event;
pub mod id;
pub mod transcript;
pub mod types;

pub use channels::{channel, channel_with_capacity, EnginePorts, FrontendPorts};
pub use command::Command;
pub use event::UiEvent;
pub use id::{PermissionId, SessionId, ToolCallId};
pub use transcript::{ConfigProvenance, TranscriptEvent, TranscriptRecord, SCHEMA_VERSION};
pub use types::{Mode, PermissionDecision, PermissionRendering, SandboxStatus, TokenUsage};
