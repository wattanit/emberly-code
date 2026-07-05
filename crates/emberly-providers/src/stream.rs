//! Normalized streaming events (Tech Spec §4.1). Every provider's SSE frames
//! are translated into this vocabulary; the engine consumes only these.
//!
//! **Design note (refines §4.1):** the spec lists `Err` alongside the other
//! stream events. We model it as the `Err` arm of the stream *item*
//! (`Result<StreamEvent, ProviderError>`) rather than a `StreamEvent` variant.
//! This is idiomatic Rust streaming and keeps [`StreamEvent`] cloneable and
//! serializable (it never has to hold the non-`Clone` [`ProviderError`]). The
//! semantics are identical: a clean turn ends with `Ok(Done)` then the stream
//! finishes; a failed turn yields `Err(..)`; a dropped connection simply ends
//! with neither (an interrupted turn, Tech Spec §4.3).

use std::pin::Pin;

use futures::Stream;
use serde::{Deserialize, Serialize};

use crate::error::ProviderError;
use crate::id::ToolCallId;
use crate::model::TokenUsage;

/// A single normalized event from a completion stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum StreamEvent {
    /// A chunk of assistant text.
    TextDelta { text: String },
    /// A tool call began; `name` is the tool, `id` correlates its parts.
    ToolCallStart { id: ToolCallId, name: String },
    /// A fragment of the tool call's JSON arguments, to be concatenated in
    /// order (providers stream arguments incrementally).
    ToolCallDelta { id: ToolCallId, args_delta: String },
    /// The tool call's arguments are complete.
    ToolCallEnd { id: ToolCallId },
    /// Authoritative token usage reported by the provider.
    Usage { usage: TokenUsage },
    /// The completion finished cleanly, with the reason it stopped.
    Done { stop_reason: StopReason },
}

/// Why a completion stopped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum StopReason {
    #[default]
    /// The model finished its turn normally.
    EndTurn,
    /// The model stopped to call one or more tools.
    ToolUse,
    /// The output hit the token limit.
    MaxTokens,
    /// A stop sequence was produced.
    Stop,
    /// Any other provider-specific reason, preserved verbatim.
    Other(String),
}

/// A boxed stream of normalized events. Errors are the `Err` arm (see the
/// module design note). `Send` so it can move across the engine's tasks.
pub type CompletionStream = Pin<Box<dyn Stream<Item = Result<StreamEvent, ProviderError>> + Send>>;
