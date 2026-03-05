use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use serde_json::{Value, json};
use ucode_core::CoreError;

use super::commit::collect_tree_entries;
use super::{open_repo, repo_path};
use crate::registry::{ToolHandler, ToolRegistry, ToolSpec};

// ── stash stack file ──────────────────────────────────────────────────────────
//
// `.git/ucode-stash-stack` holds one JSON line per entry, newest first:
//   {"commit":"<hex>","message":"<msg>"}
//
// This avoids the complexity of gix reflog manipulation while remaining
// fully self-contained within the repository.

#[derive(serde::Serialize, serde::Deserialize)]
struct StashEntry {
    commit: String,
    message: String,
}

fn stash_stack_path(git_dir: &Path) -> std::path::PathBuf {
    git_dir.join("ucode-stash-stack")
}

fn read_stack(git_dir: &Path) -> Result<Vec<StashEntry>, CoreError> {
    let path = stash_stack_path(git_dir);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| CoreError::Tool(format!("failed to read stash stack: {e}")))?;
    raw.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            serde_json::from_str::<StashEntry>(line)
                .map_err(|e| CoreError::Tool(format!("corrupt stash stack entry: {e}")))
        })
        .collect()
}

fn write_stack(git_dir: &Path, entries: &[StashEntry]) -> Result<(), CoreError> {
    let path = stash_stack_path(git_dir);
    let mut out = String::new();
    for e in entries {
        let line = serde_json::to_string(e)
            .map_err(|e| CoreError::Tool(format!("failed to serialize stash entry: {e}")))?;
        out.push_str(&line);
        out.push('\n');
    }
    std::fs::write(&path, out)
        .map_err(|e| CoreError::Tool(format!("failed to write stash stack: {e}")))
}

// ── push ──────────────────────────────────────────────────────────────────────

fn stash_push(path: &Path, message: Option<&str>) -> Result<Value, CoreError> {
    use gix::index::entry::{Flags, Mode, Stat};
    use gix::prelude::{FindExt, Write};
    use gix::refs::transaction::{Change, PreviousValue, RefEdit};

    let repo = open_repo(path)?;

    let workdir = repo
        .workdir()
        .ok_or_else(|| CoreError::Tool("bare repository has no working tree".into()))?
        .to_owned();

    // HEAD must exist — stash requires at least one commit.
    let head_id = repo
        .head_id()
        .map(|id| id.detach())
        .map_err(|_| CoreError::Tool("git_stash push: repository has no commits".into()))?;

    // ── 1. Build index tree from current index ────────────────────────────────
    let index = repo
        .open_index()
        .map_err(|e| CoreError::Tool(format!("failed to open index: {e}")))?;

    let index_tree_id = write_tree_from_index(&repo, &index)?;

    // ── 2. Build worktree tree ────────────────────────────────────────────────
    // Walk every tracked path (from the index) and hash the on-disk content.
    let worktree_tree_id = {
        // Collect (path, blob_id) for every tracked file using current disk content.
        let mut wt_blobs: Vec<(String, gix::ObjectId)> = Vec::new();

        for entry in index.entries() {
            use gix::bstr::ByteSlice;
            let rela = entry
                .path(&index)
                .to_str()
                .map_err(|_| CoreError::Tool("non-UTF-8 path in index".into()))?
                .to_string();
            let disk_path = workdir.join(&rela);
            if !disk_path.exists() {
                // Deleted in worktree — use the index blob (no change to record).
                wt_blobs.push((rela, entry.id));
                continue;
            }
            let contents = std::fs::read(&disk_path)
                .map_err(|e| CoreError::Tool(format!("cannot read '{rela}': {e}")))?;
            let blob_id = repo
                .objects
                .write_buf(gix::object::Kind::Blob, &contents)
                .map_err(|e| CoreError::Tool(format!("failed to write blob for '{rela}': {e}")))?;
            wt_blobs.push((rela, blob_id));
        }

        // Build a flat tree from these blobs (no subdirectory nesting needed here;
        // we reuse the same recursive builder used by commit.rs).
        build_tree_from_blobs(&repo, &wt_blobs)?
    };

    // ── 3. Create index commit (parent = HEAD) ────────────────────────────────
    let sig = stash_signature(&repo)?;
    let msg = message.unwrap_or("WIP on stash");

    let index_commit_id = repo
        .commit_as(
            sig.as_ref(),
            sig.as_ref(),
            // Don't update any ref — we just need the object.
            "refs/stash-index-tmp",
            format!("index on stash: {msg}"),
            index_tree_id,
            [head_id],
        )
        .map_err(|e| CoreError::Tool(format!("failed to create index commit: {e}")))?
        .detach();

    // ── 4. Create worktree commit (parents = HEAD, index_commit) ─────────────
    let wt_commit_id = repo
        .commit_as(
            sig.as_ref(),
            sig.as_ref(),
            "refs/stash-wt-tmp",
            msg,
            worktree_tree_id,
            [head_id, index_commit_id],
        )
        .map_err(|e| CoreError::Tool(format!("failed to create stash commit: {e}")))?
        .detach();

    // Clean up the temporary refs.
    for tmp_ref in ["refs/stash-index-tmp", "refs/stash-wt-tmp"] {
        let _ = repo.edit_reference(RefEdit {
            change: Change::Delete {
                expected: PreviousValue::Any,
                log: gix::refs::transaction::RefLog::AndReference,
            },
            name: tmp_ref
                .try_into()
                .map_err(|e| CoreError::Tool(format!("invalid ref: {e}")))?,
            deref: false,
        });
    }

    // ── 5. Push entry onto the stack ──────────────────────────────────────────
    let mut stack = read_stack(repo.git_dir())?;
    stack.insert(
        0,
        StashEntry {
            commit: wt_commit_id.to_string(),
            message: msg.to_string(),
        },
    );
    write_stack(repo.git_dir(), &stack)?;

    // ── 6. Reset index and worktree to HEAD ───────────────────────────────────
    let head_commit = repo
        .find_commit(head_id)
        .map_err(|e| CoreError::Tool(format!("failed to find HEAD commit: {e}")))?;
    let head_tree = head_commit
        .tree()
        .map_err(|e| CoreError::Tool(format!("failed to get HEAD tree: {e}")))?;
    let mut head_entries: HashMap<String, gix::ObjectId> = HashMap::new();
    collect_tree_entries(&repo, &head_tree, "", &mut head_entries)?;

    // Reset index.
    let mut new_index = gix::index::File::from_state(
        gix::index::State::new(repo.object_hash()),
        repo.git_dir().join("index"),
    );
    for (rela_path, blob_id) in &head_entries {
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
        new_index.dangerously_push_entry(stat, *blob_id, Flags::empty(), Mode::FILE, rela_bstr);
    }
    new_index.sort_entries();
    new_index
        .write(gix::index::write::Options::default())
        .map_err(|e| CoreError::Tool(format!("failed to write index: {e}")))?;

    // Reset worktree.
    for (rela_path, blob_id) in &head_entries {
        let dest = workdir.join(rela_path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                CoreError::Tool(format!("failed to create dir for '{rela_path}': {e}"))
            })?;
        }
        let mut buf = Vec::new();
        let blob = repo
            .objects
            .find_blob(blob_id, &mut buf)
            .map_err(|e| CoreError::Tool(format!("blob read error for '{rela_path}': {e}")))?;
        std::fs::write(&dest, blob.data)
            .map_err(|e| CoreError::Tool(format!("failed to write '{rela_path}': {e}")))?;
    }

    // Remove tracked files that are not in HEAD (new files that were stashed).
    for entry in index.entries() {
        use gix::bstr::ByteSlice;
        let rela = entry
            .path(&index)
            .to_str()
            .map_err(|_| CoreError::Tool("non-UTF-8 path in index".into()))?
            .to_string();
        if !head_entries.contains_key(&rela) {
            let disk_path = workdir.join(&rela);
            if disk_path.exists() {
                std::fs::remove_file(&disk_path)
                    .map_err(|e| CoreError::Tool(format!("failed to remove '{rela}': {e}")))?;
            }
        }
    }

    Ok(json!({
        "stashed": true,
        "message": msg,
    }))
}

// ── pop ───────────────────────────────────────────────────────────────────────

fn stash_pop(path: &Path, index: Option<usize>) -> Result<Value, CoreError> {
    use gix::prelude::FindExt;

    let repo = open_repo(path)?;

    let workdir = repo
        .workdir()
        .ok_or_else(|| CoreError::Tool("bare repository has no working tree".into()))?
        .to_owned();

    let mut stack = read_stack(repo.git_dir())?;
    if stack.is_empty() {
        return Err(CoreError::Tool("git_stash pop: no stash entries".into()));
    }

    let idx = index.unwrap_or(0);
    if idx >= stack.len() {
        return Err(CoreError::Tool(format!(
            "git_stash pop: index {idx} out of range (stack has {} entries)",
            stack.len()
        )));
    }

    let entry = stack.remove(idx);

    let stash_id = gix::ObjectId::from_hex(entry.commit.as_bytes())
        .map_err(|_| CoreError::Tool(format!("corrupt stash commit id: {}", entry.commit)))?;

    let stash_commit = repo
        .find_commit(stash_id)
        .map_err(|e| CoreError::Tool(format!("failed to find stash commit: {e}")))?;

    let stash_tree = stash_commit
        .tree()
        .map_err(|e| CoreError::Tool(format!("failed to get stash tree: {e}")))?;

    let mut stash_entries: HashMap<String, gix::ObjectId> = HashMap::new();
    collect_tree_entries(&repo, &stash_tree, "", &mut stash_entries)?;

    // Restore worktree files.
    for (rela_path, blob_id) in &stash_entries {
        let dest = workdir.join(rela_path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                CoreError::Tool(format!("failed to create dir for '{rela_path}': {e}"))
            })?;
        }
        let mut buf = Vec::new();
        let blob = repo
            .objects
            .find_blob(blob_id, &mut buf)
            .map_err(|e| CoreError::Tool(format!("blob read error for '{rela_path}': {e}")))?;
        std::fs::write(&dest, blob.data)
            .map_err(|e| CoreError::Tool(format!("failed to restore '{rela_path}': {e}")))?;
    }

    write_stack(repo.git_dir(), &stack)?;

    Ok(json!({ "restored": true }))
}

// ── list ──────────────────────────────────────────────────────────────────────

fn stash_list(path: &Path) -> Result<Value, CoreError> {
    let repo = open_repo(path)?;
    let stack = read_stack(repo.git_dir())?;

    let stashes: Vec<Value> = stack
        .iter()
        .enumerate()
        .map(|(i, e)| json!({ "index": i, "message": e.message }))
        .collect();

    Ok(json!({ "stashes": stashes }))
}

// ── drop ──────────────────────────────────────────────────────────────────────

fn stash_drop(path: &Path, index: Option<usize>) -> Result<Value, CoreError> {
    let repo = open_repo(path)?;
    let mut stack = read_stack(repo.git_dir())?;

    if stack.is_empty() {
        return Err(CoreError::Tool("git_stash drop: no stash entries".into()));
    }

    let idx = index.unwrap_or(0);
    if idx >= stack.len() {
        return Err(CoreError::Tool(format!(
            "git_stash drop: index {idx} out of range (stack has {} entries)",
            stack.len()
        )));
    }

    stack.remove(idx);
    write_stack(repo.git_dir(), &stack)?;

    Ok(json!({ "dropped": true, "index": idx }))
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Build a signature for stash commits using the repo's configured identity.
/// Falls back to a generic identity if none is configured.
fn stash_signature(repo: &gix::Repository) -> Result<OwnedSig, CoreError> {
    let now = gix::date::Time::now_local_or_utc();
    let now_str = now.format_or_unix(gix::date::time::Format::Raw);

    let (name, email) = if let Some(Ok(author)) = repo.author() {
        (author.name.to_string(), author.email.to_string())
    } else {
        ("ucode".to_string(), "ucode@localhost".to_string())
    };

    Ok(OwnedSig {
        name,
        email,
        time: now_str,
    })
}

/// Owned version of `gix::actor::SignatureRef` so we can store the time string.
struct OwnedSig {
    name: String,
    email: String,
    time: String,
}

impl OwnedSig {
    fn as_ref(&self) -> gix::actor::SignatureRef<'_> {
        gix::actor::SignatureRef {
            name: self.name.as_str().into(),
            email: self.email.as_str().into(),
            time: self.time.as_str(),
        }
    }
}

/// Build a flat tree object from a list of (relative-path, blob-id) pairs.
/// Handles subdirectories by recursively constructing subtrees.
fn build_tree_from_blobs(
    repo: &gix::Repository,
    blobs: &[(String, gix::ObjectId)],
) -> Result<gix::ObjectId, CoreError> {
    use gix::bstr::ByteSlice;
    use gix::objs::Tree;
    use std::collections::BTreeMap;

    type TreeEntry = (Vec<u8>, gix::object::tree::EntryMode, gix::ObjectId);
    let mut dir_entries: BTreeMap<Vec<u8>, Vec<TreeEntry>> = BTreeMap::new();

    for (rela, blob_id) in blobs {
        let rela_bytes = rela.as_bytes();
        let (dir, filename) = match rela_bytes.rfind_byte(b'/') {
            Some(pos) => (rela_bytes[..pos].to_vec(), rela_bytes[pos + 1..].to_vec()),
            None => (Vec::new(), rela_bytes.to_vec()),
        };
        dir_entries.entry(dir).or_default().push((
            filename,
            gix::object::tree::EntryKind::Blob.into(),
            *blob_id,
        ));
    }

    if dir_entries.is_empty() {
        let tree = Tree {
            entries: Vec::new(),
        };
        return repo
            .write_object(&tree)
            .map(|id| id.detach())
            .map_err(|e| CoreError::Tool(format!("failed to write empty tree: {e}")));
    }

    let mut all_dirs: Vec<Vec<u8>> = dir_entries.keys().cloned().collect();
    let extra: Vec<Vec<u8>> = all_dirs
        .iter()
        .flat_map(|d| {
            let mut parts: Vec<Vec<u8>> = Vec::new();
            let mut cur = d.as_slice();
            while let Some(pos) = cur.rfind_byte(b'/') {
                cur = &cur[..pos];
                parts.push(cur.to_vec());
            }
            parts
        })
        .collect();
    for d in extra {
        if !all_dirs.contains(&d) {
            all_dirs.push(d);
        }
    }
    all_dirs.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| b.cmp(a)));

    let mut tree_ids: BTreeMap<Vec<u8>, gix::ObjectId> = BTreeMap::new();

    for dir in &all_dirs {
        let mut entries: Vec<gix::objs::tree::Entry> = Vec::new();

        if let Some(files) = dir_entries.get(dir) {
            for (filename, mode, id) in files {
                entries.push(gix::objs::tree::Entry {
                    mode: *mode,
                    filename: filename.as_slice().into(),
                    oid: *id,
                });
            }
        }

        for (child_dir, child_id) in &tree_ids {
            let is_direct_child = if dir.is_empty() {
                !child_dir.is_empty() && !child_dir.contains(&b'/')
            } else {
                child_dir.starts_with(dir.as_slice())
                    && child_dir.len() > dir.len() + 1
                    && child_dir[dir.len()] == b'/'
                    && !child_dir[dir.len() + 1..].contains(&b'/')
            };

            if is_direct_child {
                let subtree_name = if dir.is_empty() {
                    child_dir.clone()
                } else {
                    child_dir[dir.len() + 1..].to_vec()
                };
                entries.push(gix::objs::tree::Entry {
                    mode: gix::object::tree::EntryKind::Tree.into(),
                    filename: subtree_name.as_slice().into(),
                    oid: *child_id,
                });
            }
        }

        entries.sort();
        let tree = Tree { entries };
        let tree_id = repo
            .write_object(&tree)
            .map_err(|e| CoreError::Tool(format!("failed to write tree: {e}")))?
            .detach();
        tree_ids.insert(dir.clone(), tree_id);
    }

    tree_ids
        .get(&Vec::<u8>::new())
        .copied()
        .ok_or_else(|| CoreError::Tool("failed to build root tree".into()))
}

/// Build a tree from the current index (mirrors commit.rs's private helper).
fn write_tree_from_index(
    repo: &gix::Repository,
    index: &gix::index::File,
) -> Result<gix::ObjectId, CoreError> {
    use gix::bstr::ByteSlice;
    use gix::objs::Tree;
    use std::collections::BTreeMap;

    type TreeEntry = (Vec<u8>, gix::object::tree::EntryMode, gix::ObjectId);
    let mut dir_entries: BTreeMap<Vec<u8>, Vec<TreeEntry>> = BTreeMap::new();

    for entry in index.entries() {
        let rela_path = entry.path(index);
        let mode = match entry.mode {
            gix::index::entry::Mode::FILE => gix::object::tree::EntryKind::Blob.into(),
            gix::index::entry::Mode::FILE_EXECUTABLE => {
                gix::object::tree::EntryKind::BlobExecutable.into()
            }
            gix::index::entry::Mode::SYMLINK => gix::object::tree::EntryKind::Link.into(),
            gix::index::entry::Mode::COMMIT => gix::object::tree::EntryKind::Commit.into(),
            _ => continue,
        };

        let (dir, filename) = match rela_path.rfind_byte(b'/') {
            Some(pos) => (rela_path[..pos].to_vec(), rela_path[pos + 1..].to_vec()),
            None => (Vec::new(), rela_path.to_vec()),
        };

        dir_entries
            .entry(dir)
            .or_default()
            .push((filename, mode, entry.id));
    }

    if dir_entries.is_empty() {
        let tree = Tree {
            entries: Vec::new(),
        };
        return repo
            .write_object(&tree)
            .map(|id| id.detach())
            .map_err(|e| CoreError::Tool(format!("failed to write empty tree: {e}")));
    }

    let mut all_dirs: Vec<Vec<u8>> = dir_entries.keys().cloned().collect();
    let extra: Vec<Vec<u8>> = all_dirs
        .iter()
        .flat_map(|d| {
            let mut parts: Vec<Vec<u8>> = Vec::new();
            let mut cur = d.as_slice();
            while let Some(pos) = cur.rfind_byte(b'/') {
                cur = &cur[..pos];
                parts.push(cur.to_vec());
            }
            parts
        })
        .collect();
    for d in extra {
        if !all_dirs.contains(&d) {
            all_dirs.push(d);
        }
    }
    all_dirs.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| b.cmp(a)));

    let mut tree_ids: BTreeMap<Vec<u8>, gix::ObjectId> = BTreeMap::new();

    for dir in &all_dirs {
        let mut entries: Vec<gix::objs::tree::Entry> = Vec::new();

        if let Some(files) = dir_entries.get(dir) {
            for (filename, mode, id) in files {
                entries.push(gix::objs::tree::Entry {
                    mode: *mode,
                    filename: filename.as_slice().into(),
                    oid: *id,
                });
            }
        }

        for (child_dir, child_id) in &tree_ids {
            let is_direct_child = if dir.is_empty() {
                !child_dir.is_empty() && !child_dir.contains(&b'/')
            } else {
                child_dir.starts_with(dir.as_slice())
                    && child_dir.len() > dir.len() + 1
                    && child_dir[dir.len()] == b'/'
                    && !child_dir[dir.len() + 1..].contains(&b'/')
            };

            if is_direct_child {
                let subtree_name = if dir.is_empty() {
                    child_dir.clone()
                } else {
                    child_dir[dir.len() + 1..].to_vec()
                };
                entries.push(gix::objs::tree::Entry {
                    mode: gix::object::tree::EntryKind::Tree.into(),
                    filename: subtree_name.as_slice().into(),
                    oid: *child_id,
                });
            }
        }

        entries.sort();
        let tree = Tree { entries };
        let tree_id = repo
            .write_object(&tree)
            .map_err(|e| CoreError::Tool(format!("failed to write tree: {e}")))?
            .detach();
        tree_ids.insert(dir.clone(), tree_id);
    }

    tree_ids
        .get(&Vec::<u8>::new())
        .copied()
        .ok_or_else(|| CoreError::Tool("failed to build root tree".into()))
}

// ── tool handler ──────────────────────────────────────────────────────────────

pub struct GitStashTool;

impl ToolHandler for GitStashTool {
    fn invoke(
        &self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, CoreError>> + Send>> {
        Box::pin(async move {
            let path = repo_path(&args);
            let action = args["action"]
                .as_str()
                .ok_or_else(|| {
                    CoreError::Tool("git_stash: 'action' is required (push|pop|list|drop)".into())
                })?
                .to_string();
            let message: Option<String> = args["message"].as_str().map(str::to_string);
            let index: Option<usize> = args["index"].as_u64().map(|n| n as usize);

            tokio::task::spawn_blocking(move || match action.as_str() {
                "push" => stash_push(&path, message.as_deref()),
                "pop" => stash_pop(&path, index),
                "list" => stash_list(&path),
                "drop" => stash_drop(&path, index),
                other => Err(CoreError::Tool(format!(
                    "git_stash: unknown action '{other}', expected push|pop|list|drop"
                ))),
            })
            .await
            .map_err(|e| CoreError::Tool(format!("git_stash task panicked: {e}")))?
        })
    }
}

pub fn register_git_stash_tool(registry: &mut ToolRegistry) {
    registry
        .register(
            ToolSpec {
                name: "git_stash".into(),
                description: "Save and restore work in progress in a git repository.".into(),
                parameters: json!({
                    "type": "object",
                    "required": ["action"],
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to the git repository (defaults to current directory)."
                        },
                        "action": {
                            "type": "string",
                            "enum": ["push", "pop", "list", "drop"],
                            "description": "Stash action to perform."
                        },
                        "message": {
                            "type": "string",
                            "description": "Description for the stash entry (push only)."
                        },
                        "index": {
                            "type": "number",
                            "description": "Stash index to pop/drop (default 0, i.e. most recent)."
                        }
                    }
                }),
            },
            Box::new(GitStashTool),
        )
        .expect("git_stash already registered");
}
