use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};
use ucode_core::{CoreError, ToolResult};

/// Metadata describing a tool's interface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    /// Unique tool name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// JSON Schema for the tool's input parameters.
    pub parameters: serde_json::Value,
}

impl ToolSpec {
    /// Convert to provider-facing ToolDef.
    pub fn to_tool_def(&self) -> ucode_providers::ToolDef {
        ucode_providers::ToolDef {
            name: self.name.clone(),
            description: self.description.clone(),
            parameters: self.parameters.clone(),
        }
    }
}

/// Async tool invocation handler.
pub trait ToolHandler: Send + Sync {
    /// Execute the tool with the given arguments.
    fn invoke(
        &self,
        args: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, CoreError>> + Send>>;
}

/// A registered tool: spec + handler.
pub struct RegisteredTool {
    pub spec: ToolSpec,
    handler: Box<dyn ToolHandler>,
}

impl RegisteredTool {
    /// Invoke this tool's handler.
    pub async fn invoke(&self, args: serde_json::Value) -> Result<serde_json::Value, CoreError> {
        self.handler.invoke(args).await
    }
}

/// Registry of available tools.
pub struct ToolRegistry {
    tools: HashMap<String, RegisteredTool>,
}

impl ToolRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Register a tool. Fails if a tool with the same name already exists.
    pub fn register(
        &mut self,
        spec: ToolSpec,
        handler: Box<dyn ToolHandler>,
    ) -> Result<(), CoreError> {
        if self.tools.contains_key(&spec.name) {
            return Err(CoreError::Tool(format!(
                "tool '{}' already registered",
                spec.name
            )));
        }
        let name = spec.name.clone();
        self.tools.insert(name, RegisteredTool { spec, handler });
        Ok(())
    }

    /// List all registered tool specs.
    pub fn list(&self) -> Vec<&ToolSpec> {
        self.tools.values().map(|rt| &rt.spec).collect()
    }

    /// Get a registered tool by name.
    pub fn get(&self, name: &str) -> Option<&RegisteredTool> {
        self.tools.get(name)
    }

    /// List all tool definitions in provider-facing format.
    pub fn tool_defs(&self) -> Vec<ucode_providers::ToolDef> {
        self.tools
            .values()
            .map(|rt| rt.spec.to_tool_def())
            .collect()
    }

    /// Invoke a tool by name, returning a canonical ToolResult.
    ///
    /// If the handler returns an error, it is captured as an error ToolResult
    /// rather than propagated. Only returns Err if the tool name is not found.
    pub async fn invoke(
        &self,
        call_id: &str,
        tool_name: &str,
        args: serde_json::Value,
    ) -> Result<ToolResult, CoreError> {
        let tool = self
            .tools
            .get(tool_name)
            .ok_or_else(|| CoreError::Tool(format!("unknown tool: '{}'", tool_name)))?;

        match tool.invoke(args).await {
            Ok(value) => Ok(ToolResult {
                id: call_id.to_string(),
                name: tool_name.to_string(),
                result: value,
                is_error: false,
            }),
            Err(e) => Ok(ToolResult {
                id: call_id.to_string(),
                name: tool_name.to_string(),
                result: serde_json::Value::String(e.to_string()),
                is_error: true,
            }),
        }
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
