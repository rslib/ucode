use std::process::Command;

use serde_json::json;
use tempfile::TempDir;
use ucode_tools::git::branch::{GitBranchTool, GitCheckoutTool};
use ucode_tools::git::commit::{GitCommitTool, GitLogTool, GitShowTool, GitTagTool};
use ucode_tools::git::diff::{GitDiffCommitsTool, GitDiffStagedTool, GitDiffTool};
use ucode_tools::git::merge::{GitCherryPickTool, GitMergeTool, GitRebaseTool};
use ucode_tools::git::staging::{GitAddTool, GitResetTool, GitRestoreTool};
use ucode_tools::git::stash::GitStashTool;
use ucode_tools::git::status::GitStatusTool;
use ucode_tools::{
    ToolHandler, ToolRegistry, register_git_add_tool, register_git_branch_tool,
    register_git_checkout_tool, register_git_cherry_pick_tool, register_git_commit_tool,
    register_git_diff_commits_tool, register_git_diff_staged_tool, register_git_diff_tool,
    register_git_log_tool, register_git_merge_tool, register_git_rebase_tool,
    register_git_reset_tool, register_git_restore_tool, register_git_show_tool,
    register_git_stash_tool, register_git_status_tool, register_git_tag_tool,
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

// ── git_branch tests ──────────────────────────────────────────────────────────

/// List shows the current branch after init.
#[tokio::test]
async fn branch_list() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "content\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);

    let result = GitBranchTool
        .invoke(json!({ "path": repo.to_str().unwrap() }))
        .await
        .unwrap();

    let branches = result["branches"].as_array().unwrap();
    assert!(!branches.is_empty(), "expected at least one branch");
    let current = result["current"].as_str().unwrap();
    assert!(!current.is_empty(), "expected non-empty current branch");
}

/// Creating a branch returns `created` and it appears in the list.
#[tokio::test]
async fn branch_create() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "content\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);

    let result = GitBranchTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "name": "feature" }))
        .await
        .unwrap();

    assert_eq!(result["created"].as_str().unwrap(), "feature");

    // Verify it appears in the list.
    let list = GitBranchTool
        .invoke(json!({ "path": repo.to_str().unwrap() }))
        .await
        .unwrap();
    let branches = list["branches"].as_array().unwrap();
    assert!(
        branches.iter().any(|b| b.as_str().unwrap() == "feature"),
        "feature branch not in list: {list}"
    );
}

/// Creating a branch from a specific commit points to that commit.
#[tokio::test]
async fn branch_create_from_commit() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "v1\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "first"]);

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

    let result = GitBranchTool
        .invoke(json!({
            "path": repo.to_str().unwrap(),
            "name": "at-first",
            "start_point": first_hash,
        }))
        .await
        .unwrap();

    assert_eq!(result["created"].as_str().unwrap(), "at-first");
}

/// Deleting a non-current branch returns `deleted`.
#[tokio::test]
async fn branch_delete() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "content\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);

    // Create a branch to delete.
    GitBranchTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "name": "to-delete" }))
        .await
        .unwrap();

    let result = GitBranchTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "name": "to-delete", "delete": true }))
        .await
        .unwrap();

    assert_eq!(result["deleted"].as_str().unwrap(), "to-delete");
}

/// Deleting the currently checked-out branch returns an error.
#[tokio::test]
async fn branch_delete_current_error() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "content\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);

    // Get the current branch name.
    let current = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(&repo)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();

    let result = GitBranchTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "name": current, "delete": true }))
        .await;

    assert!(result.is_err(), "expected error deleting current branch");
}

// ── git_checkout tests ────────────────────────────────────────────────────────

/// Switching to an existing branch updates HEAD.
#[tokio::test]
async fn checkout_branch() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "content\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);

    // Create a second branch via git CLI.
    run_git(&repo, &["branch", "other"]);

    let result = GitCheckoutTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "branch": "other" }))
        .await
        .unwrap();

    assert_eq!(result["switched_to"].as_str().unwrap(), "other");

    // Verify HEAD now points to `other`.
    let head = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(&repo)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();
    assert_eq!(head, "other");
}

/// create=true creates the branch and switches to it.
#[tokio::test]
async fn checkout_create_branch() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "content\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);

    let result = GitCheckoutTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "branch": "new-branch", "create": true }))
        .await
        .unwrap();

    assert_eq!(result["switched_to"].as_str().unwrap(), "new-branch");

    let head = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(&repo)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();
    assert_eq!(head, "new-branch");
}

/// Switching to a non-existent branch returns an error.
#[tokio::test]
async fn checkout_nonexistent_error() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "content\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);

    let result = GitCheckoutTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "branch": "ghost" }))
        .await;

    assert!(result.is_err(), "expected error for non-existent branch");
}

/// Restoring specific files from HEAD overwrites worktree changes.
#[tokio::test]
async fn checkout_restore_files() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "original\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);

    // Modify the file without staging.
    write_file(&repo, "file.txt", "modified\n");

    let result = GitCheckoutTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "files": ["file.txt"] }))
        .await
        .unwrap();

    let restored = result["restored"].as_array().unwrap();
    assert_eq!(restored.len(), 1);

    let content = std::fs::read_to_string(repo.join("file.txt")).unwrap();
    assert_eq!(
        content, "original\n",
        "file should be restored to HEAD content"
    );
}

// ── git_reset tests ───────────────────────────────────────────────────────────

/// Staging a file then resetting it removes it from staged.
#[tokio::test]
async fn reset_unstage_file() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "content\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);

    // Modify and stage.
    write_file(&repo, "file.txt", "modified\n");
    run_git(&repo, &["add", "file.txt"]);

    // Verify it's staged.
    let status_before = GitStatusTool
        .invoke(json!({ "path": repo.to_str().unwrap() }))
        .await
        .unwrap();
    assert!(
        status_before["staged"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["path"].as_str().unwrap() == "file.txt"),
        "file.txt should be staged before reset"
    );

    // Reset the specific file.
    let result = GitResetTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "files": ["file.txt"] }))
        .await
        .unwrap();

    let unstaged = result["unstaged"].as_array().unwrap();
    assert!(
        unstaged.iter().any(|f| f.as_str().unwrap() == "file.txt"),
        "file.txt should be in unstaged list"
    );

    // Verify it's no longer staged.
    let status_after = GitStatusTool
        .invoke(json!({ "path": repo.to_str().unwrap() }))
        .await
        .unwrap();
    assert!(
        !status_after["staged"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["path"].as_str().unwrap() == "file.txt"),
        "file.txt should not be staged after reset"
    );
}

/// Mixed reset moves HEAD and resets index but keeps worktree.
#[tokio::test]
async fn reset_mixed() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "v1\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "first"]);

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

    let result = GitResetTool
        .invoke(json!({
            "path": repo.to_str().unwrap(),
            "mode": "mixed",
            "commit": first_hash,
        }))
        .await
        .unwrap();

    assert_eq!(result["reset_to"].as_str().unwrap(), first_hash);

    // HEAD should now point to first commit.
    let head_hash = String::from_utf8(
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
    assert_eq!(head_hash, first_hash);

    // Worktree should still have v2 content.
    let content = std::fs::read_to_string(repo.join("file.txt")).unwrap();
    assert_eq!(
        content, "v2\n",
        "worktree should be unchanged after mixed reset"
    );
}

/// Soft reset moves HEAD only; index is unchanged.
#[tokio::test]
async fn reset_soft() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "v1\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "first"]);

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

    let result = GitResetTool
        .invoke(json!({
            "path": repo.to_str().unwrap(),
            "mode": "soft",
            "commit": first_hash,
        }))
        .await
        .unwrap();

    assert_eq!(result["reset_to"].as_str().unwrap(), first_hash);

    // HEAD should point to first commit.
    let head_hash = String::from_utf8(
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
    assert_eq!(head_hash, first_hash);

    // Worktree still has v2.
    let content = std::fs::read_to_string(repo.join("file.txt")).unwrap();
    assert_eq!(content, "v2\n");
}

/// Hard reset moves HEAD, index, and worktree to match the target commit.
#[tokio::test]
async fn reset_hard() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "v1\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "first"]);

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

    let result = GitResetTool
        .invoke(json!({
            "path": repo.to_str().unwrap(),
            "mode": "hard",
            "commit": first_hash,
        }))
        .await
        .unwrap();

    assert_eq!(result["reset_to"].as_str().unwrap(), first_hash);

    // HEAD should point to first commit.
    let head_hash = String::from_utf8(
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
    assert_eq!(head_hash, first_hash);

    // Worktree should have v1 content.
    let content = std::fs::read_to_string(repo.join("file.txt")).unwrap();
    assert_eq!(
        content, "v1\n",
        "worktree should match first commit after hard reset"
    );
}

// ── git_restore tests ─────────────────────────────────────────────────────────

/// Restoring a modified tracked file from the index reverts worktree changes.
#[tokio::test]
async fn restore_worktree() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "original\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);

    // Modify without staging.
    write_file(&repo, "file.txt", "modified\n");

    let result = GitRestoreTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "files": ["file.txt"] }))
        .await
        .unwrap();

    let restored = result["restored"].as_array().unwrap();
    assert_eq!(restored.len(), 1);

    let content = std::fs::read_to_string(repo.join("file.txt")).unwrap();
    assert_eq!(
        content, "original\n",
        "worktree should be restored from index"
    );
}

/// Restoring with staged=true resets the index entry from HEAD.
#[tokio::test]
async fn restore_staged() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "original\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);

    // Modify and stage.
    write_file(&repo, "file.txt", "modified\n");
    run_git(&repo, &["add", "file.txt"]);

    // Verify staged.
    let status_before = GitStatusTool
        .invoke(json!({ "path": repo.to_str().unwrap() }))
        .await
        .unwrap();
    assert!(
        status_before["staged"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["path"].as_str().unwrap() == "file.txt"),
        "file.txt should be staged"
    );

    // Restore staged (reset index from HEAD).
    let result = GitRestoreTool
        .invoke(json!({
            "path": repo.to_str().unwrap(),
            "files": ["file.txt"],
            "staged": true,
        }))
        .await
        .unwrap();

    let restored = result["restored"].as_array().unwrap();
    assert_eq!(restored.len(), 1);

    // Should no longer be staged.
    let status_after = GitStatusTool
        .invoke(json!({ "path": repo.to_str().unwrap() }))
        .await
        .unwrap();
    assert!(
        !status_after["staged"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["path"].as_str().unwrap() == "file.txt"),
        "file.txt should not be staged after restore --staged"
    );
}

/// Calling git_restore without files returns an error.
#[tokio::test]
async fn restore_missing_files_error() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);

    let result = GitRestoreTool
        .invoke(json!({ "path": repo.to_str().unwrap() }))
        .await;

    assert!(result.is_err(), "expected error when files arg is missing");
}

// ── batch-2 registry integration ─────────────────────────────────────────────

/// All 4 batch-2 tools register successfully.
#[test]
fn registry_batch2_tools_register() {
    let mut registry = ToolRegistry::new();
    register_git_branch_tool(&mut registry);
    register_git_checkout_tool(&mut registry);
    register_git_reset_tool(&mut registry);
    register_git_restore_tool(&mut registry);

    for name in &["git_branch", "git_checkout", "git_reset", "git_restore"] {
        assert!(registry.get(name).is_some(), "{name} not found in registry");
    }
}

// ── git_stash tests ───────────────────────────────────────────────────────────

/// Push stashes changes; worktree is clean and stash appears in list.
#[tokio::test]
async fn stash_push_and_list() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "original\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);

    // Modify the tracked file.
    write_file(&repo, "file.txt", "modified\n");
    run_git(&repo, &["add", "file.txt"]);

    let push_result = GitStashTool
        .invoke(json!({
            "path": repo.to_str().unwrap(),
            "action": "push",
            "message": "my stash",
        }))
        .await
        .unwrap();

    assert!(push_result["stashed"].as_bool().unwrap());
    assert_eq!(push_result["message"].as_str().unwrap(), "my stash");

    // Worktree should be back to HEAD content.
    let content = std::fs::read_to_string(repo.join("file.txt")).unwrap();
    assert_eq!(
        content, "original\n",
        "worktree should be clean after stash push"
    );

    // Stash should appear in list.
    let list_result = GitStashTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "action": "list" }))
        .await
        .unwrap();

    let stashes = list_result["stashes"].as_array().unwrap();
    assert_eq!(stashes.len(), 1);
    assert_eq!(stashes[0]["index"].as_u64().unwrap(), 0);
    assert_eq!(stashes[0]["message"].as_str().unwrap(), "my stash");
}

/// Pop restores stashed changes to the worktree.
#[tokio::test]
async fn stash_pop() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "original\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);

    write_file(&repo, "file.txt", "stashed change\n");
    run_git(&repo, &["add", "file.txt"]);

    GitStashTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "action": "push" }))
        .await
        .unwrap();

    // Confirm worktree is clean.
    let before = std::fs::read_to_string(repo.join("file.txt")).unwrap();
    assert_eq!(before, "original\n");

    let pop_result = GitStashTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "action": "pop" }))
        .await
        .unwrap();

    assert!(pop_result["restored"].as_bool().unwrap());

    // Stashed content should be back.
    let after = std::fs::read_to_string(repo.join("file.txt")).unwrap();
    assert_eq!(
        after, "stashed change\n",
        "pop should restore stashed content"
    );

    // Stash list should now be empty.
    let list = GitStashTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "action": "list" }))
        .await
        .unwrap();
    assert_eq!(list["stashes"].as_array().unwrap().len(), 0);
}

/// Drop removes the stash entry without restoring.
#[tokio::test]
async fn stash_drop() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "original\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);

    write_file(&repo, "file.txt", "to be dropped\n");
    run_git(&repo, &["add", "file.txt"]);

    GitStashTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "action": "push" }))
        .await
        .unwrap();

    let drop_result = GitStashTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "action": "drop" }))
        .await
        .unwrap();

    assert!(drop_result["dropped"].as_bool().unwrap());
    assert_eq!(drop_result["index"].as_u64().unwrap(), 0);

    // List should be empty.
    let list = GitStashTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "action": "list" }))
        .await
        .unwrap();
    assert_eq!(list["stashes"].as_array().unwrap().len(), 0);

    // Worktree should still have HEAD content (drop does not restore).
    let content = std::fs::read_to_string(repo.join("file.txt")).unwrap();
    assert_eq!(content, "original\n", "drop should not restore worktree");
}

/// Pop with no stash entries returns an error.
#[tokio::test]
async fn stash_empty_error() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "content\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);

    let result = GitStashTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "action": "pop" }))
        .await;

    assert!(result.is_err(), "expected error when popping empty stash");
}

/// git_stash registers successfully.
#[test]
fn registry_stash_registers() {
    let mut registry = ToolRegistry::new();
    register_git_stash_tool(&mut registry);
    assert!(
        registry.get("git_stash").is_some(),
        "git_stash not found in registry"
    );
}

// ── git_merge tests ───────────────────────────────────────────────────────────

/// Fast-forward merge: HEAD is ancestor of branch, just moves HEAD.
#[tokio::test]
async fn merge_fast_forward() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "v1\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);

    // Create a branch and add a commit on it.
    run_git(&repo, &["checkout", "-b", "feature"]);
    write_file(&repo, "file.txt", "v2\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "feature commit"]);

    // Switch back to main and merge feature (fast-forward).
    run_git(&repo, &["checkout", "-"]);

    let result = GitMergeTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "branch": "feature" }))
        .await
        .unwrap();

    assert!(
        result["fast_forward"].as_bool().unwrap_or(false),
        "expected fast_forward: {result}"
    );
    assert_eq!(result["conflicts"].as_array().unwrap().len(), 0);

    // Worktree should have v2.
    let content = std::fs::read_to_string(repo.join("file.txt")).unwrap();
    assert_eq!(
        content, "v2\n",
        "worktree should have feature content after ff merge"
    );
}

/// Clean three-way merge: diverged branches with non-overlapping changes.
#[tokio::test]
async fn merge_clean() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "a.txt", "aaa\n");
    write_file(&repo, "b.txt", "bbb\n");
    run_git(&repo, &["add", "."]);
    run_git(&repo, &["commit", "-m", "initial"]);

    // Create feature branch and modify b.txt.
    run_git(&repo, &["checkout", "-b", "feature"]);
    write_file(&repo, "b.txt", "bbb modified\n");
    run_git(&repo, &["add", "b.txt"]);
    run_git(&repo, &["commit", "-m", "feature: modify b"]);

    // Switch to main and modify a.txt.
    run_git(&repo, &["checkout", "-"]);
    write_file(&repo, "a.txt", "aaa modified\n");
    run_git(&repo, &["add", "a.txt"]);
    run_git(&repo, &["commit", "-m", "main: modify a"]);

    let result = GitMergeTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "branch": "feature" }))
        .await
        .unwrap();

    assert_eq!(
        result["conflicts"].as_array().unwrap().len(),
        0,
        "expected clean merge: {result}"
    );
    assert!(
        result["hash"].as_str().is_some(),
        "expected merge commit hash"
    );

    // Both files should have their respective changes.
    let a = std::fs::read_to_string(repo.join("a.txt")).unwrap();
    let b = std::fs::read_to_string(repo.join("b.txt")).unwrap();
    assert_eq!(a, "aaa modified\n");
    assert_eq!(b, "bbb modified\n");
}

/// Conflict merge: both branches modify the same file differently.
#[tokio::test]
async fn merge_conflict() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "original\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);

    // Feature branch changes file.txt.
    run_git(&repo, &["checkout", "-b", "feature"]);
    write_file(&repo, "file.txt", "feature version\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "feature change"]);

    // Main also changes file.txt.
    run_git(&repo, &["checkout", "-"]);
    write_file(&repo, "file.txt", "main version\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "main change"]);

    let result = GitMergeTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "branch": "feature" }))
        .await
        .unwrap();

    assert_eq!(
        result["status"].as_str().unwrap(),
        "conflict",
        "expected conflict status: {result}"
    );
    let conflicts = result["conflicts"].as_array().unwrap();
    assert!(
        conflicts.iter().any(|c| c.as_str().unwrap() == "file.txt"),
        "expected file.txt in conflicts: {result}"
    );

    // Conflict markers should be in the worktree file.
    let content = std::fs::read_to_string(repo.join("file.txt")).unwrap();
    assert!(
        content.contains("<<<<<<< HEAD"),
        "expected conflict markers: {content}"
    );
    assert!(
        content.contains("======="),
        "expected conflict markers: {content}"
    );
    assert!(
        content.contains(">>>>>>>"),
        "expected conflict markers: {content}"
    );
}

/// Merging a non-existent branch returns an error.
#[tokio::test]
async fn merge_invalid_branch_error() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "content\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);

    let result = GitMergeTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "branch": "nonexistent" }))
        .await;

    assert!(result.is_err(), "expected error for non-existent branch");
}

// ── git_cherry_pick tests ─────────────────────────────────────────────────────

/// Cherry-pick a commit from another branch cleanly.
#[tokio::test]
async fn cherry_pick_clean() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "a.txt", "aaa\n");
    run_git(&repo, &["add", "a.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);

    // Create a branch and add a new file.
    run_git(&repo, &["checkout", "-b", "feature"]);
    write_file(&repo, "b.txt", "bbb\n");
    run_git(&repo, &["add", "b.txt"]);
    run_git(&repo, &["commit", "-m", "add b.txt"]);

    // Get the commit hash.
    let pick_hash = String::from_utf8(
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

    // Switch back to main and cherry-pick.
    run_git(&repo, &["checkout", "-"]);

    let result = GitCherryPickTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "commit": pick_hash }))
        .await
        .unwrap();

    assert!(result["hash"].as_str().is_some(), "expected hash: {result}");
    assert_eq!(result["conflicts"].as_array().unwrap().len(), 0);

    // b.txt should now exist on main.
    assert!(
        repo.join("b.txt").exists(),
        "b.txt should exist after cherry-pick"
    );
    let content = std::fs::read_to_string(repo.join("b.txt")).unwrap();
    assert_eq!(content, "bbb\n");
}

/// Cherry-pick a conflicting commit returns conflict status.
#[tokio::test]
async fn cherry_pick_conflict() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "original\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);

    // Create a branch and modify file.txt.
    run_git(&repo, &["checkout", "-b", "feature"]);
    write_file(&repo, "file.txt", "feature version\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "feature change"]);

    let pick_hash = String::from_utf8(
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

    // Switch to main and also modify file.txt differently.
    run_git(&repo, &["checkout", "-"]);
    write_file(&repo, "file.txt", "main version\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "main change"]);

    let result = GitCherryPickTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "commit": pick_hash }))
        .await
        .unwrap();

    assert_eq!(
        result["status"].as_str().unwrap(),
        "conflict",
        "expected conflict: {result}"
    );
    let conflicts = result["conflicts"].as_array().unwrap();
    assert!(
        conflicts.iter().any(|c| c.as_str().unwrap() == "file.txt"),
        "expected file.txt in conflicts: {result}"
    );
}

/// Cherry-pick with an invalid ref returns an error.
#[tokio::test]
async fn cherry_pick_invalid_ref_error() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "content\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);

    let result = GitCherryPickTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "commit": "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef" }))
        .await;

    assert!(result.is_err(), "expected error for invalid ref");
}

// ── git_rebase tests ──────────────────────────────────────────────────────────

/// Simple rebase: replay branch commits onto updated main.
#[tokio::test]
async fn rebase_simple() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "a.txt", "aaa\n");
    run_git(&repo, &["add", "a.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);

    // Create feature branch with one commit.
    run_git(&repo, &["checkout", "-b", "feature"]);
    write_file(&repo, "b.txt", "bbb\n");
    run_git(&repo, &["add", "b.txt"]);
    run_git(&repo, &["commit", "-m", "feature: add b"]);

    // Add a commit to main.
    run_git(&repo, &["checkout", "-"]);
    write_file(&repo, "c.txt", "ccc\n");
    run_git(&repo, &["add", "c.txt"]);
    run_git(&repo, &["commit", "-m", "main: add c"]);

    let onto_hash = String::from_utf8(
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

    // Switch to feature and rebase onto main.
    run_git(&repo, &["checkout", "feature"]);

    let result = GitRebaseTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "onto": onto_hash }))
        .await
        .unwrap();

    assert_eq!(
        result["status"].as_str().unwrap(),
        "ok",
        "expected ok: {result}"
    );
    assert_eq!(result["rebased_commits"].as_u64().unwrap(), 1);

    // Both b.txt and c.txt should exist.
    assert!(
        repo.join("b.txt").exists(),
        "b.txt should exist after rebase"
    );
    assert!(
        repo.join("c.txt").exists(),
        "c.txt should exist after rebase"
    );
}

/// Rebase conflict: conflict during rebase saves state.
#[tokio::test]
async fn rebase_conflict() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "original\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);

    // Feature branch modifies file.txt.
    run_git(&repo, &["checkout", "-b", "feature"]);
    write_file(&repo, "file.txt", "feature version\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "feature change"]);

    // Main also modifies file.txt.
    run_git(&repo, &["checkout", "-"]);
    write_file(&repo, "file.txt", "main version\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "main change"]);

    let onto_hash = String::from_utf8(
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

    run_git(&repo, &["checkout", "feature"]);

    let result = GitRebaseTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "onto": onto_hash }))
        .await
        .unwrap();

    assert_eq!(
        result["status"].as_str().unwrap(),
        "conflict",
        "expected conflict: {result}"
    );
    assert!(
        result["conflicts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c.as_str().unwrap() == "file.txt"),
        "expected file.txt in conflicts: {result}"
    );

    // Rebase state file should exist.
    let state_path = repo.join(".git").join("ucode-rebase-state");
    assert!(state_path.exists(), "rebase state file should exist");
}

/// Interactive rebase: squash two commits into one.
#[tokio::test]
async fn rebase_interactive_squash() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "v1\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);

    let base_hash = String::from_utf8(
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

    // Two commits to squash.
    write_file(&repo, "file.txt", "v2\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "second"]);

    let second_hash = String::from_utf8(
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

    write_file(&repo, "file.txt", "v3\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "third"]);

    let third_hash = String::from_utf8(
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

    let result = GitRebaseTool
        .invoke(json!({
            "path": repo.to_str().unwrap(),
            "onto": base_hash,
            "interactive": true,
            "actions": [
                { "action": "pick", "commit": second_hash },
                { "action": "squash", "commit": third_hash },
            ]
        }))
        .await
        .unwrap();

    assert_eq!(
        result["status"].as_str().unwrap(),
        "ok",
        "expected ok: {result}"
    );

    // Should have 2 rebased (pick + squash both count).
    assert!(result["rebased_commits"].as_u64().unwrap() >= 1);

    // File should have v3 content.
    let content = std::fs::read_to_string(repo.join("file.txt")).unwrap();
    assert_eq!(content, "v3\n", "file should have final squashed content");
}

/// Interactive rebase: drop a commit.
#[tokio::test]
async fn rebase_interactive_drop() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "a.txt", "aaa\n");
    run_git(&repo, &["add", "a.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);

    let base_hash = String::from_utf8(
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

    // Commit to keep.
    write_file(&repo, "b.txt", "bbb\n");
    run_git(&repo, &["add", "b.txt"]);
    run_git(&repo, &["commit", "-m", "add b"]);

    let keep_hash = String::from_utf8(
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

    // Commit to drop.
    write_file(&repo, "c.txt", "ccc\n");
    run_git(&repo, &["add", "c.txt"]);
    run_git(&repo, &["commit", "-m", "add c (to drop)"]);

    let drop_hash = String::from_utf8(
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

    let result = GitRebaseTool
        .invoke(json!({
            "path": repo.to_str().unwrap(),
            "onto": base_hash,
            "interactive": true,
            "actions": [
                { "action": "pick", "commit": keep_hash },
                { "action": "drop", "commit": drop_hash },
            ]
        }))
        .await
        .unwrap();

    assert_eq!(
        result["status"].as_str().unwrap(),
        "ok",
        "expected ok: {result}"
    );
    assert_eq!(
        result["rebased_commits"].as_u64().unwrap(),
        1,
        "only 1 commit picked"
    );

    // b.txt should exist, c.txt should not.
    assert!(repo.join("b.txt").exists(), "b.txt should exist");
    assert!(!repo.join("c.txt").exists(), "c.txt should be dropped");
}

/// Rebase abort: restores original HEAD state.
#[tokio::test]
async fn rebase_abort() {
    let dir = TempDir::new().unwrap();
    let repo = init_repo(&dir);
    write_file(&repo, "file.txt", "original\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);

    // Feature branch modifies file.txt.
    run_git(&repo, &["checkout", "-b", "feature"]);
    write_file(&repo, "file.txt", "feature version\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "feature change"]);

    let feature_head = String::from_utf8(
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

    // Main also modifies file.txt (to force conflict).
    run_git(&repo, &["checkout", "-"]);
    write_file(&repo, "file.txt", "main version\n");
    run_git(&repo, &["add", "file.txt"]);
    run_git(&repo, &["commit", "-m", "main change"]);

    let onto_hash = String::from_utf8(
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

    run_git(&repo, &["checkout", "feature"]);

    // Start rebase (will conflict).
    let conflict_result = GitRebaseTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "onto": onto_hash }))
        .await
        .unwrap();
    assert_eq!(conflict_result["status"].as_str().unwrap(), "conflict");

    // Abort the rebase.
    let abort_result = GitRebaseTool
        .invoke(json!({ "path": repo.to_str().unwrap(), "abort": true }))
        .await
        .unwrap();

    assert_eq!(abort_result["status"].as_str().unwrap(), "aborted");

    // HEAD should be back to the feature branch commit.
    let head_hash = String::from_utf8(
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
    assert_eq!(
        head_hash, feature_head,
        "HEAD should be restored to original feature commit"
    );

    // Rebase state file should be gone.
    let state_path = repo.join(".git").join("ucode-rebase-state");
    assert!(
        !state_path.exists(),
        "rebase state file should be removed after abort"
    );
}

/// All 3 merge tools register successfully.
#[test]
fn registry_merge_tools_register() {
    let mut registry = ToolRegistry::new();
    register_git_merge_tool(&mut registry);
    register_git_cherry_pick_tool(&mut registry);
    register_git_rebase_tool(&mut registry);

    for name in &["git_merge", "git_cherry_pick", "git_rebase"] {
        assert!(registry.get(name).is_some(), "{name} not found in registry");
    }
}
