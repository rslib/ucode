use serde_json::json;
use std::fs;
use tempfile::TempDir;
use ucode_tools::fs_tools::{ListDirTool, ReadFileTool, register_fs_tools};
use ucode_tools::{ToolHandler, ToolRegistry};

#[tokio::test]
async fn read_file_success() {
    let dir = TempDir::new().unwrap();
    let file_path = dir.path().join("test.txt");
    fs::write(&file_path, "hello world").unwrap();

    let tool = ReadFileTool;
    let result = tool
        .invoke(json!({"path": file_path.to_str().unwrap()}))
        .await
        .unwrap();
    assert_eq!(result["content"], "hello world");
}

#[tokio::test]
async fn read_file_not_found() {
    let tool = ReadFileTool;
    let result = tool.invoke(json!({"path": "/nonexistent/file.txt"})).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn read_file_on_directory_fails() {
    let dir = TempDir::new().unwrap();
    let tool = ReadFileTool;
    let result = tool
        .invoke(json!({"path": dir.path().to_str().unwrap()}))
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn read_file_missing_path_arg() {
    let tool = ReadFileTool;
    let result = tool.invoke(json!({})).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn list_dir_success() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.txt"), "").unwrap();
    fs::create_dir(dir.path().join("subdir")).unwrap();

    let tool = ListDirTool;
    let result = tool
        .invoke(json!({"path": dir.path().to_str().unwrap()}))
        .await
        .unwrap();
    let entries = result["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
}

#[tokio::test]
async fn list_dir_not_a_directory() {
    let dir = TempDir::new().unwrap();
    let file_path = dir.path().join("file.txt");
    fs::write(&file_path, "").unwrap();

    let tool = ListDirTool;
    let result = tool
        .invoke(json!({"path": file_path.to_str().unwrap()}))
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn list_dir_entry_fields() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("data.bin"), b"abcde" as &[u8]).unwrap();
    fs::create_dir(dir.path().join("nested")).unwrap();

    let tool = ListDirTool;
    let result = tool
        .invoke(json!({"path": dir.path().to_str().unwrap()}))
        .await
        .unwrap();

    let entries = result["entries"].as_array().unwrap();
    let file_entry = entries
        .iter()
        .find(|e| e["name"] == "data.bin")
        .expect("data.bin entry");
    let dir_entry = entries
        .iter()
        .find(|e| e["name"] == "nested")
        .expect("nested entry");

    assert_eq!(file_entry["is_dir"], false);
    assert_eq!(file_entry["size"], 5);
    assert_eq!(dir_entry["is_dir"], true);
}

#[tokio::test]
async fn register_fs_tools_adds_both() {
    let mut registry = ToolRegistry::new();
    register_fs_tools(&mut registry);
    assert!(registry.get("read_file").is_some());
    assert!(registry.get("list_dir").is_some());
    assert_eq!(registry.list().len(), 2);
}

#[tokio::test]
async fn read_file_via_registry() {
    let dir = TempDir::new().unwrap();
    let file_path = dir.path().join("test.txt");
    fs::write(&file_path, "registry test").unwrap();

    let mut registry = ToolRegistry::new();
    register_fs_tools(&mut registry);

    let result = registry
        .invoke(
            "call-1",
            "read_file",
            json!({"path": file_path.to_str().unwrap()}),
        )
        .await
        .unwrap();
    assert!(!result.is_error);
    assert_eq!(result.result["content"], "registry test");
}

#[tokio::test]
async fn list_dir_via_registry() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("x.txt"), "").unwrap();

    let mut registry = ToolRegistry::new();
    register_fs_tools(&mut registry);

    let result = registry
        .invoke(
            "call-2",
            "list_dir",
            json!({"path": dir.path().to_str().unwrap()}),
        )
        .await
        .unwrap();
    assert!(!result.is_error);
    let entries = result.result["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["name"], "x.txt");
}
