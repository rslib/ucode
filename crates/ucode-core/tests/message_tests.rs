use serde_json::json;
use ucode_core::{Message, Part, Role, ToolCall, ToolResult};

fn roundtrip<T>(value: &T) -> T
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let json = serde_json::to_string(value).expect("serialize");
    serde_json::from_str(&json).expect("deserialize")
}

#[test]
fn text_message_roundtrip() {
    let msg = Message::user("hello world");
    assert_eq!(roundtrip(&msg), msg);
}

#[test]
fn tool_call_roundtrip() {
    let tc = ToolCall::new("call-1", "search", json!({"query": "rust streams"}));
    let msg = Message::new(Role::Assistant, vec![Part::ToolCall(tc)]);
    assert_eq!(roundtrip(&msg), msg);
}

#[test]
fn tool_result_roundtrip() {
    let tr = ToolResult::new("call-1", "search", json!({"hits": 42}), false);
    let msg = Message::new(Role::Tool, vec![Part::ToolResult(tr)]);
    assert_eq!(roundtrip(&msg), msg);
}

#[test]
fn tool_result_error_roundtrip() {
    let tr = ToolResult::new("call-2", "exec", json!({"stderr": "not found"}), true);
    let msg = Message::new(Role::Tool, vec![Part::ToolResult(tr)]);
    assert_eq!(roundtrip(&msg), msg);
}

#[test]
fn convenience_user() {
    let msg = Message::user("hi");
    assert_eq!(msg.role, Role::User);
    assert_eq!(msg.parts, vec![Part::Text("hi".into())]);
}

#[test]
fn convenience_assistant() {
    let msg = Message::assistant("sure");
    assert_eq!(msg.role, Role::Assistant);
    assert_eq!(msg.parts, vec![Part::Text("sure".into())]);
}

#[test]
fn convenience_system() {
    let msg = Message::system("you are helpful");
    assert_eq!(msg.role, Role::System);
    assert_eq!(msg.parts, vec![Part::Text("you are helpful".into())]);
}

#[test]
fn convenience_tool_result() {
    let msg = Message::tool_result("id-1", "calc", json!(7), false);
    assert_eq!(msg.role, Role::Tool);
    assert_eq!(msg.parts.len(), 1);
    let Part::ToolResult(tr) = &msg.parts[0] else {
        panic!("expected ToolResult part");
    };
    assert_eq!(tr.id, "id-1");
    assert_eq!(tr.name, "calc");
    assert_eq!(tr.result, json!(7));
    assert!(!tr.is_error);
}

#[test]
fn mixed_parts_roundtrip() {
    let msg = Message::new(
        Role::Assistant,
        vec![
            Part::Text("I'll search for that.".into()),
            Part::ToolCall(ToolCall::new("c1", "search", json!({"q": "rust"}))),
        ],
    );
    assert_eq!(roundtrip(&msg), msg);
}

#[test]
fn role_serde_snake_case() {
    assert_eq!(serde_json::to_string(&Role::System).unwrap(), r#""system""#);
    assert_eq!(serde_json::to_string(&Role::User).unwrap(), r#""user""#);
    assert_eq!(
        serde_json::to_string(&Role::Assistant).unwrap(),
        r#""assistant""#
    );
    assert_eq!(serde_json::to_string(&Role::Tool).unwrap(), r#""tool""#);
}
