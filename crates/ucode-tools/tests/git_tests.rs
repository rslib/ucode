use std::process::Command;

use serde_json::json;
use tempfile::TempDir;
use ucode_tools::git::commit::{GitCommitTool, GitLogTool, GitShowTool, GitTagTool};
use ucode_tools::git::diff::{GitDiffCommitsTool, GitDiffStagedTool, GitDiffTool};
use ucode_tools::git::staging::GitAddTool;
use ucode_tools::git::status::GitStatusTool;
use ucode_tools::{
    ToolHandler, ToolRegistry, register_git_add_tool, register_git_commit_tool,
    register_git_diff_commits_tool, register_git_diff_staged_tool, register_git_diff_tool,
    register_git_log_tool, register_git_show_tool, register_git_status_tool, register_git_tag_tool,
};

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

// ── git_add tests ─────────────────────────────────────────────────────────────

/// Staging a file returns it in the `added` list.
#[tokio::test]
async fn add_stages_file() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "hello.txt", "hello\n");

    let result = GitAddTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "files": ["hello.txt"] }))
        .await
        .unwrap();

    let added = result["added"].as_array().unwrap();
    assert_eq!(added.len(), 1);
    assert_eq!(added[0].as_str().unwrap(), "hello.txt");
}

/// Staging multiple files returns all of them.
#[tokio::test]
async fn add_stages_multiple_files() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "a.txt", "aaa\n");
    write_file(&repo, "b.txt", "bbb\n");

    let result = GitAddTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "files": ["a.txt", "b.txt"] }))
        .await
        .unwrap();

    let added = result["added"].as_array().unwrap();
    assert_eq!(added.len(), 2);
}

/// Staging a non-existent file returns an error.
#[tokio::test]
async fn add_missing_file_errors() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);

    let result = GitAddTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "files": ["ghost.txt"] }))
        .await;

    assert!(result.is_err(), "expected error for missing file");
}

/// Calling git_add with an empty files list returns an error.
#[tokio::test]
async fn add_empty_files_list_errors() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);

    let result = GitAddTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "files": [] }))
        .await;

    assert!(result.is_err(), "expected error for empty files list");
}

// ── git_commit tests ──────────────────────────────────────────────────────────

/// Committing staged changes returns a hash and message.
#[tokio::test]
async fn commit_creates_commit() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "content\n");
    run_git(&repo, &["add", "file.txt"]);

    let result = GitCommitTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "message": "initial commit" }))
        .await
        .unwrap();

    let hash = result["hash"].as_str().unwrap();
    assert_eq!(hash.len(), 40, "expected 40-char SHA1, got: {hash}");
    assert_eq!(result["message"].as_str().unwrap(), "initial commit");
}

/// Committing with an empty index returns an error.
#[tokio::test]
async fn commit_empty_index_errors() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);

    let result = GitCommitTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "message": "empty" }))
        .await;

    assert!(result.is_err(), "expected error for empty index");
}

/// Committing without a message returns an error.
#[tokio::test]
async fn commit_missing_message_errors() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "content\n");
    run_git(&repo, &["add", "file.txt"]);

    let result = GitCommitTool
        .invoke(json!({ "path": repo.to_str().unwrap() }))
        .await;

    assert!(result.is_err(), "expected error for missing message");
}

/// Author override is accepted and commit succeeds.
#[tokio::test]
async fn commit_with_author_override() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "content\n");
    run_git(&repo, &["add", "file.txt"]);

    let result = GitCommitTool
        .invoke(json!({
            "path": repo.to_str().unwrap(),
            "message": "authored commit",
            "author": "Alice <alice@example.com>"
        }))
        .await
        .unwrap();

    assert!(result["hash"].as_str().unwrap().len() == 40);
}

// ── git_log tests ─────────────────────────────────────────────────────────────

/// Log returns commits in reverse-chronological order.
#[tokio::test]
async fn log_returns_commits() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "v1\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "first"]);
    write_file(&repo, "file.txt", "v2\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "second"]);

    let result = GitLogTool
        .invoke(json!({ "path": repo.to_str().unwrap() }))
        .await
        .unwrap();

    let commits = result["commits"].as_array().unwrap();
    assert_eq!(commits.len(), 2);
    assert_eq!(commits[0]["message"].as_str().unwrap(), "second");
    assert_eq!(commits[1]["message"].as_str().unwrap(), "first");
}

/// max_count limits the number of commits returned.
#[tokio::test]
async fn log_max_count() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    for i in 0..5 {
        write_file(&repo, "file.txt", &format!("v{i}\n"));
        run_git(&repo, &["add", "file.txt"]);
        run_git(&repo, &["commit", "-m", &format!("commit {i}")]);
    }

    let result = GitLogTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "max_count": 2 }))
        .await
        .unwrap();

    let commits = result["commits"].as_array().unwrap();
    assert_eq!(commits.len(), 2);
}

/// Log on a repo with no commits returns an error (HEAD doesn't exist).
#[tokio::test]
async fn log_empty_repo_errors() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);

    let result = GitLogTool
        .invoke(json!({ "path": repo.to_str().unwrap() }))
        .await;

    assert!(result.is_err(), "expected error for empty repo");
}

// ── git_show tests ────────────────────────────────────────────────────────────

/// Show returns commit metadata and a diff for the initial commit.
#[tokio::test]
async fn show_initial_commit() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "hello\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);

    let result = GitShowTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "commit": "HEAD" }))
        .await
        .unwrap();

    assert_eq!(result["message"].as_str().unwrap().trim(), "initial");
    assert!(result["hash"].as_str().unwrap().len() == 40);
    let diff = result["diff"].as_str().unwrap();
    assert!(diff.contains("+hello"), "expected '+hello' in diff: {diff}");
}

/// Show a second commit includes only the changed file in the diff.
#[tokio::test]
async fn show_second_commit_diff() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "v1\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "first"]);
    write_file(&repo, "file.txt", "v2\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "second"]);

    let result = GitShowTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "commit": "HEAD" }))
        .await
        .unwrap();

    let diff = result["diff"].as_str().unwrap();
    assert!(diff.contains("+v2"), "expected '+v2' in diff: {diff}");
    assert!(diff.contains("-v1"), "expected '-v1' in diff: {diff}");
}

/// Show with an invalid ref returns an error.
#[tokio::test]
async fn show_invalid_ref_errors() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);

    let result = GitShowTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "commit": "deadbeef" }))
        .await;

    assert!(result.is_err(), "expected error for invalid ref");
}

// ── git_tag tests ─────────────────────────────────────────────────────────────

/// Creating a tag returns `created`.
#[tokio::test]
async fn tag_create() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "content\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);

    let result = GitTagTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "name": "v1.0", "list": false }))
        .await
        .unwrap();

    assert_eq!(result["created"].as_str().unwrap(), "v1.0");
}

/// Listing tags returns the created tag (tag created via GitTagTool to avoid shell git tag).
#[tokio::test]
async fn tag_list() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "content\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);

    // Create the tag via our tool (avoids shell git tag which may hang).
    GitTagTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "name": "v1.0", "list": false }))
        .await
        .unwrap();

    let result = GitTagTool
        .invoke(json!({ "path": repo.to_str().unwrap() }))
        .await
        .unwrap();

    let tags = result["tags"].as_array().unwrap();
    assert!(
        tags.iter().any(|t| t.as_str().unwrap() == "v1.0"),
        "expected v1.0 in tags: {result}"
    );
}

/// Deleting a tag returns `deleted` (tag created via GitTagTool to avoid shell git tag).
#[tokio::test]
async fn tag_delete() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "content\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);

    // Create the tag via our tool.
    GitTagTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "name": "v1.0", "list": false }))
        .await
        .unwrap();

    let result = GitTagTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "name": "v1.0", "delete": true }))
        .await
        .unwrap();

    assert_eq!(result["deleted"].as_str().unwrap(), "v1.0");
}

/// Deleting without a name returns an error.
#[tokio::test]
async fn tag_delete_without_name_errors() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);

    let result = GitTagTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "delete": true }))
        .await;

    assert!(result.is_err(), "expected error for delete without name");
}

// ── git_diff_staged tests ─────────────────────────────────────────────────────

/// Staged new file shows up as an addition.
#[tokio::test]
async fn diff_staged_new_file() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "new.txt", "brand new\n");
    run_git(&repo, &["add", "new.txt"]);

    let result = GitDiffStagedTool
        .invoke(json!({ "path": repo.to_str().unwrap() }))
        .await
        .unwrap();

    let diff = result["diff"].as_str().unwrap();
    assert!(
        diff.contains("+brand new"),
        "expected '+brand new' in staged diff: {diff}"
    );
}

/// Staged modification shows the changed line.
#[tokio::test]
async fn diff_staged_modification() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "original\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);

    write_file(&repo, "file.txt", "modified\n");
    run_git(&repo, &["add", "file.txt"]);

    let result = GitDiffStagedTool
        .invoke(json!({ "path": repo.to_str().unwrap() }))
        .await
        .unwrap();

    let diff = result["diff"].as_str().unwrap();
    assert!(diff.contains("+modified"), "expected '+modified': {diff}");
    assert!(diff.contains("-original"), "expected '-original': {diff}");
}

/// No staged changes returns empty diff with message.
#[tokio::test]
async fn diff_staged_no_changes() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "content\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);

    let result = GitDiffStagedTool
        .invoke(json!({ "path": repo.to_str().unwrap() }))
        .await
        .unwrap();

    assert_eq!(result["diff"].as_str().unwrap(), "");
    assert!(result["message"].as_str().is_some());
}

// ── git_diff_commits tests ────────────────────────────────────────────────────

/// Diff between two commits shows the change.
#[tokio::test]
async fn diff_commits_shows_change() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "v1\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "first"]);

    // Capture first commit hash.
    let first_hash = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&repo)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();

    write_file(&repo, "file.txt", "v2\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "second"]);

    let result = GitDiffCommitsTool
        .invoke(json!({
            "path": repo.to_str().unwrap(),
            "from": first_hash,
            "to": "HEAD"
        }))
        .await
        .unwrap();

    let diff = result["diff"].as_str().unwrap();
    assert!(diff.contains("+v2"), "expected '+v2': {diff}");
    assert!(diff.contains("-v1"), "expected '-v1': {diff}");
}

/// Diff between identical commits returns empty diff.
#[tokio::test]
async fn diff_commits_same_commit() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "content\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);

    let result = GitDiffCommitsTool
        .invoke(json!({
            "path": repo.to_str().unwrap(),
            "from": "HEAD",
            "to": "HEAD"
        }))
        .await
        .unwrap();

    assert_eq!(result["diff"].as_str().unwrap(), "");
}

/// Missing `from` argument returns an error.
#[tokio::test]
async fn diff_commits_missing_from_errors() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);

    let result = GitDiffCommitsTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "to": "HEAD" }))
        .await;

    assert!(result.is_err(), "expected error for missing 'from'");
}

// ── registry integration for new tools ───────────────────────────────────────

/// All 7 new tools register successfully.
#[test]
fn registry_new_tools_register() {
    let mut registry = ToolRegistry::new();
    register_git_add_tool(&mut registry);
    register_git_commit_tool(&mut registry);
    register_git_log_tool(&mut registry);
    register_git_show_tool(&mut registry);
    register_git_tag_tool(&mut registry);
    register_git_diff_staged_tool(&mut registry);
    register_git_diff_commits_tool(&mut registry);

    for name in &[
        "git_add",
        "git_commit",
        "git_log",
        "git_show",
        "git_tag",
        "git_diff_staged",
        "git_diff_commits",
    ] {
        assert!(registry.get(name).is_some(), "{name} not found in registry");
    }
}
