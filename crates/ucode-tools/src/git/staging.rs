use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use serde_json::{Value, json};
use ucode_core::CoreError;

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
