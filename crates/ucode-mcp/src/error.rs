#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("failed to spawn MCP server: {0}")]
    SpawnFailed(String),
    #[error("MCP server process exited unexpectedly")]
    ServerExited,
    #[error("I/O error communicating with MCP server: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("JSON-RPC error {code}: {message}")]
    JsonRpc { code: i64, message: String },
    #[error("MCP protocol error: {0}")]
    Protocol(String),
    #[error("MCP request timed out after {0:?}")]
    Timeout(std::time::Duration),
    #[error("launcher not trusted: {0}")]
    LauncherNotTrusted(String),
    #[error("launcher fingerprint drifted for {server}: expected {expected}, got {actual}")]
    FingerprintDrift {
        server: String,
        expected: String,
        actual: String,
    },
}
