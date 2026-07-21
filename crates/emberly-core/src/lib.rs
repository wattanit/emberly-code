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
pub mod factory;
pub mod gate;
pub mod id;
pub mod memory;
pub mod prompts;
pub mod provider_writer;
pub mod resume;
pub mod skills;
pub mod spawn;
pub mod transcript;
pub mod types;
pub mod view_cache;

pub use channels::{channel, channel_with_capacity, EnginePorts, FrontendPorts};
pub use command::Command;
pub use emberly_providers::{Message, RetryPolicy};
pub use emberly_tools::AskUserOutcome;
pub use emberly_tools::{MemoryOp, MemoryScope};
pub use emberly_tools::{SkillMeta, SkillOrigin};
pub use emberly_tools::{TaskItem, TaskStatus};
pub use engine::{
    CompletionCheck, CompletionConfig, ContextConfig, Engine, EngineConfig, LoopConfig,
    MemoryConfig, SkillsConfig,
};
pub use event::UiEvent;
pub use factory::{ConfigReloader, ProviderChoice, ProviderFactory, ReloadedConfig};
pub use gate::{AskUserAsk, MemoryAsk, PermissionAsk, SkillAsk, TaskListAsk};
pub use id::{AskId, PermissionId, SessionId, ToolCallId};
pub use memory::EntrySummary;
pub use provider_writer::{NewProviderProfile, ProviderProfileWriter};
pub use transcript::{
    append_abnormal_exit, CaptureSink, CompactTrigger, ConfigProvenance, FileTranscript, NoopSink,
    TranscriptEvent, TranscriptRecord, TranscriptSink, SCHEMA_VERSION,
};
pub use types::{
    AskAnswer, CheckResult, Effort, GateResolution, LoopResolution, Mode, PermissionDecision,
    PermissionRendering, SandboxStatus, TokenUsage,
};
pub use view_cache::{view_cache_path, ViewCache, VIEW_CACHE_VERSION};
// The rule engine and confinement probe live in the security crate; re-export
// the pieces the composition root (the binary) wires so it depends only on core.
pub use emberly_sandbox::probe::probe;
pub use emberly_sandbox::{parse_rules, ModeUnavailable, Rule, RuleEngine, RuleSource};
