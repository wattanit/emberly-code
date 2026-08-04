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

// ---------------------------------------------------------------------------
// Tunables. Every bound this module enforces is named here, so the numbers can
// be reviewed together rather than hunted through the code. These are the
// defaults `StreamTimeouts::default()` falls back to; `[stream]` in
// config.toml overrides them per the composition root (#15).
// ---------------------------------------------------------------------------

/// How long to wait for the **first** chunk of a completion stream.
///
/// This covers work that happens before any token exists — the provider queueing
/// the request and prefilling a possibly very large prompt — so it is much longer
/// than [`DEFAULT_STREAM_IDLE_TIMEOUT`] (issue #15).
///
/// Sharing one window with the idle timeout is what made #15 bite, and it bit
/// *non-reasoning* models hardest, which reads backwards until you see why: a
/// reasoning model streams thinking tokens almost at once, so bytes flow early
/// and keep resetting the timer, while a non-reasoning model sends nothing at all
/// until its first output token — so the whole prefill elapses with the
/// connection silent but perfectly healthy.
const DEFAULT_FIRST_CHUNK_TIMEOUT: Duration = Duration::from_secs(300);

/// How long a completion stream may go without a chunk **once it has started**
/// (issue #11).
///
/// Bytes have already flowed, so a gap this long is a dead connection rather than
/// a slow model. Any traffic resets it, including SSE keep-alive comments and
/// Anthropic's `ping` events, because the window wraps the body read rather than
/// the production of a normalized event.
const DEFAULT_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(90);

/// Cap on how much of a failed response's body is read for diagnostics.
/// [`clip`] keeps only the first 500 chars, so this just has to be comfortably
/// above that — the point is that the peer does not choose how much we buffer.
const MAX_ERROR_BODY_BYTES: usize = 8 * 1024;

/// How long to wait for that body, per read. It is a diagnostic on a path where
/// the typed error is already decided from the status code, so a peer that sends
/// its headers and then goes quiet must not hold the request open: we keep what
/// arrived and report the error anyway.
const ERROR_BODY_TIMEOUT: Duration = Duration::from_secs(10);

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
    let body = read_error_body(Box::pin(response.bytes_stream())).await;
    Err(match code {
        401 | 403 => ProviderError::Auth,
        429 => ProviderError::RateLimited { retry_after },
        _ => ProviderError::Http {
            status: code,
            message: clip(&body),
        },
    })
}

/// Read a failed response's body for diagnostics, bounded in both size and time.
///
/// `reqwest::Response::text` is bounded by neither: it buffers whatever the peer
/// sends, for as long as the peer takes. Nor does an overall request timeout
/// cover it — the client deliberately sets none, because a streaming completion
/// legitimately runs for minutes — so both bounds belong here. Same reasoning as
/// the per-chunk idle timeout on the stream loop below, applied to the one other
/// place this crate reads a whole body.
///
/// Takes the body stream rather than the `Response` so it is testable without a
/// server (as with [`sse_stream_from_body`]), and is generic over the stream's
/// error — which it discards, since the status code already decided the outcome —
/// so a test can supply a failing body without conjuring a `reqwest::Error`.
async fn read_error_body<E: 'static>(
    mut body: Pin<Box<dyn Stream<Item = Result<Bytes, E>> + Send>>,
) -> String {
    let mut buf: Vec<u8> = Vec::new();
    while buf.len() < MAX_ERROR_BODY_BYTES {
        // Ends the read on a stall, on end-of-body, or on a transport error
        // mid-body — keeping whatever arrived before it.
        let Ok(Some(Ok(chunk))) = tokio::time::timeout(ERROR_BODY_TIMEOUT, body.next()).await
        else {
            break;
        };
        let room = MAX_ERROR_BODY_BYTES - buf.len();
        buf.extend_from_slice(&chunk[..chunk.len().min(room)]);
    }
    String::from_utf8_lossy(&buf).into_owned()
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

/// The two liveness windows a completion stream needs (issues #11, #15). Both
/// are enforced per SSE chunk inside the stream loop, because no client-level
/// request timeout can tell a slow generation from a dead connection.
///
/// The defaults, and why they differ by so much, are at the top of this file.
/// Configurable (`[stream]` in `config.toml`) because how long is reasonable
/// depends on the inference engine on the other end — a local/cloud model's
/// prefill and token-generation speed is nothing the client controls (#15).
#[derive(Debug, Clone, Copy)]
pub struct StreamTimeouts {
    /// Waiting for the first chunk — see [`DEFAULT_FIRST_CHUNK_TIMEOUT`].
    pub first_chunk: Duration,
    /// Waiting for a later chunk — see [`DEFAULT_STREAM_IDLE_TIMEOUT`].
    pub idle: Duration,
}

impl StreamTimeouts {
    #[must_use]
    pub fn new(first_chunk: Duration, idle: Duration) -> Self {
        Self { first_chunk, idle }
    }
}

impl Default for StreamTimeouts {
    fn default() -> Self {
        Self {
            first_chunk: DEFAULT_FIRST_CHUNK_TIMEOUT,
            idle: DEFAULT_STREAM_IDLE_TIMEOUT,
        }
    }
}

/// Drive a streaming response body through `mapper`, yielding normalized
/// events. Transport errors mid-stream surface as the `Err` arm and end the
/// stream; the stream ending without a `Done` is a drop (the engine decides
/// whether to retry the turn). `timeouts` bounds how long the stream may go
/// without producing a chunk before it's torn down as stalled.
pub(crate) fn sse_completion_stream<M: SseMapper>(
    response: reqwest::Response,
    mapper: M,
    timeouts: StreamTimeouts,
) -> CompletionStream {
    sse_stream_from_body(Box::pin(response.bytes_stream()), mapper, timeouts)
}

fn sse_stream_from_body<M: SseMapper>(
    body: Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>,
    mapper: M,
    timeouts: StreamTimeouts,
) -> CompletionStream {
    struct State<M> {
        body: Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>,
        parser: SseParser,
        mapper: M,
        queue: VecDeque<Result<StreamEvent, ProviderError>>,
        finished: bool,
        timeouts: StreamTimeouts,
        /// Whether any chunk has arrived, selecting which window applies.
        started: bool,
    }

    let state = State {
        body,
        parser: SseParser::new(),
        mapper,
        queue: VecDeque::new(),
        finished: false,
        timeouts,
        started: false,
    };

    let stream = futures::stream::unfold(state, |mut state| async move {
        loop {
            if let Some(item) = state.queue.pop_front() {
                return Some((item, state));
            }
            if state.finished {
                return None;
            }
            // Before the first chunk this is waiting on queueing and prefill;
            // after it, on the next token (issue #15).
            let window = if state.started {
                state.timeouts.idle
            } else {
                state.timeouts.first_chunk
            };
            match tokio::time::timeout(window, state.body.next()).await {
                Ok(Some(Ok(bytes))) => {
                    state.started = true;
                    for event in state.parser.push(&bytes) {
                        state.queue.extend(state.mapper.map(event));
                    }
                    // A frame past the size cap ends the stream, after the
                    // complete events that preceded it. Terminal, not retryable:
                    // a peer flooding one frame would only do it again.
                    if state.parser.overflowed() {
                        state.queue.push_back(Err(ProviderError::Decode(format!(
                            "a single SSE frame exceeded {} bytes",
                            crate::sse::MAX_FRAME_BYTES
                        ))));
                        state.finished = true;
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
                    // Name the window that actually elapsed, so the message says
                    // whether the model never started or went quiet mid-answer.
                    state.queue.push_back(Err(ProviderError::Timeout(window)));
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

    /// Short, clearly distinct windows so a test can tell which one fired.
    fn windows() -> StreamTimeouts {
        StreamTimeouts {
            first_chunk: Duration::from_millis(500),
            idle: Duration::from_millis(50),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn idle_stream_times_out_instead_of_hanging_forever() {
        // A stream that started and then went quiet trips the *idle* window.
        let body: Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>> = Box::pin(
            stream::once(async { Ok(Bytes::from_static(b": ping\n\n")) }).chain(stream::pending()),
        );
        let mut events = sse_stream_from_body(body, NoopMapper, windows());

        match events.next().await {
            Some(Err(ProviderError::Timeout(dur))) => {
                assert_eq!(dur, windows().idle, "the idle window should have fired");
            }
            other => panic!("expected a Timeout error, got {other:?}"),
        }
        assert!(
            events.next().await.is_none(),
            "stream must end after the timeout"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_slow_first_token_gets_the_longer_window() {
        // Issue #15: a model that has sent nothing yet is still queueing or
        // prefilling, not dead. The wait before the first byte must not be
        // governed by the short inter-chunk window — this is the case that made
        // non-reasoning models look broken while reasoning models worked, because
        // only the latter emit bytes early enough to keep resetting the timer.
        let quiet_then_answer = stream::once(async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            Ok(Bytes::from_static(b"data: hello\n\n"))
        });
        let body: Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>> =
            Box::pin(quiet_then_answer);
        let mut events = sse_stream_from_body(body, NoopMapper, windows());
        // 200ms is past the 50ms idle window but inside the 500ms first-chunk
        // window, so the stream must survive to its clean end.
        assert!(
            events.next().await.is_none(),
            "a slow first token must not be torn down as a stall"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_first_chunk_that_never_arrives_still_times_out() {
        // The longer window is still a bound, and its error names it, so the
        // message distinguishes "never started" from "went quiet mid-answer".
        let body: Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>> =
            Box::pin(stream::pending());
        let mut events = sse_stream_from_body(body, NoopMapper, windows());
        match events.next().await {
            Some(Err(ProviderError::Timeout(dur))) => {
                assert_eq!(dur, windows().first_chunk);
            }
            other => panic!("expected a Timeout error, got {other:?}"),
        }
    }

    /// A stand-in for the transport error `read_error_body` discards.
    #[derive(Debug)]
    struct BodyError;

    type TestBody = Pin<Box<dyn Stream<Item = Result<Bytes, BodyError>> + Send>>;

    #[tokio::test]
    async fn error_body_is_capped_rather_than_buffered_whole() {
        // A peer does not get to choose how much we buffer on the error path.
        let huge = Bytes::from(vec![b'x'; MAX_ERROR_BODY_BYTES * 4]);
        let body: TestBody = Box::pin(stream::iter(vec![Ok(huge.clone()), Ok(huge)]));
        assert_eq!(read_error_body(body).await.len(), MAX_ERROR_BODY_BYTES);
    }

    #[tokio::test(start_paused = true)]
    async fn a_stalled_error_body_does_not_hang_the_request() {
        // Headers arrived, so the typed error is already decided; a peer that
        // then goes quiet must not hold the request open. What arrived is kept.
        let body: TestBody = Box::pin(
            stream::once(async { Ok(Bytes::from_static(b"rate limit details")) })
                .chain(stream::pending()),
        );
        assert_eq!(read_error_body(body).await, "rate limit details");
    }

    #[tokio::test]
    async fn a_transport_error_mid_body_keeps_what_arrived() {
        let body: TestBody = Box::pin(
            stream::iter(vec![Ok(Bytes::from_static(b"partial detail"))])
                .chain(stream::once(async { Err(BodyError) })),
        );
        assert_eq!(read_error_body(body).await, "partial detail");
    }

    #[tokio::test]
    async fn active_stream_is_unaffected_by_the_idle_timeout() {
        let body: Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>> =
            Box::pin(stream::iter(vec![Ok(Bytes::from_static(b"data: {}\n\n"))]));
        let mut events = sse_stream_from_body(body, NoopMapper, StreamTimeouts::default());
        assert!(events.next().await.is_none());
    }
}
