use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use serde_json::{Value, json};
use ucode_core::CoreError;

use super::commit::{collect_tree_entries, resolve_ref};
use super::{open_repo, repo_path};
use crate::registry::{ToolHandler, ToolRegistry, ToolSpec};

// ── git_add ───────────────────────────────────────────────────────────────────

pub struct GitAddTool;

impl ToolHandler for GitAddTool {
    fn invoke(
        &self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, CoreError>> + Send>> {
        Box::pin(async move {
            let path = repo_path(&args);
            let files: Vec<String> = match args["files"].as_array() {
                Some(arr) => arr
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect(),
                None => {
                    return Err(CoreError::Tool(
                        "git_add: 'files' argument is required".into(),
                    ));
                }
            };
            if files.is_empty() {
                return Err(CoreError::Tool(
                    "git_add: 'files' list must not be empty".into(),
                ));
            }
            tokio::task::spawn_blocking(move || git_add_impl(&path, &files))
                .await
                .map_err(|e| CoreError::Tool(format!("git_add task panicked: {e}")))?
        })
    }
}

fn git_add_impl(path: &Path, files: &[String]) -> Result<Value, CoreError> {
    use gix::index::entry::{Flags, Mode, Stat};
    use gix::prelude::Write;

    let repo = open_repo(path)?;

    let workdir = repo
        .workdir()
        .ok_or_else(|| CoreError::Tool("bare repository has no working tree".into()))?
        .to_owned();

    // Load the current index (or empty if none exists yet).
    let mut index = repo
        .open_index()
        .or_else(|_| {
            // Fresh repo with no index yet — create an empty one.
            Ok::<_, CoreError>(gix::index::File::from_state(
                gix::index::State::new(repo.object_hash()),
                repo.git_dir().join("index"),
            ))
        })
        .map_err(|e| CoreError::Tool(format!("failed to open index: {e}")))?;

    let mut added: Vec<String> = Vec::new();

    for rela_str in files {
        let worktree_path = workdir.join(rela_str);

        // Verify the file exists.
        let metadata = std::fs::metadata(&worktree_path)
            .map_err(|e| CoreError::Tool(format!("cannot stat '{rela_str}': {e}")))?;

        if !metadata.is_file() {
            return Err(CoreError::Tool(format!(
                "'{rela_str}' is not a regular file"
            )));
        }

        // Read file contents and write as a blob.
        let contents = std::fs::read(&worktree_path)
            .map_err(|e| CoreError::Tool(format!("cannot read '{rela_str}': {e}")))?;

        let blob_id = repo
            .objects
            .write_buf(gix::object::Kind::Blob, &contents)
            .map_err(|e| CoreError::Tool(format!("failed to write blob for '{rela_str}': {e}")))?;

        // Build stat from filesystem metadata via gix's own stat type.
        let fs_meta = gix::index::fs::Metadata::from_path_no_follow(&worktree_path)
            .map_err(|e| CoreError::Tool(format!("stat error for '{rela_str}': {e}")))?;
        let stat = Stat::from_fs(&fs_meta)
            .map_err(|e| CoreError::Tool(format!("stat conversion error for '{rela_str}': {e}")))?;

        // Determine mode: executable bit → FILE_EXECUTABLE, otherwise FILE.
        #[cfg(unix)]
        let mode = {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::metadata(&worktree_path)
                .map(|m| m.permissions())
                .unwrap_or_else(|_| std::fs::Permissions::from_mode(0o644));
            if perms.mode() & 0o111 != 0 {
                Mode::FILE_EXECUTABLE
            } else {
                Mode::FILE
            }
        };
        #[cfg(not(unix))]
        let mode = Mode::FILE;

        let rela_bstr: &gix::bstr::BStr = rela_str.as_str().into();

        // Remove any existing entry for this path (all stages) before re-adding.
        index.remove_entries(|_, p, _| p == rela_bstr);

        index.dangerously_push_entry(stat, blob_id, Flags::empty(), mode, rela_bstr);
        added.push(rela_str.clone());
    }

    // Re-sort so binary-search lookups remain valid.
    index.sort_entries();

    // Persist the index to disk.
    index
        .write(gix::index::write::Options::default())
        .map_err(|e| CoreError::Tool(format!("failed to write index: {e}")))?;

    Ok(json!({ "added": added }))
}

// ── git_reset ─────────────────────────────────────────────────────────────────

pub struct GitResetTool;

impl ToolHandler for GitResetTool {
    fn invoke(
        &self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, CoreError>> + Send>> {
        Box::pin(async move {
            let path = repo_path(&args);
            let files: Option<Vec<String>> = args["files"].as_array().map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            });
            let mode: String = args["mode"].as_str().unwrap_or("mixed").to_string();
            let commit: String = args["commit"].as_str().unwrap_or("HEAD").to_string();
            tokio::task::spawn_blocking(move || {
                git_reset_impl(&path, files.as_deref(), &mode, &commit)
            })
            .await
            .map_err(|e| CoreError::Tool(format!("git_reset task panicked: {e}")))?
        })
    }
}

fn git_reset_impl(
    path: &Path,
    files: Option<&[String]>,
    mode: &str,
    commit: &str,
) -> Result<Value, CoreError> {
    use gix::index::entry::{Flags, Mode, Stat};
    use gix::prelude::FindExt;

    let repo = open_repo(path)?;
    let target_id = resolve_ref(&repo, commit)?;

    // Collect tree entries from the target commit.
    let target_commit = repo
        .find_commit(target_id)
        .map_err(|e| CoreError::Tool(format!("not a commit '{commit}': {e}")))?;
    let target_tree = target_commit
        .tree()
        .map_err(|e| CoreError::Tool(format!("failed to get tree for '{commit}': {e}")))?;
    let mut tree_entries: std::collections::HashMap<String, gix::ObjectId> =
        std::collections::HashMap::new();
    collect_tree_entries(&repo, &target_tree, "", &mut tree_entries)?;

    if let Some(files) = files {
        // Unstage specific files: reset their index entries to match the target tree.
        let mut index = repo
            .open_index()
            .map_err(|e| CoreError::Tool(format!("failed to open index: {e}")))?;

        let workdir = repo
            .workdir()
            .ok_or_else(|| CoreError::Tool("bare repository has no working tree".into()))?
            .to_owned();

        let mut unstaged: Vec<String> = Vec::new();

        for rela_str in files {
            let rela_bstr: &gix::bstr::BStr = rela_str.as_str().into();
            // Remove existing index entry.
            index.remove_entries(|_, p, _| p == rela_bstr);

            if let Some(blob_id) = tree_entries.get(rela_str.as_str()) {
                // Re-add from the target tree.
                let worktree_path = workdir.join(rela_str);
                let stat = if worktree_path.exists() {
                    let fs_meta = gix::index::fs::Metadata::from_path_no_follow(&worktree_path)
                        .map_err(|e| {
                            CoreError::Tool(format!("stat error for '{rela_str}': {e}"))
                        })?;
                    Stat::from_fs(&fs_meta).map_err(|e| {
                        CoreError::Tool(format!("stat conversion for '{rela_str}': {e}"))
                    })?
                } else {
                    Stat::default()
                };
                index.dangerously_push_entry(stat, *blob_id, Flags::empty(), Mode::FILE, rela_bstr);
            }
            // If not in tree, the entry is simply removed (deleted file reset).
            unstaged.push(rela_str.clone());
        }

        index.sort_entries();
        index
            .write(gix::index::write::Options::default())
            .map_err(|e| CoreError::Tool(format!("failed to write index: {e}")))?;

        return Ok(json!({
            "reset_to": target_id.to_string(),
            "unstaged": unstaged,
        }));
    }

    // No files: operate on HEAD ref + index + worktree based on mode.
    match mode {
        "soft" => {
            // Move HEAD only.
            move_head_to(&repo, target_id)?;
        }
        "mixed" => {
            // Move HEAD + reset index.
            move_head_to(&repo, target_id)?;
            reset_index_to_tree(&repo, &tree_entries)?;
        }
        "hard" => {
            // Move HEAD + reset index + reset worktree.
            move_head_to(&repo, target_id)?;
            reset_index_to_tree(&repo, &tree_entries)?;

            let workdir = repo
                .workdir()
                .ok_or_else(|| CoreError::Tool("bare repository has no working tree".into()))?
                .to_owned();

            for (rela_path, blob_id) in &tree_entries {
                let dest = workdir.join(rela_path);
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| {
                        CoreError::Tool(format!(
                            "failed to create directory for '{rela_path}': {e}"
                        ))
                    })?;
                }
                let mut buf = Vec::new();
                let blob = repo.objects.find_blob(blob_id, &mut buf).map_err(|e| {
                    CoreError::Tool(format!("blob read error for '{rela_path}': {e}"))
                })?;
                std::fs::write(&dest, blob.data)
                    .map_err(|e| CoreError::Tool(format!("failed to write '{rela_path}': {e}")))?;
            }
        }
        other => {
            return Err(CoreError::Tool(format!(
                "git_reset: unknown mode '{other}', expected soft|mixed|hard"
            )));
        }
    }

    Ok(json!({ "reset_to": target_id.to_string() }))
}

/// Move the ref that HEAD points to (or HEAD itself if detached) to `target_id`.
fn move_head_to(repo: &gix::Repository, target_id: gix::ObjectId) -> Result<(), CoreError> {
    use gix::refs::transaction::{Change, LogChange, PreviousValue, RefEdit};

    // If HEAD is symbolic, update the branch it points to.
    // If detached, update HEAD directly.
    let head_ref_name: String = repo
        .head_name()
        .map_err(|e| CoreError::Tool(format!("failed to read HEAD: {e}")))?
        .map(|n| n.as_bstr().to_string())
        .unwrap_or_else(|| "HEAD".to_string());

    repo.edit_reference(RefEdit {
        change: Change::Update {
            log: LogChange::default(),
            expected: PreviousValue::Any,
            new: gix::refs::Target::Object(target_id),
        },
        name: head_ref_name
            .as_str()
            .try_into()
            .map_err(|e| CoreError::Tool(format!("invalid ref name: {e}")))?,
        deref: false,
    })
    .map_err(|e| CoreError::Tool(format!("failed to move HEAD: {e}")))?;

    Ok(())
}

/// Rebuild the index to exactly match `tree_entries` (path → blob id).
fn reset_index_to_tree(
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
        let stat = if worktree_path.exists() {
            let fs_meta = gix::index::fs::Metadata::from_path_no_follow(&worktree_path)
                .map_err(|e| CoreError::Tool(format!("stat error for '{rela_path}': {e}")))?;
            Stat::from_fs(&fs_meta)
                .map_err(|e| CoreError::Tool(format!("stat conversion for '{rela_path}': {e}")))?
        } else {
            Stat::default()
        };

        let rela_bstr: &gix::bstr::BStr = rela_path.as_str().into();
        index.dangerously_push_entry(stat, *blob_id, Flags::empty(), Mode::FILE, rela_bstr);
    }

    index.sort_entries();
    index
        .write(gix::index::write::Options::default())
        .map_err(|e| CoreError::Tool(format!("failed to write index: {e}")))?;

    Ok(())
}

pub fn register_git_reset_tool(registry: &mut ToolRegistry) {
    registry
        .register(
            ToolSpec {
                name: "git_reset".into(),
                description: "Unstage files or reset HEAD to a commit in a git repository.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to the git repository (defaults to current directory)."
                        },
                        "files": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Specific files to unstage (reset index to HEAD). Omit for full reset."
                        },
                        "mode": {
                            "type": "string",
                            "enum": ["soft", "mixed", "hard"],
                            "description": "Reset mode: soft (HEAD only), mixed (HEAD+index), hard (HEAD+index+worktree). Default: mixed."
                        },
                        "commit": {
                            "type": "string",
                            "description": "Commit to reset to (default HEAD)."
                        }
                    }
                }),
            },
            Box::new(GitResetTool),
        )
        .expect("git_reset already registered");
}

// ── git_restore ───────────────────────────────────────────────────────────────

pub struct GitRestoreTool;

impl ToolHandler for GitRestoreTool {
    fn invoke(
        &self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, CoreError>> + Send>> {
        Box::pin(async move {
            let path = repo_path(&args);
            let files: Vec<String> = match args["files"].as_array() {
                Some(arr) => arr
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect(),
                None => {
                    return Err(CoreError::Tool(
                        "git_restore: 'files' argument is required".into(),
                    ));
                }
            };
            if files.is_empty() {
                return Err(CoreError::Tool(
                    "git_restore: 'files' list must not be empty".into(),
                ));
            }
            let staged = args["staged"].as_bool().unwrap_or(false);
            let source: String = args["source"].as_str().unwrap_or("HEAD").to_string();
            tokio::task::spawn_blocking(move || git_restore_impl(&path, &files, staged, &source))
                .await
                .map_err(|e| CoreError::Tool(format!("git_restore task panicked: {e}")))?
        })
    }
}

fn git_restore_impl(
    path: &Path,
    files: &[String],
    staged: bool,
    source: &str,
) -> Result<Value, CoreError> {
    use gix::index::entry::{Flags, Mode, Stat};
    use gix::prelude::FindExt;

    let repo = open_repo(path)?;

    let workdir = repo
        .workdir()
        .ok_or_else(|| CoreError::Tool("bare repository has no working tree".into()))?
        .to_owned();

    let mut restored: Vec<String> = Vec::new();

    if staged {
        // Restore index entries from the source commit tree.
        let source_id = resolve_ref(&repo, source)?;
        let source_commit = repo
            .find_commit(source_id)
            .map_err(|e| CoreError::Tool(format!("not a commit '{source}': {e}")))?;
        let source_tree = source_commit
            .tree()
            .map_err(|e| CoreError::Tool(format!("failed to get tree for '{source}': {e}")))?;
        let mut tree_entries: std::collections::HashMap<String, gix::ObjectId> =
            std::collections::HashMap::new();
        collect_tree_entries(&repo, &source_tree, "", &mut tree_entries)?;

        let mut index = repo
            .open_index()
            .map_err(|e| CoreError::Tool(format!("failed to open index: {e}")))?;

        for rela_str in files {
            let rela_bstr: &gix::bstr::BStr = rela_str.as_str().into();
            index.remove_entries(|_, p, _| p == rela_bstr);

            if let Some(blob_id) = tree_entries.get(rela_str.as_str()) {
                let worktree_path = workdir.join(rela_str);
                let stat = if worktree_path.exists() {
                    let fs_meta = gix::index::fs::Metadata::from_path_no_follow(&worktree_path)
                        .map_err(|e| {
                            CoreError::Tool(format!("stat error for '{rela_str}': {e}"))
                        })?;
                    Stat::from_fs(&fs_meta).map_err(|e| {
                        CoreError::Tool(format!("stat conversion for '{rela_str}': {e}"))
                    })?
                } else {
                    Stat::default()
                };
                index.dangerously_push_entry(stat, *blob_id, Flags::empty(), Mode::FILE, rela_bstr);
            }
            restored.push(rela_str.clone());
        }

        index.sort_entries();
        index
            .write(gix::index::write::Options::default())
            .map_err(|e| CoreError::Tool(format!("failed to write index: {e}")))?;
    } else {
        // Restore worktree files from the index.
        let index = repo
            .open_index()
            .map_err(|e| CoreError::Tool(format!("failed to open index: {e}")))?;

        for rela_str in files {
            let rela_bstr: &gix::bstr::BStr = rela_str.as_str().into();

            let entry = index
                .entry_by_path(rela_bstr)
                .ok_or_else(|| CoreError::Tool(format!("'{rela_str}' not in index")))?;

            let blob_id = entry.id;
            let dest = workdir.join(rela_str);
            let mut buf = Vec::new();
            let blob = repo.objects.find_blob(&blob_id, &mut buf).map_err(|e| {
                CoreError::Tool(format!("failed to read blob for '{rela_str}': {e}"))
            })?;
            std::fs::write(&dest, blob.data)
                .map_err(|e| CoreError::Tool(format!("failed to restore '{rela_str}': {e}")))?;
            restored.push(rela_str.clone());
        }
    }

    Ok(json!({ "restored": restored }))
}

pub fn register_git_restore_tool(registry: &mut ToolRegistry) {
    registry
        .register(
            ToolSpec {
                name: "git_restore".into(),
                description: "Discard working tree or staged changes in a git repository.".into(),
                parameters: json!({
                    "type": "object",
                    "required": ["files"],
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to the git repository (defaults to current directory)."
                        },
                        "files": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Files to restore."
                        },
                        "staged": {
                            "type": "boolean",
                            "description": "If true, restore index from source commit (unstage). Default false (restore worktree from index)."
                        },
                        "source": {
                            "type": "string",
                            "description": "Source commit for --staged mode (default HEAD)."
                        }
                    }
                }),
            },
            Box::new(GitRestoreTool),
        )
        .expect("git_restore already registered");
}

pub fn register_git_add_tool(registry: &mut ToolRegistry) {
    registry
        .register(
            ToolSpec {
                name: "git_add".into(),
                description: "Stage files for commit in a git repository.".into(),
                parameters: json!({
                    "type": "object",
                    "required": ["files"],
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to the git repository (defaults to current directory)."
                        },
                        "files": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "List of repository-relative file paths to stage."
                        }
                    }
                }),
            },
            Box::new(GitAddTool),
        )
        .expect("git_add already registered");
}
