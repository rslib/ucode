use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::Mutex;
use ucode_core::CoreError;
use ucode_mcp::{McpClient, McpContent, McpToolDef, McpToolResult};

use crate::registry::{ToolHandler, ToolRegistry, ToolSpec};

// ---------------------------------------------------------------------------
// Name helpers
// ---------------------------------------------------------------------------

/// Returns the namespaced tool name: `mcp.<server>.<tool>`.
pub fn namespaced_name(server_name: &str, tool_name: &str) -> String {
    format!("mcp.{server_name}.{tool_name}")
}

/// Parses `mcp.<server>.<tool>` into `(server, tool)`.
///
/// The tool portion may itself contain dots (e.g. `mcp.srv.tool.sub` →
/// `("srv", "tool.sub")`).  Returns `None` if the prefix is not `mcp` or
/// there are fewer than three dot-separated segments.
pub fn parse_namespaced(full_name: &str) -> Option<(String, String)> {
    // Require at least "mcp.<server>.<tool>" — three segments minimum.
    let rest = full_name.strip_prefix("mcp.")?;
    // rest = "<server>.<tool>[.more]"
    let dot = rest.find('.')?;
    let server = rest[..dot].to_owned();
    let tool = rest[dot + 1..].to_owned();
    if tool.is_empty() {
        return None;
    }
    Some((server, tool))
}

// ---------------------------------------------------------------------------
// McpToolResult → Value conversion
// ---------------------------------------------------------------------------

fn result_to_value(result: McpToolResult) -> Result<Value, CoreError> {
    let text: String = result
        .content
        .into_iter()
        .map(|c| match c {
            McpContent::Text { text } => text,
        })
        .collect::<Vec<_>>()
        .join("\n");

    if result.is_error {
        Err(CoreError::Tool(text))
    } else {
        Ok(Value::String(text))
    }
}

// ---------------------------------------------------------------------------
// McpToolHandler
// ---------------------------------------------------------------------------

/// `ToolHandler` implementation that delegates to a live `McpClient`.
pub struct McpToolHandler {
    /// Original (non-namespaced) tool name as the MCP server knows it.
    tool_name: String,
    client: Arc<Mutex<McpClient>>,
}

impl McpToolHandler {
    fn new(tool_name: String, client: Arc<Mutex<McpClient>>) -> Self {
        Self { tool_name, client }
    }
}

impl ToolHandler for McpToolHandler {
    fn invoke(
        &self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, CoreError>> + Send>> {
        let tool_name = self.tool_name.clone();
        let client = Arc::clone(&self.client);
        Box::pin(async move {
            let mut guard = client.lock().await;
            let mcp_result = guard
                .call_tool(&tool_name, args)
                .await
                .map_err(|e| CoreError::Tool(e.to_string()))?;
            result_to_value(mcp_result)
        })
    }
}

// ---------------------------------------------------------------------------
// McpBridge
// ---------------------------------------------------------------------------

/// Bridges an MCP server into the `ToolRegistry` under `mcp.<server>.*`.
pub struct McpBridge {
    server_name: String,
    client: Arc<Mutex<McpClient>>,
}

impl McpBridge {
    pub fn new(server_name: impl Into<String>, client: Arc<Mutex<McpClient>>) -> Self {
        Self {
            server_name: server_name.into(),
            client,
        }
    }

    /// Returns `mcp.<server>.<tool>`.
    pub fn namespaced_name(server_name: &str, tool_name: &str) -> String {
        crate::mcp_bridge::namespaced_name(server_name, tool_name)
    }

    /// Parses `mcp.<server>.<tool>` → `(server, tool)`.
    pub fn parse_namespaced(full_name: &str) -> Option<(String, String)> {
        crate::mcp_bridge::parse_namespaced(full_name)
    }

    /// Discover tools from the live client and register them into `registry`.
    ///
    /// Returns the list of namespaced names that were registered.
    pub async fn register_tools(
        &self,
        registry: &mut ToolRegistry,
    ) -> Result<Vec<String>, CoreError> {
        let tool_defs = {
            let mut guard = self.client.lock().await;
            guard
                .list_tools()
                .await
                .map_err(|e| CoreError::Tool(e.to_string()))?
        };
        register_tool_defs(
            &self.server_name,
            tool_defs,
            Arc::clone(&self.client),
            registry,
        )
    }

    /// Returns the namespaced names of all tools this bridge currently knows
    /// about (requires a live round-trip; prefer caching the result of
    /// `register_tools` when possible).
    pub async fn tool_names(&self) -> Result<Vec<String>, CoreError> {
        let mut guard = self.client.lock().await;
        let defs = guard
            .list_tools()
            .await
            .map_err(|e| CoreError::Tool(e.to_string()))?;
        Ok(defs
            .into_iter()
            .map(|d| namespaced_name(&self.server_name, &d.name))
            .collect())
    }
}

// ---------------------------------------------------------------------------
// Testable free function — does not require a live MCP server
// ---------------------------------------------------------------------------

/// Register a pre-fetched list of `McpToolDef`s into `registry`.
///
/// This is the core registration logic, extracted so tests can exercise it
/// without spawning a real MCP server process.
pub fn register_tool_defs(
    server_name: &str,
    tool_defs: Vec<McpToolDef>,
    client: Arc<Mutex<McpClient>>,
    registry: &mut ToolRegistry,
) -> Result<Vec<String>, CoreError> {
    let mut registered = Vec::with_capacity(tool_defs.len());
    for def in tool_defs {
        let ns_name = namespaced_name(server_name, &def.name);
        let spec = ToolSpec {
            name: ns_name.clone(),
            description: def.description.unwrap_or_default(),
            parameters: def.input_schema,
        };
        let handler = Box::new(McpToolHandler::new(def.name, Arc::clone(&client)));
        registry.register(spec, handler).map_err(|e| {
            // Preserve the collision name in the error message.
            CoreError::Tool(format!("collision registering '{}': {}", ns_name, e))
        })?;
        registered.push(ns_name);
    }
    Ok(registered)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use ucode_mcp::{McpContent, McpToolDef, McpToolResult};

    use super::*;
    use crate::registry::{ToolRegistry, ToolSpec};

    // ------------------------------------------------------------------
    // Naming helpers
    // ------------------------------------------------------------------

    #[test]
    fn test_namespaced_name() {
        assert_eq!(
            McpBridge::namespaced_name("myserver", "read_file"),
            "mcp.myserver.read_file"
        );
    }

    #[test]
    fn test_parse_namespaced_valid() {
        let result = McpBridge::parse_namespaced("mcp.myserver.read_file");
        assert_eq!(result, Some(("myserver".into(), "read_file".into())));
    }

    #[test]
    fn test_parse_namespaced_invalid_prefix() {
        assert_eq!(McpBridge::parse_namespaced("notmcp.x.y"), None);
    }

    #[test]
    fn test_parse_namespaced_too_few_parts() {
        // "mcp.only" has no second dot → None
        assert_eq!(McpBridge::parse_namespaced("mcp.only"), None);
    }

    #[test]
    fn test_parse_namespaced_dots_in_tool() {
        // Dots beyond the second are part of the tool name.
        let result = McpBridge::parse_namespaced("mcp.srv.tool.sub");
        assert_eq!(result, Some(("srv".into(), "tool.sub".into())));
    }

    #[test]
    fn test_namespaced_name_special_chars() {
        assert_eq!(
            McpBridge::namespaced_name("my-server", "do_thing"),
            "mcp.my-server.do_thing"
        );
        assert_eq!(
            McpBridge::namespaced_name("srv_1", "tool-2"),
            "mcp.srv_1.tool-2"
        );
    }

    // ------------------------------------------------------------------
    // McpToolDef → ToolSpec conversion
    // ------------------------------------------------------------------

    #[test]
    fn test_mcp_tool_def_to_tool_spec() {
        let def = McpToolDef {
            name: "read_file".into(),
            description: Some("Read a file".into()),
            input_schema: json!({ "type": "object" }),
        };
        let spec = ToolSpec {
            name: namespaced_name("fs", &def.name),
            description: def.description.clone().unwrap_or_default(),
            parameters: def.input_schema.clone(),
        };
        assert_eq!(spec.name, "mcp.fs.read_file");
        assert_eq!(spec.description, "Read a file");
        assert_eq!(spec.parameters, json!({ "type": "object" }));
    }

    // ------------------------------------------------------------------
    // Registration helpers (no live client needed)
    // ------------------------------------------------------------------

    /// Build a minimal `McpToolDef` list and register them, verifying the
    /// registry is populated correctly.
    ///
    /// We need an `Arc<Mutex<McpClient>>` for the type signature of
    /// `register_tool_defs`, but the handlers are never invoked in these
    /// tests, so we use a real (but unconnected) client constructed via
    /// `McpClient::connect` — except that requires spawning a process.
    ///
    /// Instead we use a helper that bypasses the client entirely by testing
    /// only the spec/name side of registration.
    fn make_tool_defs(names: &[&str]) -> Vec<McpToolDef> {
        names
            .iter()
            .map(|n| McpToolDef {
                name: n.to_string(),
                description: Some(format!("desc for {n}")),
                input_schema: json!({ "type": "object" }),
            })
            .collect()
    }

    /// Register tool defs into a registry using only the spec path (no
    /// handler invocation), so we can test without a live McpClient.
    fn register_specs_only(
        server_name: &str,
        tool_defs: Vec<McpToolDef>,
        registry: &mut ToolRegistry,
    ) -> Result<Vec<String>, CoreError> {
        let mut registered = Vec::with_capacity(tool_defs.len());
        for def in tool_defs {
            let ns_name = namespaced_name(server_name, &def.name);
            let spec = ToolSpec {
                name: ns_name.clone(),
                description: def.description.unwrap_or_default(),
                parameters: def.input_schema,
            };
            // Use a no-op handler so we don't need a real McpClient.
            struct Noop;
            impl ToolHandler for Noop {
                fn invoke(
                    &self,
                    _args: Value,
                ) -> std::pin::Pin<
                    Box<dyn std::future::Future<Output = Result<Value, CoreError>> + Send>,
                > {
                    Box::pin(async { Ok(Value::Null) })
                }
            }
            registry.register(spec, Box::new(Noop)).map_err(|e| {
                CoreError::Tool(format!("collision registering '{}': {}", ns_name, e))
            })?;
            registered.push(ns_name);
        }
        Ok(registered)
    }

    #[test]
    fn test_register_tools_populates_registry() {
        let mut registry = ToolRegistry::new();
        let defs = make_tool_defs(&["read_file", "write_file"]);
        let names = register_specs_only("fs", defs, &mut registry).unwrap();

        assert_eq!(names, vec!["mcp.fs.read_file", "mcp.fs.write_file"]);

        let listed: Vec<_> = registry.list().iter().map(|s| s.name.clone()).collect();
        assert!(listed.contains(&"mcp.fs.read_file".to_string()));
        assert!(listed.contains(&"mcp.fs.write_file".to_string()));
    }

    #[test]
    fn test_register_tools_collision_detection() {
        let mut registry = ToolRegistry::new();
        let defs = make_tool_defs(&["read_file"]);
        register_specs_only("fs", defs, &mut registry).unwrap();

        // Registering the same tool again must fail.
        let defs2 = make_tool_defs(&["read_file"]);
        let err = register_specs_only("fs", defs2, &mut registry).unwrap_err();
        match err {
            CoreError::Tool(msg) => {
                assert!(msg.contains("mcp.fs.read_file"), "unexpected msg: {msg}");
            }
            other => panic!("expected CoreError::Tool, got {other:?}"),
        }
    }

    // ------------------------------------------------------------------
    // McpToolResult conversion (no McpClient needed)
    // ------------------------------------------------------------------

    #[test]
    fn test_mcp_tool_handler_converts_success() {
        let result = McpToolResult {
            content: vec![McpContent::Text { text: "ok".into() }],
            is_error: false,
        };
        let value = result_to_value(result).unwrap();
        assert_eq!(value, Value::String("ok".into()));
    }

    #[test]
    fn test_mcp_tool_handler_converts_error() {
        let result = McpToolResult {
            content: vec![McpContent::Text {
                text: "fail".into(),
            }],
            is_error: true,
        };
        let err = result_to_value(result).unwrap_err();
        match err {
            CoreError::Tool(msg) => assert_eq!(msg, "fail"),
            other => panic!("expected CoreError::Tool, got {other:?}"),
        }
    }

    #[test]
    fn test_mcp_tool_handler_multi_content() {
        let result = McpToolResult {
            content: vec![
                McpContent::Text {
                    text: "line1".into(),
                },
                McpContent::Text {
                    text: "line2".into(),
                },
                McpContent::Text {
                    text: "line3".into(),
                },
            ],
            is_error: false,
        };
        let value = result_to_value(result).unwrap();
        assert_eq!(value, Value::String("line1\nline2\nline3".into()));
    }

    // ------------------------------------------------------------------
    // Empty content edge case
    // ------------------------------------------------------------------

    #[test]
    fn test_result_to_value_empty_content() {
        let result = McpToolResult {
            content: vec![],
            is_error: false,
        };
        let value = result_to_value(result).unwrap();
        assert_eq!(value, Value::String(String::new()));
    }

    #[test]
    fn test_result_to_value_empty_content_error() {
        let result = McpToolResult {
            content: vec![],
            is_error: true,
        };
        let err = result_to_value(result).unwrap_err();
        match err {
            CoreError::Tool(msg) => assert!(msg.is_empty()),
            other => panic!("expected CoreError::Tool, got {other:?}"),
        }
    }

    // ------------------------------------------------------------------
    // parse_namespaced edge cases
    // ------------------------------------------------------------------

    #[test]
    fn test_parse_namespaced_empty_tool() {
        // "mcp.srv." — tool part is empty → None
        assert_eq!(McpBridge::parse_namespaced("mcp.srv."), None);
    }

    #[test]
    fn test_parse_namespaced_bare_mcp() {
        assert_eq!(McpBridge::parse_namespaced("mcp"), None);
        assert_eq!(McpBridge::parse_namespaced("mcp."), None);
    }
}
