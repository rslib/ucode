use serde_json::json;
use std::fs;
use tempfile::TempDir;
use ucode_tools::ast_tool::{AstRewriteTool, AstSearchTool};
use ucode_tools::{ToolHandler, ToolRegistry, register_ast_rewrite_tool, register_ast_search_tool};

// ── helpers ──────────────────────────────────────────────────────────────────

async fn search(pattern: &str, path: &str, lang: &str) -> serde_json::Value {
    AstSearchTool
        .invoke(json!({ "pattern": pattern, "path": path, "lang": lang }))
        .await
        .unwrap()
}

async fn rewrite(pattern: &str, replacement: &str, path: &str, lang: &str) -> serde_json::Value {
    AstRewriteTool
        .invoke(json!({
            "pattern": pattern,
            "replacement": replacement,
            "path": path,
            "lang": lang,
        }))
        .await
        .unwrap()
}

// ── ast_search tests ──────────────────────────────────────────────────────────

#[tokio::test]
async fn search_finds_function_in_rust() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("funcs.rs");
    fs::write(&file, "fn foo() {}\nfn bar() {}\n").unwrap();

    let result = search("fn $NAME() {}", file.to_str().unwrap(), "rust").await;
    let matches = result["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 2, "expected 2 function matches");
    assert_eq!(result["total_matches"], 2);
}

#[tokio::test]
async fn search_finds_console_log_in_js() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("app.js");
    fs::write(&file, r#"console.log("hello");"#).unwrap();

    let result = search("console.log($MSG)", file.to_str().unwrap(), "javascript").await;
    let matches = result["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 1);
    assert!(matches[0]["text"].as_str().unwrap().contains("console.log"));
}

#[tokio::test]
async fn search_no_match() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("empty.rs");
    fs::write(&file, "fn main() {}\n").unwrap();

    let result = search("println!($MSG)", file.to_str().unwrap(), "rust").await;
    let matches = result["matches"].as_array().unwrap();
    assert!(matches.is_empty());
    assert_eq!(result["total_matches"], 0);
}

#[tokio::test]
async fn search_directory_walk() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.rs"), "fn alpha() {}\n").unwrap();
    fs::write(dir.path().join("b.rs"), "fn beta() {}\n").unwrap();
    fs::write(dir.path().join("c.rs"), "fn gamma() {}\n").unwrap();

    let result = search("fn $NAME() {}", dir.path().to_str().unwrap(), "rust").await;
    let matches = result["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 3, "should find one function per file");
    assert_eq!(result["total_matches"], 3);
}

#[tokio::test]
async fn search_respects_lang_filter() {
    let dir = TempDir::new().unwrap();
    // Rust file with a function
    fs::write(dir.path().join("code.rs"), "fn foo() {}\n").unwrap();
    // Python file — should be ignored when lang=rust
    fs::write(dir.path().join("script.py"), "def foo(): pass\n").unwrap();

    let result = search("fn $NAME() {}", dir.path().to_str().unwrap(), "rust").await;
    let matches = result["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 1, "only .rs file should be searched");
    assert!(matches[0]["file"].as_str().unwrap().ends_with(".rs"));
}

#[tokio::test]
async fn search_result_has_correct_line_numbers() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("lines.rs");
    // println! is on line 3 (1-based)
    fs::write(
        &file,
        "fn main() {\n    let x = 1;\n    println!(\"hi\");\n}\n",
    )
    .unwrap();

    let result = search("println!($MSG)", file.to_str().unwrap(), "rust").await;
    let matches = result["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0]["line"], 3, "line numbers should be 1-based");
}

// ── ast_rewrite tests ─────────────────────────────────────────────────────────

#[tokio::test]
async fn rewrite_replaces_pattern() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("main.rs");
    fs::write(
        &file,
        "fn main() {\n    println!(\"hello\");\n    println!(\"world\");\n}\n",
    )
    .unwrap();

    let result = rewrite(
        "println!($MSG)",
        "log::info!($MSG)",
        file.to_str().unwrap(),
        "rust",
    )
    .await;

    assert_eq!(result["total_replacements"], 2);
    let changed: Vec<_> = result["files_changed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(changed.len(), 1);

    let new_content = fs::read_to_string(&file).unwrap();
    assert!(
        new_content.contains("log::info!"),
        "replacement should appear in file"
    );
    assert!(
        !new_content.contains("println!"),
        "original pattern should be gone"
    );
}

#[tokio::test]
async fn rewrite_no_match_no_change() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("noop.rs");
    let original = "fn main() {}\n";
    fs::write(&file, original).unwrap();

    let result = rewrite(
        "println!($MSG)",
        "log::info!($MSG)",
        file.to_str().unwrap(),
        "rust",
    )
    .await;

    assert_eq!(result["total_replacements"], 0);
    let changed = result["files_changed"].as_array().unwrap();
    assert!(changed.is_empty());

    let content = fs::read_to_string(&file).unwrap();
    assert_eq!(content, original, "file should be unchanged");
}

#[tokio::test]
async fn rewrite_directory_multiple_files() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.rs"), "fn main() { println!(\"a\"); }\n").unwrap();
    fs::write(dir.path().join("b.rs"), "fn main() { println!(\"b\"); }\n").unwrap();

    let result = rewrite(
        "println!($MSG)",
        "eprintln!($MSG)",
        dir.path().to_str().unwrap(),
        "rust",
    )
    .await;

    assert_eq!(result["total_replacements"], 2);
    let changed = result["files_changed"].as_array().unwrap();
    assert_eq!(changed.len(), 2);
}

// ── error handling tests ──────────────────────────────────────────────────────

#[tokio::test]
async fn search_invalid_lang_returns_error() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("f.rs"), "fn foo() {}").unwrap();

    let tool = AstSearchTool;
    let result = tool
        .invoke(json!({
            "pattern": "fn $NAME() {}",
            "path": dir.path().to_str().unwrap(),
            "lang": "brainfuck",
        }))
        .await;

    assert!(result.is_err(), "unsupported language should return error");
}

#[tokio::test]
async fn search_missing_pattern_returns_error() {
    let dir = TempDir::new().unwrap();
    let tool = AstSearchTool;
    let result = tool
        .invoke(json!({
            "path": dir.path().to_str().unwrap(),
            "lang": "rust",
        }))
        .await;

    assert!(result.is_err(), "missing 'pattern' should return error");
}

#[tokio::test]
async fn search_missing_lang_returns_error() {
    let dir = TempDir::new().unwrap();
    let tool = AstSearchTool;
    let result = tool
        .invoke(json!({
            "pattern": "fn $NAME() {}",
            "path": dir.path().to_str().unwrap(),
        }))
        .await;

    assert!(result.is_err(), "missing 'lang' should return error");
}

#[tokio::test]
async fn rewrite_missing_replacement_returns_error() {
    let dir = TempDir::new().unwrap();
    let tool = AstRewriteTool;
    let result = tool
        .invoke(json!({
            "pattern": "println!($MSG)",
            "path": dir.path().to_str().unwrap(),
            "lang": "rust",
        }))
        .await;

    assert!(result.is_err(), "missing 'replacement' should return error");
}

// ── registry integration ──────────────────────────────────────────────────────

#[tokio::test]
async fn registry_integration() {
    let mut registry = ToolRegistry::new();
    register_ast_search_tool(&mut registry);
    register_ast_rewrite_tool(&mut registry);

    assert!(registry.get("ast_search").is_some());
    assert!(registry.get("ast_rewrite").is_some());

    // Invoke via registry to exercise the full dispatch path.
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("reg.rs"), "fn hello() {}\n").unwrap();

    let result = registry
        .invoke(
            "call-1",
            "ast_search",
            json!({
                "pattern": "fn $NAME() {}",
                "path": dir.path().to_str().unwrap(),
                "lang": "rust",
            }),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    let matches = result.result["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 1);
}
