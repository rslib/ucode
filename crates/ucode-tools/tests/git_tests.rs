use std::process::Command;

use serde_json::json;
use tempfile::TempDir;
use ucode_tools::git_tool::{GitDiffTool, GitStatusTool};
use ucode_tools::{ToolHandler, ToolRegistry, register_git_diff_tool, register_git_status_tool};

// ── test helpers ──────────────────────────────────────────────────────────────

/// Initialize a bare-minimum git repo with a configured identity so commits work.
fn init_repo(dir: &TempDir) -> std::path::PathBuf {
    let path = dir.path().to_path_buf();
    run_git(&path, &["init"]);
    run_git(&path, &["config", "user.email", "test@example.com"]);
    run_git(&path, &["config", "user.name", "Test"]);
    path
}

fn run_git(dir: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("git command failed to spawn");
    assert!(status.success(), "git {args:?} failed in {}", dir.display());
}

fn write_file(dir: &std::path::Path, name: &str, content: &str) {
    std::fs::write(dir.join(name), content).expect("write failed");
}

// ── git_status tests ──────────────────────────────────────────────────────────

/// Untracked file appears in the `untracked` list.
#[tokio::test]
async fn status_untracked_file() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "hello.txt", "hello\n");

    let result = GitStatusTool
        .invoke(json!({ "path": repo.to_str().unwrap() }))
        .await
        .unwrap();

    let untracked = result["untracked"].as_array().unwrap();
    assert!(
        untracked
            .iter()
            .any(|e| e["path"].as_str().unwrap() == "hello.txt"),
        "expected hello.txt in untracked, got: {result}"
    );
}

/// Staged file (git add) appears in the `staged` list.
#[tokio::test]
async fn status_staged_file() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "staged.txt", "content\n");
    run_git(&repo, &["add", "staged.txt"]);

    let result = GitStatusTool
        .invoke(json!({ "path": repo.to_str().unwrap() }))
        .await
        .unwrap();

    let staged = result["staged"].as_array().unwrap();
    assert!(
        staged
            .iter()
            .any(|e| e["path"].as_str().unwrap() == "staged.txt"),
        "expected staged.txt in staged, got: {result}"
    );
}

/// Committed then modified file appears in `unstaged`.
#[tokio::test]
async fn status_unstaged_modified() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "tracked.txt", "original\n");
    run_git(&repo, &["add", "tracked.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);

    // Modify without staging.
    write_file(&repo, "tracked.txt", "modified\n");

    let result = GitStatusTool
        .invoke(json!({ "path": repo.to_str().unwrap() }))
        .await
        .unwrap();

    let unstaged = result["unstaged"].as_array().unwrap();
    assert!(
        unstaged
            .iter()
            .any(|e| e["path"].as_str().unwrap() == "tracked.txt"),
        "expected tracked.txt in unstaged, got: {result}"
    );
}

/// Non-git directory returns an error.
#[tokio::test]
async fn status_non_git_directory() {
    let dir = TempDir::new().unwrap();
    let result = GitStatusTool
        .invoke(json!({ "path": dir.path().to_str().unwrap() }))
        .await;
    assert!(result.is_err(), "expected error for non-git dir");
}

/// Clean repo (no changes) returns empty lists.
#[tokio::test]
async fn status_clean_repo() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "clean.txt", "clean\n");
    run_git(&repo, &["add", "clean.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);

    let result = GitStatusTool
        .invoke(json!({ "path": repo.to_str().unwrap() }))
        .await
        .unwrap();

    assert_eq!(result["staged"].as_array().unwrap().len(), 0);
    assert_eq!(result["unstaged"].as_array().unwrap().len(), 0);
    assert_eq!(result["untracked"].as_array().unwrap().len(), 0);
}

// ── git_diff tests ────────────────────────────────────────────────────────────

/// Modified tracked file produces a diff containing the change.
#[tokio::test]
async fn diff_shows_modification() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "line one\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);

    write_file(&repo, "file.txt", "line one\nline two\n");

    let result = GitDiffTool
        .invoke(json!({ "path": repo.to_str().unwrap() }))
        .await
        .unwrap();

    let diff = result["diff"].as_str().unwrap();
    assert!(
        diff.contains("+line two"),
        "expected '+line two' in diff, got: {diff}"
    );
    assert!(
        diff.contains("file.txt"),
        "expected filename in diff, got: {diff}"
    );
}

/// Clean repo produces an empty diff with a message.
#[tokio::test]
async fn diff_no_changes() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "content\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);

    let result = GitDiffTool
        .invoke(json!({ "path": repo.to_str().unwrap() }))
        .await
        .unwrap();

    assert_eq!(result["diff"].as_str().unwrap(), "");
    assert!(
        result["message"].as_str().is_some(),
        "expected 'message' field for empty diff"
    );
}

/// File filter limits diff to only the specified file.
#[tokio::test]
async fn diff_specific_file() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "a.txt", "aaa\n");
    write_file(&repo, "b.txt", "bbb\n");
    run_git(&repo, &["add", "."]);
    run_git(&repo, &["commit", "-m", "initial"]);

    write_file(&repo, "a.txt", "aaa modified\n");
    write_file(&repo, "b.txt", "bbb modified\n");

    let result = GitDiffTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "file": "a.txt" }))
        .await
        .unwrap();

    let diff = result["diff"].as_str().unwrap();
    assert!(diff.contains("a.txt"), "expected a.txt in diff");
    assert!(
        !diff.contains("b.txt"),
        "b.txt should not appear in filtered diff"
    );
}

/// Non-git directory returns an error.
#[tokio::test]
async fn diff_non_git_directory() {
    let dir = TempDir::new().unwrap();
    let result = GitDiffTool
        .invoke(json!({ "path": dir.path().to_str().unwrap() }))
        .await;
    assert!(result.is_err(), "expected error for non-git dir");
}

// ── registry integration ──────────────────────────────────────────────────────

/// Both tools register and can be looked up by name.
#[test]
fn registry_integration() {
    let mut registry = ToolRegistry::new();
    register_git_status_tool(&mut registry);
    register_git_diff_tool(&mut registry);

    assert!(
        registry.get("git_status").is_some(),
        "git_status not found in registry"
    );
    assert!(
        registry.get("git_diff").is_some(),
        "git_diff not found in registry"
    );
}

/// Registering the same tool twice panics (via expect).
#[test]
#[should_panic(expected = "git_status already registered")]
fn registry_duplicate_panics() {
    let mut registry = ToolRegistry::new();
    register_git_status_tool(&mut registry);
    register_git_status_tool(&mut registry);
}

/// Registry invoke round-trip for git_status.
#[tokio::test]
async fn registry_invoke_git_status() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);

    let mut registry = ToolRegistry::new();
    register_git_status_tool(&mut registry);

    let result = registry
        .invoke(
            "call-1",
            "git_status",
            json!({ "path": repo.to_str().unwrap() }),
        )
        .await
        .unwrap();

    assert!(
        !result.is_error,
        "expected success, got: {:?}",
        result.result
    );
    assert!(result.result["staged"].is_array());
    assert!(result.result["unstaged"].is_array());
    assert!(result.result["untracked"].is_array());
}
