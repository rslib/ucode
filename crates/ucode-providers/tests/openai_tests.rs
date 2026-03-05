use ucode_core::Event;
use ucode_providers::openai::{ToolCallAccumulator, parse_sse_line};

#[test]
fn parse_token_chunk() {
    let line = r#"data: {"id":"chatcmpl-123","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}"#;
    let mut acc = ToolCallAccumulator::default();
    let events = parse_sse_line(line, &mut acc);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0], Event::Token("Hello".into()));
}

#[test]
fn parse_empty_content_skipped() {
    let line = r#"data: {"id":"chatcmpl-123","choices":[{"index":0,"delta":{"content":""},"finish_reason":null}]}"#;
    let mut acc = ToolCallAccumulator::default();
    let events = parse_sse_line(line, &mut acc);
    assert!(events.is_empty());
}

#[test]
fn parse_done_signal() {
    let line = "data: [DONE]";
    let mut acc = ToolCallAccumulator::default();
    let events = parse_sse_line(line, &mut acc);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0], Event::Done);
}

#[test]
fn parse_non_data_line_ignored() {
    let mut acc = ToolCallAccumulator::default();
    assert!(parse_sse_line("", &mut acc).is_empty());
    assert!(parse_sse_line(": keep-alive", &mut acc).is_empty());
    assert!(parse_sse_line("event: message", &mut acc).is_empty());
}

#[test]
fn parse_tool_call_accumulated() {
    let mut acc = ToolCallAccumulator::default();

    // First chunk: tool call start with id and function name
    let line1 = r#"data: {"id":"chatcmpl-123","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_abc","function":{"name":"get_weather","arguments":""}}]},"finish_reason":null}]}"#;
    let events = parse_sse_line(line1, &mut acc);
    assert!(events.is_empty()); // Not emitted yet

    // Second chunk: argument fragment
    let line2 = r#"data: {"id":"chatcmpl-123","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"city\":"}}]},"finish_reason":null}]}"#;
    let events = parse_sse_line(line2, &mut acc);
    assert!(events.is_empty());

    // Third chunk: more arguments
    let line3 = r#"data: {"id":"chatcmpl-123","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"London\"}"}}]},"finish_reason":null}]}"#;
    let events = parse_sse_line(line3, &mut acc);
    assert!(events.is_empty());

    // Fourth chunk: finish_reason = tool_calls
    let line4 = r#"data: {"id":"chatcmpl-123","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#;
    let events = parse_sse_line(line4, &mut acc);
    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::ToolCall(tc) => {
            assert_eq!(tc.id, "call_abc");
            assert_eq!(tc.name, "get_weather");
            assert_eq!(tc.args, serde_json::json!({"city": "London"}));
        }
        other => panic!("expected ToolCall, got {other:?}"),
    }
}

#[test]
fn parse_done_drains_pending_tool_calls() {
    let mut acc = ToolCallAccumulator::default();

    // Start a tool call
    let line1 = r#"data: {"id":"chatcmpl-123","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_xyz","function":{"name":"read_file","arguments":"{\"path\":\"foo.rs\"}"}}]},"finish_reason":null}]}"#;
    parse_sse_line(line1, &mut acc);

    // [DONE] should drain pending tool calls
    let events = parse_sse_line("data: [DONE]", &mut acc);
    assert_eq!(events.len(), 2); // ToolCall + Done
    assert!(matches!(&events[0], Event::ToolCall(_)));
    assert_eq!(events[1], Event::Done);
}

#[test]
fn parse_multiple_tokens() {
    let mut acc = ToolCallAccumulator::default();
    let mut all_events = Vec::new();

    let lines = [
        r#"data: {"id":"chatcmpl-123","choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}"#,
        r#"data: {"id":"chatcmpl-123","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}"#,
        r#"data: {"id":"chatcmpl-123","choices":[{"index":0,"delta":{"content":" world"},"finish_reason":null}]}"#,
        r#"data: {"id":"chatcmpl-123","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
        "data: [DONE]",
    ];

    for line in &lines {
        all_events.extend(parse_sse_line(line, &mut acc));
    }

    assert_eq!(all_events.len(), 3); // "Hello", " world", Done
    assert_eq!(all_events[0], Event::Token("Hello".into()));
    assert_eq!(all_events[1], Event::Token(" world".into()));
    assert_eq!(all_events[2], Event::Done);
}

#[test]
fn openai_provider_name_and_capabilities() {
    use ucode_providers::{OpenaiProvider, Provider};

    let provider = OpenaiProvider::new("test-key".into());
    assert_eq!(provider.name(), "openai");

    let caps = provider.capabilities();
    assert!(caps.tool_calls);
    assert!(caps.json_mode);
    assert!(caps.streaming);
    assert_eq!(caps.max_context, 128_000);
    assert_eq!(caps.max_output, 16_384);
}
