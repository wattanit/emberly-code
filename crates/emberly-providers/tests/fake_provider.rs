//! Behavior coverage for [`FakeProvider`] (Phase 1, group 2). Exercises each
//! termination mode — clean, tool-use, mid-stream error, mid-stream drop,
//! connect error — plus the `chars/4` token estimate and trait-object use.
//!
//! No `.unwrap()`/`.expect()`: async tests return `Result` and thread `?`, or
//! match explicitly, so the crate's lint gate covers tests too.

use emberly_providers::{
    CompletionRequest, FakeProvider, Provider, ProviderError, ScriptedResponse, StopReason,
    StreamEvent, StreamEvent as SE,
};
use futures::StreamExt;

/// Drain a completion stream into a vector of items.
async fn drain(
    provider: &FakeProvider,
    req: CompletionRequest,
) -> Result<Vec<Result<StreamEvent, ProviderError>>, ProviderError> {
    let mut stream = provider.stream_completion(req).await?;
    let mut items = Vec::new();
    while let Some(item) = stream.next().await {
        items.push(item);
    }
    Ok(items)
}

fn req() -> CompletionRequest {
    CompletionRequest::new("fake-1")
}

#[tokio::test]
async fn yields_scripted_text_then_done() -> Result<(), ProviderError> {
    let provider = FakeProvider::new([ScriptedResponse::text("hello สวัสดี")]);
    let items = drain(&provider, req()).await?;

    assert_eq!(items.len(), 2, "one delta + one done");
    match &items[0] {
        Ok(SE::TextDelta { text }) => assert_eq!(text, "hello สวัสดี"),
        other => panic!("expected text delta, got {other:?}"),
    }
    match &items[1] {
        Ok(SE::Done { stop_reason }) => assert_eq!(*stop_reason, StopReason::EndTurn),
        other => panic!("expected done(end_turn), got {other:?}"),
    }
    Ok(())
}

#[tokio::test]
async fn tool_call_sequence_stops_for_tool_use() -> Result<(), ProviderError> {
    let provider = FakeProvider::new([ScriptedResponse::tool_call(
        "call_1",
        "read_file",
        r#"{"path":"src/main.rs"}"#,
    )]);
    let items = drain(&provider, req()).await?;

    assert!(
        items.iter().all(Result::is_ok),
        "no errors expected: {items:?}"
    );
    assert!(
        matches!(items.first(), Some(Ok(SE::ToolCallStart { name, .. })) if name == "read_file")
    );
    assert!(matches!(items.get(1), Some(Ok(SE::ToolCallDelta { .. }))));
    assert!(matches!(items.get(2), Some(Ok(SE::ToolCallEnd { .. }))));
    assert!(matches!(
        items.get(3),
        Some(Ok(SE::Done {
            stop_reason: StopReason::ToolUse
        }))
    ));
    Ok(())
}

#[tokio::test]
async fn mid_stream_error_surfaces_after_partial_text() -> Result<(), ProviderError> {
    let provider = FakeProvider::new([ScriptedResponse::error_after(
        vec![SE::TextDelta {
            text: "partial".into(),
        }],
        ProviderError::Connect("socket closed".into()),
    )]);
    let items = drain(&provider, req()).await?;

    assert!(matches!(items.first(), Some(Ok(SE::TextDelta { .. }))));
    match items.last() {
        Some(Err(e)) => assert!(e.is_retryable(), "connect error should be retryable"),
        other => panic!("expected trailing error, got {other:?}"),
    }
    // No clean Done was emitted.
    assert!(!items.iter().any(|i| matches!(i, Ok(SE::Done { .. }))));
    Ok(())
}

#[tokio::test]
async fn mid_stream_drop_ends_without_done_or_error() -> Result<(), ProviderError> {
    let provider = FakeProvider::new([ScriptedResponse::drop_after(vec![SE::TextDelta {
        text: "interrupted".into(),
    }])]);
    let items = drain(&provider, req()).await?;

    assert_eq!(items.len(), 1, "only the partial delta, then silence");
    assert!(matches!(items[0], Ok(SE::TextDelta { .. })));
    Ok(())
}

#[tokio::test]
async fn connect_error_fails_before_streaming() {
    let provider = FakeProvider::new([ScriptedResponse::connect_error(ProviderError::Auth)]);
    match provider.stream_completion(req()).await {
        Err(ProviderError::Auth) => {}
        Err(e) => panic!("expected auth error before stream, got {e:?}"),
        Ok(_) => panic!("expected auth error before stream, got a stream"),
    }
}

#[tokio::test]
async fn responses_are_consumed_in_order() -> Result<(), ProviderError> {
    let provider = FakeProvider::new([
        ScriptedResponse::text("first"),
        ScriptedResponse::text("second"),
    ]);

    let first = drain(&provider, req()).await?;
    assert!(matches!(&first[0], Ok(SE::TextDelta { text }) if text == "first"));
    let second = drain(&provider, req()).await?;
    assert!(matches!(&second[0], Ok(SE::TextDelta { text }) if text == "second"));

    // Third call: script exhausted → explicit error, not a hang.
    match provider.stream_completion(req()).await {
        Err(ProviderError::InvalidRequest(msg)) => assert!(msg.contains("exhausted")),
        Err(e) => panic!("expected exhaustion error, got {e:?}"),
        Ok(_) => panic!("expected exhaustion error, got a stream"),
    }
    Ok(())
}

#[test]
fn count_tokens_is_chars_over_four_and_flagged_approximate() {
    let provider = FakeProvider::new([]);
    let est = provider.count_tokens("12345678"); // 8 chars → 2 tokens
    assert_eq!(est.tokens, 2);
    assert!(est.approximate);

    // Rounds up (div_ceil): 9 chars → 3 tokens.
    assert_eq!(provider.count_tokens("123456789").tokens, 3);
    assert_eq!(provider.count_tokens("").tokens, 0);
}

#[tokio::test]
async fn usable_as_trait_object() -> Result<(), ProviderError> {
    let provider: Box<dyn Provider> = Box::new(FakeProvider::new([ScriptedResponse::text("hi")]));
    assert_eq!(provider.id().to_string(), "fake");
    assert_eq!(provider.model_info().context_window, 200_000);
    let mut stream = provider.stream_completion(req()).await?;
    assert!(stream.next().await.is_some());
    Ok(())
}
