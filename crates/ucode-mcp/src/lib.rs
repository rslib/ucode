//! ucode-mcp: MCP client, server registry, native launchers (uvx/npx/bunx)

pub mod client;
pub mod error;
pub mod jsonrpc;
pub mod launchers;
pub mod resources;
pub mod server_policy;
pub mod transport;
pub mod types;

pub use client::McpClient;
pub use error::McpError;
pub use launchers::{
    LauncherDef, LauncherType, ServerIdentity, TrustRecord, TrustStatus, compute_fingerprint,
    launcher_to_command, load_trust_cache, save_trust_cache, trust_cache_path, verify_trust,
};
pub use resources::{
    McpResourceRegistry, NamespacedPrompt, NamespacedResource, namespaced_prompt,
    namespaced_resource, parse_namespaced as parse_namespaced_resource,
};
pub use server_policy::{
    AuditEvent, AuditEventType, ServerLifecycle, ServerNetworkPolicy, ServerPolicy,
    ServerPolicyStore, ServerState, ServerTier, ToolCheckResult, ToolPermission,
};
pub use types::{
    McpContent, McpPromptArgument, McpPromptDef, McpPromptMessage, McpPromptMessageContent,
    McpResourceContent, McpResourceDef, McpToolDef, McpToolResult, ServerCapabilities, ServerInfo,
};
