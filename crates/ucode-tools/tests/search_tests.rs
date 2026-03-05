use serde_json::json;
use std::fs;
use tempfile::TempDir;
use ucode_tools::search_tool::{SearchTool, register_search_tool};
use ucode_tools::{ToolHandler, ToolRegistry};

#[tokio::test]
async fn search_finds_match() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("hello.txt"), "hello world\ngoodbye world\n").unwrap();

    let tool = SearchTool;
    let result = tool
        .invoke(json!({"query": "hello", "path": dir.path().to_str().unwrap()}))
        .await
        .unwrap();

    let matches = result["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0]["line"], "hello world");
    assert_eq!(matches[0]["line_number"], 1);
}

#[tokio::test]
async fn search_no_match() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("hello.txt"), "hello world\n").unwrap();

    let tool = SearchTool;
    let result = tool
        .invoke(json!({"query": "zzzzz", "path": dir.path().to_str().unwrap()}))
        .await
        .unwrap();

    let matches = result["matches"].as_array().unwrap();
    assert!(matches.is_empty());
    assert_eq!(result["total_matches"], 0);
}

#[tokio::test]
async fn search_respects_max_results() {
    let dir = TempDir::new().unwrap();
    let content = (0..100)
        .map(|i| format!("line {}", i))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(dir.path().join("many.txt"), &content).unwrap();

    let tool = SearchTool;
    let result = tool
        .invoke(json!({"query": "line", "path": dir.path().to_str().unwrap(), "max_results": 5}))
        .await
        .unwrap();

    let matches = result["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 5);
    assert_eq!(result["total_matches"], 100);
    assert_eq!(result["truncated"], true);
}

#[tokio::test]
async fn search_with_context_lines() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("ctx.txt"), "aaa\nbbb\nccc\nddd\neee\n").unwrap();

    let tool = SearchTool;
    let result = tool
        .invoke(json!({"query": "ccc", "path": dir.path().to_str().unwrap(), "context_lines": 1}))
        .await
        .unwrap();

    let matches = result["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0]["context_before"], json!(["bbb"]));
    assert_eq!(matches[0]["context_after"], json!(["ddd"]));
}

#[tokio::test]
async fn search_invalid_regex() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("f.txt"), "test").unwrap();

    let tool = SearchTool;
    let result = tool
        .invoke(json!({"query": "[invalid", "path": dir.path().to_str().unwrap()}))
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn search_not_a_directory() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("f.txt");
    fs::write(&file, "test").unwrap();

    let tool = SearchTool;
    let result = tool
        .invoke(json!({"query": "test", "path": file.to_str().unwrap()}))
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn search_missing_query_arg() {
    let dir = TempDir::new().unwrap();
    let tool = SearchTool;
    let result = tool
        .invoke(json!({"path": dir.path().to_str().unwrap()}))
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn register_search_tool_works() {
    let mut registry = ToolRegistry::new();
    register_search_tool(&mut registry);
    assert!(registry.get("search").is_some());
}

#[tokio::test]
async fn search_respects_gitignore() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join(".gitignore"), "ignored.txt\n").unwrap();
    fs::write(dir.path().join("included.txt"), "findme\n").unwrap();
    fs::write(dir.path().join("ignored.txt"), "findme\n").unwrap();

    let tool = SearchTool;
    let result = tool
        .invoke(json!({"query": "findme", "path": dir.path().to_str().unwrap()}))
        .await
        .unwrap();

    let matches = result["matches"].as_array().unwrap();
    // Only included.txt should match; ignored.txt is excluded by .gitignore
    assert_eq!(matches.len(), 1);
    let path = matches[0]["path"].as_str().unwrap();
    assert!(path.contains("included.txt"));
}
