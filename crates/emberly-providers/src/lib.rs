//! `emberly-providers` — the [`Provider`] trait and the normalized message,
//! tool-schema, and streaming vocabulary that every backend maps to and from
//! (Requirements P-1, Tech Spec §4). No provider-specific wire type ever
//! crosses this boundary.
//!
//! The trait, the normalized types, the scripted [`FakeProvider`] used by
//! tests (A-2), and the live Anthropic and OpenAI-compatible clients (P-2,
//! P-3).
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]

pub mod anthropic;
pub mod auth;
pub mod error;
pub mod fake;
pub mod id;
pub mod message;
pub mod model;
pub mod openai;
pub mod provider;
pub mod retry;
pub mod sse;
pub mod stream;
mod wire;

pub use anthropic::AnthropicProvider;
pub use auth::Auth;
pub use error::ProviderError;
pub use fake::{FakeProvider, ScriptOutcome, ScriptedResponse};
pub use id::ToolCallId;
pub use message::{CompletionRequest, ContentBlock, Message, Role, ToolSchema};
pub use model::{Effort, ModelInfo, Pricing, ProviderId, TokenEstimate, TokenUsage};
pub use openai::OpenAiProvider;
pub use provider::Provider;
pub use retry::RetryPolicy;
pub use stream::{CompletionStream, StopReason, StreamEvent};
pub use wire::StreamTimeouts;
