use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

pub type ArtifactId = String;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactType {
    MarkdownReport,
    UnifiedDiff,
    CommandLog,
    TestLog,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactEnvelope {
    pub id: ArtifactId,
    pub artifact_type: ArtifactType,
    pub source: String,
    pub title: String,
    pub metadata: serde_json::Value,
    pub checksum: String,
    pub content_size: u64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub session_id: Option<String>,
    pub tool_call_id: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ArtifactError {
    #[error("artifact not found: {0}")]
    NotFound(String),
    #[error("artifact I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("artifact checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },
    #[error("artifact metadata error: {0}")]
    Metadata(String),
}

fn compute_checksum(content: &[u8]) -> String {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn timestamp_nanos() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

pub struct ArtifactStore {
    base_dir: PathBuf,
}

impl ArtifactStore {
    pub fn new(base_dir: &Path) -> Self {
        Self {
            base_dir: base_dir.to_path_buf(),
        }
    }

    /// Create a new artifact from content bytes.
    /// Computes checksum, writes content file and envelope metadata.
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        &self,
        artifact_type: ArtifactType,
        source: &str,
        title: &str,
        content: &[u8],
        metadata: serde_json::Value,
        session_id: Option<&str>,
        tool_call_id: Option<&str>,
    ) -> Result<ArtifactEnvelope, ArtifactError> {
        let id = format!("art-{:016x}", timestamp_nanos());
        let checksum = compute_checksum(content);
        let content_size = content.len() as u64;
        let created_at = chrono::Utc::now();

        let artifact_dir = self.base_dir.join(&id);
        fs::create_dir_all(&artifact_dir)?;

        fs::write(artifact_dir.join("content"), content)?;

        let envelope = ArtifactEnvelope {
            id,
            artifact_type,
            source: source.to_owned(),
            title: title.to_owned(),
            metadata,
            checksum,
            content_size,
            created_at,
            session_id: session_id.map(str::to_owned),
            tool_call_id: tool_call_id.map(str::to_owned),
        };

        let json = serde_json::to_string_pretty(&envelope)
            .map_err(|e| ArtifactError::Metadata(e.to_string()))?;
        fs::write(artifact_dir.join("envelope.json"), json)?;

        Ok(envelope)
    }

    /// Get artifact envelope by id.
    pub fn get(&self, id: &str) -> Result<ArtifactEnvelope, ArtifactError> {
        let envelope_path = self.base_dir.join(id).join("envelope.json");
        if !envelope_path.exists() {
            return Err(ArtifactError::NotFound(id.to_owned()));
        }
        let data = fs::read(&envelope_path)?;
        serde_json::from_slice(&data).map_err(|e| ArtifactError::Metadata(e.to_string()))
    }

    /// Read artifact content by id.
    pub fn read_content(&self, id: &str) -> Result<Vec<u8>, ArtifactError> {
        let content_path = self.base_dir.join(id).join("content");
        if !content_path.exists() {
            return Err(ArtifactError::NotFound(id.to_owned()));
        }
        Ok(fs::read(&content_path)?)
    }

    /// List all artifacts, sorted by created_at descending.
    pub fn list(&self) -> Result<Vec<ArtifactEnvelope>, ArtifactError> {
        let mut envelopes = Vec::new();

        if !self.base_dir.exists() {
            return Ok(envelopes);
        }

        for entry in fs::read_dir(&self.base_dir)? {
            let entry = entry?;
            let envelope_path = entry.path().join("envelope.json");
            if envelope_path.exists() {
                let data = fs::read(&envelope_path)?;
                match serde_json::from_slice::<ArtifactEnvelope>(&data) {
                    Ok(env) => envelopes.push(env),
                    Err(e) => {
                        return Err(ArtifactError::Metadata(format!(
                            "failed to parse {}: {e}",
                            envelope_path.display()
                        )));
                    }
                }
            }
        }

        envelopes.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(envelopes)
    }

    /// List artifacts filtered by type.
    pub fn list_by_type(
        &self,
        artifact_type: &ArtifactType,
    ) -> Result<Vec<ArtifactEnvelope>, ArtifactError> {
        Ok(self
            .list()?
            .into_iter()
            .filter(|e| &e.artifact_type == artifact_type)
            .collect())
    }

    /// List artifacts linked to a specific session.
    pub fn list_by_session(
        &self,
        session_id: &str,
    ) -> Result<Vec<ArtifactEnvelope>, ArtifactError> {
        Ok(self
            .list()?
            .into_iter()
            .filter(|e| e.session_id.as_deref() == Some(session_id))
            .collect())
    }

    /// Verify artifact content integrity (checksum match).
    pub fn verify(&self, id: &str) -> Result<bool, ArtifactError> {
        let envelope = self.get(id)?;
        let content = self.read_content(id)?;
        let actual = compute_checksum(&content);
        Ok(actual == envelope.checksum)
    }

    /// Delete an artifact.
    pub fn delete(&self, id: &str) -> Result<(), ArtifactError> {
        let artifact_dir = self.base_dir.join(id);
        if !artifact_dir.exists() {
            return Err(ArtifactError::NotFound(id.to_owned()));
        }
        fs::remove_dir_all(&artifact_dir)?;
        Ok(())
    }

    /// Export artifact to a target path (copies content file).
    pub fn export(&self, id: &str, target: &Path) -> Result<PathBuf, ArtifactError> {
        let content_path = self.base_dir.join(id).join("content");
        if !content_path.exists() {
            return Err(ArtifactError::NotFound(id.to_owned()));
        }
        if let Some(parent) = target.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&content_path, target)?;
        Ok(target.to_path_buf())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_store() -> (TempDir, ArtifactStore) {
        let dir = TempDir::new().unwrap();
        let store = ArtifactStore::new(dir.path());
        (dir, store)
    }

    #[test]
    fn test_create_artifact() {
        let (_dir, store) = make_store();
        let content = b"# Report\nSome analysis.";
        let env = store
            .create(
                ArtifactType::MarkdownReport,
                "test_source",
                "My Report",
                content,
                serde_json::json!({"key": "value"}),
                Some("sess-1"),
                Some("tc-1"),
            )
            .unwrap();

        assert!(env.id.starts_with("art-"));
        assert_eq!(env.artifact_type, ArtifactType::MarkdownReport);
        assert_eq!(env.source, "test_source");
        assert_eq!(env.title, "My Report");
        assert_eq!(env.content_size, content.len() as u64);
        assert_eq!(env.session_id.as_deref(), Some("sess-1"));
        assert_eq!(env.tool_call_id.as_deref(), Some("tc-1"));
        assert!(!env.checksum.is_empty());
    }

    #[test]
    fn test_create_artifact_checksum() {
        let (_dir, store) = make_store();
        let content = b"deterministic content";

        let env1 = store
            .create(
                ArtifactType::CommandLog,
                "src",
                "t1",
                content,
                serde_json::Value::Null,
                None,
                None,
            )
            .unwrap();

        // Recompute independently and compare.
        let expected = compute_checksum(content);
        assert_eq!(env1.checksum, expected);

        // Same content always yields same checksum.
        let env2 = store
            .create(
                ArtifactType::CommandLog,
                "src",
                "t2",
                content,
                serde_json::Value::Null,
                None,
                None,
            )
            .unwrap();
        assert_eq!(env1.checksum, env2.checksum);
    }

    #[test]
    fn test_read_content() {
        let (_dir, store) = make_store();
        let content = b"hello artifact world";
        let env = store
            .create(
                ArtifactType::TestLog,
                "src",
                "title",
                content,
                serde_json::Value::Null,
                None,
                None,
            )
            .unwrap();

        let read_back = store.read_content(&env.id).unwrap();
        assert_eq!(read_back, content);
    }

    #[test]
    fn test_get_artifact() {
        let (_dir, store) = make_store();
        let env = store
            .create(
                ArtifactType::UnifiedDiff,
                "git_diff",
                "Diff title",
                b"--- a\n+++ b\n",
                serde_json::json!({"commit": "abc123"}),
                Some("sess-42"),
                None,
            )
            .unwrap();

        let fetched = store.get(&env.id).unwrap();
        assert_eq!(fetched.id, env.id);
        assert_eq!(fetched.artifact_type, ArtifactType::UnifiedDiff);
        assert_eq!(fetched.source, "git_diff");
        assert_eq!(fetched.title, "Diff title");
        assert_eq!(fetched.session_id.as_deref(), Some("sess-42"));
        assert_eq!(fetched.checksum, env.checksum);
    }

    #[test]
    fn test_get_nonexistent() {
        let (_dir, store) = make_store();
        let result = store.get("art-0000000000000000");
        assert!(matches!(result, Err(ArtifactError::NotFound(_))));
    }

    #[test]
    fn test_list_empty() {
        let (_dir, store) = make_store();
        let list = store.list().unwrap();
        assert!(list.is_empty());
    }

    #[test]
    fn test_list_ordered() {
        let (_dir, store) = make_store();

        // Create three artifacts with small sleeps to ensure distinct timestamps.
        let e1 = store
            .create(
                ArtifactType::CommandLog,
                "src",
                "first",
                b"a",
                serde_json::Value::Null,
                None,
                None,
            )
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let e2 = store
            .create(
                ArtifactType::CommandLog,
                "src",
                "second",
                b"b",
                serde_json::Value::Null,
                None,
                None,
            )
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let e3 = store
            .create(
                ArtifactType::CommandLog,
                "src",
                "third",
                b"c",
                serde_json::Value::Null,
                None,
                None,
            )
            .unwrap();

        let list = store.list().unwrap();
        assert_eq!(list.len(), 3);
        // Newest first.
        assert_eq!(list[0].id, e3.id);
        assert_eq!(list[1].id, e2.id);
        assert_eq!(list[2].id, e1.id);
    }

    #[test]
    fn test_list_by_type() {
        let (_dir, store) = make_store();

        store
            .create(
                ArtifactType::MarkdownReport,
                "src",
                "r1",
                b"report",
                serde_json::Value::Null,
                None,
                None,
            )
            .unwrap();
        store
            .create(
                ArtifactType::UnifiedDiff,
                "src",
                "d1",
                b"diff",
                serde_json::Value::Null,
                None,
                None,
            )
            .unwrap();
        store
            .create(
                ArtifactType::MarkdownReport,
                "src",
                "r2",
                b"report2",
                serde_json::Value::Null,
                None,
                None,
            )
            .unwrap();

        let reports = store.list_by_type(&ArtifactType::MarkdownReport).unwrap();
        assert_eq!(reports.len(), 2);
        assert!(
            reports
                .iter()
                .all(|e| e.artifact_type == ArtifactType::MarkdownReport)
        );

        let diffs = store.list_by_type(&ArtifactType::UnifiedDiff).unwrap();
        assert_eq!(diffs.len(), 1);

        let logs = store.list_by_type(&ArtifactType::TestLog).unwrap();
        assert!(logs.is_empty());
    }

    #[test]
    fn test_list_by_session() {
        let (_dir, store) = make_store();

        store
            .create(
                ArtifactType::CommandLog,
                "src",
                "t1",
                b"a",
                serde_json::Value::Null,
                Some("sess-A"),
                None,
            )
            .unwrap();
        store
            .create(
                ArtifactType::CommandLog,
                "src",
                "t2",
                b"b",
                serde_json::Value::Null,
                Some("sess-B"),
                None,
            )
            .unwrap();
        store
            .create(
                ArtifactType::CommandLog,
                "src",
                "t3",
                b"c",
                serde_json::Value::Null,
                Some("sess-A"),
                None,
            )
            .unwrap();
        store
            .create(
                ArtifactType::CommandLog,
                "src",
                "t4",
                b"d",
                serde_json::Value::Null,
                None,
                None,
            )
            .unwrap();

        let sess_a = store.list_by_session("sess-A").unwrap();
        assert_eq!(sess_a.len(), 2);
        assert!(
            sess_a
                .iter()
                .all(|e| e.session_id.as_deref() == Some("sess-A"))
        );

        let sess_b = store.list_by_session("sess-B").unwrap();
        assert_eq!(sess_b.len(), 1);

        let sess_c = store.list_by_session("sess-C").unwrap();
        assert!(sess_c.is_empty());
    }

    #[test]
    fn test_verify_intact() {
        let (_dir, store) = make_store();
        let env = store
            .create(
                ArtifactType::TestLog,
                "src",
                "title",
                b"intact content",
                serde_json::Value::Null,
                None,
                None,
            )
            .unwrap();

        assert!(store.verify(&env.id).unwrap());
    }

    #[test]
    fn test_verify_corrupted() {
        let (dir, store) = make_store();
        let env = store
            .create(
                ArtifactType::TestLog,
                "src",
                "title",
                b"original content",
                serde_json::Value::Null,
                None,
                None,
            )
            .unwrap();

        // Corrupt the content file directly.
        let content_path = dir.path().join(&env.id).join("content");
        fs::write(&content_path, b"corrupted content").unwrap();

        assert!(!store.verify(&env.id).unwrap());
    }

    #[test]
    fn test_delete_artifact() {
        let (dir, store) = make_store();
        let env = store
            .create(
                ArtifactType::CommandLog,
                "src",
                "title",
                b"to be deleted",
                serde_json::Value::Null,
                None,
                None,
            )
            .unwrap();

        let artifact_dir = dir.path().join(&env.id);
        assert!(artifact_dir.exists());

        store.delete(&env.id).unwrap();

        assert!(!artifact_dir.exists());
        assert!(matches!(
            store.get(&env.id),
            Err(ArtifactError::NotFound(_))
        ));
    }

    #[test]
    fn test_export_artifact() {
        let (dir, store) = make_store();
        let content = b"export me";
        let env = store
            .create(
                ArtifactType::MarkdownReport,
                "src",
                "title",
                content,
                serde_json::Value::Null,
                None,
                None,
            )
            .unwrap();

        let target = dir.path().join("exported_output.md");
        let result_path = store.export(&env.id, &target).unwrap();

        assert_eq!(result_path, target);
        assert!(target.exists());
        assert_eq!(fs::read(&target).unwrap(), content);
    }

    #[test]
    fn test_artifact_type_serde() {
        let types = [
            ArtifactType::MarkdownReport,
            ArtifactType::UnifiedDiff,
            ArtifactType::CommandLog,
            ArtifactType::TestLog,
        ];

        for t in &types {
            let json = serde_json::to_string(t).unwrap();
            let roundtrip: ArtifactType = serde_json::from_str(&json).unwrap();
            assert_eq!(&roundtrip, t);
        }
    }
}
