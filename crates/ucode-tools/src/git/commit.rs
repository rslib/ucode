use std::collections::BTreeMap;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use serde_json::{Value, json};
use ucode_core::CoreError;

use super::diff::diff_blobs;
use super::{open_repo, repo_path};
use crate::registry::{ToolHandler, ToolRegistry, ToolSpec};

// ── helpers ───────────────────────────────────────────────────────────────────

/// Resolve a ref-like string (branch name, "HEAD", full ref, or hex commit id)
/// to an `ObjectId` without requiring the `revision` feature.
pub(crate) fn resolve_ref(repo: &gix::Repository, rev: &str) -> Result<gix::ObjectId, CoreError> {
    // Try as a full/partial reference name first.
    if let Ok(mut r) = repo.find_reference(rev) {
        return r
            .peel_to_id()
            .map(|id| id.detach())
            .map_err(|e| CoreError::Tool(format!("failed to peel reference '{rev}': {e}")));
    }
    // Try as a hex object id.
    gix::ObjectId::from_hex(rev.as_bytes())
        .map_err(|_| CoreError::Tool(format!("cannot resolve ref or id '{rev}'")))
}

/// Build a tree object from the current index entries and return its id.
fn write_tree_from_index(
    repo: &gix::Repository,
    index: &gix::index::File,
) -> Result<gix::ObjectId, CoreError> {
    use gix::bstr::ByteSlice;
    use gix::objs::Tree;

    // Collect all entries grouped by their directory prefix.
    // Key: directory path bytes (empty = root), Value: list of (filename, mode, id).
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
        // Empty index → write an empty tree.
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
    // Deeper directories first.
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

        // Add immediate child sub-trees.
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

/// Format a `gix::date::Time` as an ISO-8601 UTC string.
fn format_time(t: gix::date::Time) -> String {
    use chrono::{DateTime, TimeZone, Utc};
    let dt: DateTime<Utc> = Utc.timestamp_opt(t.seconds, 0).single().unwrap_or_default();
    dt.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

// ── git_commit ────────────────────────────────────────────────────────────────

pub struct GitCommitTool;

impl ToolHandler for GitCommitTool {
    fn invoke(
        &self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, CoreError>> + Send>> {
        Box::pin(async move {
            let path = repo_path(&args);
            let message = args["message"]
                .as_str()
                .ok_or_else(|| CoreError::Tool("git_commit: 'message' is required".into()))?
                .to_string();
            let author_override: Option<String> = args["author"].as_str().map(str::to_string);
            tokio::task::spawn_blocking(move || {
                git_commit_impl(&path, &message, author_override.as_deref())
            })
            .await
            .map_err(|e| CoreError::Tool(format!("git_commit task panicked: {e}")))?
        })
    }
}

fn git_commit_impl(
    path: &Path,
    message: &str,
    author_override: Option<&str>,
) -> Result<Value, CoreError> {
    let repo = open_repo(path)?;

    let index = repo
        .open_index()
        .map_err(|e| CoreError::Tool(format!("failed to open index: {e}")))?;

    if index.entries().is_empty() {
        return Err(CoreError::Tool("nothing to commit (index is empty)".into()));
    }

    let tree_id = write_tree_from_index(&repo, &index)?;

    let parent: Option<gix::ObjectId> = repo.head_id().ok().map(|id| id.detach());

    let now = gix::date::Time::now_local_or_utc();
    let now_str = now.format_or_unix(gix::date::time::Format::Raw);

    let (author_name, author_email) = if let Some(spec) = author_override {
        parse_author_spec(spec)?
    } else {
        let author = repo
            .author()
            .ok_or_else(|| CoreError::Tool("author identity not configured".into()))?
            .map_err(|e| CoreError::Tool(format!("author config error: {e}")))?;
        (author.name.to_string(), author.email.to_string())
    };

    let committer = repo
        .committer()
        .ok_or_else(|| CoreError::Tool("committer identity not configured".into()))?
        .map_err(|e| CoreError::Tool(format!("committer config error: {e}")))?;

    // committer.time is already a raw git time &str (e.g. "1234567890 +0000").
    // Use it directly; fall back to now_str if it's empty.
    let committer_time_str: &str = if committer.time.is_empty() {
        now_str.as_str()
    } else {
        committer.time
    };

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

    let parents: Vec<gix::ObjectId> = parent.into_iter().collect();

    let commit_id = repo
        .commit_as(committer_sig, author_sig, "HEAD", message, tree_id, parents)
        .map_err(|e| CoreError::Tool(format!("failed to create commit: {e}")))?
        .detach();

    Ok(json!({
        "hash": commit_id.to_string(),
        "message": message,
    }))
}

fn parse_author_spec(spec: &str) -> Result<(String, String), CoreError> {
    let lt = spec.rfind('<').ok_or_else(|| {
        CoreError::Tool(format!(
            "invalid author format '{spec}', expected 'Name <email>'"
        ))
    })?;
    let gt = spec.rfind('>').ok_or_else(|| {
        CoreError::Tool(format!(
            "invalid author format '{spec}', expected 'Name <email>'"
        ))
    })?;
    if gt < lt {
        return Err(CoreError::Tool(format!("invalid author format '{spec}'")));
    }
    let name = spec[..lt].trim().to_string();
    let email = spec[lt + 1..gt].trim().to_string();
    Ok((name, email))
}

pub fn register_git_commit_tool(registry: &mut ToolRegistry) {
    registry
        .register(
            ToolSpec {
                name: "git_commit".into(),
                description: "Create a commit from staged changes in a git repository.".into(),
                parameters: json!({
                    "type": "object",
                    "required": ["message"],
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to the git repository (defaults to current directory)."
                        },
                        "message": {
                            "type": "string",
                            "description": "Commit message."
                        },
                        "author": {
                            "type": "string",
                            "description": "Author in 'Name <email>' format. Defaults to repo config."
                        }
                    }
                }),
            },
            Box::new(GitCommitTool),
        )
        .expect("git_commit already registered");
}

// ── git_log ───────────────────────────────────────────────────────────────────

pub struct GitLogTool;

impl ToolHandler for GitLogTool {
    fn invoke(
        &self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, CoreError>> + Send>> {
        Box::pin(async move {
            let path = repo_path(&args);
            let max_count = args["max_count"].as_u64().unwrap_or(10) as usize;
            let rev: String = args["rev"].as_str().unwrap_or("HEAD").to_string();
            tokio::task::spawn_blocking(move || git_log_impl(&path, max_count, &rev))
                .await
                .map_err(|e| CoreError::Tool(format!("git_log task panicked: {e}")))?
        })
    }
}

fn git_log_impl(path: &Path, max_count: usize, rev: &str) -> Result<Value, CoreError> {
    let repo = open_repo(path)?;

    let tip_id = resolve_ref(&repo, rev)?;

    let mut commits: Vec<Value> = Vec::new();

    let walk = repo
        .rev_walk([tip_id])
        .all()
        .map_err(|e| CoreError::Tool(format!("failed to start rev walk: {e}")))?;

    for info in walk.take(max_count) {
        let info = info.map_err(|e| CoreError::Tool(format!("rev walk error: {e}")))?;
        let commit = info
            .object()
            .map_err(|e| CoreError::Tool(format!("failed to read commit: {e}")))?;

        let author = commit
            .author()
            .map_err(|e| CoreError::Tool(format!("failed to decode author: {e}")))?;
        let time = commit
            .time()
            .map_err(|e| CoreError::Tool(format!("failed to decode time: {e}")))?;
        let message = commit
            .message_raw()
            .map_err(|e| CoreError::Tool(format!("failed to decode message: {e}")))?
            .to_string();

        commits.push(json!({
            "hash": info.id.to_string(),
            "author": format!("{} <{}>", author.name, author.email),
            "date": format_time(time),
            "message": message.trim(),
        }));
    }

    Ok(json!({ "commits": commits }))
}

pub fn register_git_log_tool(registry: &mut ToolRegistry) {
    registry
        .register(
            ToolSpec {
                name: "git_log".into(),
                description: "Walk commit history in a git repository.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to the git repository (defaults to current directory)."
                        },
                        "max_count": {
                            "type": "number",
                            "description": "Maximum number of commits to return (default 10)."
                        },
                        "rev": {
                            "type": "string",
                            "description": "Starting revision (default HEAD)."
                        }
                    }
                }),
            },
            Box::new(GitLogTool),
        )
        .expect("git_log already registered");
}

// ── git_show ──────────────────────────────────────────────────────────────────

pub struct GitShowTool;

impl ToolHandler for GitShowTool {
    fn invoke(
        &self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, CoreError>> + Send>> {
        Box::pin(async move {
            let path = repo_path(&args);
            let commit_ref = args["commit"]
                .as_str()
                .ok_or_else(|| CoreError::Tool("git_show: 'commit' is required".into()))?
                .to_string();
            tokio::task::spawn_blocking(move || git_show_impl(&path, &commit_ref))
                .await
                .map_err(|e| CoreError::Tool(format!("git_show task panicked: {e}")))?
        })
    }
}

fn git_show_impl(path: &Path, commit_ref: &str) -> Result<Value, CoreError> {
    let repo = open_repo(path)?;

    let commit_id = resolve_ref(&repo, commit_ref)?;
    let commit = repo
        .find_commit(commit_id)
        .map_err(|e| CoreError::Tool(format!("not a commit '{commit_ref}': {e}")))?;

    let author = commit
        .author()
        .map_err(|e| CoreError::Tool(format!("failed to decode author: {e}")))?;
    let time = commit
        .time()
        .map_err(|e| CoreError::Tool(format!("failed to decode time: {e}")))?;
    let message = commit
        .message_raw()
        .map_err(|e| CoreError::Tool(format!("failed to decode message: {e}")))?
        .to_string();

    let this_tree = commit
        .tree()
        .map_err(|e| CoreError::Tool(format!("failed to get tree: {e}")))?;

    let parent_tree = commit
        .parent_ids()
        .next()
        .and_then(|pid| repo.find_commit(pid).ok())
        .and_then(|pc| pc.tree().ok());

    let mut parent_entries: std::collections::HashMap<String, gix::ObjectId> =
        std::collections::HashMap::new();
    if let Some(ref pt) = parent_tree {
        collect_tree_entries(&repo, pt, "", &mut parent_entries)?;
    }

    let mut this_entries: std::collections::HashMap<String, gix::ObjectId> =
        std::collections::HashMap::new();
    collect_tree_entries(&repo, &this_tree, "", &mut this_entries)?;

    let mut all_paths: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    all_paths.extend(parent_entries.keys().cloned());
    all_paths.extend(this_entries.keys().cloned());

    let mut diff_output = String::new();
    let mut buf_before = Vec::new();
    let mut buf_after = Vec::new();

    for file_path in &all_paths {
        let before_id = parent_entries.get(file_path).copied();
        let after_id = this_entries.get(file_path).copied();
        diff_entry(&mut DiffEntryCtx {
            repo: &repo,
            file_path,
            before_id,
            after_id,
            buf_before: &mut buf_before,
            buf_after: &mut buf_after,
            out: &mut diff_output,
        })?;
    }

    Ok(json!({
        "hash": commit_id.to_string(),
        "author": format!("{} <{}>", author.name, author.email),
        "date": format_time(time),
        "message": message.trim(),
        "diff": diff_output,
    }))
}

/// Context for diffing a single file entry.
pub(crate) struct DiffEntryCtx<'a> {
    pub repo: &'a gix::Repository,
    pub file_path: &'a str,
    pub before_id: Option<gix::ObjectId>,
    pub after_id: Option<gix::ObjectId>,
    pub buf_before: &'a mut Vec<u8>,
    pub buf_after: &'a mut Vec<u8>,
    pub out: &'a mut String,
}

/// Diff a single file between `before_id` and `after_id`, appending to `out`.
pub(crate) fn diff_entry(ctx: &mut DiffEntryCtx<'_>) -> Result<(), CoreError> {
    use gix::prelude::FindExt;

    match (ctx.before_id, ctx.after_id) {
        (None, Some(aid)) => {
            ctx.buf_after.clear();
            let blob = ctx
                .repo
                .objects
                .find_blob(&aid, ctx.buf_after)
                .map_err(|e| CoreError::Tool(format!("blob read error: {e}")))?;
            let hunk = diff_blobs(ctx.file_path, b"", blob.data)?;
            ctx.out.push_str(&hunk);
        }
        (Some(bid), None) => {
            ctx.buf_before.clear();
            let blob = ctx
                .repo
                .objects
                .find_blob(&bid, ctx.buf_before)
                .map_err(|e| CoreError::Tool(format!("blob read error: {e}")))?;
            let hunk = diff_blobs(ctx.file_path, blob.data, b"")?;
            ctx.out.push_str(&hunk);
        }
        (Some(bid), Some(aid)) if bid != aid => {
            ctx.buf_before.clear();
            let before_blob = ctx
                .repo
                .objects
                .find_blob(&bid, ctx.buf_before)
                .map_err(|e| CoreError::Tool(format!("blob read error: {e}")))?;
            let before_data = before_blob.data.to_vec();
            ctx.buf_after.clear();
            let after_blob = ctx
                .repo
                .objects
                .find_blob(&aid, ctx.buf_after)
                .map_err(|e| CoreError::Tool(format!("blob read error: {e}")))?;
            let hunk = diff_blobs(ctx.file_path, &before_data, after_blob.data)?;
            ctx.out.push_str(&hunk);
        }
        _ => {}
    }
    Ok(())
}

/// Recursively collect all blob entries from a tree into `out` (path → blob id).
pub(crate) fn collect_tree_entries(
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
                let mut sub_buf = Vec::new();
                let sub_obj = repo
                    .objects
                    .find(entry.oid, &mut sub_buf)
                    .map_err(|e| CoreError::Tool(format!("failed to find subtree: {e}")))?;
                let sub_tree = gix::Tree {
                    id: entry.oid.to_owned(),
                    data: sub_obj.data.to_vec(),
                    repo,
                };
                collect_tree_entries(repo, &sub_tree, &full_path, out)?;
            }
            EntryKind::Commit => {}
        }
    }
    Ok(())
}

pub fn register_git_show_tool(registry: &mut ToolRegistry) {
    registry
        .register(
            ToolSpec {
                name: "git_show".into(),
                description: "Show a specific commit's metadata and diff.".into(),
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
                            "description": "Commit ref or hash to show."
                        }
                    }
                }),
            },
            Box::new(GitShowTool),
        )
        .expect("git_show already registered");
}

// ── git_tag ───────────────────────────────────────────────────────────────────

pub struct GitTagTool;

impl ToolHandler for GitTagTool {
    fn invoke(
        &self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, CoreError>> + Send>> {
        Box::pin(async move {
            let path = repo_path(&args);
            let name: Option<String> = args["name"].as_str().map(str::to_string);
            let delete = args["delete"].as_bool().unwrap_or(false);
            let list = args["list"].as_bool().unwrap_or(true);
            let commit_ref: String = args["commit"].as_str().unwrap_or("HEAD").to_string();
            tokio::task::spawn_blocking(move || {
                git_tag_impl(&path, name.as_deref(), delete, list, &commit_ref)
            })
            .await
            .map_err(|e| CoreError::Tool(format!("git_tag task panicked: {e}")))?
        })
    }
}

fn git_tag_impl(
    path: &Path,
    name: Option<&str>,
    delete: bool,
    list: bool,
    commit_ref: &str,
) -> Result<Value, CoreError> {
    use gix::refs::transaction::{Change, PreviousValue, RefEdit, RefLog};

    let repo = open_repo(path)?;

    if delete {
        let tag_name =
            name.ok_or_else(|| CoreError::Tool("git_tag: 'name' is required for delete".into()))?;
        let full_ref = format!("refs/tags/{tag_name}");
        repo.edit_reference(RefEdit {
            change: Change::Delete {
                expected: PreviousValue::Any,
                log: RefLog::AndReference,
            },
            name: full_ref
                .as_str()
                .try_into()
                .map_err(|e| CoreError::Tool(format!("invalid tag name: {e}")))?,
            deref: false,
        })
        .map_err(|e| CoreError::Tool(format!("failed to delete tag '{tag_name}': {e}")))?;
        return Ok(json!({ "deleted": tag_name }));
    }

    if let Some(tag_name) = name {
        if !list {
            // Create a lightweight tag.
            let target_id = resolve_ref(&repo, commit_ref)?;
            repo.tag_reference(tag_name, target_id, PreviousValue::MustNotExist)
                .map_err(|e| CoreError::Tool(format!("failed to create tag '{tag_name}': {e}")))?;
            return Ok(json!({ "created": tag_name }));
        }
        // name provided but list=true → create the tag.
        let target_id = resolve_ref(&repo, commit_ref)?;
        repo.tag_reference(tag_name, target_id, PreviousValue::MustNotExist)
            .map_err(|e| CoreError::Tool(format!("failed to create tag '{tag_name}': {e}")))?;
        return Ok(json!({ "created": tag_name }));
    }

    // List tags.
    let refs = repo
        .references()
        .map_err(|e| CoreError::Tool(format!("failed to list references: {e}")))?;

    let tags: Vec<String> = refs
        .prefixed("refs/tags/")
        .map_err(|e| CoreError::Tool(format!("failed to iterate tags: {e}")))?
        .filter_map(|r| r.ok())
        .map(|r| r.name().shorten().to_string())
        .collect();

    Ok(json!({ "tags": tags }))
}

pub fn register_git_tag_tool(registry: &mut ToolRegistry) {
    registry
        .register(
            ToolSpec {
                name: "git_tag".into(),
                description: "Create, list, or delete lightweight tags in a git repository."
                    .into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to the git repository (defaults to current directory)."
                        },
                        "name": {
                            "type": "string",
                            "description": "Tag name (required for create/delete)."
                        },
                        "delete": {
                            "type": "boolean",
                            "description": "If true, delete the named tag."
                        },
                        "list": {
                            "type": "boolean",
                            "description": "If true (default), list all tags."
                        },
                        "commit": {
                            "type": "string",
                            "description": "Commit to tag (default HEAD)."
                        }
                    }
                }),
            },
            Box::new(GitTagTool),
        )
        .expect("git_tag already registered");
}
