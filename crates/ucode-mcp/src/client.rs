use serde_json::json;

use crate::error::McpError;
use crate::server_config::{ServerConfig, TransportType};
use crate::transport::{StdioTransport, Transport};
use crate::transport_http::HttpTransport;
use crate::transport_sse::SseTransport;
use crate::types::{
    McpPromptDef, McpPromptMessage, McpResourceContent, McpResourceDef, McpToolDef, McpToolResult,
    ServerCapabilities, ServerInfo,
};

const PROTOCOL_VERSION: &str = "2024-11-05";

/// Client identity sent during the MCP initialize handshake.
#[derive(Debug, Clone)]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
}

impl Default for ClientInfo {
    fn default() -> Self {
        Self {
            name: "ucode".into(),
            version: "0.1.0".into(),
        }
    }
}

impl ClientInfo {
    pub fn from_config(config: &ServerConfig) -> Self {
        Self {
            name: config.client_name.clone().unwrap_or_else(|| "ucode".into()),
            version: config
                .client_version
                .clone()
                .unwrap_or_else(|| "0.1.0".into()),
        }
    }
}

/// High-level MCP client that manages the protocol lifecycle.
pub struct McpClient {
    transport: Box<dyn Transport>,
    server_info: Option<ServerInfo>,
    server_capabilities: Option<ServerCapabilities>,
    client_info: ClientInfo,
}

impl McpClient {
    /// Connect to an MCP server by spawning the given command.
    pub async fn connect(command: &str, args: &[&str]) -> Result<Self, McpError> {
        let transport = StdioTransport::spawn(command, args).await?;
        Ok(Self {
            transport: Box::new(transport),
            server_info: None,
            server_capabilities: None,
            client_info: ClientInfo::default(),
        })
    }

    /// Connect to an MCP server using a `ServerConfig`.
    pub async fn connect_with_config(config: &ServerConfig) -> Result<Self, McpError> {
        let headers = config.expanded_headers();
        let client_info = ClientInfo::from_config(config);
        let transport: Box<dyn Transport> = match &config.transport_type {
            TransportType::Stdio { command, args, .. } => {
                let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                Box::new(StdioTransport::spawn(command, &args_refs).await?)
            }
            TransportType::Sse { url } => Box::new(
                SseTransport::connect(url.clone(), &headers, config.reconnect.clone()).await?,
            ),
            TransportType::StreamableHttp { url } => Box::new(HttpTransport::new(
                url.clone(),
                &headers,
                config.reconnect.clone(),
            )?),
        };
        Ok(Self::from_transport(transport, client_info))
    }

    /// Create a client from an existing transport.
    pub fn from_transport(transport: Box<dyn Transport>, client_info: ClientInfo) -> Self {
        Self {
            transport,
            server_info: None,
            server_capabilities: None,
            client_info,
        }
    }

    /// Perform the MCP initialize handshake.
    ///
    /// Sends `initialize`, stores server info/capabilities, then sends
    /// `notifications/initialized`.  Returns a reference to the server info.
    pub async fn initialize(&mut self) -> Result<&ServerInfo, McpError> {
        let params = json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {
                "name": self.client_info.name,
                "version": self.client_info.version
            }
        });

        let result = self.transport.request("initialize", Some(params)).await?;

        let server_info: ServerInfo =
            serde_json::from_value(result.get("serverInfo").cloned().ok_or_else(|| {
                McpError::Protocol("initialize response missing serverInfo".into())
            })?)?;

        let capabilities: ServerCapabilities = result
            .get("capabilities")
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()?
            .unwrap_or_default();

        self.server_info = Some(server_info);
        self.server_capabilities = Some(capabilities);

        // Notify the server that initialization is complete.
        self.transport
            .notify("notifications/initialized", None)
            .await?;

        Ok(self.server_info.as_ref().expect("just set above"))
    }

    /// List available tools from the server.
    pub async fn list_tools(&mut self) -> Result<Vec<McpToolDef>, McpError> {
        let result = self.transport.request("tools/list", None).await?;
        let tools: Vec<McpToolDef> =
            serde_json::from_value(result.get("tools").cloned().ok_or_else(|| {
                McpError::Protocol("tools/list response missing tools field".into())
            })?)?;
        Ok(tools)
    }

    /// Call a tool on the server.
    pub async fn call_tool(
        &mut self,
        name: &str,
        args: serde_json::Value,
    ) -> Result<McpToolResult, McpError> {
        let params = json!({ "name": name, "arguments": args });
        let result = self.transport.request("tools/call", Some(params)).await?;
        let tool_result: McpToolResult = serde_json::from_value(result)?;
        Ok(tool_result)
    }

    /// List available resources from the server.
    pub async fn list_resources(&mut self) -> Result<Vec<McpResourceDef>, McpError> {
        let result = self.transport.request("resources/list", None).await?;
        let resources: Vec<McpResourceDef> =
            serde_json::from_value(result.get("resources").cloned().ok_or_else(|| {
                McpError::Protocol("resources/list response missing resources field".into())
            })?)?;
        Ok(resources)
    }

    /// Read a resource by URI.
    pub async fn read_resource(&mut self, uri: &str) -> Result<Vec<McpResourceContent>, McpError> {
        let params = serde_json::json!({ "uri": uri });
        let result = self
            .transport
            .request("resources/read", Some(params))
            .await?;
        let contents: Vec<McpResourceContent> =
            serde_json::from_value(result.get("contents").cloned().ok_or_else(|| {
                McpError::Protocol("resources/read response missing contents field".into())
            })?)?;
        Ok(contents)
    }

    /// List available prompts from the server.
    pub async fn list_prompts(&mut self) -> Result<Vec<McpPromptDef>, McpError> {
        let result = self.transport.request("prompts/list", None).await?;
        let prompts: Vec<McpPromptDef> =
            serde_json::from_value(result.get("prompts").cloned().ok_or_else(|| {
                McpError::Protocol("prompts/list response missing prompts field".into())
            })?)?;
        Ok(prompts)
    }

    /// Get a prompt with arguments resolved.
    pub async fn get_prompt(
        &mut self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<Vec<McpPromptMessage>, McpError> {
        let params = serde_json::json!({ "name": name, "arguments": arguments });
        let result = self.transport.request("prompts/get", Some(params)).await?;
        let messages: Vec<McpPromptMessage> =
            serde_json::from_value(result.get("messages").cloned().ok_or_else(|| {
                McpError::Protocol("prompts/get response missing messages field".into())
            })?)?;
        Ok(messages)
    }

    /// Check if the server supports resources (based on capabilities from initialize).
    pub fn supports_resources(&self) -> bool {
        self.server_capabilities
            .as_ref()
            .is_some_and(|c| c.resources.is_some())
    }

    /// Check if the server supports prompts (based on capabilities from initialize).
    pub fn supports_prompts(&self) -> bool {
        self.server_capabilities
            .as_ref()
            .is_some_and(|c| c.prompts.is_some())
    }

    /// Shut down the client and the server process.
    pub async fn shutdown(&mut self) -> Result<(), McpError> {
        self.transport.shutdown().await
    }

    /// Returns server info if `initialize` has been called.
    pub fn server_info(&self) -> Option<&ServerInfo> {
        self.server_info.as_ref()
    }

    /// Returns server capabilities if `initialize` has been called.
    pub fn server_capabilities(&self) -> Option<&ServerCapabilities> {
        self.server_capabilities.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reconnect::ReconnectConfig;
    use crate::server_config::TransportType;
    use std::collections::HashMap;

    #[test]
    fn client_info_default() {
        let info = ClientInfo::default();
        assert_eq!(info.name, "ucode");
        assert_eq!(info.version, "0.1.0");
    }

    #[test]
    fn client_info_from_config_custom() {
        let config = ServerConfig {
            name: "test".into(),
            transport_type: TransportType::Stdio {
                command: "echo".into(),
                args: vec![],
                env: HashMap::new(),
            },
            headers: HashMap::new(),
            reconnect: ReconnectConfig::default(),
            client_name: Some("kimi-code".into()),
            client_version: Some("2.0.0".into()),
        };
        let info = ClientInfo::from_config(&config);
        assert_eq!(info.name, "kimi-code");
        assert_eq!(info.version, "2.0.0");
    }

    #[test]
    fn client_info_from_config_partial() {
        let config = ServerConfig {
            name: "test".into(),
            transport_type: TransportType::Stdio {
                command: "echo".into(),
                args: vec![],
                env: HashMap::new(),
            },
            headers: HashMap::new(),
            reconnect: ReconnectConfig::default(),
            client_name: Some("custom".into()),
            client_version: None,
        };
        let info = ClientInfo::from_config(&config);
        assert_eq!(info.name, "custom");
        assert_eq!(info.version, "0.1.0");
    }

    #[test]
    fn client_info_from_config_defaults() {
        let config = ServerConfig {
            name: "test".into(),
            transport_type: TransportType::Stdio {
                command: "echo".into(),
                args: vec![],
                env: HashMap::new(),
            },
            headers: HashMap::new(),
            reconnect: ReconnectConfig::default(),
            client_name: None,
            client_version: None,
        };
        let info = ClientInfo::from_config(&config);
        assert_eq!(info.name, "ucode");
        assert_eq!(info.version, "0.1.0");
    }
}
