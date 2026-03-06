use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::task::JoinHandle;

use crate::error::McpError;
use crate::jsonrpc::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};

/// Stdio transport for MCP communication.
///
/// # I/O contract
///
/// MCP servers communicate exclusively via JSON-RPC over the child process's
/// stdin/stdout.  This transport:
///
/// - **Writes** JSON-RPC requests to the child's **stdin** (newline-delimited JSON).
/// - **Reads** JSON-RPC responses from the child's **stdout** (newline-delimited JSON).
/// - **Captures** the child's **stderr** in a background task and routes each line
///   through `tracing::debug!(target: "mcp_server")` so server diagnostics appear
///   in our structured logging system rather than interleaving with host stderr.
///
/// The host process's own stdout is never used for MCP communication.  All host
/// logging goes to stderr and/or file sinks via the `ucode-core` logging subsystem.
pub struct StdioTransport {
    child: Child,
    writer: BufWriter<ChildStdin>,
    reader: BufReader<ChildStdout>,
    next_id: u64,
    /// Background task draining the child's stderr into tracing.
    stderr_task: Option<JoinHandle<()>>,
    /// Command name carried for diagnostic context.
    server_command: String,
}

impl StdioTransport {
    /// Spawn a child process and create a transport.
    ///
    /// The child's stderr is piped and drained by a background tokio task that
    /// emits each non-empty line as `tracing::debug!(target: "mcp_server")`.
    pub async fn spawn(command: &str, args: &[&str]) -> Result<Self, McpError> {
        let mut child = Command::new(command)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped()) // Capture; do not inherit.
            .spawn()
            .map_err(|e| McpError::SpawnFailed(format!("{command}: {e}")))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| McpError::SpawnFailed("failed to capture stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| McpError::SpawnFailed("failed to capture stdout".into()))?;
        let stderr = child.stderr.take();

        let server_command = command.to_owned();

        // Drain the child's stderr through tracing so server diagnostics appear
        // in our structured log stream rather than interleaving with host stderr.
        let stderr_task = stderr.map(|stderr| {
            let cmd = server_command.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr);
                let mut line = String::new();
                loop {
                    line.clear();
                    match reader.read_line(&mut line).await {
                        Ok(0) => break, // EOF — child closed its stderr
                        Ok(_) => {
                            let trimmed = line.trim_end();
                            if !trimmed.is_empty() {
                                tracing::debug!(
                                    target: "mcp_server",
                                    server = %cmd,
                                    "{trimmed}"
                                );
                            }
                        }
                        Err(_) => break,
                    }
                }
            })
        });

        Ok(Self {
            child,
            writer: BufWriter::new(stdin),
            reader: BufReader::new(stdout),
            next_id: 1,
            stderr_task,
            server_command,
        })
    }

    /// Send a JSON-RPC request and wait for the matching response.
    pub async fn request(
        &mut self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, McpError> {
        let id = self.next_id;
        self.next_id += 1;

        let req = JsonRpcRequest::new(id, method, params);
        let mut line = serde_json::to_string(&req)?;
        line.push('\n');
        self.writer.write_all(line.as_bytes()).await?;
        self.writer.flush().await?;

        // Read responses until we find the one matching our id.
        // A well-behaved MCP server won't interleave unrelated messages during
        // a synchronous request/response cycle, but we skip any that don't match.
        loop {
            let resp = self.read_response().await?;
            if resp.id == Some(id) {
                if let Some(err) = resp.error {
                    return Err(McpError::JsonRpc {
                        code: err.code,
                        message: err.message,
                    });
                }
                return resp.result.ok_or_else(|| {
                    McpError::Protocol("response has neither result nor error".into())
                });
            }
            // Discard responses for other ids (shouldn't happen in normal use).
        }
    }

    /// Send a JSON-RPC notification (no response expected).
    pub async fn notify(
        &mut self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<(), McpError> {
        let notif = JsonRpcNotification::new(method, params);
        let mut line = serde_json::to_string(&notif)?;
        line.push('\n');
        self.writer.write_all(line.as_bytes()).await?;
        self.writer.flush().await?;
        Ok(())
    }

    /// Read the next line from stdout and parse as a JSON-RPC response.
    async fn read_response(&mut self) -> Result<JsonRpcResponse, McpError> {
        let mut line = String::new();
        let n = self.reader.read_line(&mut line).await?;
        if n == 0 {
            return Err(McpError::ServerExited);
        }
        let resp: JsonRpcResponse = serde_json::from_str(line.trim_end())?;
        Ok(resp)
    }

    /// Kill the child process and clean up the stderr reader task.
    pub async fn shutdown(&mut self) -> Result<(), McpError> {
        // Stop the stderr drain before killing the child so the task doesn't
        // race against a closed pipe.
        if let Some(task) = self.stderr_task.take() {
            task.abort();
        }
        // Best-effort: ignore errors from kill (process may have already exited).
        let _ = self.child.kill().await;
        Ok(())
    }

    /// The command used to spawn this transport's child process.
    pub fn server_command(&self) -> &str {
        &self.server_command
    }
}
