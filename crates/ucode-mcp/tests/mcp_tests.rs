use std::io::Write;

use tempfile::NamedTempFile;
use ucode_mcp::McpClient;

/// Write a minimal MCP server as a Python script to a temp file and return it.
///
/// The script reads newline-delimited JSON-RPC from stdin and writes responses
/// to stdout.  It handles `initialize`, `tools/list`, and `tools/call`.
fn write_mock_server() -> NamedTempFile {
    let script = r#"#!/usr/bin/env python3
import sys
import json

def respond(id_, result):
    msg = json.dumps({"jsonrpc": "2.0", "id": id_, "result": result})
    sys.stdout.write(msg + "\n")
    sys.stdout.flush()

def error_response(id_, code, message):
    msg = json.dumps({"jsonrpc": "2.0", "id": id_, "error": {"code": code, "message": message}})
    sys.stdout.write(msg + "\n")
    sys.stdout.flush()

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        req = json.loads(line)
    except json.JSONDecodeError:
        continue

    method = req.get("method", "")
    id_ = req.get("id")  # None for notifications

    if method == "initialize":
        respond(id_, {
            "protocolVersion": "2024-11-05",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "mock-server", "version": "0.0.1"}
        })
    elif method == "notifications/initialized":
        # notification — no response
        pass
    elif method == "tools/list":
        respond(id_, {
            "tools": [
                {
                    "name": "echo",
                    "description": "Echo the input",
                    "inputSchema": {
                        "type": "object",
                        "properties": {"message": {"type": "string"}},
                        "required": ["message"]
                    }
                }
            ]
        })
    elif method == "tools/call":
        params = req.get("params", {})
        name = params.get("name", "")
        args = params.get("arguments", {})
        if name == "echo":
            respond(id_, {
                "content": [{"type": "text", "text": args.get("message", "")}],
                "isError": False
            })
        else:
            error_response(id_, -32601, f"unknown tool: {name}")
    else:
        if id_ is not None:
            error_response(id_, -32601, f"method not found: {method}")
"#;

    let mut f = NamedTempFile::new().expect("tempfile");
    f.write_all(script.as_bytes()).expect("write script");
    // Make executable
    use std::os::unix::fs::PermissionsExt;
    let mut perms = f.as_file().metadata().expect("metadata").permissions();
    perms.set_mode(0o755);
    f.as_file().set_permissions(perms).expect("chmod");
    f
}

#[tokio::test]
async fn spawn_and_communicate() {
    let script = write_mock_server();
    let path = script.path().to_str().expect("path");

    let mut client = McpClient::connect("python3", &[path])
        .await
        .expect("connect");

    let info = client.initialize().await.expect("initialize");
    assert_eq!(info.name, "mock-server");
    assert_eq!(info.version, "0.0.1");

    let tools = client.list_tools().await.expect("list_tools");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "echo");
    assert!(tools[0].description.is_some());

    let result = client
        .call_tool("echo", serde_json::json!({"message": "hello MCP"}))
        .await
        .expect("call_tool");
    assert!(!result.is_error);
    assert_eq!(result.content.len(), 1);
    let ucode_mcp::McpContent::Text { text } = &result.content[0];
    assert_eq!(text, "hello MCP");

    client.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn server_info_none_before_initialize() {
    let script = write_mock_server();
    let path = script.path().to_str().expect("path");

    let client = McpClient::connect("python3", &[path])
        .await
        .expect("connect");

    // Before initialize, server_info must be None.
    assert!(client.server_info().is_none());
    assert!(client.server_capabilities().is_none());
}
