use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use serde_json::{Value, json};
use ucode_core::CoreError;

use super::commit::{collect_tree_entries, resolve_ref};
use super::{open_repo, repo_path};
use crate::registry::{ToolHandler, ToolRegistry, ToolSpec};

// ── merge base ────────────────────────────────────────────────────────────────

/// Find the lowest common ancestor of two commits via BFS.
/// Returns `None` if no common ancestor exists (unrelated histories).
fn find_merge_base(
    repo: &gix::Repository,
    a: gix::ObjectId,
    b: gix::ObjectId,
) -> Result<Option<gix::ObjectId>, CoreError> {
    // Collect all ancestors of `a` (inclusive).
    let mut a_ancestors: HashSet<gix::ObjectId> = HashSet::new();
    let mut queue: VecDeque<gix::ObjectId> = VecDeque::new();
    queue.push_back(a);
    while let Some(id) = queue.pop_front() {
        if a_ancestors.insert(id) {
            let commit = repo
                .find_commit(id)
                .map_err(|e| CoreError::Tool(format!("failed to read commit {id}: {e}")))?;
            for pid in commit.parent_ids() {
                queue.push_back(pid.detach());
            }
        }
    }

    // Walk ancestors of `b`; first one in `a_ancestors` is the merge base.
    let mut visited: HashSet<gix::ObjectId> = HashSet::new();
    queue.push_back(b);
    while let Some(id) = queue.pop_front() {
        if !visited.insert(id) {
            continue;
        }
        if a_ancestors.contains(&id) {
            return Ok(Some(id));
        }
        let commit = repo
            .find_commit(id)
            .map_err(|e| CoreError::Tool(format!("failed to read commit {id}: {e}")))?;
        for pid in commit.parent_ids() {
            queue.push_back(pid.detach());
        }
    }
    Ok(None)
}

/// Returns true if `ancestor` is reachable from `descendant`.
fn is_ancestor(
    repo: &gix::Repository,
    ancestor: gix::ObjectId,
    descendant: gix::ObjectId,
) -> Result<bool, CoreError> {
    let mut visited: HashSet<gix::ObjectId> = HashSet::new();
    let mut queue: VecDeque<gix::ObjectId> = VecDeque::new();
    queue.push_back(descendant);
    while let Some(id) = queue.pop_front() {
        if !visited.insert(id) {
            continue;
        }
        if id == ancestor {
            return Ok(true);
        }
        let commit = repo
            .find_commit(id)
            .map_err(|e| CoreError::Tool(format!("failed to read commit {id}: {e}")))?;
        for pid in commit.parent_ids() {
            queue.push_back(pid.detach());
        }
    }
    Ok(false)
}

// ── three-way merge ───────────────────────────────────────────────────────────

/// Result of a three-way merge.
enum MergeResult {
    Clean(HashMap<String, MergeFile>),
    Conflict(HashMap<String, MergeFile>, Vec<String>),
}

enum MergeFile {
    /// Use this blob id as the merged content.
    Blob(gix::ObjectId),
    /// Write conflict markers with these two byte strings.
    Conflict { ours: Vec<u8>, theirs: Vec<u8> },
    /// File was deleted in the merge.
    Deleted,
}

/// Perform a three-way merge of base/ours/theirs tree entry maps.
/// Returns the merged file map and a list of conflicting paths.
fn three_way_merge(
    repo: &gix::Repository,
    base: &HashMap<String, gix::ObjectId>,
    ours: &HashMap<String, gix::ObjectId>,
    theirs: &HashMap<String, gix::ObjectId>,
) -> Result<MergeResult, CoreError> {
    let mut all_paths: HashSet<&str> = HashSet::new();
    for k in base.keys() {
        all_paths.insert(k.as_str());
    }
    for k in ours.keys() {
        all_paths.insert(k.as_str());
    }
    for k in theirs.keys() {
        all_paths.insert(k.as_str());
    }

    let mut merged: HashMap<String, MergeFile> = HashMap::new();
    let mut conflicts: Vec<String> = Vec::new();

    for path in all_paths {
        let base_id = base.get(path).copied();
        let our_id = ours.get(path).copied();
        let their_id = theirs.get(path).copied();

        let file = match (base_id, our_id, their_id) {
            // Both sides deleted — skip.
            (_, None, None) => continue,

            // Only ours — keep ours.
            (None, Some(oid), None) => MergeFile::Blob(oid),

            // Only theirs — take theirs.
            (None, None, Some(tid)) => MergeFile::Blob(tid),

            // Both added the same content — take either.
            (None, Some(oid), Some(tid)) if oid == tid => MergeFile::Blob(oid),

            // Both added different content — conflict.
            (None, Some(oid), Some(tid)) => {
                conflicts.push(path.to_string());
                let ours_data = read_blob(repo, oid)?;
                let theirs_data = read_blob(repo, tid)?;
                MergeFile::Conflict {
                    ours: ours_data,
                    theirs: theirs_data,
                }
            }

            // Base had it, ours deleted, theirs unchanged — delete.
            (Some(bid), None, Some(tid)) if bid == tid => MergeFile::Deleted,

            // Base had it, ours deleted, theirs changed — conflict.
            (Some(_), None, Some(tid)) => {
                conflicts.push(path.to_string());
                let theirs_data = read_blob(repo, tid)?;
                MergeFile::Conflict {
                    ours: Vec::new(),
                    theirs: theirs_data,
                }
            }

            // Base had it, theirs deleted, ours unchanged — delete.
            (Some(bid), Some(oid), None) if bid == oid => MergeFile::Deleted,

            // Base had it, theirs deleted, ours changed — conflict.
            (Some(_), Some(oid), None) => {
                conflicts.push(path.to_string());
                let ours_data = read_blob(repo, oid)?;
                MergeFile::Conflict {
                    ours: ours_data,
                    theirs: Vec::new(),
                }
            }

            // All three present.
            (Some(bid), Some(oid), Some(tid)) => {
                if oid == tid {
                    // Both sides made the same change — take either.
                    MergeFile::Blob(oid)
                } else if oid == bid {
                    // Only theirs changed — take theirs.
                    MergeFile::Blob(tid)
                } else if tid == bid {
                    // Only ours changed — take ours.
                    MergeFile::Blob(oid)
                } else {
                    // Both sides changed differently — conflict.
                    conflicts.push(path.to_string());
                    let ours_data = read_blob(repo, oid)?;
                    let theirs_data = read_blob(repo, tid)?;
                    MergeFile::Conflict {
                        ours: ours_data,
                        theirs: theirs_data,
                    }
                }
            }
        };
        merged.insert(path.to_string(), file);
    }

    if conflicts.is_empty() {
        Ok(MergeResult::Clean(merged))
    } else {
        Ok(MergeResult::Conflict(merged, conflicts))
    }
}

fn read_blob(repo: &gix::Repository, id: gix::ObjectId) -> Result<Vec<u8>, CoreError> {
    use gix::prelude::FindExt;
    let mut buf = Vec::new();
    let blob = repo
        .objects
        .find_blob(&id, &mut buf)
        .map_err(|e| CoreError::Tool(format!("failed to read blob {id}: {e}")))?;
    Ok(blob.data.to_vec())
}

/// Write conflict markers for a file.
fn conflict_content(ours: &[u8], theirs: &[u8], branch_name: &str) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"<<<<<<< HEAD\n");
    out.extend_from_slice(ours);
    if !ours.is_empty() && !ours.ends_with(b"\n") {
        out.push(b'\n');
    }
    out.extend_from_slice(b"=======\n");
    out.extend_from_slice(theirs);
    if !theirs.is_empty() && !theirs.ends_with(b"\n") {
        out.push(b'\n');
    }
    out.extend_from_slice(format!(">>>>>>> {branch_name}\n").as_bytes());
    out
}

/// Write merged files to the worktree and return the new blob ids for clean files.
/// For conflict files, writes conflict markers to disk.
fn apply_merge_to_worktree(
    repo: &gix::Repository,
    workdir: &Path,
    merged: &HashMap<String, MergeFile>,
    branch_name: &str,
) -> Result<HashMap<String, gix::ObjectId>, CoreError> {
    use gix::prelude::Write;

    let mut result: HashMap<String, gix::ObjectId> = HashMap::new();

    for (path, file) in merged {
        let dest = workdir.join(path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CoreError::Tool(format!("failed to create dir for '{path}': {e}")))?;
        }
        match file {
            MergeFile::Blob(id) => {
                let data = read_blob(repo, *id)?;
                std::fs::write(&dest, &data)
                    .map_err(|e| CoreError::Tool(format!("failed to write '{path}': {e}")))?;
                result.insert(path.clone(), *id);
            }
            MergeFile::Conflict { ours, theirs } => {
                let content = conflict_content(ours, theirs, branch_name);
                std::fs::write(&dest, &content).map_err(|e| {
                    CoreError::Tool(format!("failed to write conflict '{path}': {e}"))
                })?;
                // Write the conflict content as a blob so it can be staged.
                let blob_id = repo
                    .objects
                    .write_buf(gix::object::Kind::Blob, &content)
                    .map_err(|e| CoreError::Tool(format!("failed to write conflict blob: {e}")))?;
                result.insert(path.clone(), blob_id);
            }
            MergeFile::Deleted => {
                // Remove from worktree if it exists.
                if dest.exists() {
                    std::fs::remove_file(&dest)
                        .map_err(|e| CoreError::Tool(format!("failed to remove '{path}': {e}")))?;
                }
            }
        }
    }
    Ok(result)
}

/// Build a tree object from a flat path→blob-id map.
fn write_tree_from_map(
    repo: &gix::Repository,
    entries: &HashMap<String, gix::ObjectId>,
) -> Result<gix::ObjectId, CoreError> {
    use gix::bstr::ByteSlice;
    use gix::objs::Tree;
    use std::collections::BTreeMap;

    type TreeEntry = (Vec<u8>, gix::object::tree::EntryMode, gix::ObjectId);
    let mut dir_entries: BTreeMap<Vec<u8>, Vec<TreeEntry>> = BTreeMap::new();

    for (rela_path, blob_id) in entries {
        let (dir, filename) = match rela_path.as_bytes().rfind_byte(b'/') {
            Some(pos) => (
                rela_path.as_bytes()[..pos].to_vec(),
                rela_path.as_bytes()[pos + 1..].to_vec(),
            ),
            None => (Vec::new(), rela_path.as_bytes().to_vec()),
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

    // Collect all unique directory paths including intermediate ones.
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

/// Update the index to match a flat path→blob-id map.
fn update_index_from_map(
    repo: &gix::Repository,
    entries: &HashMap<String, gix::ObjectId>,
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

    for (rela_path, blob_id) in entries {
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

/// Move the ref that HEAD points to (or HEAD itself if detached) to `target_id`.
fn move_head_to(repo: &gix::Repository, target_id: gix::ObjectId) -> Result<(), CoreError> {
    use gix::refs::transaction::{Change, LogChange, PreviousValue, RefEdit};

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

/// Create a commit with the given tree, parents, and message, updating HEAD atomically.
///
/// Uses `commit_as` which enforces `PreviousValue::MustExistAndMatch(first_parent)`.
/// Only call this when HEAD currently points to `parents[0]`.
fn create_commit(
    repo: &gix::Repository,
    tree_id: gix::ObjectId,
    parents: Vec<gix::ObjectId>,
    message: &str,
) -> Result<gix::ObjectId, CoreError> {
    let now = gix::date::Time::now_local_or_utc();
    let now_str = now.format_or_unix(gix::date::time::Format::Raw);

    let author = repo
        .author()
        .ok_or_else(|| CoreError::Tool("author identity not configured".into()))?
        .map_err(|e| CoreError::Tool(format!("author config error: {e}")))?;
    let committer = repo
        .committer()
        .ok_or_else(|| CoreError::Tool("committer identity not configured".into()))?
        .map_err(|e| CoreError::Tool(format!("committer config error: {e}")))?;

    let committer_time_str: &str = if committer.time.is_empty() {
        now_str.as_str()
    } else {
        committer.time
    };

    let author_name = author.name.to_string();
    let author_email = author.email.to_string();

    let author_sig = gix::actor::SignatureRef {
        name: author_name.as_str().into(),
        email: author_email.as_str().into(),
        time: now_str.as_str(),
    };
    let committer_sig = gix::actor::SignatureRef {
        name: committer.name,
        email: committer.email,
        time: committer_time_str,
    };

    let commit_id = repo
        .commit_as(committer_sig, author_sig, "HEAD", message, tree_id, parents)
        .map_err(|e| CoreError::Tool(format!("failed to create commit: {e}")))?
        .detach();
    Ok(commit_id)
}

/// Write a commit object directly to the object store without touching any refs.
///
/// Use this when the first parent is NOT the current HEAD (e.g. squash replaces
/// the previous commit with a new one whose parents are the grandparents).
/// The caller is responsible for calling `move_head_to` afterwards.
fn write_commit_object(
    repo: &gix::Repository,
    tree_id: gix::ObjectId,
    parents: Vec<gix::ObjectId>,
    message: &str,
) -> Result<gix::ObjectId, CoreError> {
    use gix::prelude::Write;

    let now = gix::date::Time::now_local_or_utc();

    let author = repo
        .author()
        .ok_or_else(|| CoreError::Tool("author identity not configured".into()))?
        .map_err(|e| CoreError::Tool(format!("author config error: {e}")))?;
    let committer = repo
        .committer()
        .ok_or_else(|| CoreError::Tool("committer identity not configured".into()))?
        .map_err(|e| CoreError::Tool(format!("committer config error: {e}")))?;

    let author_name = author.name.to_string();
    let author_email = author.email.to_string();
    let committer_name = committer.name.to_string();
    let committer_email = committer.email.to_string();
    let _ = committer.time; // committer time unused; always use now for rebased commits

    let commit = gix::objs::Commit {
        message: message.into(),
        tree: tree_id,
        author: gix::actor::Signature {
            name: author_name.into(),
            email: author_email.into(),
            time: now,
        },
        committer: gix::actor::Signature {
            name: committer_name.into(),
            email: committer_email.into(),
            time: now,
        },
        encoding: None,
        parents: parents.into(),
        extra_headers: Default::default(),
    };

    let commit_id = repo
        .objects
        .write(&commit)
        .map_err(|e| CoreError::Tool(format!("failed to write commit object: {e}")))?;
    Ok(commit_id)
}

/// Collect tree entries from a commit id.
fn tree_entries_for_commit(
    repo: &gix::Repository,
    commit_id: gix::ObjectId,
) -> Result<HashMap<String, gix::ObjectId>, CoreError> {
    let commit = repo
        .find_commit(commit_id)
        .map_err(|e| CoreError::Tool(format!("failed to read commit {commit_id}: {e}")))?;
    let tree = commit
        .tree()
        .map_err(|e| CoreError::Tool(format!("failed to get tree for {commit_id}: {e}")))?;
    let mut entries = HashMap::new();
    collect_tree_entries(repo, &tree, "", &mut entries)?;
    Ok(entries)
}

/// Collect tree entries from an optional commit id; empty map if None.
fn tree_entries_opt(
    repo: &gix::Repository,
    commit_id: Option<gix::ObjectId>,
) -> Result<HashMap<String, gix::ObjectId>, CoreError> {
    match commit_id {
        Some(id) => tree_entries_for_commit(repo, id),
        None => Ok(HashMap::new()),
    }
}

// ── rebase state ──────────────────────────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize)]
struct RebaseState {
    original_head: String,
    onto: String,
    remaining: Vec<RebaseAction>,
    /// The commit being applied when we stopped (conflict).
    current_commit: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct RebaseAction {
    action: String,
    commit: String,
    message: Option<String>,
}

fn rebase_state_path(repo: &gix::Repository) -> std::path::PathBuf {
    repo.git_dir().join("ucode-rebase-state")
}

fn save_rebase_state(repo: &gix::Repository, state: &RebaseState) -> Result<(), CoreError> {
    let path = rebase_state_path(repo);
    let json = serde_json::to_string(state)
        .map_err(|e| CoreError::Tool(format!("failed to serialize rebase state: {e}")))?;
    std::fs::write(&path, json)
        .map_err(|e| CoreError::Tool(format!("failed to write rebase state: {e}")))?;
    Ok(())
}

fn load_rebase_state(repo: &gix::Repository) -> Result<RebaseState, CoreError> {
    let path = rebase_state_path(repo);
    let json = std::fs::read_to_string(&path)
        .map_err(|e| CoreError::Tool(format!("failed to read rebase state: {e}")))?;
    serde_json::from_str(&json)
        .map_err(|e| CoreError::Tool(format!("failed to parse rebase state: {e}")))
}

fn remove_rebase_state(repo: &gix::Repository) -> Result<(), CoreError> {
    let path = rebase_state_path(repo);
    if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|e| CoreError::Tool(format!("failed to remove rebase state: {e}")))?;
    }
    Ok(())
}

// ── cherry-pick core ──────────────────────────────────────────────────────────

/// Apply a single commit on top of HEAD via three-way merge.
/// Returns `Ok((new_commit_id, conflicts))`.
/// If conflicts is non-empty, the worktree has conflict markers but no commit is made.
fn cherry_pick_onto_head(
    repo: &gix::Repository,
    commit_id: gix::ObjectId,
    message_override: Option<&str>,
) -> Result<(Option<gix::ObjectId>, Vec<String>), CoreError> {
    let commit = repo
        .find_commit(commit_id)
        .map_err(|e| CoreError::Tool(format!("not a commit '{commit_id}': {e}")))?;

    // Parent of the commit being cherry-picked is the base.
    let parent_id = commit.parent_ids().next().map(|id| id.detach());
    let base_entries = tree_entries_opt(repo, parent_id)?;

    // Their tree = the commit being cherry-picked.
    let their_entries = tree_entries_for_commit(repo, commit_id)?;

    // Our tree = current HEAD.
    let head_id = repo
        .head_id()
        .map(|id| id.detach())
        .map_err(|e| CoreError::Tool(format!("cannot resolve HEAD: {e}")))?;
    let our_entries = tree_entries_for_commit(repo, head_id)?;

    let workdir = repo
        .workdir()
        .ok_or_else(|| CoreError::Tool("bare repository has no working tree".into()))?
        .to_owned();

    let commit_msg = commit
        .message_raw()
        .map_err(|e| CoreError::Tool(format!("failed to decode commit message: {e}")))?
        .to_string();
    let message = message_override.unwrap_or(commit_msg.trim());

    match three_way_merge(repo, &base_entries, &our_entries, &their_entries)? {
        MergeResult::Clean(merged) => {
            let blob_map = apply_merge_to_worktree(repo, &workdir, &merged, "cherry-pick")?;
            let tree_id = write_tree_from_map(repo, &blob_map)?;
            update_index_from_map(repo, &blob_map)?;
            let new_id = create_commit(repo, tree_id, vec![head_id], message)?;
            move_head_to(repo, new_id)?;
            Ok((Some(new_id), Vec::new()))
        }
        MergeResult::Conflict(merged, conflicts) => {
            let blob_map = apply_merge_to_worktree(repo, &workdir, &merged, "cherry-pick")?;
            update_index_from_map(repo, &blob_map)?;
            Ok((None, conflicts))
        }
    }
}

// ── git_merge ─────────────────────────────────────────────────────────────────

pub struct GitMergeTool;

impl ToolHandler for GitMergeTool {
    fn invoke(
        &self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, CoreError>> + Send>> {
        Box::pin(async move {
            let path = repo_path(&args);
            let branch = args["branch"]
                .as_str()
                .ok_or_else(|| CoreError::Tool("git_merge: 'branch' is required".into()))?
                .to_string();
            let message: Option<String> = args["message"].as_str().map(str::to_string);
            tokio::task::spawn_blocking(move || git_merge_impl(&path, &branch, message.as_deref()))
                .await
                .map_err(|e| CoreError::Tool(format!("git_merge task panicked: {e}")))?
        })
    }
}

fn git_merge_impl(path: &Path, branch: &str, message: Option<&str>) -> Result<Value, CoreError> {
    let repo = open_repo(path)?;

    let head_id = repo
        .head_id()
        .map(|id| id.detach())
        .map_err(|e| CoreError::Tool(format!("cannot resolve HEAD: {e}")))?;

    // Resolve the branch to merge.
    let their_id = resolve_ref(&repo, branch)
        .or_else(|_| resolve_ref(&repo, &format!("refs/heads/{branch}")))?;

    // Fast-forward: if HEAD is an ancestor of theirs, just move HEAD.
    if is_ancestor(&repo, head_id, their_id)? {
        move_head_to(&repo, their_id)?;
        // Update worktree and index to match the new HEAD.
        let their_entries = tree_entries_for_commit(&repo, their_id)?;
        let workdir = repo
            .workdir()
            .ok_or_else(|| CoreError::Tool("bare repository has no working tree".into()))?
            .to_owned();
        for (rela_path, blob_id) in &their_entries {
            let dest = workdir.join(rela_path);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    CoreError::Tool(format!("failed to create dir for '{rela_path}': {e}"))
                })?;
            }
            let data = read_blob(&repo, *blob_id)?;
            std::fs::write(&dest, &data)
                .map_err(|e| CoreError::Tool(format!("failed to write '{rela_path}': {e}")))?;
        }
        update_index_from_map(&repo, &their_entries)?;
        return Ok(json!({
            "hash": their_id.to_string(),
            "fast_forward": true,
            "conflicts": [],
        }));
    }

    // Find merge base.
    let base_id = find_merge_base(&repo, head_id, their_id)?;
    let base_entries = tree_entries_opt(&repo, base_id)?;
    let our_entries = tree_entries_for_commit(&repo, head_id)?;
    let their_entries = tree_entries_for_commit(&repo, their_id)?;

    let workdir = repo
        .workdir()
        .ok_or_else(|| CoreError::Tool("bare repository has no working tree".into()))?
        .to_owned();

    match three_way_merge(&repo, &base_entries, &our_entries, &their_entries)? {
        MergeResult::Clean(merged) => {
            let blob_map = apply_merge_to_worktree(&repo, &workdir, &merged, branch)?;
            let tree_id = write_tree_from_map(&repo, &blob_map)?;
            update_index_from_map(&repo, &blob_map)?;
            let msg = message
                .map(str::to_string)
                .unwrap_or_else(|| format!("Merge branch '{branch}'"));
            let commit_id = create_commit(&repo, tree_id, vec![head_id, their_id], &msg)?;
            move_head_to(&repo, commit_id)?;
            Ok(json!({
                "hash": commit_id.to_string(),
                "conflicts": [],
            }))
        }
        MergeResult::Conflict(merged, conflicts) => {
            let blob_map = apply_merge_to_worktree(&repo, &workdir, &merged, branch)?;
            update_index_from_map(&repo, &blob_map)?;
            Ok(json!({
                "status": "conflict",
                "conflicts": conflicts,
            }))
        }
    }
}

pub fn register_git_merge_tool(registry: &mut ToolRegistry) {
    registry
        .register(
            ToolSpec {
                name: "git_merge".into(),
                description: "Merge a branch into the current branch using three-way merge."
                    .into(),
                parameters: json!({
                    "type": "object",
                    "required": ["branch"],
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to the git repository (defaults to current directory)."
                        },
                        "branch": {
                            "type": "string",
                            "description": "Branch name to merge into HEAD."
                        },
                        "message": {
                            "type": "string",
                            "description": "Merge commit message (default: 'Merge branch <name>')."
                        }
                    }
                }),
            },
            Box::new(GitMergeTool),
        )
        .expect("git_merge already registered");
}

// ── git_cherry_pick ───────────────────────────────────────────────────────────

pub struct GitCherryPickTool;

impl ToolHandler for GitCherryPickTool {
    fn invoke(
        &self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, CoreError>> + Send>> {
        Box::pin(async move {
            let path = repo_path(&args);
            let commit_ref = args["commit"]
                .as_str()
                .ok_or_else(|| CoreError::Tool("git_cherry_pick: 'commit' is required".into()))?
                .to_string();
            tokio::task::spawn_blocking(move || git_cherry_pick_impl(&path, &commit_ref))
                .await
                .map_err(|e| CoreError::Tool(format!("git_cherry_pick task panicked: {e}")))?
        })
    }
}

fn git_cherry_pick_impl(path: &Path, commit_ref: &str) -> Result<Value, CoreError> {
    let repo = open_repo(path)?;
    let commit_id = resolve_ref(&repo, commit_ref)?;

    match cherry_pick_onto_head(&repo, commit_id, None)? {
        (Some(new_id), _) => Ok(json!({
            "hash": new_id.to_string(),
            "conflicts": [],
        })),
        (None, conflicts) => Ok(json!({
            "status": "conflict",
            "conflicts": conflicts,
        })),
    }
}

pub fn register_git_cherry_pick_tool(registry: &mut ToolRegistry) {
    registry
        .register(
            ToolSpec {
                name: "git_cherry_pick".into(),
                description: "Apply a single commit onto the current branch.".into(),
                parameters: json!({
                    "type": "object",
                    "required": ["commit"],
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to the git repository (defaults to current directory)."
                        },
                        "commit": {
                            "type": "string",
                            "description": "Commit ref or hash to cherry-pick."
                        }
                    }
                }),
            },
            Box::new(GitCherryPickTool),
        )
        .expect("git_cherry_pick already registered");
}

// ── git_rebase ────────────────────────────────────────────────────────────────

pub struct GitRebaseTool;

impl ToolHandler for GitRebaseTool {
    fn invoke(
        &self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, CoreError>> + Send>> {
        Box::pin(async move {
            let path = repo_path(&args);
            let abort = args["abort"].as_bool().unwrap_or(false);
            let continue_rebase = args["continue_rebase"].as_bool().unwrap_or(false);
            let onto: Option<String> = args["onto"].as_str().map(str::to_string);
            let interactive = args["interactive"].as_bool().unwrap_or(false);
            let actions: Option<Vec<Value>> = args["actions"].as_array().cloned();

            tokio::task::spawn_blocking(move || {
                git_rebase_impl(
                    &path,
                    abort,
                    continue_rebase,
                    onto.as_deref(),
                    interactive,
                    actions.as_deref(),
                )
            })
            .await
            .map_err(|e| CoreError::Tool(format!("git_rebase task panicked: {e}")))?
        })
    }
}

fn git_rebase_impl(
    path: &Path,
    abort: bool,
    continue_rebase: bool,
    onto: Option<&str>,
    interactive: bool,
    actions: Option<&[Value]>,
) -> Result<Value, CoreError> {
    let repo = open_repo(path)?;

    // ── abort ──────────────────────────────────────────────────────────────────
    if abort {
        let state = load_rebase_state(&repo)?;
        let original_id = gix::ObjectId::from_hex(state.original_head.as_bytes())
            .map_err(|_| CoreError::Tool("invalid original_head in rebase state".into()))?;
        // Hard reset to original HEAD.
        let orig_entries = tree_entries_for_commit(&repo, original_id)?;
        let workdir = repo
            .workdir()
            .ok_or_else(|| CoreError::Tool("bare repository has no working tree".into()))?
            .to_owned();
        for (rela_path, blob_id) in &orig_entries {
            let dest = workdir.join(rela_path);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    CoreError::Tool(format!("failed to create dir for '{rela_path}': {e}"))
                })?;
            }
            let data = read_blob(&repo, *blob_id)?;
            std::fs::write(&dest, &data)
                .map_err(|e| CoreError::Tool(format!("failed to write '{rela_path}': {e}")))?;
        }
        update_index_from_map(&repo, &orig_entries)?;
        move_head_to(&repo, original_id)?;
        remove_rebase_state(&repo)?;
        return Ok(json!({ "status": "aborted" }));
    }

    // ── continue ───────────────────────────────────────────────────────────────
    if continue_rebase {
        let mut state = load_rebase_state(&repo)?;

        // Commit the current conflict-resolved state.
        let current_commit_str = state
            .current_commit
            .as_deref()
            .ok_or_else(|| CoreError::Tool("no current commit in rebase state".into()))?;
        let current_commit_id = gix::ObjectId::from_hex(current_commit_str.as_bytes())
            .map_err(|_| CoreError::Tool("invalid current_commit in rebase state".into()))?;

        let commit = repo
            .find_commit(current_commit_id)
            .map_err(|e| CoreError::Tool(format!("failed to read commit: {e}")))?;
        let msg = commit
            .message_raw()
            .map_err(|e| CoreError::Tool(format!("failed to decode message: {e}")))?
            .to_string();

        // Build tree from current index.
        let index = repo
            .open_index()
            .map_err(|e| CoreError::Tool(format!("failed to open index: {e}")))?;
        let mut index_entries: HashMap<String, gix::ObjectId> = HashMap::new();
        for entry in index.entries() {
            let rela_path = entry.path(&index);
            let rela_str = std::str::from_utf8(rela_path)
                .map_err(|_| CoreError::Tool("non-UTF-8 path in index".into()))?
                .to_string();
            index_entries.insert(rela_str, entry.id);
        }
        let tree_id = write_tree_from_map(&repo, &index_entries)?;
        let head_id = repo
            .head_id()
            .map(|id| id.detach())
            .map_err(|e| CoreError::Tool(format!("cannot resolve HEAD: {e}")))?;
        let new_id = create_commit(&repo, tree_id, vec![head_id], msg.trim())?;
        move_head_to(&repo, new_id)?;

        state.current_commit = None;
        let remaining = std::mem::take(&mut state.remaining);

        return apply_rebase_actions(&repo, remaining, &mut 1, &state.original_head, &state.onto);
    }

    // ── start rebase ───────────────────────────────────────────────────────────
    let onto_ref = onto.ok_or_else(|| CoreError::Tool("git_rebase: 'onto' is required".into()))?;
    let onto_id = resolve_ref(&repo, onto_ref)
        .or_else(|_| resolve_ref(&repo, &format!("refs/heads/{onto_ref}")))?;

    let original_head = repo
        .head_id()
        .map(|id| id.detach())
        .map_err(|e| CoreError::Tool(format!("cannot resolve HEAD: {e}")))?;

    let rebase_actions: Vec<RebaseAction> = if interactive {
        // Use provided actions list.
        let acts = actions.ok_or_else(|| {
            CoreError::Tool("git_rebase: 'actions' required for interactive rebase".into())
        })?;
        acts.iter()
            .map(|a| {
                Ok(RebaseAction {
                    action: a["action"]
                        .as_str()
                        .ok_or_else(|| CoreError::Tool("action missing 'action' field".into()))?
                        .to_string(),
                    commit: a["commit"]
                        .as_str()
                        .ok_or_else(|| CoreError::Tool("action missing 'commit' field".into()))?
                        .to_string(),
                    message: a["message"].as_str().map(str::to_string),
                })
            })
            .collect::<Result<Vec<_>, CoreError>>()?
    } else {
        // Collect commits on current branch not reachable from onto.
        collect_commits_not_in(&repo, original_head, onto_id)?
            .into_iter()
            .map(|id| RebaseAction {
                action: "pick".to_string(),
                commit: id.to_string(),
                message: None,
            })
            .collect()
    };

    // Move HEAD to onto.
    let onto_entries = tree_entries_for_commit(&repo, onto_id)?;
    let workdir = repo
        .workdir()
        .ok_or_else(|| CoreError::Tool("bare repository has no working tree".into()))?
        .to_owned();
    for (rela_path, blob_id) in &onto_entries {
        let dest = workdir.join(rela_path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                CoreError::Tool(format!("failed to create dir for '{rela_path}': {e}"))
            })?;
        }
        let data = read_blob(&repo, *blob_id)?;
        std::fs::write(&dest, &data)
            .map_err(|e| CoreError::Tool(format!("failed to write '{rela_path}': {e}")))?;
    }
    update_index_from_map(&repo, &onto_entries)?;
    move_head_to(&repo, onto_id)?;

    let mut rebased = 0usize;
    apply_rebase_actions(
        &repo,
        rebase_actions,
        &mut rebased,
        &original_head.to_string(),
        onto_ref,
    )
}

/// Collect commits reachable from `tip` but not from `base`, in topological order
/// (oldest first, so we can replay them in order).
fn collect_commits_not_in(
    repo: &gix::Repository,
    tip: gix::ObjectId,
    base: gix::ObjectId,
) -> Result<Vec<gix::ObjectId>, CoreError> {
    // Collect all ancestors of base (inclusive).
    let mut base_set: HashSet<gix::ObjectId> = HashSet::new();
    let mut queue: VecDeque<gix::ObjectId> = VecDeque::new();
    queue.push_back(base);
    while let Some(id) = queue.pop_front() {
        if base_set.insert(id) {
            let commit = repo
                .find_commit(id)
                .map_err(|e| CoreError::Tool(format!("failed to read commit {id}: {e}")))?;
            for pid in commit.parent_ids() {
                queue.push_back(pid.detach());
            }
        }
    }

    // Walk from tip, collecting commits not in base_set.
    let mut result: Vec<gix::ObjectId> = Vec::new();
    let mut visited: HashSet<gix::ObjectId> = HashSet::new();
    let mut stack: Vec<gix::ObjectId> = vec![tip];
    while let Some(id) = stack.pop() {
        if !visited.insert(id) || base_set.contains(&id) {
            continue;
        }
        result.push(id);
        let commit = repo
            .find_commit(id)
            .map_err(|e| CoreError::Tool(format!("failed to read commit {id}: {e}")))?;
        for pid in commit.parent_ids() {
            stack.push(pid.detach());
        }
    }
    // Reverse so oldest commit is first.
    result.reverse();
    Ok(result)
}

/// Apply a list of rebase actions, returning the final result.
fn apply_rebase_actions(
    repo: &gix::Repository,
    actions: Vec<RebaseAction>,
    rebased: &mut usize,
    original_head: &str,
    onto: &str,
) -> Result<Value, CoreError> {
    let mut squash_message: Option<String> = None;

    for (i, action) in actions.iter().enumerate() {
        let commit_id = resolve_ref(repo, &action.commit)?;

        match action.action.as_str() {
            "drop" => continue,

            "pick" | "reword" => {
                let msg_override = if action.action == "reword" {
                    action.message.as_deref()
                } else {
                    None
                };
                match cherry_pick_onto_head(repo, commit_id, msg_override)? {
                    (Some(_new_id), _) => {
                        *rebased += 1;
                        squash_message = None;
                    }
                    (None, conflicts) => {
                        // Save state and return conflict.
                        let state = RebaseState {
                            original_head: original_head.to_string(),
                            onto: onto.to_string(),
                            remaining: actions[i + 1..].to_vec(),
                            current_commit: Some(action.commit.clone()),
                        };
                        save_rebase_state(repo, &state)?;
                        return Ok(json!({
                            "status": "conflict",
                            "conflicts": conflicts,
                            "current_commit": action.commit,
                        }));
                    }
                }
            }

            "squash" => {
                // Cherry-pick the commit's diff onto HEAD, then amend the previous commit.
                let commit = repo
                    .find_commit(commit_id)
                    .map_err(|e| CoreError::Tool(format!("failed to read commit: {e}")))?;
                let parent_id = commit.parent_ids().next().map(|id| id.detach());
                let base_entries = tree_entries_opt(repo, parent_id)?;
                let their_entries = tree_entries_for_commit(repo, commit_id)?;

                let head_id = repo
                    .head_id()
                    .map(|id| id.detach())
                    .map_err(|e| CoreError::Tool(format!("cannot resolve HEAD: {e}")))?;
                let our_entries = tree_entries_for_commit(repo, head_id)?;

                let workdir = repo
                    .workdir()
                    .ok_or_else(|| CoreError::Tool("bare repository has no working tree".into()))?
                    .to_owned();

                // Get the message for the squashed commit.
                let this_msg = commit
                    .message_raw()
                    .map_err(|e| CoreError::Tool(format!("failed to decode message: {e}")))?
                    .to_string();
                let combined_msg = match &squash_message {
                    Some(prev) => format!("{prev}\n\n{}", this_msg.trim()),
                    None => {
                        // Get previous commit message.
                        let prev_commit = repo.find_commit(head_id).map_err(|e| {
                            CoreError::Tool(format!("failed to read HEAD commit: {e}"))
                        })?;
                        let prev_msg = prev_commit
                            .message_raw()
                            .map_err(|e| CoreError::Tool(format!("failed to decode message: {e}")))?
                            .to_string();
                        format!("{}\n\n{}", prev_msg.trim(), this_msg.trim())
                    }
                };

                match three_way_merge(repo, &base_entries, &our_entries, &their_entries)? {
                    MergeResult::Clean(merged) => {
                        let blob_map = apply_merge_to_worktree(repo, &workdir, &merged, "squash")?;
                        let tree_id = write_tree_from_map(repo, &blob_map)?;
                        update_index_from_map(repo, &blob_map)?;

                        // Get the parent of the previous commit (we're replacing it).
                        let prev_commit = repo.find_commit(head_id).map_err(|e| {
                            CoreError::Tool(format!("failed to read HEAD commit: {e}"))
                        })?;
                        let grandparent: Vec<gix::ObjectId> =
                            prev_commit.parent_ids().map(|id| id.detach()).collect();

                        // write_commit_object bypasses commit_as's PreviousValue check,
                        // which would fail because HEAD points to the pick commit (not grandparent).
                        let new_id =
                            write_commit_object(repo, tree_id, grandparent, &combined_msg)?;
                        move_head_to(repo, new_id)?;
                        squash_message = Some(combined_msg);
                        *rebased += 1;
                    }
                    MergeResult::Conflict(merged, conflicts) => {
                        let blob_map = apply_merge_to_worktree(repo, &workdir, &merged, "squash")?;
                        update_index_from_map(repo, &blob_map)?;
                        let state = RebaseState {
                            original_head: original_head.to_string(),
                            onto: onto.to_string(),
                            remaining: actions[i + 1..].to_vec(),
                            current_commit: Some(action.commit.clone()),
                        };
                        save_rebase_state(repo, &state)?;
                        return Ok(json!({
                            "status": "conflict",
                            "conflicts": conflicts,
                            "current_commit": action.commit,
                        }));
                    }
                }
            }

            other => {
                return Err(CoreError::Tool(format!(
                    "git_rebase: unknown action '{other}'"
                )));
            }
        }

        // Reset squash accumulator if this wasn't a squash.
        if action.action != "squash" {
            squash_message = None;
        }
    }

    remove_rebase_state(repo)?;

    // Sync the worktree to exactly match the final HEAD tree.
    // This removes files from dropped commits that were present in the original worktree.
    sync_worktree_to_head(repo)?;

    Ok(json!({
        "status": "ok",
        "rebased_commits": *rebased,
    }))
}

/// Sync the working tree to exactly match the current HEAD tree.
///
/// Writes all files present in HEAD and removes any worktree files that are
/// not in HEAD. This is needed after rebase to clean up files from dropped
/// commits that were present in the original working tree.
fn sync_worktree_to_head(repo: &gix::Repository) -> Result<(), CoreError> {
    let workdir = repo
        .workdir()
        .ok_or_else(|| CoreError::Tool("bare repository has no working tree".into()))?
        .to_owned();

    let head_id = repo
        .head_id()
        .map(|id| id.detach())
        .map_err(|e| CoreError::Tool(format!("cannot resolve HEAD: {e}")))?;
    let head_entries = tree_entries_for_commit(repo, head_id)?;

    // Write all files from HEAD tree to worktree.
    for (rela_path, blob_id) in &head_entries {
        let dest = workdir.join(rela_path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                CoreError::Tool(format!("failed to create dir for '{rela_path}': {e}"))
            })?;
        }
        let data = read_blob(repo, *blob_id)?;
        std::fs::write(&dest, &data)
            .map_err(|e| CoreError::Tool(format!("failed to write '{rela_path}': {e}")))?;
    }

    // Remove worktree files not in HEAD tree.
    // Walk the worktree and remove any regular files not tracked by HEAD.
    remove_untracked_files(&workdir, &workdir, &head_entries)?;

    Ok(())
}

/// Recursively remove files under `dir` that are not in `tracked`.
/// `root` is the worktree root used to compute relative paths.
fn remove_untracked_files(
    root: &std::path::Path,
    dir: &std::path::Path,
    tracked: &HashMap<String, gix::ObjectId>,
) -> Result<(), CoreError> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| CoreError::Tool(format!("failed to read dir '{}': {e}", dir.display())))?;

    for entry in entries {
        let entry = entry.map_err(|e| CoreError::Tool(format!("failed to read dir entry: {e}")))?;
        let path = entry.path();
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();

        // Skip .git directory.
        if name == ".git" {
            continue;
        }

        let file_type = entry
            .file_type()
            .map_err(|e| CoreError::Tool(format!("failed to get file type: {e}")))?;

        if file_type.is_dir() {
            remove_untracked_files(root, &path, tracked)?;
            // Remove empty directories.
            let _ = std::fs::remove_dir(&path);
        } else if file_type.is_file() {
            let rela = path
                .strip_prefix(root)
                .map_err(|e| CoreError::Tool(format!("path strip error: {e}")))?
                .to_string_lossy()
                .replace('\\', "/");
            if !tracked.contains_key(rela.as_str()) {
                std::fs::remove_file(&path).map_err(|e| {
                    CoreError::Tool(format!("failed to remove '{}': {e}", path.display()))
                })?;
            }
        }
    }
    Ok(())
}

pub fn register_git_rebase_tool(registry: &mut ToolRegistry) {
    registry
        .register(
            ToolSpec {
                name: "git_rebase".into(),
                description: "Replay commits onto a new base (rebase).".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to the git repository (defaults to current directory)."
                        },
                        "onto": {
                            "type": "string",
                            "description": "Branch or commit to rebase onto (required unless abort/continue)."
                        },
                        "branch": {
                            "type": "string",
                            "description": "Branch to rebase (default: current branch)."
                        },
                        "interactive": {
                            "type": "boolean",
                            "description": "If true, use the 'actions' list instead of auto-collecting commits."
                        },
                        "actions": {
                            "type": "array",
                            "description": "Interactive rebase actions.",
                            "items": {
                                "type": "object",
                                "required": ["action", "commit"],
                                "properties": {
                                    "action": {
                                        "type": "string",
                                        "enum": ["pick", "squash", "reword", "drop"]
                                    },
                                    "commit": { "type": "string" },
                                    "message": { "type": "string" }
                                }
                            }
                        },
                        "continue_rebase": {
                            "type": "boolean",
                            "description": "Continue a paused rebase after resolving conflicts."
                        },
                        "abort": {
                            "type": "boolean",
                            "description": "Abort the current rebase and restore original HEAD."
                        }
                    }
                }),
            },
            Box::new(GitRebaseTool),
        )
        .expect("git_rebase already registered");
}
