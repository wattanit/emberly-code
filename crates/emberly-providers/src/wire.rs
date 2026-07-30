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

/// Default per-chunk idle window (issue #11): a live model can legitimately
/// go quiet for a while between tokens, but a dead connection producing
/// nothing for this long is presumed stalled and torn down. `reqwest`'s
/// overall request timeout can't tell those two apart, so this is enforced
/// independently, per SSE chunk, inside the stream loop.
pub(crate) const DEFAULT_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(90);

/// Drive a streaming response body through `mapper`, yielding normalized
/// events. Transport errors mid-stream surface as the `Err` arm and end the
/// stream; the stream ending without a `Done` is a drop (the engine decides
/// whether to retry the turn). `idle_timeout` bounds how long the stream may
/// go without producing a chunk before it's torn down as stalled.
pub(crate) fn sse_completion_stream<M: SseMapper>(
    response: reqwest::Response,
    mapper: M,
    idle_timeout: Duration,
) -> CompletionStream {
    sse_stream_from_body(Box::pin(response.bytes_stream()), mapper, idle_timeout)
}

fn sse_stream_from_body<M: SseMapper>(
    body: Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>,
    mapper: M,
    idle_timeout: Duration,
) -> CompletionStream {
    struct State<M> {
        body: Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>,
        parser: SseParser,
        mapper: M,
        queue: VecDeque<Result<StreamEvent, ProviderError>>,
        finished: bool,
        idle_timeout: Duration,
    }

    let state = State {
        body,
        parser: SseParser::new(),
        mapper,
        queue: VecDeque::new(),
        finished: false,
        idle_timeout,
    };

    let stream = futures::stream::unfold(state, |mut state| async move {
        loop {
            if let Some(item) = state.queue.pop_front() {
                return Some((item, state));
            }
            if state.finished {
                return None;
            }
            match tokio::time::timeout(state.idle_timeout, state.body.next()).await {
                Ok(Some(Ok(bytes))) => {
                    for event in state.parser.push(&bytes) {
                        state.queue.extend(state.mapper.map(event));
                    }
                }
                Ok(Some(Err(error))) => {
                    state
                        .queue
                        .push_back(Err(ProviderError::Connect(error.to_string())));
                    state.finished = true;
                }
                Ok(None) => {
                    if let Some(event) = state.parser.finish() {
                        state.queue.extend(state.mapper.map(event));
                    }
                    state.finished = true;
                }
                Err(_elapsed) => {
                    state
                        .queue
                        .push_back(Err(ProviderError::Timeout(state.idle_timeout)));
                    state.finished = true;
                }
            }
        }
    });

    Box::pin(stream)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream;

    struct NoopMapper;

    impl SseMapper for NoopMapper {
        fn map(&mut self, _event: SseEvent) -> Vec<Result<StreamEvent, ProviderError>> {
            Vec::new()
        }
    }

    #[tokio::test(start_paused = true)]
    async fn idle_stream_times_out_instead_of_hanging_forever() {
        let body: Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>> = Box::pin(
            stream::once(async { Ok(Bytes::from_static(b": ping\n\n")) }).chain(stream::pending()),
        );
        let mut events = sse_stream_from_body(body, NoopMapper, Duration::from_millis(50));

        match events.next().await {
            Some(Err(ProviderError::Timeout(dur))) => {
                assert_eq!(dur, Duration::from_millis(50));
            }
            other => panic!("expected a Timeout error, got {other:?}"),
        }
        assert!(
            events.next().await.is_none(),
            "stream must end after the timeout"
        );
    }

    #[tokio::test]
    async fn active_stream_is_unaffected_by_the_idle_timeout() {
        let body: Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>> =
            Box::pin(stream::iter(vec![Ok(Bytes::from_static(b"data: {}\n\n"))]));
        let mut events = sse_stream_from_body(body, NoopMapper, Duration::from_secs(90));
        assert!(events.next().await.is_none());
    }
}
