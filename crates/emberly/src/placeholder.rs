//! A stand-in [`Provider`] used only to satisfy `EngineConfig.provider` when
//! no `[providers.*]` profile is configured — `Engine` now refuses a
//! `Command::UserInput` before ever calling it (Requirements C-7: `/model`'s
//! guided wizard is the fix, not a canned reply), so `stream_completion`
//! below should be unreachable in normal operation. It stays deliberately
//! inert rather than a plausible-looking fake reply, in case some future
//! code path calls it anyway.

use async_trait::async_trait;
use emberly_providers::{
    CompletionRequest, CompletionStream, ModelInfo, Provider, ProviderError, ProviderId,
    StopReason, StreamEvent, TokenEstimate,
};

const REPLY: &str = "no provider is configured for this session — run /model to add one";

pub struct PlaceholderProvider;

impl PlaceholderProvider {
    pub fn new() -> Self {
        Self
    }
}

impl Default for PlaceholderProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Provider for PlaceholderProvider {
    fn id(&self) -> ProviderId {
        ProviderId::new("placeholder")
    }

    fn model_info(&self) -> ModelInfo {
        ModelInfo {
            model: "placeholder".into(),
            context_window: 8_192,
            max_output_tokens: 1_024,
            pricing: None,
            effort_levels: Vec::new(),
            default_effort: None,
            vision: false,
            documents: false,
        }
    }

    async fn stream_completion(
        &self,
        _request: CompletionRequest,
    ) -> Result<CompletionStream, ProviderError> {
        let events = vec![
            Ok(StreamEvent::TextDelta { text: REPLY.into() }),
            Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
            }),
        ];
        Ok(Box::pin(futures::stream::iter(events)))
    }

    fn count_tokens(&self, text: &str) -> TokenEstimate {
        let chars = u64::try_from(text.chars().count()).unwrap_or(u64::MAX);
        TokenEstimate {
            tokens: chars.div_ceil(4),
            approximate: true,
        }
    }
}
