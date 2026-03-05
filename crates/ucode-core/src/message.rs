use serde::{Deserialize, Serialize};

/// Conversation participant role.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// A tool invocation requested by the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub args: serde_json::Value,
}

/// The result of executing a tool call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    pub id: String,
    pub name: String,
    pub result: serde_json::Value,
    pub is_error: bool,
}

/// A single content unit within a message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "value")]
pub enum Part {
    Text(String),
    ToolCall(ToolCall),
    ToolResult(ToolResult),
}

/// A single turn in a conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub parts: Vec<Part>,
}

impl ToolCall {
    pub fn new(id: impl Into<String>, name: impl Into<String>, args: serde_json::Value) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            args,
        }
    }
}

impl ToolResult {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        result: serde_json::Value,
        is_error: bool,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            result,
            is_error,
        }
    }
}

impl Message {
    pub fn new(role: Role, parts: Vec<Part>) -> Self {
        Self { role, parts }
    }

    pub fn user(text: impl Into<String>) -> Self {
        Self::new(Role::User, vec![Part::Text(text.into())])
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self::new(Role::Assistant, vec![Part::Text(text.into())])
    }

    pub fn system(text: impl Into<String>) -> Self {
        Self::new(Role::System, vec![Part::Text(text.into())])
    }

    pub fn tool_result(
        id: impl Into<String>,
        name: impl Into<String>,
        result: serde_json::Value,
        is_error: bool,
    ) -> Self {
        Self::new(
            Role::Tool,
            vec![Part::ToolResult(ToolResult::new(
                id, name, result, is_error,
            ))],
        )
    }
}
