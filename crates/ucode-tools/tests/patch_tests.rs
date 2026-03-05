use serde_json::json;
use std::fs;
use tempfile::TempDir;
use ucode_tools::patch_tool::{PatchTool, register_patch_tool};
use ucode_tools::{ToolHandler, ToolRegistry};

// ── helpers ───────────────────────────────────────────────────────────────────

fn write(dir: &TempDir, name: &str, content: &str) {
    fs::write(dir.path().join(name), content).unwrap();
}

fn read(dir: &TempDir, name: &str) -> String {
    fs::read_to_string(dir.path().join(name)).unwrap()
}

async fn invoke(diff: &str, base_dir: &str) -> serde_json::Value {
    PatchTool
        .invoke(json!({ "diff": diff, "base_dir": base_dir }))
        .await
        .unwrap()
}

// ── Test 1: simple single-hunk patch ─────────────────────────────────────────

#[tokio::test]
async fn single_hunk_applies() {
    let dir = TempDir::new().unwrap();
    write(&dir, "file.txt", "line1\nold line\nline3\n");

    let diff =
        "--- a/file.txt\n+++ b/file.txt\n@@ -1,3 +1,3 @@\n line1\n-old line\n+new line\n line3\n";
    let result = invoke(diff, dir.path().to_str().unwrap()).await;

    assert_eq!(result["applied"], true);
    assert!(result["rejects"].as_array().unwrap().is_empty());
    assert_eq!(read(&dir, "file.txt"), "line1\nnew line\nline3\n");
}

// ── Test 2: multi-hunk patch ──────────────────────────────────────────────────

#[tokio::test]
async fn multi_hunk_applies() {
    let dir = TempDir::new().unwrap();
    write(
        &dir,
        "multi.txt",
        "aaa\nbbb\nccc\nddd\neee\nfff\nggg\nhhh\n",
    );

    let diff = concat!(
        "--- a/multi.txt\n",
        "+++ b/multi.txt\n",
        "@@ -1,3 +1,3 @@\n",
        " aaa\n",
        "-bbb\n",
        "+BBB\n",
        " ccc\n",
        "@@ -6,3 +6,3 @@\n",
        " fff\n",
        "-ggg\n",
        "+GGG\n",
        " hhh\n",
    );
    let result = invoke(diff, dir.path().to_str().unwrap()).await;

    assert_eq!(result["applied"], true);
    assert!(result["rejects"].as_array().unwrap().is_empty());
    let content = read(&dir, "multi.txt");
    assert!(content.contains("BBB"), "first hunk applied");
    assert!(content.contains("GGG"), "second hunk applied");
    assert!(!content.contains("bbb"), "old line removed");
    assert!(!content.contains("ggg"), "old line removed");
}

// ── Test 3: new file creation (--- /dev/null) ─────────────────────────────────

#[tokio::test]
async fn new_file_creation() {
    let dir = TempDir::new().unwrap();

    let diff = concat!(
        "--- /dev/null\n",
        "+++ b/newfile.txt\n",
        "@@ -0,0 +1,3 @@\n",
        "+first line\n",
        "+second line\n",
        "+third line\n",
    );
    let result = invoke(diff, dir.path().to_str().unwrap()).await;

    assert_eq!(result["applied"], true);
    let content = read(&dir, "newfile.txt");
    assert!(content.contains("first line"));
    assert!(content.contains("third line"));
}

// ── Test 4: "deletion" patch removes content ──────────────────────────────────
//
// mpatch is a content-patcher: it applies the hunk (strips the removed lines)
// but does not unlink the file. The result is an empty file, not a missing one.

#[tokio::test]
async fn file_deletion_empties_file() {
    let dir = TempDir::new().unwrap();
    write(&dir, "todelete.txt", "bye\n");

    let diff = concat!(
        "--- a/todelete.txt\n",
        "+++ /dev/null\n",
        "@@ -1,1 +0,0 @@\n",
        "-bye\n",
    );
    let result = invoke(diff, dir.path().to_str().unwrap()).await;

    assert_eq!(result["applied"], true);
    // mpatch removes the content but leaves the file on disk.
    let content = read(&dir, "todelete.txt");
    assert!(
        content.is_empty() || !content.contains("bye"),
        "content should be removed"
    );
}

// ── Test 5: missing target file produces reject (hard error) ──────────────────

#[tokio::test]
async fn missing_target_file_produces_reject() {
    let dir = TempDir::new().unwrap();
    // "missing.txt" is never created.

    let diff = "--- a/missing.txt\n+++ b/missing.txt\n@@ -1 +1 @@\n-foo\n+bar\n";
    let result = invoke(diff, dir.path().to_str().unwrap()).await;

    // mpatch returns a hard PatchError::TargetNotFound for a non-creation patch
    // against a missing file, which we surface as a reject with hunk=0.
    assert_eq!(result["applied"], false);
    let rejects = result["rejects"].as_array().unwrap();
    assert_eq!(rejects.len(), 1);
}

// ── Test 6: missing diff arg returns error ────────────────────────────────────

#[tokio::test]
async fn missing_diff_arg_returns_error() {
    let result = PatchTool.invoke(json!({ "base_dir": "/tmp" })).await;
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("diff"), "error should mention 'diff'");
}

// ── Test 7: missing base_dir arg returns error ────────────────────────────────

#[tokio::test]
async fn missing_base_dir_arg_returns_error() {
    let result = PatchTool
        .invoke(json!({ "diff": "--- a/f\n+++ b/f\n" }))
        .await;
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("base_dir"), "error should mention 'base_dir'");
}

// ── Test 8: invalid diff (no patches found) returns error ─────────────────────

#[tokio::test]
async fn invalid_diff_returns_error() {
    let dir = TempDir::new().unwrap();
    let result = PatchTool
        .invoke(
            json!({ "diff": "not a valid diff at all", "base_dir": dir.path().to_str().unwrap() }),
        )
        .await;
    assert!(result.is_err(), "no-patch input should return Err");
}

// ── Test 9: registry integration ─────────────────────────────────────────────

#[tokio::test]
async fn registry_integration() {
    let mut registry = ToolRegistry::new();
    register_patch_tool(&mut registry);

    assert!(registry.get("apply_patch").is_some());

    let dir = TempDir::new().unwrap();
    write(&dir, "reg.txt", "hello\nworld\n");

    let diff = "--- a/reg.txt\n+++ b/reg.txt\n@@ -1,2 +1,2 @@\n hello\n-world\n+WORLD\n";
    let result = registry
        .invoke(
            "call-patch-1",
            "apply_patch",
            json!({ "diff": diff, "base_dir": dir.path().to_str().unwrap() }),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert_eq!(result.result["applied"], true);
    assert_eq!(read(&dir, "reg.txt"), "hello\nWORLD\n");
}

// ── Test 10: multi-file patch ─────────────────────────────────────────────────

#[tokio::test]
async fn multi_file_patch() {
    let dir = TempDir::new().unwrap();
    write(&dir, "a.txt", "aaa\n");
    write(&dir, "b.txt", "bbb\n");

    let diff = concat!(
        "--- a/a.txt\n",
        "+++ b/a.txt\n",
        "@@ -1,1 +1,1 @@\n",
        "-aaa\n",
        "+AAA\n",
        "--- a/b.txt\n",
        "+++ b/b.txt\n",
        "@@ -1,1 +1,1 @@\n",
        "-bbb\n",
        "+BBB\n",
    );
    let result = invoke(diff, dir.path().to_str().unwrap()).await;

    assert_eq!(result["applied"], true);
    let changed = result["files_changed"].as_array().unwrap();
    assert_eq!(changed.len(), 2);
    assert_eq!(read(&dir, "a.txt"), "AAA\n");
    assert_eq!(read(&dir, "b.txt"), "BBB\n");
}

// ── Test 11: fuzzy matching — shifted hunk ────────────────────────────────────

#[tokio::test]
async fn fuzzy_matching_shifted_hunk() {
    let dir = TempDir::new().unwrap();
    // Extra lines at the top shift the target context down by 5.
    let prefix = "extra1\nextra2\nextra3\nextra4\nextra5\n";
    write(
        &dir,
        "shifted.txt",
        &format!("{prefix}line1\nold line\nline3\n"),
    );

    // Diff was written against the file without the prefix (old_start=1).
    let diff = "--- a/shifted.txt\n+++ b/shifted.txt\n@@ -1,3 +1,3 @@\n line1\n-old line\n+new line\n line3\n";
    let result = invoke(diff, dir.path().to_str().unwrap()).await;

    assert_eq!(
        result["applied"], true,
        "shifted hunk should apply via fuzzy matching"
    );
    let content = read(&dir, "shifted.txt");
    assert!(content.contains("new line"), "replacement applied");
    assert!(!content.contains("old line"), "old line removed");
}

// ── Test 12: markdown-embedded diff ──────────────────────────────────────────

#[tokio::test]
async fn markdown_embedded_diff() {
    let dir = TempDir::new().unwrap();
    write(
        &dir,
        "main.rs",
        "fn main() {\n    println!(\"Hello\");\n}\n",
    );

    let diff = r#"Here is the change:

```diff
--- a/main.rs
+++ b/main.rs
@@ -1,3 +1,3 @@
 fn main() {
-    println!("Hello");
+    println!("World");
 }
```

That's it."#;

    let result = invoke(diff, dir.path().to_str().unwrap()).await;

    assert_eq!(result["applied"], true);
    let content = read(&dir, "main.rs");
    assert!(content.contains("World"));
    assert!(!content.contains("Hello"));
}

// ── Test 13: add-only hunk ────────────────────────────────────────────────────

#[tokio::test]
async fn patch_adds_lines_only() {
    let dir = TempDir::new().unwrap();
    write(&dir, "addonly.txt", "line1\nline3\n");

    let diff = concat!(
        "--- a/addonly.txt\n",
        "+++ b/addonly.txt\n",
        "@@ -1,2 +1,3 @@\n",
        " line1\n",
        "+line2\n",
        " line3\n",
    );
    let result = invoke(diff, dir.path().to_str().unwrap()).await;

    assert_eq!(result["applied"], true);
    assert_eq!(read(&dir, "addonly.txt"), "line1\nline2\nline3\n");
}

// ── Test 14: remove-only hunk ─────────────────────────────────────────────────

#[tokio::test]
async fn patch_removes_lines_only() {
    let dir = TempDir::new().unwrap();
    write(&dir, "removeonly.txt", "line1\nline2\nline3\n");

    let diff = concat!(
        "--- a/removeonly.txt\n",
        "+++ b/removeonly.txt\n",
        "@@ -1,3 +1,2 @@\n",
        " line1\n",
        "-line2\n",
        " line3\n",
    );
    let result = invoke(diff, dir.path().to_str().unwrap()).await;

    assert_eq!(result["applied"], true);
    assert_eq!(read(&dir, "removeonly.txt"), "line1\nline3\n");
}
