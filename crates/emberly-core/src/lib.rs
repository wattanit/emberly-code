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
pub mod engine;
pub mod event;
pub mod gate;
pub mod id;
pub mod prompts;
pub mod resume;
pub mod transcript;
pub mod types;

pub use channels::{channel, channel_with_capacity, EnginePorts, FrontendPorts};
pub use command::Command;
pub use emberly_providers::{Message, RetryPolicy};
pub use engine::{Engine, EngineConfig};
pub use event::UiEvent;
pub use gate::PermissionAsk;
pub use id::{PermissionId, SessionId, ToolCallId};
pub use transcript::{
    append_abnormal_exit, CaptureSink, ConfigProvenance, FileTranscript, NoopSink, TranscriptEvent,
    TranscriptRecord, TranscriptSink, SCHEMA_VERSION,
};
pub use types::{Mode, PermissionDecision, PermissionRendering, SandboxStatus, TokenUsage};
