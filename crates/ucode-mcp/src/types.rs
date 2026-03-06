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
}
