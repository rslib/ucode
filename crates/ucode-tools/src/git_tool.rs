use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use serde_json::{Value, json};
use ucode_core::CoreError;

use crate::registry::{ToolHandler, ToolRegistry, ToolSpec};

// ── helpers ──────────────────────────────────────────────────────────────────

fn repo_path(args: &Value) -> PathBuf {
    args["path"]
        .as_str()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn open_repo(path: &Path) -> Result<gix::Repository, CoreError> {
    gix::discover(path).map_err(|e| CoreError::Tool(format!("not a git repository: {e}")))
}

// ── git_status ────────────────────────────────────────────────────────────────

pub struct GitStatusTool;

impl ToolHandler for GitStatusTool {
    fn invoke(
        &self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, CoreError>> + Send>> {
        Box::pin(async move {
            let path = repo_path(&args);
            tokio::task::spawn_blocking(move || git_status_impl(&path))
                .await
                .map_err(|e| CoreError::Tool(format!("git_status task panicked: {e}")))?
        })
    }
}

fn git_status_impl(path: &Path) -> Result<Value, CoreError> {
    use gix::status::index_worktree;
    use gix::status::plumbing::index_as_worktree::{Change, EntryStatus};

    let repo = open_repo(path)?;

    let platform = repo
        .status(gix::progress::Discard)
        .map_err(|e| CoreError::Tool(format!("failed to create status platform: {e}")))?;

    let iter = platform
        .into_iter(std::iter::empty::<gix::bstr::BString>())
        .map_err(|e| CoreError::Tool(format!("failed to start status iterator: {e}")))?;

    let mut staged: Vec<Value> = Vec::new();
    let mut unstaged: Vec<Value> = Vec::new();
    let mut untracked: Vec<Value> = Vec::new();

    for item in iter {
        let item = item.map_err(|e| CoreError::Tool(format!("status error: {e}")))?;

        match item {
            // HEAD-tree vs index → staged changes
            gix::status::Item::TreeIndex(change) => {
                let (path_str, status_str) = tree_index_change_info(&change);
                staged.push(json!({ "path": path_str, "status": status_str }));
            }

            // index vs worktree → unstaged modifications and untracked files
            gix::status::Item::IndexWorktree(iw) => match iw {
                index_worktree::Item::Modification {
                    rela_path, status, ..
                } => {
                    let status_str = match &status {
                        EntryStatus::Change(Change::Removed) => "deleted",
                        EntryStatus::Change(Change::Type { .. }) => "typechange",
                        EntryStatus::Change(Change::Modification { .. }) => "modified",
                        EntryStatus::Change(Change::SubmoduleModification(_)) => "submodule",
                        EntryStatus::Conflict { .. } => "conflict",
                        // NeedsUpdate is a stat-only refresh, not a user-visible change.
                        EntryStatus::NeedsUpdate(_) | EntryStatus::IntentToAdd => continue,
                    };
                    unstaged.push(json!({
                        "path": rela_path.to_string(),
                        "status": status_str,
                    }));
                }

                index_worktree::Item::DirectoryContents { entry, .. } => {
                    // Only report untracked; skip ignored entries.
                    if entry.status == gix::dir::entry::Status::Untracked {
                        untracked.push(json!({
                            "path": entry.rela_path.to_string(),
                            "status": "untracked",
                        }));
                    }
                }

                index_worktree::Item::Rewrite {
                    dirwalk_entry,
                    source,
                    copy,
                    ..
                } => {
                    let status_str = if copy { "copied" } else { "renamed" };
                    let src_path = source.rela_path().to_string();
                    unstaged.push(json!({
                        "path": dirwalk_entry.rela_path.to_string(),
                        "status": status_str,
                        "source": src_path,
                    }));
                }
            },
        }
    }

    Ok(json!({
        "staged": staged,
        "unstaged": unstaged,
        "untracked": untracked,
    }))
}

fn tree_index_change_info(change: &gix::diff::index::Change) -> (String, &'static str) {
    use gix::diff::index::ChangeRef;
    match change {
        ChangeRef::Addition { location, .. } => (location.to_string(), "added"),
        ChangeRef::Deletion { location, .. } => (location.to_string(), "deleted"),
        ChangeRef::Modification { location, .. } => (location.to_string(), "modified"),
        ChangeRef::Rewrite { location, copy, .. } => (
            location.to_string(),
            if *copy { "copied" } else { "renamed" },
        ),
    }
}

pub fn register_git_status_tool(registry: &mut ToolRegistry) {
    registry
        .register(
            ToolSpec {
                name: "git_status".into(),
                description: "List staged, unstaged, and untracked files in a git repository."
                    .into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to the git repository (defaults to current directory)."
                        }
                    }
                }),
            },
            Box::new(GitStatusTool),
        )
        .expect("git_status already registered");
}

// ── git_diff ──────────────────────────────────────────────────────────────────

pub struct GitDiffTool;

impl ToolHandler for GitDiffTool {
    fn invoke(
        &self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, CoreError>> + Send>> {
        Box::pin(async move {
            let path = repo_path(&args);
            let file_filter: Option<String> = args["file"].as_str().map(str::to_string);
            tokio::task::spawn_blocking(move || git_diff_impl(&path, file_filter.as_deref()))
                .await
                .map_err(|e| CoreError::Tool(format!("git_diff task panicked: {e}")))?
        })
    }
}

fn git_diff_impl(path: &Path, file_filter: Option<&str>) -> Result<Value, CoreError> {
    use gix::prelude::FindExt;

    let repo = open_repo(path)?;

    let workdir = repo
        .workdir()
        .ok_or_else(|| CoreError::Tool("bare repository has no working tree".into()))?
        .to_owned();

    let index = repo
        .index_or_empty()
        .map_err(|e| CoreError::Tool(format!("failed to load index: {e}")))?;

    let mut diff_output = String::new();

    for entry in index.entries() {
        let rela_path = entry.path(&index);
        let rela_str = std::str::from_utf8(rela_path)
            .map_err(|_| CoreError::Tool("non-UTF-8 path in index".into()))?;

        if let Some(filter) = file_filter
            && rela_str != filter
            && !rela_str.ends_with(filter)
        {
            continue;
        }

        // Skip non-regular-file entries (submodules = COMMIT, dirs = DIR).
        let mode = entry.mode;
        if mode != gix::index::entry::Mode::FILE && mode != gix::index::entry::Mode::FILE_EXECUTABLE
        {
            continue;
        }

        let worktree_path = workdir.join(rela_str);
        let worktree_bytes = match std::fs::read(&worktree_path) {
            Ok(b) => b,
            Err(_) => continue,
        };

        // Read the blob from the object database.
        let mut buf = Vec::new();
        let blob = repo
            .objects
            .find_blob(&entry.id, &mut buf)
            .map_err(|e| CoreError::Tool(format!("failed to read blob {}: {e}", entry.id)))?;
        let index_bytes = blob.data;

        if index_bytes == worktree_bytes.as_slice() {
            continue;
        }

        let hunk = diff_blobs(rela_str, index_bytes, &worktree_bytes)?;
        if !hunk.is_empty() {
            diff_output.push_str(&hunk);
        }
    }

    if diff_output.is_empty() {
        Ok(json!({ "diff": "", "message": "no changes" }))
    } else {
        Ok(json!({ "diff": diff_output }))
    }
}

/// Produce a unified diff between `before` and `after` bytes for the given `path`.
fn diff_blobs(path: &str, before: &[u8], after: &[u8]) -> Result<String, CoreError> {
    use gix::diff::blob::intern::InternedInput;
    use gix::diff::blob::{Algorithm, UnifiedDiffBuilder};

    let before_str = match std::str::from_utf8(before) {
        Ok(s) => s,
        Err(_) => return Ok(format!("Binary files a/{path} and b/{path} differ\n")),
    };
    let after_str = match std::str::from_utf8(after) {
        Ok(s) => s,
        Err(_) => return Ok(format!("Binary files a/{path} and b/{path} differ\n")),
    };

    let input = InternedInput::new(before_str, after_str);
    let raw = gix::diff::blob::diff(
        Algorithm::Histogram,
        &input,
        UnifiedDiffBuilder::new(&input),
    );

    if raw.is_empty() {
        return Ok(String::new());
    }

    let header = format!("--- a/{path}\n+++ b/{path}\n");
    Ok(format!("{header}{raw}"))
}

pub fn register_git_diff_tool(registry: &mut ToolRegistry) {
    registry
        .register(
            ToolSpec {
                name: "git_diff".into(),
                description:
                    "Return unified diff of working tree changes (unstaged) in a git repository."
                        .into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to the git repository (defaults to current directory)."
                        },
                        "file": {
                            "type": "string",
                            "description": "Limit diff to this specific file path (relative to repo root)."
                        }
                    }
                }),
            },
            Box::new(GitDiffTool),
        )
        .expect("git_diff already registered");
}
