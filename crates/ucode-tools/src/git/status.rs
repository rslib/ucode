use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use serde_json::{Value, json};
use ucode_core::CoreError;

use super::{open_repo, repo_path};
use crate::registry::{ToolHandler, ToolRegistry, ToolSpec};

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
