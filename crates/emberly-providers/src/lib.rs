//! `emberly-providers` — the [`Provider`] trait and the normalized message,
//! tool-schema, and streaming vocabulary that every backend maps to and from
//! (Requirements P-1, Tech Spec §4). No provider-specific wire type ever
//! crosses this boundary.
//!
//! Phase 1, group 2 (current): the trait, the normalized types, and the
//! scripted [`FakeProvider`] (A-2). The live Anthropic and OpenAI-compatible
//! clients (P-2, P-3) land in Phase 3.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]

pub mod error;
pub mod fake;
pub mod id;
pub mod message;
pub mod model;
pub mod provider;
pub mod stream;

pub use error::ProviderError;
pub use fake::{FakeProvider, ScriptOutcome, ScriptedResponse};
pub use id::ToolCallId;
pub use message::{CompletionRequest, ContentBlock, Message, Role, ToolSchema};
pub use model::{ModelInfo, Pricing, ProviderId, TokenEstimate, TokenUsage};
pub use provider::Provider;
pub use stream::{CompletionStream, StopReason, StreamEvent};
