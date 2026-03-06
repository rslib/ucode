use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

pub type CheckpointId = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointInfo {
    pub id: CheckpointId,
    pub name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub file_count: usize,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionPolicy {
    pub max_count: Option<usize>,
    pub max_age: Option<Duration>,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            max_count: Some(10),
            max_age: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CheckpointError {
    #[error("checkpoint not found: {0}")]
    NotFound(String),
    #[error("checkpoint I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("checkpoint metadata error: {0}")]
    Metadata(String),
}

pub struct CheckpointStore {
    base_dir: PathBuf,
    checkpoints_dir: PathBuf,
}

fn generate_id() -> CheckpointId {
    let nanos = Utc::now()
        .timestamp_nanos_opt()
        .unwrap_or_else(|| Utc::now().timestamp_millis() * 1_000_000);
    format!("{:016x}", nanos as u64)
}

fn copy_file_preserving_path(
    src: &Path,
    dst_root: &Path,
    rel: &Path,
) -> Result<u64, CheckpointError> {
    let dst = dst_root.join(rel);
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(src, &dst)?;
    Ok(fs::metadata(&dst)?.len())
}

fn read_meta(checkpoint_dir: &Path) -> Result<CheckpointInfo, CheckpointError> {
    let meta_path = checkpoint_dir.join("meta.json");
    let data = fs::read_to_string(&meta_path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            CheckpointError::NotFound(
                checkpoint_dir
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            )
        } else {
            CheckpointError::Io(e)
        }
    })?;
    serde_json::from_str(&data)
        .map_err(|e| CheckpointError::Metadata(format!("invalid meta.json: {e}")))
}

impl CheckpointStore {
    pub fn new(workspace_root: &Path) -> Self {
        let checkpoints_dir = workspace_root.join(".ucode").join(".checkpoints");
        Self {
            base_dir: workspace_root.to_path_buf(),
            checkpoints_dir,
        }
    }

    pub fn checkpoints_dir(&self) -> &Path {
        &self.checkpoints_dir
    }

    /// Create a checkpoint of the given files (relative paths from workspace root).
    pub fn create(
        &self,
        name: &str,
        description: Option<&str>,
        files: &[&Path],
    ) -> Result<CheckpointInfo, CheckpointError> {
        let id = generate_id();
        let checkpoint_dir = self.checkpoints_dir.join(&id);
        let files_dir = checkpoint_dir.join("files");
        fs::create_dir_all(&files_dir)?;

        let mut total_bytes: u64 = 0;
        for rel in files {
            let src = self.base_dir.join(rel);
            total_bytes += copy_file_preserving_path(&src, &files_dir, rel)?;
        }

        let info = CheckpointInfo {
            id: id.clone(),
            name: name.to_owned(),
            description: description.map(str::to_owned),
            created_at: Utc::now(),
            file_count: files.len(),
            total_bytes,
        };

        let meta_path = checkpoint_dir.join("meta.json");
        let json = serde_json::to_string_pretty(&info)
            .map_err(|e| CheckpointError::Metadata(format!("serialization failed: {e}")))?;
        fs::write(meta_path, json)?;

        Ok(info)
    }

    /// List all checkpoints, sorted by created_at descending (newest first).
    pub fn list(&self) -> Result<Vec<CheckpointInfo>, CheckpointError> {
        if !self.checkpoints_dir.exists() {
            return Ok(Vec::new());
        }

        let mut infos = Vec::new();
        for entry in fs::read_dir(&self.checkpoints_dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            match read_meta(&path) {
                Ok(info) => infos.push(info),
                Err(CheckpointError::Metadata(_)) => continue, // skip corrupt entries
                Err(e) => return Err(e),
            }
        }

        infos.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(infos)
    }

    /// Restore a checkpoint by id — copies files back to workspace root.
    pub fn restore(&self, id: &CheckpointId) -> Result<Vec<PathBuf>, CheckpointError> {
        let checkpoint_dir = self.checkpoints_dir.join(id);
        if !checkpoint_dir.exists() {
            return Err(CheckpointError::NotFound(id.clone()));
        }

        let info = read_meta(&checkpoint_dir)?;
        let files_dir = checkpoint_dir.join("files");

        let mut restored = Vec::with_capacity(info.file_count);
        restore_dir(&files_dir, &files_dir, &self.base_dir, &mut restored)?;
        Ok(restored)
    }

    /// Delete a specific checkpoint.
    pub fn delete(&self, id: &CheckpointId) -> Result<(), CheckpointError> {
        let checkpoint_dir = self.checkpoints_dir.join(id);
        if !checkpoint_dir.exists() {
            return Err(CheckpointError::NotFound(id.clone()));
        }
        fs::remove_dir_all(checkpoint_dir)?;
        Ok(())
    }

    /// Prune checkpoints according to retention policy.
    /// Returns the ids of pruned checkpoints.
    pub fn prune(&self, policy: &RetentionPolicy) -> Result<Vec<CheckpointId>, CheckpointError> {
        let mut checkpoints = self.list()?; // newest first
        let mut pruned = Vec::new();

        // Apply max_age: remove any checkpoint older than the cutoff.
        if let Some(max_age) = policy.max_age {
            let cutoff = Utc::now() - max_age;
            let mut i = 0;
            while i < checkpoints.len() {
                if checkpoints[i].created_at < cutoff {
                    let id = checkpoints.remove(i).id;
                    self.delete(&id)?;
                    pruned.push(id);
                } else {
                    i += 1;
                }
            }
        }

        // Apply max_count: keep only the newest N, remove the rest.
        if let Some(max_count) = policy.max_count
            && checkpoints.len() > max_count
        {
            for info in checkpoints.drain(max_count..) {
                self.delete(&info.id)?;
                pruned.push(info.id);
            }
        }

        Ok(pruned)
    }

    /// Get info for a specific checkpoint.
    pub fn get(&self, id: &CheckpointId) -> Result<CheckpointInfo, CheckpointError> {
        let checkpoint_dir = self.checkpoints_dir.join(id);
        if !checkpoint_dir.exists() {
            return Err(CheckpointError::NotFound(id.clone()));
        }
        read_meta(&checkpoint_dir)
    }
}

/// Recursively walk `src_dir`, copying each file to `dst_root` preserving the
/// path relative to `files_root`.
fn restore_dir(
    src_dir: &Path,
    files_root: &Path,
    dst_root: &Path,
    restored: &mut Vec<PathBuf>,
) -> Result<(), CheckpointError> {
    for entry in fs::read_dir(src_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            restore_dir(&path, files_root, dst_root, restored)?;
        } else {
            let rel = path
                .strip_prefix(files_root)
                .map_err(|e| CheckpointError::Metadata(format!("path strip error: {e}")))?;
            let dst = dst_root.join(rel);
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&path, &dst)?;
            restored.push(rel.to_path_buf());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::Duration as StdDuration;

    use tempfile::TempDir;

    use super::*;

    fn make_file(dir: &Path, rel: &str, content: &str) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    fn read_file(dir: &Path, rel: &str) -> String {
        fs::read_to_string(dir.join(rel)).unwrap()
    }

    #[test]
    fn test_create_checkpoint() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        make_file(root, "a.txt", "hello");
        make_file(root, "b.txt", "world");

        let store = CheckpointStore::new(root);
        let info = store
            .create(
                "snap1",
                Some("first snap"),
                &[Path::new("a.txt"), Path::new("b.txt")],
            )
            .unwrap();

        assert_eq!(info.name, "snap1");
        assert_eq!(info.description.as_deref(), Some("first snap"));
        assert_eq!(info.file_count, 2);
        assert!(info.total_bytes > 0);
        assert!(!info.id.is_empty());
    }

    #[test]
    fn test_create_checkpoint_preserves_content() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        make_file(root, "data.txt", "original content");

        let store = CheckpointStore::new(root);
        let info = store
            .create("snap", None, &[Path::new("data.txt")])
            .unwrap();

        let stored = store
            .checkpoints_dir()
            .join(&info.id)
            .join("files")
            .join("data.txt");
        assert_eq!(fs::read_to_string(stored).unwrap(), "original content");
    }

    #[test]
    fn test_list_checkpoints_empty() {
        let tmp = TempDir::new().unwrap();
        let store = CheckpointStore::new(tmp.path());
        let list = store.list().unwrap();
        assert!(list.is_empty());
    }

    #[test]
    fn test_list_checkpoints_ordered() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        make_file(root, "f.txt", "x");

        let store = CheckpointStore::new(root);
        let i1 = store.create("first", None, &[Path::new("f.txt")]).unwrap();
        // Small sleep so timestamps differ; nanos-based ids should still differ,
        // but created_at ordering is what we test.
        thread::sleep(StdDuration::from_millis(5));
        let i2 = store.create("second", None, &[Path::new("f.txt")]).unwrap();
        thread::sleep(StdDuration::from_millis(5));
        let i3 = store.create("third", None, &[Path::new("f.txt")]).unwrap();

        let list = store.list().unwrap();
        assert_eq!(list.len(), 3);
        // Newest first.
        assert_eq!(list[0].id, i3.id);
        assert_eq!(list[1].id, i2.id);
        assert_eq!(list[2].id, i1.id);
    }

    #[test]
    fn test_restore_checkpoint() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        make_file(root, "restore_me.txt", "before");

        let store = CheckpointStore::new(root);
        let info = store
            .create("snap", None, &[Path::new("restore_me.txt")])
            .unwrap();

        // Mutate the file.
        make_file(root, "restore_me.txt", "after");
        assert_eq!(read_file(root, "restore_me.txt"), "after");

        let restored = store.restore(&info.id).unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(read_file(root, "restore_me.txt"), "before");
    }

    #[test]
    fn test_restore_nonexistent() {
        let tmp = TempDir::new().unwrap();
        let store = CheckpointStore::new(tmp.path());
        let err = store.restore(&"deadbeefdeadbeef".to_string()).unwrap_err();
        assert!(matches!(err, CheckpointError::NotFound(_)));
    }

    #[test]
    fn test_delete_checkpoint() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        make_file(root, "d.txt", "data");

        let store = CheckpointStore::new(root);
        let info = store
            .create("to_delete", None, &[Path::new("d.txt")])
            .unwrap();
        assert_eq!(store.list().unwrap().len(), 1);

        store.delete(&info.id).unwrap();
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn test_get_checkpoint() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        make_file(root, "g.txt", "get me");

        let store = CheckpointStore::new(root);
        let created = store
            .create("getme", Some("desc"), &[Path::new("g.txt")])
            .unwrap();

        let fetched = store.get(&created.id).unwrap();
        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.name, "getme");
        assert_eq!(fetched.description.as_deref(), Some("desc"));
        assert_eq!(fetched.file_count, 1);
    }

    #[test]
    fn test_get_nonexistent() {
        let tmp = TempDir::new().unwrap();
        let store = CheckpointStore::new(tmp.path());
        let err = store.get(&"0000000000000000".to_string()).unwrap_err();
        assert!(matches!(err, CheckpointError::NotFound(_)));
    }

    #[test]
    fn test_prune_by_count() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        make_file(root, "p.txt", "prune");

        let store = CheckpointStore::new(root);
        let mut ids = Vec::new();
        for i in 0..5 {
            thread::sleep(StdDuration::from_millis(5));
            let info = store
                .create(&format!("snap{i}"), None, &[Path::new("p.txt")])
                .unwrap();
            ids.push(info.id);
        }

        let policy = RetentionPolicy {
            max_count: Some(2),
            max_age: None,
        };
        let pruned = store.prune(&policy).unwrap();
        assert_eq!(pruned.len(), 3);

        let remaining = store.list().unwrap();
        assert_eq!(remaining.len(), 2);
        // The two newest should survive.
        let newest_id = ids.last().unwrap();
        assert!(remaining.iter().any(|i| &i.id == newest_id));
    }

    #[test]
    fn test_prune_by_age() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        make_file(root, "age.txt", "age");

        let store = CheckpointStore::new(root);
        store.create("old", None, &[Path::new("age.txt")]).unwrap();

        // max_age = 0 means everything created before now is expired.
        let policy = RetentionPolicy {
            max_count: None,
            max_age: Some(Duration::zero()),
        };
        let pruned = store.prune(&policy).unwrap();
        assert_eq!(pruned.len(), 1);
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn test_prune_no_policy() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        make_file(root, "np.txt", "no prune");

        let store = CheckpointStore::new(root);
        for i in 0..3 {
            store
                .create(&format!("s{i}"), None, &[Path::new("np.txt")])
                .unwrap();
        }

        let policy = RetentionPolicy {
            max_count: None,
            max_age: None,
        };
        let pruned = store.prune(&policy).unwrap();
        assert!(pruned.is_empty());
        assert_eq!(store.list().unwrap().len(), 3);
    }

    #[test]
    fn test_checkpoint_with_nested_paths() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        make_file(root, "src/lib.rs", "pub fn foo() {}");
        make_file(root, "src/main.rs", "fn main() {}");
        make_file(root, "docs/readme.md", "# readme");

        let store = CheckpointStore::new(root);
        let info = store
            .create(
                "nested",
                None,
                &[
                    Path::new("src/lib.rs"),
                    Path::new("src/main.rs"),
                    Path::new("docs/readme.md"),
                ],
            )
            .unwrap();

        assert_eq!(info.file_count, 3);

        let files_dir = store.checkpoints_dir().join(&info.id).join("files");
        assert_eq!(
            fs::read_to_string(files_dir.join("src/lib.rs")).unwrap(),
            "pub fn foo() {}"
        );
        assert_eq!(
            fs::read_to_string(files_dir.join("docs/readme.md")).unwrap(),
            "# readme"
        );

        // Mutate and restore.
        make_file(root, "src/lib.rs", "CHANGED");
        store.restore(&info.id).unwrap();
        assert_eq!(read_file(root, "src/lib.rs"), "pub fn foo() {}");
    }
}
