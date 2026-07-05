//! Shared HTTP/SSE plumbing for the live provider clients: turning a non-2xx
//! response into a typed [`ProviderError`], and driving a streaming response
//! body through the [`SseParser`] and a provider-specific [`SseMapper`] into a
//! normalized [`CompletionStream`].

use std::collections::VecDeque;
use std::pin::Pin;
use std::time::Duration;

use bytes::Bytes;
use futures::{Stream, StreamExt};

use crate::error::ProviderError;
use crate::sse::{SseEvent, SseParser};
use crate::stream::{CompletionStream, StreamEvent};

/// Map a completed (non-streaming) response's status into `Ok` or a typed
/// error. Reads and includes a clipped body on failure for diagnostics.
pub(crate) async fn check_response(
    response: reqwest::Response,
) -> Result<reqwest::Response, ProviderError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let retry_after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|text| text.parse::<u64>().ok())
        .map(Duration::from_secs);
    let code = status.as_u16();
    let body = response.text().await.unwrap_or_default();
    Err(match code {
        401 | 403 => ProviderError::Auth,
        429 => ProviderError::RateLimited { retry_after },
        _ => ProviderError::Http {
            status: code,
            message: clip(&body),
        },
    })
}

fn clip(body: &str) -> String {
    let trimmed = body.trim();
    trimmed.chars().take(500).collect()
}

/// Provider-specific translation of an SSE event into zero or more normalized
/// stream events. Stateful (tool-call ids/indices accumulate across events).
pub(crate) trait SseMapper: Send + 'static {
    fn map(&mut self, event: SseEvent) -> Vec<Result<StreamEvent, ProviderError>>;
}

/// Drive a streaming response body through `mapper`, yielding normalized
/// events. Transport errors mid-stream surface as the `Err` arm and end the
/// stream; the stream ending without a `Done` is a drop (the engine decides
/// whether to retry the turn).
pub(crate) fn sse_completion_stream<M: SseMapper>(
    response: reqwest::Response,
    mapper: M,
) -> CompletionStream {
    struct State<M> {
        body: Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>,
        parser: SseParser,
        mapper: M,
        queue: VecDeque<Result<StreamEvent, ProviderError>>,
        finished: bool,
    }

    let state = State {
        body: Box::pin(response.bytes_stream()),
        parser: SseParser::new(),
        mapper,
        queue: VecDeque::new(),
        finished: false,
    };

    let stream = futures::stream::unfold(state, |mut state| async move {
        loop {
            if let Some(item) = state.queue.pop_front() {
                return Some((item, state));
            }
            if state.finished {
                return None;
            }
            match state.body.next().await {
                Some(Ok(bytes)) => {
                    for event in state.parser.push(&bytes) {
                        state.queue.extend(state.mapper.map(event));
                    }
                }
                Some(Err(error)) => {
                    state
                        .queue
                        .push_back(Err(ProviderError::Connect(error.to_string())));
                    state.finished = true;
                }
                None => {
                    if let Some(event) = state.parser.finish() {
                        state.queue.extend(state.mapper.map(event));
                    }
                    state.finished = true;
                }
            }
        }
    });

    Box::pin(stream)
}
