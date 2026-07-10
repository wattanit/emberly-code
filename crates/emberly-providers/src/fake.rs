//! [`FakeProvider`] — the scripted test double that makes the whole agent
//! loop, permission flow, truncation, and compaction testable offline and
//! deterministically (Requirements A-2, Tech Spec §14.1).
//!
//! Construct it with a queue of [`ScriptedResponse`]s, one consumed per
//! `stream_completion` call (each agent turn is one completion). A response
//! scripts the events yielded and how the stream terminates — cleanly, with a
//! mid-stream error, with a silent mid-stream drop, or with a pre-stream
//! connect error.

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::error::ProviderError;
use crate::id::ToolCallId;
use crate::message::CompletionRequest;
use crate::model::{Effort, ModelInfo, ProviderId, TokenEstimate};
use crate::provider::Provider;
use crate::stream::{CompletionStream, StopReason, StreamEvent};

/// How a scripted stream terminates after its events are yielded.
pub enum ScriptOutcome {
    /// End cleanly with `Ok(Done { stop_reason })`.
    Done(StopReason),
    /// Yield the events, then an `Err` (a mid-stream provider failure).
    Error(ProviderError),
    /// End the stream with no `Done` and no `Err` — a silent connection drop
    /// producing an interrupted turn (Tech Spec §4.3).
    Drop,
    /// Fail before streaming: `stream_completion` returns this `Err` and no
    /// stream is produced. `events` are ignored.
    ConnectError(ProviderError),
}

/// One scripted completion: the events to yield and how it ends.
pub struct ScriptedResponse {
    pub events: Vec<StreamEvent>,
    pub outcome: ScriptOutcome,
}

impl ScriptedResponse {
    /// A single assistant text turn that ends cleanly.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            events: vec![StreamEvent::TextDelta { text: text.into() }],
            outcome: ScriptOutcome::Done(StopReason::EndTurn),
        }
    }

    /// A turn that streams several text deltas, then ends cleanly.
    #[must_use]
    pub fn text_deltas<I, S>(parts: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            events: parts
                .into_iter()
                .map(|p| StreamEvent::TextDelta { text: p.into() })
                .collect(),
            outcome: ScriptOutcome::Done(StopReason::EndTurn),
        }
    }

    /// A turn that requests one tool call (start → args → end) and stops for
    /// tool use. `args_json` is streamed as a single argument fragment.
    #[must_use]
    pub fn tool_call(
        id: impl Into<String>,
        name: impl Into<String>,
        args_json: impl Into<String>,
    ) -> Self {
        let id = ToolCallId::new(id);
        Self {
            events: vec![
                StreamEvent::ToolCallStart {
                    id: id.clone(),
                    name: name.into(),
                },
                StreamEvent::ToolCallDelta {
                    id: id.clone(),
                    args_delta: args_json.into(),
                },
                StreamEvent::ToolCallEnd { id },
            ],
            outcome: ScriptOutcome::Done(StopReason::ToolUse),
        }
    }

    /// A turn that yields `events`, then fails mid-stream with `error`.
    #[must_use]
    pub fn error_after(events: Vec<StreamEvent>, error: ProviderError) -> Self {
        Self {
            events,
            outcome: ScriptOutcome::Error(error),
        }
    }

    /// A turn that yields `events`, then the connection silently drops.
    #[must_use]
    pub fn drop_after(events: Vec<StreamEvent>) -> Self {
        Self {
            events,
            outcome: ScriptOutcome::Drop,
        }
    }

    /// A completion that fails to start (returned as an `Err`).
    #[must_use]
    pub fn connect_error(error: ProviderError) -> Self {
        Self {
            events: Vec::new(),
            outcome: ScriptOutcome::ConnectError(error),
        }
    }
}

/// A [`Provider`] that replays a scripted queue of responses.
pub struct FakeProvider {
    id: ProviderId,
    model_info: ModelInfo,
    scripts: Mutex<VecDeque<ScriptedResponse>>,
}

impl FakeProvider {
    /// Build a fake provider from an ordered sequence of scripted responses.
    #[must_use]
    pub fn new(scripts: impl IntoIterator<Item = ScriptedResponse>) -> Self {
        Self {
            id: ProviderId::new("fake"),
            model_info: ModelInfo {
                model: "fake-1".into(),
                context_window: 200_000,
                max_output_tokens: 8_192,
                pricing: None,
                // The fake model exposes the full ladder so effort round-trips
                // are testable headlessly (Tech Spec §14).
                effort_levels: Effort::ALL.to_vec(),
                default_effort: Some(Effort::Medium),
            },
            scripts: Mutex::new(scripts.into_iter().collect()),
        }
    }

    /// Override the reported model info (window/pricing) for accounting tests.
    #[must_use]
    pub fn with_model_info(mut self, info: ModelInfo) -> Self {
        self.model_info = info;
        self
    }

    /// Pop the next scripted response, recovering from a poisoned lock rather
    /// than panicking (HC-3: no `unwrap`/`expect`).
    fn pop(&self) -> Option<ScriptedResponse> {
        let mut queue = match self.scripts.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        queue.pop_front()
    }
}

#[async_trait]
impl Provider for FakeProvider {
    fn id(&self) -> ProviderId {
        self.id.clone()
    }

    fn model_info(&self) -> ModelInfo {
        self.model_info.clone()
    }

    async fn stream_completion(
        &self,
        _req: CompletionRequest,
    ) -> Result<CompletionStream, ProviderError> {
        let Some(response) = self.pop() else {
            // Under-scripted test: fail loudly rather than looping forever.
            return Err(ProviderError::InvalidRequest(
                "fake provider: script exhausted".into(),
            ));
        };

        if let ScriptOutcome::ConnectError(error) = response.outcome {
            return Err(error);
        }

        let mut items: Vec<Result<StreamEvent, ProviderError>> =
            response.events.into_iter().map(Ok).collect();
        match response.outcome {
            ScriptOutcome::Done(stop_reason) => {
                items.push(Ok(StreamEvent::Done { stop_reason }));
            }
            ScriptOutcome::Error(error) => items.push(Err(error)),
            ScriptOutcome::Drop => {}
            // Handled above; unreachable but matched exhaustively.
            ScriptOutcome::ConnectError(_) => {}
        }

        Ok(Box::pin(futures::stream::iter(items)))
    }

    fn count_tokens(&self, text: &str) -> TokenEstimate {
        // chars/4 approximation (P-6): a reliable trigger, not an exact count.
        let chars = u64::try_from(text.chars().count()).unwrap_or(u64::MAX);
        TokenEstimate {
            tokens: chars.div_ceil(4),
            approximate: true,
        }
    }
}
