use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::process::Command;
use tokio::time::timeout;
use ucode_core::CoreError;

use crate::registry::{ToolHandler, ToolRegistry, ToolSpec};

const OUTPUT_CAP: usize = 100 * 1024; // 100 KB
const DEFAULT_TIMEOUT_SECS: u64 = 30;
const MAX_TIMEOUT_SECS: u64 = 300;

fn cap_output(raw: Vec<u8>) -> String {
    if raw.len() > OUTPUT_CAP {
        let mut s = String::from_utf8_lossy(&raw[..OUTPUT_CAP]).into_owned();
        s.push_str("\n[truncated at 100KB]");
        s
    } else {
        String::from_utf8_lossy(&raw).into_owned()
    }
}

/// Tool handler for `run_cmd`.
pub struct CmdTool;

impl ToolHandler for CmdTool {
    fn invoke(
        &self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, CoreError>> + Send>> {
        Box::pin(async move {
            let cmd = args["cmd"]
                .as_str()
                .ok_or_else(|| CoreError::Tool("missing required argument: 'cmd'".into()))?
                .to_string();

            let cwd: Option<PathBuf> = args["cwd"].as_str().map(PathBuf::from);

            if let Some(ref dir) = cwd {
                let meta = tokio::fs::metadata(dir).await.map_err(|e| {
                    CoreError::Tool(format!("cwd '{}' not accessible: {}", dir.display(), e))
                })?;
                if !meta.is_dir() {
                    return Err(CoreError::Tool(format!(
                        "cwd '{}' is not a directory",
                        dir.display()
                    )));
                }
            }

            let timeout_secs = args["timeout_secs"]
                .as_u64()
                .unwrap_or(DEFAULT_TIMEOUT_SECS)
                .clamp(1, MAX_TIMEOUT_SECS);

            let env_vars: HashMap<String, String> = args["env"]
                .as_object()
                .map(|obj| {
                    obj.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect()
                })
                .unwrap_or_default();

            #[cfg(unix)]
            let mut child = {
                let mut c = Command::new("sh");
                c.arg("-c").arg(&cmd);
                c
            };

            #[cfg(windows)]
            let mut child = {
                let mut c = Command::new("cmd");
                c.arg("/C").arg(&cmd);
                c
            };

            child
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());

            if let Some(ref dir) = cwd {
                child.current_dir(dir);
            }

            for (k, v) in &env_vars {
                child.env(k, v);
            }

            let mut spawned = child
                .spawn()
                .map_err(|e| CoreError::Tool(format!("failed to spawn command: {e}")))?;

            // Take the stdio handles before the timeout so we can kill the child
            // without consuming it via wait_with_output (which takes ownership).
            let stdout_handle = spawned
                .stdout
                .take()
                .ok_or_else(|| CoreError::Tool("failed to capture stdout".into()))?;
            let stderr_handle = spawned
                .stderr
                .take()
                .ok_or_else(|| CoreError::Tool("failed to capture stderr".into()))?;

            let deadline = Duration::from_secs(timeout_secs);

            let collect = async {
                use tokio::io::AsyncReadExt;
                let mut out = Vec::new();
                let mut err = Vec::new();
                let mut stdout_reader = tokio::io::BufReader::new(stdout_handle);
                let mut stderr_reader = tokio::io::BufReader::new(stderr_handle);
                tokio::try_join!(
                    stdout_reader.read_to_end(&mut out),
                    stderr_reader.read_to_end(&mut err),
                )?;
                let status = spawned.wait().await?;
                Ok::<_, std::io::Error>((out, err, status))
            };

            match timeout(deadline, collect).await {
                Ok(Ok((out, err, status))) => {
                    let exit_code = status.code().unwrap_or(-1);
                    Ok(json!({
                        "success": status.success(),
                        "exit_code": exit_code,
                        "stdout": cap_output(out),
                        "stderr": cap_output(err),
                        "timed_out": false,
                    }))
                }
                Ok(Err(e)) => Err(CoreError::Tool(format!("command I/O error: {e}"))),
                Err(_elapsed) => {
                    // Kill the child; ignore errors (process may have already exited).
                    let _ = spawned.kill().await;
                    Ok(json!({
                        "success": false,
                        "exit_code": null,
                        "stdout": "",
                        "stderr": "",
                        "timed_out": true,
                    }))
                }
            }
        })
    }
}

/// Register the `run_cmd` tool into the given registry.
pub fn register_cmd_tool(registry: &mut ToolRegistry) {
    registry
        .register(
            ToolSpec {
                name: "run_cmd".into(),
                description: "Execute a shell command with timeout and output caps.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "cmd": {
                            "type": "string",
                            "description": "Shell command to execute."
                        },
                        "cwd": {
                            "type": "string",
                            "description": "Working directory (optional)."
                        },
                        "timeout_secs": {
                            "type": "integer",
                            "description": "Timeout in seconds (1-300, default 30)."
                        },
                        "env": {
                            "type": "object",
                            "description": "Additional environment variables."
                        }
                    },
                    "required": ["cmd"]
                }),
            },
            Box::new(CmdTool),
        )
        .expect("run_cmd already registered");
}
