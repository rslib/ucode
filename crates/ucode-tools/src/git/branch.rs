use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use serde_json::{Value, json};
use ucode_core::CoreError;

use super::commit::{collect_tree_entries, resolve_ref};
use super::{open_repo, repo_path};
use crate::registry::{ToolHandler, ToolRegistry, ToolSpec};

// ── git_branch ────────────────────────────────────────────────────────────────

pub struct GitBranchTool;

impl ToolHandler for GitBranchTool {
    fn invoke(
        &self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, CoreError>> + Send>> {
        Box::pin(async move {
            let path = repo_path(&args);
            let name: Option<String> = args["name"].as_str().map(str::to_string);
            let delete = args["delete"].as_bool().unwrap_or(false);
            let start_point: String = args["start_point"].as_str().unwrap_or("HEAD").to_string();
            tokio::task::spawn_blocking(move || {
                git_branch_impl(&path, name.as_deref(), delete, &start_point)
            })
            .await
            .map_err(|e| CoreError::Tool(format!("git_branch task panicked: {e}")))?
        })
    }
}

fn git_branch_impl(
    path: &Path,
    name: Option<&str>,
    delete: bool,
    start_point: &str,
) -> Result<Value, CoreError> {
    use gix::refs::transaction::{Change, LogChange, PreviousValue, RefEdit, RefLog};

    let repo = open_repo(path)?;

    // Current branch name (short, e.g. "main").
    let current = current_branch_name(&repo);

    if delete {
        let branch_name =
            name.ok_or_else(|| CoreError::Tool("git_branch: 'name' required for delete".into()))?;

        if current.as_deref() == Some(branch_name) {
            return Err(CoreError::Tool(format!(
                "git_branch: cannot delete the currently checked-out branch '{branch_name}'"
            )));
        }

        let full_ref = format!("refs/heads/{branch_name}");
        repo.edit_reference(RefEdit {
            change: Change::Delete {
                expected: PreviousValue::Any,
                log: RefLog::AndReference,
            },
            name: full_ref
                .as_str()
                .try_into()
                .map_err(|e| CoreError::Tool(format!("invalid branch name: {e}")))?,
            deref: false,
        })
        .map_err(|e| CoreError::Tool(format!("failed to delete branch '{branch_name}': {e}")))?;

        return Ok(json!({ "deleted": branch_name }));
    }

    if let Some(branch_name) = name {
        // Create branch pointing to start_point.
        let target_id = resolve_ref(&repo, start_point)?;
        let full_ref = format!("refs/heads/{branch_name}");
        repo.edit_reference(RefEdit {
            change: Change::Update {
                log: LogChange::default(),
                expected: PreviousValue::MustNotExist,
                new: gix::refs::Target::Object(target_id),
            },
            name: full_ref
                .as_str()
                .try_into()
                .map_err(|e| CoreError::Tool(format!("invalid branch name: {e}")))?,
            deref: false,
        })
        .map_err(|e| CoreError::Tool(format!("failed to create branch '{branch_name}': {e}")))?;

        return Ok(json!({ "created": branch_name }));
    }

    // List branches.
    let refs = repo
        .references()
        .map_err(|e| CoreError::Tool(format!("failed to list references: {e}")))?;

    let branches: Vec<String> = refs
        .prefixed("refs/heads/")
        .map_err(|e| CoreError::Tool(format!("failed to iterate branches: {e}")))?
        .filter_map(|r| r.ok())
        .map(|r| r.name().shorten().to_string())
        .collect();

    Ok(json!({
        "branches": branches,
        "current": current.unwrap_or_default(),
    }))
}

/// Return the short branch name HEAD points to, or None if detached.
pub(crate) fn current_branch_name(repo: &gix::Repository) -> Option<String> {
    repo.head_name()
        .ok()
        .flatten()
        .map(|n| n.shorten().to_string())
}

pub fn register_git_branch_tool(registry: &mut ToolRegistry) {
    registry
        .register(
            ToolSpec {
                name: "git_branch".into(),
                description: "Create, list, or delete branches in a git repository.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to the git repository (defaults to current directory)."
                        },
                        "name": {
                            "type": "string",
                            "description": "Branch name (required for create/delete)."
                        },
                        "delete": {
                            "type": "boolean",
                            "description": "If true, delete the named branch."
                        },
                        "start_point": {
                            "type": "string",
                            "description": "Ref or commit to base the new branch on (default HEAD)."
                        }
                    }
                }),
            },
            Box::new(GitBranchTool),
        )
        .expect("git_branch already registered");
}

// ── git_checkout ──────────────────────────────────────────────────────────────

pub struct GitCheckoutTool;

impl ToolHandler for GitCheckoutTool {
    fn invoke(
        &self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, CoreError>> + Send>> {
        Box::pin(async move {
            let path = repo_path(&args);
            let branch: Option<String> = args["branch"].as_str().map(str::to_string);
            let create = args["create"].as_bool().unwrap_or(false);
            let files: Option<Vec<String>> = args["files"].as_array().map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            });
            tokio::task::spawn_blocking(move || {
                git_checkout_impl(&path, branch.as_deref(), create, files.as_deref())
            })
            .await
            .map_err(|e| CoreError::Tool(format!("git_checkout task panicked: {e}")))?
        })
    }
}

fn git_checkout_impl(
    path: &Path,
    branch: Option<&str>,
    create: bool,
    files: Option<&[String]>,
) -> Result<Value, CoreError> {
    use gix::refs::transaction::{Change, LogChange, PreviousValue, RefEdit};

    let repo = open_repo(path)?;

    // Restore specific files from HEAD to worktree.
    if let Some(files) = files {
        return restore_files_from_head(&repo, files);
    }

    let branch_name = branch
        .ok_or_else(|| CoreError::Tool("git_checkout: 'branch' or 'files' is required".into()))?;

    let full_ref = format!("refs/heads/{branch_name}");

    if create {
        // Create the branch at HEAD first.
        let head_id = repo
            .head_id()
            .map(|id| id.detach())
            .map_err(|e| CoreError::Tool(format!("cannot resolve HEAD: {e}")))?;

        repo.edit_reference(RefEdit {
            change: Change::Update {
                log: LogChange::default(),
                expected: PreviousValue::MustNotExist,
                new: gix::refs::Target::Object(head_id),
            },
            name: full_ref
                .as_str()
                .try_into()
                .map_err(|e| CoreError::Tool(format!("invalid branch name: {e}")))?,
            deref: false,
        })
        .map_err(|e| CoreError::Tool(format!("failed to create branch '{branch_name}': {e}")))?;
    } else {
        // Verify the branch exists.
        repo.find_reference(full_ref.as_str())
            .map_err(|_| CoreError::Tool(format!("branch '{branch_name}' does not exist")))?;
    }

    // Resolve the branch's commit and tree.
    let branch_commit_id = resolve_ref(&repo, &full_ref)?;
    let branch_commit = repo
        .find_commit(branch_commit_id)
        .map_err(|e| CoreError::Tool(format!("failed to read commit for '{branch_name}': {e}")))?;
    let branch_tree = branch_commit
        .tree()
        .map_err(|e| CoreError::Tool(format!("failed to get tree for '{branch_name}': {e}")))?;

    // Collect all blob entries from the target tree.
    let mut target_entries: std::collections::HashMap<String, gix::ObjectId> =
        std::collections::HashMap::new();
    collect_tree_entries(&repo, &branch_tree, "", &mut target_entries)?;

    let workdir = repo
        .workdir()
        .ok_or_else(|| CoreError::Tool("bare repository has no working tree".into()))?
        .to_owned();

    // Write all target tree files to the worktree.
    write_tree_to_worktree(&repo, &workdir, &target_entries)?;

    // Update the index to match the target tree.
    update_index_from_tree(&repo, &target_entries)?;

    // Update HEAD to point to the new branch (symbolic ref).
    repo.edit_reference(RefEdit {
        change: Change::Update {
            log: LogChange::default(),
            expected: PreviousValue::Any,
            new: gix::refs::Target::Symbolic(
                full_ref
                    .as_str()
                    .try_into()
                    .map_err(|e| CoreError::Tool(format!("invalid ref name: {e}")))?,
            ),
        },
        name: "HEAD"
            .try_into()
            .map_err(|e| CoreError::Tool(format!("invalid HEAD ref: {e}")))?,
        deref: false,
    })
    .map_err(|e| CoreError::Tool(format!("failed to update HEAD: {e}")))?;

    Ok(json!({ "switched_to": branch_name }))
}

/// Write all entries from `tree_entries` (path → blob id) to the worktree.
fn write_tree_to_worktree(
    repo: &gix::Repository,
    workdir: &Path,
    tree_entries: &std::collections::HashMap<String, gix::ObjectId>,
) -> Result<(), CoreError> {
    use gix::prelude::FindExt;

    for (rela_path, blob_id) in tree_entries {
        let dest = workdir.join(rela_path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                CoreError::Tool(format!("failed to create directory for '{rela_path}': {e}"))
            })?;
        }
        let mut buf = Vec::new();
        let blob = repo
            .objects
            .find_blob(blob_id, &mut buf)
            .map_err(|e| CoreError::Tool(format!("failed to read blob for '{rela_path}': {e}")))?;
        std::fs::write(&dest, blob.data).map_err(|e| {
            CoreError::Tool(format!("failed to write '{rela_path}' to worktree: {e}"))
        })?;
    }
    Ok(())
}

/// Rebuild the index to exactly match `tree_entries`.
fn update_index_from_tree(
    repo: &gix::Repository,
    tree_entries: &std::collections::HashMap<String, gix::ObjectId>,
) -> Result<(), CoreError> {
    use gix::index::entry::{Flags, Mode, Stat};

    let workdir = repo
        .workdir()
        .ok_or_else(|| CoreError::Tool("bare repository has no working tree".into()))?
        .to_owned();

    let mut index = gix::index::File::from_state(
        gix::index::State::new(repo.object_hash()),
        repo.git_dir().join("index"),
    );

    for (rela_path, blob_id) in tree_entries {
        let worktree_path = workdir.join(rela_path);
        let fs_meta = gix::index::fs::Metadata::from_path_no_follow(&worktree_path)
            .map_err(|e| CoreError::Tool(format!("stat error for '{rela_path}': {e}")))?;
        let stat = Stat::from_fs(&fs_meta)
            .map_err(|e| CoreError::Tool(format!("stat conversion for '{rela_path}': {e}")))?;

        let rela_bstr: &gix::bstr::BStr = rela_path.as_str().into();
        index.dangerously_push_entry(stat, *blob_id, Flags::empty(), Mode::FILE, rela_bstr);
    }

    index.sort_entries();
    index
        .write(gix::index::write::Options::default())
        .map_err(|e| CoreError::Tool(format!("failed to write index: {e}")))?;

    Ok(())
}

/// Restore specific files from HEAD tree to the worktree.
fn restore_files_from_head(repo: &gix::Repository, files: &[String]) -> Result<Value, CoreError> {
    use gix::prelude::FindExt;

    let workdir = repo
        .workdir()
        .ok_or_else(|| CoreError::Tool("bare repository has no working tree".into()))?
        .to_owned();

    let head_commit = repo
        .head_commit()
        .map_err(|e| CoreError::Tool(format!("cannot resolve HEAD: {e}")))?;
    let head_tree = head_commit
        .tree()
        .map_err(|e| CoreError::Tool(format!("failed to get HEAD tree: {e}")))?;

    let mut head_entries: std::collections::HashMap<String, gix::ObjectId> =
        std::collections::HashMap::new();
    collect_tree_entries(repo, &head_tree, "", &mut head_entries)?;

    let mut restored: Vec<String> = Vec::new();

    for rela_path in files {
        let blob_id = head_entries
            .get(rela_path.as_str())
            .ok_or_else(|| CoreError::Tool(format!("'{rela_path}' not found in HEAD tree")))?;

        let dest = workdir.join(rela_path);
        let mut buf = Vec::new();
        let blob = repo
            .objects
            .find_blob(blob_id, &mut buf)
            .map_err(|e| CoreError::Tool(format!("failed to read blob for '{rela_path}': {e}")))?;
        std::fs::write(&dest, blob.data)
            .map_err(|e| CoreError::Tool(format!("failed to restore '{rela_path}': {e}")))?;
        restored.push(rela_path.clone());
    }

    Ok(json!({ "restored": restored }))
}

pub fn register_git_checkout_tool(registry: &mut ToolRegistry) {
    registry
        .register(
            ToolSpec {
                name: "git_checkout".into(),
                description: "Switch branches or restore working tree files in a git repository."
                    .into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to the git repository (defaults to current directory)."
                        },
                        "branch": {
                            "type": "string",
                            "description": "Branch to switch to."
                        },
                        "create": {
                            "type": "boolean",
                            "description": "If true, create the branch before switching (default false)."
                        },
                        "files": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Restore these specific files from HEAD instead of switching branches."
                        }
                    }
                }),
            },
            Box::new(GitCheckoutTool),
        )
        .expect("git_checkout already registered");
}
