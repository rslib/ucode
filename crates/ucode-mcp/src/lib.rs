//! ucode-mcp: MCP client, server registry, native launchers (uvx/npx/bunx)

pub mod client;
pub mod error;
pub mod jsonrpc;
pub mod transport;
pub mod types;

pub use client::McpClient;
pub use error::McpError;
pub use types::{McpContent, McpToolDef, McpToolResult, ServerCapabilities, ServerInfo};
