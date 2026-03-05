use serde_json::json;
use tempfile::TempDir;
use ucode_tools::cmd_tool::CmdTool;
use ucode_tools::{ToolHandler, ToolRegistry, register_cmd_tool};

// ── Test 1: echo hello ────────────────────────────────────────────────────────

#[tokio::test]
async fn echo_hello() {
    let result = CmdTool.invoke(json!({"cmd": "echo hello"})).await.unwrap();
    assert_eq!(result["success"], true);
    assert_eq!(result["timed_out"], false);
    assert!(result["stdout"].as_str().unwrap().contains("hello"));
}

// ── Test 2: non-zero exit code ────────────────────────────────────────────────

#[tokio::test]
async fn exit_code_nonzero() {
    let result = CmdTool.invoke(json!({"cmd": "exit 42"})).await.unwrap();
    assert_eq!(result["success"], false);
    assert_eq!(result["exit_code"], 42);
    assert_eq!(result["timed_out"], false);
}

// ── Test 3: timeout kills process ─────────────────────────────────────────────

#[tokio::test]
async fn timeout_kills_process() {
    let result = CmdTool
        .invoke(json!({"cmd": "sleep 10", "timeout_secs": 1}))
        .await
        .unwrap();
    assert_eq!(result["timed_out"], true);
    assert_eq!(result["success"], false);
}

// ── Test 4: custom cwd ────────────────────────────────────────────────────────

#[tokio::test]
async fn custom_cwd() {
    let dir = TempDir::new().unwrap();
    let dir_path = dir.path().to_str().unwrap().to_string();

    let result = CmdTool
        .invoke(json!({"cmd": "pwd", "cwd": dir_path}))
        .await
        .unwrap();

    assert_eq!(result["success"], true);
    let stdout = result["stdout"].as_str().unwrap();
    // Resolve symlinks on both sides so /tmp vs /private/tmp doesn't bite us.
    let canonical_dir = dir.path().canonicalize().unwrap();
    let canonical_out = std::path::Path::new(stdout.trim()).canonicalize().unwrap();
    assert_eq!(canonical_out, canonical_dir);
}

// ── Test 5: custom env var ────────────────────────────────────────────────────

#[tokio::test]
async fn custom_env() {
    let result = CmdTool
        .invoke(json!({
            "cmd": "echo $MY_VAR",
            "env": {"MY_VAR": "hello_from_env"}
        }))
        .await
        .unwrap();

    assert_eq!(result["success"], true);
    assert!(
        result["stdout"]
            .as_str()
            .unwrap()
            .contains("hello_from_env")
    );
}

// ── Test 6: invalid cwd returns error ─────────────────────────────────────────

#[tokio::test]
async fn invalid_cwd_returns_error() {
    let result = CmdTool
        .invoke(json!({"cmd": "echo hi", "cwd": "/this/path/does/not/exist/at/all"}))
        .await;
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("cwd"), "error should mention 'cwd'");
}

// ── Test 7: missing cmd arg returns error ─────────────────────────────────────

#[tokio::test]
async fn missing_cmd_arg_returns_error() {
    let result = CmdTool.invoke(json!({"cwd": "/tmp"})).await;
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("cmd"), "error should mention 'cmd'");
}

// ── Test 8: registry integration ─────────────────────────────────────────────

#[tokio::test]
async fn registry_integration() {
    let mut registry = ToolRegistry::new();
    register_cmd_tool(&mut registry);

    assert!(registry.get("run_cmd").is_some());

    let result = registry
        .invoke("call-cmd-1", "run_cmd", json!({"cmd": "echo registry_ok"}))
        .await
        .unwrap();

    assert!(!result.is_error);
    assert_eq!(result.result["success"], true);
    assert!(
        result.result["stdout"]
            .as_str()
            .unwrap()
            .contains("registry_ok")
    );
}

// ── Test 9: stdout cap truncation ─────────────────────────────────────────────

#[tokio::test]
async fn stdout_cap_truncation() {
    // Generate ~110 KB of output: 1100 lines of 100 bytes each.
    // `yes` is not portable; use a shell loop instead.
    let cmd =
        r#"python3 -c "print('x' * 99, end='\n')" | head -1 | awk '{for(i=0;i<1200;i++) print}'"#;
    let result = CmdTool.invoke(json!({"cmd": cmd})).await.unwrap();

    assert_eq!(result["success"], true);
    let stdout = result["stdout"].as_str().unwrap();
    assert!(
        stdout.contains("[truncated at 100KB]"),
        "stdout should be truncated; got {} bytes",
        stdout.len()
    );
    assert!(
        stdout.len() <= OUTPUT_CAP + 30,
        "truncated output should not exceed cap by much"
    );
}

const OUTPUT_CAP: usize = 100 * 1024;
