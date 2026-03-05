use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use serde_json::{Value, json};
use ucode_core::CoreError;

use super::{open_repo, repo_path};
use crate::registry::{ToolHandler, ToolRegistry, ToolSpec};

// ── git_diff (unstaged: index vs worktree) ───────────────────────────────────

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

// ── shared diff helper ───────────────────────────────────────────────────────

/// Produce a unified diff between `before` and `after` bytes for the given `path`.
pub(crate) fn diff_blobs(path: &str, before: &[u8], after: &[u8]) -> Result<String, CoreError> {
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

// ── git_diff_staged (HEAD tree vs index) ─────────────────────────────────────

pub struct GitDiffStagedTool;

impl ToolHandler for GitDiffStagedTool {
    fn invoke(
        &self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, CoreError>> + Send>> {
        Box::pin(async move {
            let path = repo_path(&args);
            let file_filter: Option<String> = args["file"].as_str().map(str::to_string);
            tokio::task::spawn_blocking(move || git_diff_staged_impl(&path, file_filter.as_deref()))
                .await
                .map_err(|e| CoreError::Tool(format!("git_diff_staged task panicked: {e}")))?
        })
    }
}

fn git_diff_staged_impl(path: &Path, file_filter: Option<&str>) -> Result<Value, CoreError> {
    use gix::prelude::FindExt;

    let repo = open_repo(path)?;

    let index = repo
        .index_or_empty()
        .map_err(|e| CoreError::Tool(format!("failed to load index: {e}")))?;

    // Collect HEAD tree entries: path → blob id.
    let mut head_entries: std::collections::HashMap<String, gix::ObjectId> =
        std::collections::HashMap::new();

    if let Ok(head_commit) = repo.head_commit()
        && let Ok(tree) = head_commit.tree()
    {
        collect_blob_entries_from_tree(&repo, &tree, "", &mut head_entries)?;
    }

    let mut diff_output = String::new();

    // Compare index entries against HEAD.
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

        let mode = entry.mode;
        if mode != gix::index::entry::Mode::FILE && mode != gix::index::entry::Mode::FILE_EXECUTABLE
        {
            continue;
        }

        let mut buf_after = Vec::new();
        let after_blob = repo
            .objects
            .find_blob(&entry.id, &mut buf_after)
            .map_err(|e| CoreError::Tool(format!("failed to read blob: {e}")))?;
        let after_data = after_blob.data.to_vec();

        let before_data: Vec<u8> = if let Some(head_id) = head_entries.get(rela_str) {
            let mut buf_before = Vec::new();
            let before_blob = repo
                .objects
                .find_blob(head_id, &mut buf_before)
                .map_err(|e| CoreError::Tool(format!("failed to read HEAD blob: {e}")))?;
            before_blob.data.to_vec()
        } else {
            Vec::new() // new file
        };

        if before_data == after_data {
            continue;
        }

        let hunk = diff_blobs(rela_str, &before_data, &after_data)?;
        if !hunk.is_empty() {
            diff_output.push_str(&hunk);
        }
    }

    // Also check for deletions (files in HEAD but not in index).
    let index_paths: std::collections::HashSet<String> = index
        .entries()
        .iter()
        .filter_map(|e| std::str::from_utf8(e.path(&index)).ok().map(str::to_string))
        .collect();

    for (head_path, head_id) in &head_entries {
        if let Some(filter) = file_filter
            && head_path != filter
            && !head_path.ends_with(filter)
        {
            continue;
        }
        if !index_paths.contains(head_path) {
            let mut buf = Vec::new();
            let blob = repo
                .objects
                .find_blob(head_id, &mut buf)
                .map_err(|e| CoreError::Tool(format!("failed to read HEAD blob: {e}")))?;
            let hunk = diff_blobs(head_path, blob.data, b"")?;
            if !hunk.is_empty() {
                diff_output.push_str(&hunk);
            }
        }
    }

    if diff_output.is_empty() {
        Ok(json!({ "diff": "", "message": "no staged changes" }))
    } else {
        Ok(json!({ "diff": diff_output }))
    }
}

/// Recursively collect blob entries from a tree into `out` (path → blob id).
fn collect_blob_entries_from_tree(
    repo: &gix::Repository,
    tree: &gix::Tree<'_>,
    prefix: &str,
    out: &mut std::collections::HashMap<String, gix::ObjectId>,
) -> Result<(), CoreError> {
    use gix::bstr::ByteSlice;
    use gix::object::tree::EntryKind;
    use gix::prelude::FindExt;

    let tree_ref = tree
        .decode()
        .map_err(|e| CoreError::Tool(format!("failed to decode tree: {e}")))?;

    for entry in &tree_ref.entries {
        let name = entry
            .filename
            .to_str()
            .map_err(|_| CoreError::Tool("non-UTF-8 filename in tree".into()))?;
        let full_path = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}/{name}")
        };

        match entry.mode.kind() {
            EntryKind::Blob | EntryKind::BlobExecutable | EntryKind::Link => {
                out.insert(full_path, entry.oid.to_owned());
            }
            EntryKind::Tree => {
                let oid = entry.oid.to_owned();
                let mut sub_buf = Vec::new();
                let sub_obj = repo
                    .objects
                    .find(&oid, &mut sub_buf)
                    .map_err(|e| CoreError::Tool(format!("failed to find subtree: {e}")))?;
                let sub_tree = gix::Tree {
                    id: oid,
                    data: sub_obj.data.to_vec(),
                    repo,
                };
                collect_blob_entries_from_tree(repo, &sub_tree, &full_path, out)?;
            }
            EntryKind::Commit => {}
        }
    }
    Ok(())
}

pub fn register_git_diff_staged_tool(registry: &mut ToolRegistry) {
    registry
        .register(
            ToolSpec {
                name: "git_diff_staged".into(),
                description: "Return unified diff of staged changes (HEAD vs index) in a git repository.".into(),
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
            Box::new(GitDiffStagedTool),
        )
        .expect("git_diff_staged already registered");
}

// ── git_diff_commits (diff between two arbitrary refs) ────────────────────────

pub struct GitDiffCommitsTool;

impl ToolHandler for GitDiffCommitsTool {
    fn invoke(
        &self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, CoreError>> + Send>> {
        Box::pin(async move {
            let path = repo_path(&args);
            let from = args["from"]
                .as_str()
                .ok_or_else(|| CoreError::Tool("git_diff_commits: 'from' is required".into()))?
                .to_string();
            let to = args["to"]
                .as_str()
                .ok_or_else(|| CoreError::Tool("git_diff_commits: 'to' is required".into()))?
                .to_string();
            let file_filter: Option<String> = args["file"].as_str().map(str::to_string);
            tokio::task::spawn_blocking(move || {
                git_diff_commits_impl(&path, &from, &to, file_filter.as_deref())
            })
            .await
            .map_err(|e| CoreError::Tool(format!("git_diff_commits task panicked: {e}")))?
        })
    }
}

fn git_diff_commits_impl(
    path: &Path,
    from: &str,
    to: &str,
    file_filter: Option<&str>,
) -> Result<Value, CoreError> {
    use super::commit::resolve_ref;
    use gix::prelude::FindExt;

    let repo = open_repo(path)?;

    let from_id = resolve_ref(&repo, from)?;
    let to_id = resolve_ref(&repo, to)?;

    let from_commit = repo
        .find_commit(from_id)
        .map_err(|e| CoreError::Tool(format!("not a commit '{from}': {e}")))?;
    let to_commit = repo
        .find_commit(to_id)
        .map_err(|e| CoreError::Tool(format!("not a commit '{to}': {e}")))?;

    let from_tree = from_commit
        .tree()
        .map_err(|e| CoreError::Tool(format!("failed to get tree for '{from}': {e}")))?;
    let to_tree = to_commit
        .tree()
        .map_err(|e| CoreError::Tool(format!("failed to get tree for '{to}': {e}")))?;

    let mut from_entries: std::collections::HashMap<String, gix::ObjectId> =
        std::collections::HashMap::new();
    let mut to_entries: std::collections::HashMap<String, gix::ObjectId> =
        std::collections::HashMap::new();

    collect_blob_entries_from_tree(&repo, &from_tree, "", &mut from_entries)?;
    collect_blob_entries_from_tree(&repo, &to_tree, "", &mut to_entries)?;

    let mut all_paths: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    all_paths.extend(from_entries.keys().cloned());
    all_paths.extend(to_entries.keys().cloned());

    let mut diff_output = String::new();
    let mut buf_before = Vec::new();
    let mut buf_after = Vec::new();

    for file_path in &all_paths {
        if let Some(filter) = file_filter
            && file_path != filter
            && !file_path.ends_with(filter)
        {
            continue;
        }

        let before_id = from_entries.get(file_path).copied();
        let after_id = to_entries.get(file_path).copied();

        match (before_id, after_id) {
            (None, Some(aid)) => {
                buf_after.clear();
                let blob = repo
                    .objects
                    .find_blob(&aid, &mut buf_after)
                    .map_err(|e| CoreError::Tool(format!("blob read error: {e}")))?;
                let hunk = diff_blobs(file_path, b"", blob.data)?;
                diff_output.push_str(&hunk);
            }
            (Some(bid), None) => {
                buf_before.clear();
                let blob = repo
                    .objects
                    .find_blob(&bid, &mut buf_before)
                    .map_err(|e| CoreError::Tool(format!("blob read error: {e}")))?;
                let hunk = diff_blobs(file_path, blob.data, b"")?;
                diff_output.push_str(&hunk);
            }
            (Some(bid), Some(aid)) if bid != aid => {
                buf_before.clear();
                let before_blob = repo
                    .objects
                    .find_blob(&bid, &mut buf_before)
                    .map_err(|e| CoreError::Tool(format!("blob read error: {e}")))?;
                let before_data = before_blob.data.to_vec();
                buf_after.clear();
                let after_blob = repo
                    .objects
                    .find_blob(&aid, &mut buf_after)
                    .map_err(|e| CoreError::Tool(format!("blob read error: {e}")))?;
                let hunk = diff_blobs(file_path, &before_data, after_blob.data)?;
                diff_output.push_str(&hunk);
            }
            _ => {}
        }
    }

    Ok(json!({ "diff": diff_output }))
}

pub fn register_git_diff_commits_tool(registry: &mut ToolRegistry) {
    registry
        .register(
            ToolSpec {
                name: "git_diff_commits".into(),
                description: "Return unified diff between two arbitrary refs in a git repository.".into(),
                parameters: json!({
                    "type": "object",
                    "required": ["from", "to"],
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to the git repository (defaults to current directory)."
                        },
                        "from": {
                            "type": "string",
                            "description": "Starting ref or commit hash."
                        },
                        "to": {
                            "type": "string",
                            "description": "Ending ref or commit hash."
                        },
                        "file": {
                            "type": "string",
                            "description": "Limit diff to this specific file path (relative to repo root)."
                        }
                    }
                }),
            },
            Box::new(GitDiffCommitsTool),
        )
        .expect("git_diff_commits already registered");
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
