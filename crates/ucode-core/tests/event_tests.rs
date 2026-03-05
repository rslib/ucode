use futures_util::stream::{self, StreamExt};
use serde_json::json;
use ucode_core::{AuthErrorKind, CoreError, Event, ToolCall, ToolResult};

fn roundtrip<T>(value: &T) -> T
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let json = serde_json::to_string(value).expect("serialize");
    serde_json::from_str(&json).expect("deserialize")
}

#[test]
fn token_roundtrip() {
    let e = Event::Token("hello".into());
    assert_eq!(roundtrip(&e), e);
}

#[test]
fn tool_call_roundtrip() {
    let e = Event::ToolCall(ToolCall::new("c1", "search", json!({"q": "rust"})));
    assert_eq!(roundtrip(&e), e);
}

#[test]
fn tool_result_roundtrip() {
    let e = Event::ToolResult(ToolResult::new("c1", "search", json!({"hits": 3}), false));
    assert_eq!(roundtrip(&e), e);
}

#[test]
fn patch_roundtrip() {
    let e = Event::Patch(r#"[{"op":"add","path":"/x","value":1}]"#.into());
    assert_eq!(roundtrip(&e), e);
}

#[test]
fn log_roundtrip() {
    let e = Event::Log("debug info".into());
    assert_eq!(roundtrip(&e), e);
}

#[test]
fn error_provider_roundtrip() {
    let e = Event::Error(CoreError::Provider {
        provider: "openai".into(),
        message: "rate limited".into(),
    });
    assert_eq!(roundtrip(&e), e);
}

#[test]
fn error_auth_roundtrip() {
    let e = Event::Error(CoreError::Auth {
        provider: "anthropic".into(),
        auth_kind: AuthErrorKind::Expired,
    });
    assert_eq!(roundtrip(&e), e);
}

#[test]
fn error_context_too_large_roundtrip() {
    let e = Event::Error(CoreError::ContextTooLarge {
        limit: 8192,
        actual: 10000,
    });
    assert_eq!(roundtrip(&e), e);
}

#[test]
fn error_tool_failed_roundtrip() {
    let e = Event::Error(CoreError::ToolFailed {
        tool: "bash".into(),
        message: "exit 1".into(),
    });
    assert_eq!(roundtrip(&e), e);
}

#[test]
fn error_timeout_roundtrip() {
    let e = Event::Error(CoreError::Timeout {
        operation: "llm_call".into(),
        duration_ms: 30_000,
    });
    assert_eq!(roundtrip(&e), e);
}

#[test]
fn error_internal_roundtrip() {
    let e = Event::Error(CoreError::Internal {
        message: "unexpected state".into(),
    });
    assert_eq!(roundtrip(&e), e);
}

#[test]
fn done_roundtrip() {
    let e = Event::Done;
    assert_eq!(roundtrip(&e), e);
}

#[tokio::test]
async fn stream_transcript_reconstruction() {
    let events = vec![
        Event::Token("Hello".into()),
        Event::Token(", ".into()),
        Event::Token("world".into()),
        Event::ToolCall(ToolCall::new("c1", "search", json!({"q": "rust"}))),
        Event::ToolResult(ToolResult::new("c1", "search", json!({"hits": 5}), false)),
        Event::Token("!".into()),
        Event::Log("done processing".into()),
        Event::Done,
    ];

    let mut transcript = String::new();
    let mut tool_call_count = 0usize;
    let mut done_seen = false;

    let mut s = stream::iter(events);
    while let Some(event) = s.next().await {
        match event {
            Event::Token(t) => transcript.push_str(&t),
            Event::ToolCall(_) => tool_call_count += 1,
            Event::Done => {
                done_seen = true;
                break;
            }
            _ => {}
        }
    }

    assert_eq!(transcript, "Hello, world!");
    assert_eq!(tool_call_count, 1);
    assert!(done_seen);
}

#[test]
fn auth_error_kind_serde() {
    assert_eq!(
        serde_json::to_string(&AuthErrorKind::Missing).unwrap(),
        r#""missing""#
    );
    assert_eq!(
        serde_json::to_string(&AuthErrorKind::Invalid).unwrap(),
        r#""invalid""#
    );
    assert_eq!(
        serde_json::to_string(&AuthErrorKind::Expired).unwrap(),
        r#""expired""#
    );
}

#[test]
fn core_error_display() {
    let e = CoreError::Provider {
        provider: "openai".into(),
        message: "rate limited".into(),
    };
    assert_eq!(e.to_string(), "provider 'openai' error: rate limited");

    let e = CoreError::ContextTooLarge {
        limit: 8192,
        actual: 10000,
    };
    assert_eq!(e.to_string(), "context too large: limit 8192, actual 10000");

    let e = CoreError::Timeout {
        operation: "llm_call".into(),
        duration_ms: 5000,
    };
    assert_eq!(e.to_string(), "operation 'llm_call' timed out after 5000ms");
}
