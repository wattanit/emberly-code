//! Client tests for the Anthropic and OpenAI-compatible providers (Phase 3,
//! groups 2–3), driven against a `wiremock` server over **plain HTTP** — no
//! API key, no TLS, deterministic. Proves request shape and SSE→StreamEvent
//! mapping without touching the network.
//!
//! No `.unwrap()`/`.expect()`: tests thread `Result` or `panic!` with context.

use emberly_providers::{
    AnthropicProvider, CompletionRequest, Message, ModelInfo, OpenAiProvider, Provider,
    ProviderError, StopReason, StreamEvent, ToolSchema,
};
use futures::StreamExt;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A reqwest client with the pure-Rust crypto provider installed once. Needed
/// because under `--workspace` reqwest's rustls feature is unified on, so even
/// a plain-HTTP client's construction configures TLS.
fn http_client() -> reqwest::Client {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let _ = rustls_rustcrypto::provider().install_default();
    });
    reqwest::Client::new()
}

fn model_info() -> ModelInfo {
    ModelInfo {
        model: "test-model".into(),
        context_window: 200_000,
        max_output_tokens: 4096,
        pricing: None,
    }
}

fn sample_request() -> CompletionRequest {
    let mut req = CompletionRequest::new("test-model");
    req.messages = vec![Message::user_text("hello")];
    req
}

async fn drain(
    result: Result<emberly_providers::CompletionStream, ProviderError>,
) -> Vec<StreamEvent> {
    let mut stream = match result {
        Ok(s) => s,
        Err(e) => panic!("stream_completion failed: {e}"),
    };
    let mut events = Vec::new();
    while let Some(item) = stream.next().await {
        match item {
            Ok(event) => events.push(event),
            Err(e) => panic!("stream error: {e}"),
        }
    }
    events
}

fn text(events: &[StreamEvent]) -> String {
    events
        .iter()
        .filter_map(|e| match e {
            StreamEvent::TextDelta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

// ---- Anthropic ----------------------------------------------------------

const ANTHROPIC_SSE: &str = "\
event: message_start
data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":10}}}

event: content_block_delta
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello \"}}

event: content_block_delta
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"สวัสดี\"}}

event: message_delta
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":5}}

event: message_stop
data: {\"type\":\"message_stop\"}

";

#[tokio::test]
async fn anthropic_streams_text_and_usage() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(ANTHROPIC_SSE, "text/event-stream"))
        .mount(&server)
        .await;

    let provider = AnthropicProvider::new(http_client(), "test-key", server.uri(), model_info());
    let events = drain(provider.stream_completion(sample_request()).await).await;

    assert_eq!(text(&events), "Hello สวัสดี");
    assert!(events.iter().any(
        |e| matches!(e, StreamEvent::Usage { usage } if usage.input == 10 && usage.output == 5)
    ));
    assert!(matches!(
        events.last(),
        Some(StreamEvent::Done {
            stop_reason: StopReason::EndTurn
        })
    ));
}

const ANTHROPIC_TOOL_SSE: &str = "\
event: content_block_start
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"read_file\"}}

event: content_block_delta
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"path\\\":\\\"a.rs\\\"}\"}}

event: content_block_stop
data: {\"type\":\"content_block_stop\",\"index\":0}

event: message_delta
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"}}

event: message_stop
data: {\"type\":\"message_stop\"}

";

#[tokio::test]
async fn anthropic_streams_tool_call() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(ANTHROPIC_TOOL_SSE, "text/event-stream"),
        )
        .mount(&server)
        .await;

    let provider = AnthropicProvider::new(http_client(), "k", server.uri(), model_info());
    let mut req = sample_request();
    req.tools = vec![ToolSchema {
        name: "read_file".into(),
        description: "read".into(),
        input_schema: json!({"type": "object"}),
    }];
    let events = drain(provider.stream_completion(req).await).await;

    assert!(
        matches!(events.first(), Some(StreamEvent::ToolCallStart { name, .. }) if name == "read_file")
    );
    let args: String = events
        .iter()
        .filter_map(|e| match e {
            StreamEvent::ToolCallDelta { args_delta, .. } => Some(args_delta.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(args, r#"{"path":"a.rs"}"#);
    assert!(matches!(
        events.last(),
        Some(StreamEvent::Done {
            stop_reason: StopReason::ToolUse
        })
    ));
}

#[tokio::test]
async fn anthropic_maps_auth_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
        .mount(&server)
        .await;

    let provider = AnthropicProvider::new(http_client(), "bad", server.uri(), model_info());
    match provider.stream_completion(sample_request()).await {
        Err(ProviderError::Auth) => {}
        other => panic!("expected auth error, got {:?}", other.err()),
    }
}

// ---- OpenAI-compatible --------------------------------------------------

const OPENAI_SSE: &str = "\
data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}

data: {\"choices\":[{\"delta\":{\"content\":\" world\"},\"finish_reason\":null}]}

data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}

data: {\"choices\":[],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":3}}

data: [DONE]

";

#[tokio::test]
async fn openai_streams_text_and_usage() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(OPENAI_SSE, "text/event-stream"))
        .mount(&server)
        .await;

    let provider = OpenAiProvider::new(
        http_client(),
        "test-key",
        format!("{}/v1", server.uri()),
        model_info(),
    );
    let events = drain(provider.stream_completion(sample_request()).await).await;

    assert_eq!(text(&events), "Hello world");
    assert!(events.iter().any(
        |e| matches!(e, StreamEvent::Usage { usage } if usage.input == 7 && usage.output == 3)
    ));
    assert!(events.iter().any(|e| matches!(
        e,
        StreamEvent::Done {
            stop_reason: StopReason::EndTurn
        }
    )));
}

const OPENAI_TOOL_SSE: &str = "\
data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"bash\",\"arguments\":\"{\\\"cmd\\\"\"}}]},\"finish_reason\":null}]}

data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\":\\\"ls\\\"}\"}}]},\"finish_reason\":null}]}

data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}

data: [DONE]

";

#[tokio::test]
async fn openai_streams_tool_call_across_chunks() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(OPENAI_TOOL_SSE, "text/event-stream"))
        .mount(&server)
        .await;

    let provider = OpenAiProvider::new(
        http_client(),
        "k",
        format!("{}/v1", server.uri()),
        model_info(),
    );
    let events = drain(provider.stream_completion(sample_request()).await).await;

    assert!(
        matches!(events.first(), Some(StreamEvent::ToolCallStart { name, .. }) if name == "bash")
    );
    let args: String = events
        .iter()
        .filter_map(|e| match e {
            StreamEvent::ToolCallDelta { args_delta, .. } => Some(args_delta.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(args, r#"{"cmd":"ls"}"#);
    assert!(events.iter().any(|e| matches!(
        e,
        StreamEvent::Done {
            stop_reason: StopReason::ToolUse
        }
    )));
}
