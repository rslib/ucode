use serde::{Deserialize, Serialize};

/// An MCP tool definition discovered from a server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolDef {
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "inputSchema")]
    pub input_schema: serde_json::Value,
}

/// Result of calling an MCP tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolResult {
    pub content: Vec<McpContent>,
    /// The MCP spec uses `isError` (camelCase) on the wire.
    #[serde(rename = "isError", default)]
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum McpContent {
    #[serde(rename = "text")]
    Text { text: String },
}

/// Server info from initialize response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

/// Server capabilities from initialize response.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServerCapabilities {
    #[serde(default)]
    pub tools: Option<serde_json::Value>,
    #[serde(default)]
    pub resources: Option<serde_json::Value>,
    #[serde(default)]
    pub prompts: Option<serde_json::Value>,
}

/// An MCP resource definition discovered from a server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpResourceDef {
    pub uri: String,
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "mimeType")]
    pub mime_type: Option<String>,
}

/// Content of a resolved MCP resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpResourceContent {
    pub uri: String,
    #[serde(rename = "mimeType")]
    pub mime_type: Option<String>,
    pub text: Option<String>,
    /// Base64-encoded binary content.
    pub blob: Option<String>,
}

/// An MCP prompt definition discovered from a server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpPromptDef {
    pub name: String,
    pub description: Option<String>,
    pub arguments: Option<Vec<McpPromptArgument>>,
}

/// An argument for an MCP prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpPromptArgument {
    pub name: String,
    pub description: Option<String>,
    pub required: Option<bool>,
}

/// A message returned from prompt resolution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpPromptMessage {
    pub role: String,
    pub content: McpPromptMessageContent,
}

/// Content of a prompt message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum McpPromptMessageContent {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "resource")]
    Resource { resource: McpResourceContent },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deserialize_tool_def() {
        let raw = json!({
            "name": "read_file",
            "description": "Read a file from disk",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" }
                },
                "required": ["path"]
            }
        });
        let tool: McpToolDef = serde_json::from_value(raw).unwrap();
        assert_eq!(tool.name, "read_file");
        assert_eq!(tool.description.as_deref(), Some("Read a file from disk"));
        assert_eq!(tool.input_schema["type"], "object");
    }

    #[test]
    fn deserialize_tool_def_no_description() {
        let raw = json!({
            "name": "ping",
            "inputSchema": { "type": "object" }
        });
        let tool: McpToolDef = serde_json::from_value(raw).unwrap();
        assert_eq!(tool.name, "ping");
        assert!(tool.description.is_none());
    }

    #[test]
    fn deserialize_tool_result_text() {
        let raw = json!({
            "content": [
                { "type": "text", "text": "hello world" }
            ],
            "isError": false
        });
        let result: McpToolResult = serde_json::from_value(raw).unwrap();
        assert!(!result.is_error);
        assert_eq!(result.content.len(), 1);
        let McpContent::Text { text } = &result.content[0];
        assert_eq!(text, "hello world");
    }

    #[test]
    fn deserialize_tool_result_error() {
        let raw = json!({
            "content": [
                { "type": "text", "text": "file not found" }
            ],
            "isError": true
        });
        let result: McpToolResult = serde_json::from_value(raw).unwrap();
        assert!(result.is_error);
        let McpContent::Text { text } = &result.content[0];
        assert_eq!(text, "file not found");
    }

    #[test]
    fn deserialize_tool_result_default_is_error() {
        // isError defaults to false when absent
        let raw = json!({
            "content": [{ "type": "text", "text": "ok" }]
        });
        let result: McpToolResult = serde_json::from_value(raw).unwrap();
        assert!(!result.is_error);
    }

    #[test]
    fn test_deserialize_resource_def() {
        let raw = json!({
            "uri": "file:///tmp/data.csv",
            "name": "data_csv",
            "description": "A CSV data file",
            "mimeType": "text/csv"
        });
        let def: McpResourceDef = serde_json::from_value(raw).unwrap();
        assert_eq!(def.uri, "file:///tmp/data.csv");
        assert_eq!(def.name, "data_csv");
        assert_eq!(def.description.as_deref(), Some("A CSV data file"));
        assert_eq!(def.mime_type.as_deref(), Some("text/csv"));
    }

    #[test]
    fn test_deserialize_resource_def_minimal() {
        let raw = json!({
            "uri": "mem://scratch",
            "name": "scratch"
        });
        let def: McpResourceDef = serde_json::from_value(raw).unwrap();
        assert_eq!(def.uri, "mem://scratch");
        assert_eq!(def.name, "scratch");
        assert!(def.description.is_none());
        assert!(def.mime_type.is_none());
    }

    #[test]
    fn test_deserialize_resource_content_text() {
        let raw = json!({
            "uri": "file:///tmp/hello.txt",
            "mimeType": "text/plain",
            "text": "hello world"
        });
        let content: McpResourceContent = serde_json::from_value(raw).unwrap();
        assert_eq!(content.uri, "file:///tmp/hello.txt");
        assert_eq!(content.mime_type.as_deref(), Some("text/plain"));
        assert_eq!(content.text.as_deref(), Some("hello world"));
        assert!(content.blob.is_none());
    }

    #[test]
    fn test_deserialize_resource_content_blob() {
        let raw = json!({
            "uri": "file:///tmp/img.png",
            "mimeType": "image/png",
            "blob": "aGVsbG8="
        });
        let content: McpResourceContent = serde_json::from_value(raw).unwrap();
        assert_eq!(content.uri, "file:///tmp/img.png");
        assert_eq!(content.mime_type.as_deref(), Some("image/png"));
        assert!(content.text.is_none());
        assert_eq!(content.blob.as_deref(), Some("aGVsbG8="));
    }

    #[test]
    fn test_deserialize_prompt_def() {
        let raw = json!({
            "name": "summarize",
            "description": "Summarize a document",
            "arguments": [
                { "name": "text", "description": "The text to summarize", "required": true },
                { "name": "length", "description": "Desired length", "required": false }
            ]
        });
        let def: McpPromptDef = serde_json::from_value(raw).unwrap();
        assert_eq!(def.name, "summarize");
        assert_eq!(def.description.as_deref(), Some("Summarize a document"));
        let args = def.arguments.as_ref().unwrap();
        assert_eq!(args.len(), 2);
        assert_eq!(args[0].name, "text");
        assert_eq!(args[0].required, Some(true));
        assert_eq!(args[1].name, "length");
        assert_eq!(args[1].required, Some(false));
    }

    #[test]
    fn test_deserialize_prompt_def_no_args() {
        let raw = json!({
            "name": "hello",
            "description": "Say hello"
        });
        let def: McpPromptDef = serde_json::from_value(raw).unwrap();
        assert_eq!(def.name, "hello");
        assert!(def.arguments.is_none());
    }

    #[test]
    fn test_deserialize_prompt_message_text() {
        let raw = json!({
            "role": "user",
            "content": { "type": "text", "text": "Please summarize this." }
        });
        let msg: McpPromptMessage = serde_json::from_value(raw).unwrap();
        assert_eq!(msg.role, "user");
        let McpPromptMessageContent::Text { text } = &msg.content else {
            panic!("expected Text variant");
        };
        assert_eq!(text, "Please summarize this.");
    }

    #[test]
    fn test_deserialize_prompt_message_resource() {
        let raw = json!({
            "role": "assistant",
            "content": {
                "type": "resource",
                "resource": {
                    "uri": "file:///tmp/doc.txt",
                    "mimeType": "text/plain",
                    "text": "document contents"
                }
            }
        });
        let msg: McpPromptMessage = serde_json::from_value(raw).unwrap();
        assert_eq!(msg.role, "assistant");
        let McpPromptMessageContent::Resource { resource } = &msg.content else {
            panic!("expected Resource variant");
        };
        assert_eq!(resource.uri, "file:///tmp/doc.txt");
        assert_eq!(resource.text.as_deref(), Some("document contents"));
    }

    #[test]
    fn test_server_capabilities_with_resources_and_prompts() {
        let raw = json!({
            "tools": { "listChanged": true },
            "resources": { "subscribe": false },
            "prompts": {}
        });
        let caps: ServerCapabilities = serde_json::from_value(raw).unwrap();
        assert!(caps.tools.is_some());
        assert!(caps.resources.is_some());
        assert!(caps.prompts.is_some());
    }
}
