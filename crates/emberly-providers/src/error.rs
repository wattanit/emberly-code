//! [`ProviderError`] — the single error type the provider layer surfaces
//! (Tech Spec §4.3, §11). Library crate, so `thiserror` (never `anyhow`).
//!
//! Retry classification lives here as [`ProviderError::is_retryable`] /
//! [`ProviderError::retry_after`], so the retry policy is a
//! consequence of the error type rather than scattered matching.

use std::time::Duration;

use thiserror::Error;

/// An error from a provider backend, either before streaming (returned from
/// `stream_completion`) or mid-stream (the `Err` arm of the stream item).
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ProviderError {
    /// Transport/connection failure (DNS, TCP, TLS, dropped socket).
    #[error("connection failed: {0}")]
    Connect(String),

    /// Authentication rejected (401/403). Not retryable.
    #[error("authentication failed")]
    Auth,

    /// Rate limited (429), optionally with a server-provided backoff hint.
    #[error("rate limited")]
    RateLimited { retry_after: Option<Duration> },

    /// A non-success HTTP status not covered by a more specific variant.
    #[error("provider returned HTTP {status}: {message}")]
    Http { status: u16, message: String },

    /// The provider reported an application-level error in its response body.
    #[error("provider error: {message}")]
    Api { message: String },

    /// The request was malformed before it left the harness.
    #[error("invalid request: {0}")]
    InvalidRequest(String),

    /// The response (or an SSE frame) could not be decoded.
    #[error("failed to decode provider response: {0}")]
    Decode(String),

    /// The in-flight request was canceled by the user (Command::Cancel).
    #[error("request canceled")]
    Canceled,

    /// No SSE data arrived for the given idle window; the connection is
    /// presumed dead (issue #11 — the overall request timeout alone can't
    /// tell a stalled connection apart from legitimately slow generation).
    #[error("stream stalled: no data received for {0:?}")]
    Timeout(Duration),
}

impl ProviderError {
    /// Whether an idempotent retry is safe (Tech Spec §4.3): connection
    /// errors, 429, and 5xx. Auth, invalid-request, decode, and cancel are
    /// terminal.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Connect(_) | Self::RateLimited { .. } | Self::Timeout(_) => true,
            Self::Http { status, .. } => *status >= 500,
            _ => false,
        }
    }

    /// A server-provided backoff hint, when present (429 `Retry-After`).
    #[must_use]
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::RateLimited { retry_after } => *retry_after,
            _ => None,
        }
    }
}
