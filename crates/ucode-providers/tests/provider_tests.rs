use futures_util::StreamExt;

use ucode_core::{CoreError, Event, Message};
use ucode_providers::{Capabilities, ChatRequest, MockProvider, Provider};

#[tokio::test]
async fn mock_provider_name() {
    let provider = MockProvider::new(vec!["hello".into()]);
    assert_eq!(provider.name(), "mock");
}

#[tokio::test]
async fn mock_provider_default_capabilities() {
    let provider = MockProvider::new(vec![]);
    let caps = provider.capabilities();
    assert!(!caps.tool_calls);
    assert!(!caps.json_mode);
    assert_eq!(caps.max_context, 128_000);
    assert_eq!(caps.max_output, 4_096);
    assert!(caps.streaming);
    assert!(!caps.token_counting);
}

#[tokio::test]
async fn mock_provider_custom_capabilities() {
    let caps = Capabilities {
        tool_calls: true,
        json_mode: true,
        max_context: 200_000,
        max_output: 8_192,
        streaming: true,
        token_counting: true,
    };
    let provider = MockProvider::new(vec![]).with_capabilities(caps.clone());
    let reported = provider.capabilities();
    assert!(reported.tool_calls);
    assert!(reported.json_mode);
    assert_eq!(reported.max_context, 200_000);
    assert_eq!(reported.max_output, 8_192);
    assert!(reported.token_counting);
}

#[tokio::test]
async fn mock_provider_streams_tokens_then_done() {
    let provider = MockProvider::new(vec!["Hello".into(), " world".into(), "!".into()]);
    let req = ChatRequest::new("test-model", vec![Message::user("hi")]);

    let mut stream = provider.stream_chat(req).await.unwrap();

    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(event);
    }

    assert_eq!(events.len(), 4); // 3 tokens + Done
    assert_eq!(events[0], Event::Token("Hello".into()));
    assert_eq!(events[1], Event::Token(" world".into()));
    assert_eq!(events[2], Event::Token("!".into()));
    assert_eq!(events[3], Event::Done);
}

#[tokio::test]
async fn mock_provider_empty_tokens_just_done() {
    let provider = MockProvider::new(vec![]);
    let req = ChatRequest::new("test-model", vec![Message::user("hi")]);

    let mut stream = provider.stream_chat(req).await.unwrap();

    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(event);
    }

    assert_eq!(events.len(), 1);
    assert_eq!(events[0], Event::Done);
}

#[tokio::test]
async fn mock_provider_count_tokens_returns_none() {
    let provider = MockProvider::new(vec![]);
    let msgs = vec![Message::user("hello")];
    assert!(provider.count_tokens(&msgs).is_none());
}

#[tokio::test]
async fn chat_request_new_defaults() {
    let req = ChatRequest::new("gpt-4o", vec![Message::user("test")]);
    assert_eq!(req.model, "gpt-4o");
    assert_eq!(req.messages.len(), 1);
    assert!(req.temperature.is_none());
    assert!(req.max_tokens.is_none());
    assert!(req.tools.is_empty());
    assert!(!req.json_mode);
}

#[tokio::test]
async fn capabilities_default_all_false() {
    let caps = Capabilities::default();
    assert!(!caps.tool_calls);
    assert!(!caps.json_mode);
    assert_eq!(caps.max_context, 0);
    assert_eq!(caps.max_output, 0);
    assert!(!caps.streaming);
    assert!(!caps.token_counting);
}

#[test]
fn auth_error_kind_variants() {
    let err = CoreError::Auth {
        provider: "openai".into(),
        auth_kind: ucode_core::AuthErrorKind::Invalid,
    };
    assert!(matches!(
        err,
        CoreError::Auth {
            auth_kind: ucode_core::AuthErrorKind::Invalid,
            ..
        }
    ));

    let err = CoreError::Auth {
        provider: "test".into(),
        auth_kind: ucode_core::AuthErrorKind::Missing,
    };
    assert!(matches!(
        err,
        CoreError::Auth {
            auth_kind: ucode_core::AuthErrorKind::Missing,
            ..
        }
    ));

    let err = CoreError::Auth {
        provider: "test".into(),
        auth_kind: ucode_core::AuthErrorKind::Expired,
    };
    assert!(matches!(
        err,
        CoreError::Auth {
            auth_kind: ucode_core::AuthErrorKind::Expired,
            ..
        }
    ));
}
