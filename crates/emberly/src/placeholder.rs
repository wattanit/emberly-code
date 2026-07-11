//! A stand-in [`Provider`] for the Phase 1 binary. The engine, tools,
//! permission gate, and line frontend are all wired and working, but no live
//! model backend exists until Phase 3 (Anthropic + OpenAI-compatible). This
//! placeholder streams a fixed reply for any request so `emberly` runs
//! end-to-end and the whole stack can be exercised by hand.

use async_trait::async_trait;
use emberly_providers::{
    CompletionRequest, CompletionStream, ModelInfo, Provider, ProviderError, ProviderId,
    StopReason, StreamEvent, TokenEstimate,
};

const REPLY: &str = "I'm the Phase 1 placeholder model — the engine, tools, \
permission gate, and line frontend are wired and working. Connect a real \
provider (Anthropic or an OpenAI-compatible endpoint) in Phase 3.";

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
