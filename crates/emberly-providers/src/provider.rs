//! The [`Provider`] trait (Tech Spec §4.1, Requirements P-1). A single
//! abstraction over every model backend; the engine holds a
//! `Box<dyn Provider>` and never knows which vendor is behind it.

use async_trait::async_trait;

use crate::error::ProviderError;
use crate::message::CompletionRequest;
use crate::model::{ModelInfo, ProviderId, TokenEstimate};
use crate::stream::CompletionStream;

/// A model backend. Implementations are thin, first-party clients (no vendor
/// SDKs, P-4). At least two live implementations exist before v1 ships (P-3);
/// [`FakeProvider`](crate::FakeProvider) is the offline test double (A-2).
#[async_trait]
pub trait Provider: Send + Sync {
    /// Stable backend identifier.
    fn id(&self) -> ProviderId;

    /// The active model's window and pricing.
    fn model_info(&self) -> ModelInfo;

    /// Start a streaming completion. A failure to *begin* (auth, malformed
    /// request, connect) is the returned `Err`; a mid-stream failure is the
    /// `Err` arm of a stream item.
    async fn stream_completion(
        &self,
        req: CompletionRequest,
    ) -> Result<CompletionStream, ProviderError>;

    /// Estimate the token count of some text. May be approximate (P-6) — the
    /// requirement is a reliable trigger, not exactness.
    fn count_tokens(&self, text: &str) -> TokenEstimate;
}
