use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::budget::{CompactionRecord, SessionUsage, UsageRecord};
use crate::message::{Part, Role};
use crate::{CoreError, Message};

/// Unique session identifier.
pub type SessionId = String;

/// How a session title was set.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TitleSource {
    #[default]
    Auto,
    Manual,
}

/// Metadata about the active session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: SessionId,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub active_model: Option<String>,
    pub active_skill: Option<String>,
    pub working_dir: PathBuf,

    /// Session title (None = untitled).
    #[serde(default)]
    pub title: Option<String>,

    /// How the title was set.
    #[serde(default)]
    pub title_source: TitleSource,

    /// Whether this session is archived.
    #[serde(default)]
    pub archived: bool,

    /// Parent session ID (set when this session was forked from another).
    #[serde(default)]
    pub parent_session_id: Option<SessionId>,

    /// Transcript index in the parent where this fork occurred.
    #[serde(default)]
    pub fork_source_index: Option<usize>,
}

/// A recorded tool invocation for the audit trail.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolAuditEntry {
    pub timestamp: DateTime<Utc>,
    pub tool_name: String,
    pub approved: bool,
    pub duration_ms: u64,
}

/// Full session state: metadata + transcript + audit log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub meta: SessionMeta,
    pub transcript: Vec<Message>,
    pub tool_audit: Vec<ToolAuditEntry>,
    /// Compaction/distillation audit trail.
    #[serde(default)]
    pub compaction_log: Vec<CompactionRecord>,
    /// Accumulated token/cost usage for this session.
    #[serde(default)]
    pub usage: SessionUsage,
}

/// Directory-based session store.
pub struct SessionStore {
    dir: PathBuf,
}

fn generate_session_id() -> SessionId {
    format!(
        "ses_{}_{}",
        Utc::now().format("%Y%m%d%H%M%S"),
        rand::random::<u32>()
    )
}

impl Session {
    /// Create a new session with a generated ID and current timestamp.
    pub fn new(working_dir: PathBuf) -> Self {
        let now = Utc::now();
        Self {
            meta: SessionMeta {
                id: generate_session_id(),
                created_at: now,
                updated_at: now,
                active_model: None,
                active_skill: None,
                working_dir,
                title: None,
                title_source: TitleSource::Auto,
                archived: false,
                parent_session_id: None,
                fork_source_index: None,
            },
            transcript: Vec::new(),
            tool_audit: Vec::new(),
            compaction_log: Vec::new(),
            usage: SessionUsage::default(),
        }
    }

    /// Append a message to the transcript and update `updated_at`.
    pub fn push_message(&mut self, msg: Message) {
        self.transcript.push(msg);
        self.meta.updated_at = Utc::now();
    }

    /// Record a tool invocation in the audit log.
    pub fn record_tool_use(&mut self, tool_name: String, approved: bool, duration_ms: u64) {
        self.tool_audit.push(ToolAuditEntry {
            timestamp: Utc::now(),
            tool_name,
            approved,
            duration_ms,
        });
        self.meta.updated_at = Utc::now();
    }

    /// Set the active model.
    pub fn set_active_model(&mut self, model: Option<String>) {
        self.meta.active_model = model;
        self.meta.updated_at = Utc::now();
    }

    /// Set the active skill.
    pub fn set_active_skill(&mut self, skill: Option<String>) {
        self.meta.active_skill = skill;
        self.meta.updated_at = Utc::now();
    }

    /// Record compaction/distillation results in the audit trail.
    pub fn record_compaction(&mut self, records: Vec<CompactionRecord>) {
        self.compaction_log.extend(records);
        self.meta.updated_at = Utc::now();
    }

    /// Record token/cost usage for a model request.
    pub fn record_usage(&mut self, record: UsageRecord) {
        self.usage.record(record);
        self.meta.updated_at = Utc::now();
    }

    /// Set title from auto-generation (only if not manually locked).
    pub fn set_auto_title(&mut self, title: String) {
        if self.meta.title_source != TitleSource::Manual {
            self.meta.title = Some(title);
            self.meta.title_source = TitleSource::Auto;
            self.meta.updated_at = Utc::now();
        }
    }

    /// Manually rename the session (locks title from auto-overwrite).
    pub fn rename(&mut self, title: String) {
        self.meta.title = Some(title);
        self.meta.title_source = TitleSource::Manual;
        self.meta.updated_at = Utc::now();
    }

    /// Archive the session.
    pub fn archive(&mut self) {
        self.meta.archived = true;
        self.meta.updated_at = Utc::now();
    }

    /// Unarchive the session.
    pub fn unarchive(&mut self) {
        self.meta.archived = false;
        self.meta.updated_at = Utc::now();
    }

    /// Generate a fallback title from the first user message in the transcript.
    /// Truncates to 50 chars. Returns None if no user messages exist.
    pub fn generate_fallback_title(&self) -> Option<String> {
        for msg in &self.transcript {
            if msg.role == Role::User {
                for part in &msg.parts {
                    if let Part::Text(text) = part {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() {
                            let title: String = trimmed.chars().take(50).collect();
                            let title = if trimmed.chars().count() > 50 {
                                format!("{title}...")
                            } else {
                                title
                            };
                            return Some(title);
                        }
                    }
                }
            }
        }
        None
    }

    /// Apply auto-title if no title is set yet and transcript has user messages.
    pub fn auto_title_if_needed(&mut self) {
        if self.meta.title.is_none()
            && self.meta.title_source != TitleSource::Manual
            && let Some(title) = self.generate_fallback_title()
        {
            self.set_auto_title(title);
        }
    }

    /// Fork this session at the given transcript index, creating a new child session.
    /// The child gets a new ID, copies transcript up to `at_index` (exclusive),
    /// inherits model/skill/working_dir, and records lineage metadata.
    /// Returns the new child session (not yet persisted).
    pub fn fork(&self, at_index: Option<usize>) -> Self {
        let idx = at_index.unwrap_or(self.transcript.len());
        let idx = idx.min(self.transcript.len());
        let now = Utc::now();
        Self {
            meta: SessionMeta {
                id: generate_session_id(),
                created_at: now,
                updated_at: now,
                active_model: self.meta.active_model.clone(),
                active_skill: self.meta.active_skill.clone(),
                working_dir: self.meta.working_dir.clone(),
                title: None,
                title_source: TitleSource::Auto,
                archived: false,
                parent_session_id: Some(self.meta.id.clone()),
                fork_source_index: Some(idx),
            },
            transcript: self.transcript[..idx].to_vec(),
            tool_audit: Vec::new(),
            compaction_log: Vec::new(),
            usage: SessionUsage::default(),
        }
    }

    /// Serialize to JSON and write to the given path.
    pub fn save(&self, path: &Path) -> Result<(), CoreError> {
        let json = serde_json::to_string_pretty(self).map_err(|e| CoreError::Internal {
            message: format!("serialize session: {e}"),
        })?;
        fs::write(path, json).map_err(|e| CoreError::Internal {
            message: format!("write session to {}: {e}", path.display()),
        })
    }

    /// Load a session from a JSON file.
    pub fn load(path: &Path) -> Result<Self, CoreError> {
        let data = fs::read_to_string(path).map_err(|e| CoreError::Internal {
            message: format!("read session from {}: {e}", path.display()),
        })?;
        serde_json::from_str(&data).map_err(|e| CoreError::Internal {
            message: format!("deserialize session from {}: {e}", path.display()),
        })
    }
}

impl SessionStore {
    /// Create a new store backed by the given directory.
    /// Creates the directory if it doesn't exist.
    pub fn new(dir: PathBuf) -> Result<Self, CoreError> {
        fs::create_dir_all(&dir).map_err(|e| CoreError::Internal {
            message: format!("create session dir {}: {e}", dir.display()),
        })?;
        Ok(Self { dir })
    }

    fn session_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    /// Create and persist a new session.
    pub fn create(&self, working_dir: PathBuf) -> Result<Session, CoreError> {
        let session = Session::new(working_dir);
        session.save(&self.session_path(&session.meta.id))?;
        Ok(session)
    }

    /// Load a session by ID.
    pub fn load(&self, id: &str) -> Result<Session, CoreError> {
        Session::load(&self.session_path(id))
    }

    /// Save an existing session.
    pub fn save(&self, session: &Session) -> Result<(), CoreError> {
        session.save(&self.session_path(&session.meta.id))
    }

    /// List all sessions (metadata only, sorted by updated_at descending).
    /// If `include_archived` is false, archived sessions are excluded.
    pub fn list(&self, include_archived: bool) -> Result<Vec<SessionMeta>, CoreError> {
        let mut metas = Vec::new();
        let entries = fs::read_dir(&self.dir).map_err(|e| CoreError::Internal {
            message: format!("read session dir {}: {e}", self.dir.display()),
        })?;
        for entry in entries {
            let entry = entry.map_err(|e| CoreError::Internal {
                message: format!("read dir entry: {e}"),
            })?;
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                match Session::load(&path) {
                    Ok(session) => {
                        if include_archived || !session.meta.archived {
                            metas.push(session.meta);
                        }
                    }
                    Err(_) => continue, // skip corrupt files
                }
            }
        }
        metas.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(metas)
    }

    /// Delete a session file.
    pub fn delete(&self, id: &str) -> Result<(), CoreError> {
        let path = self.session_path(id);
        fs::remove_file(&path).map_err(|e| CoreError::Internal {
            message: format!("delete session {}: {e}", path.display()),
        })
    }

    /// Fork an existing session at the given transcript index.
    /// Creates and persists the child session, returns it.
    pub fn fork(&self, parent_id: &str, at_index: Option<usize>) -> Result<Session, CoreError> {
        let parent = self.load(parent_id)?;
        let child = parent.fork(at_index);
        self.save(&child)?;
        Ok(child)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_source_default_is_auto() {
        assert_eq!(TitleSource::default(), TitleSource::Auto);
    }

    #[test]
    fn set_auto_title() {
        let mut s = Session::new(PathBuf::from("/tmp"));
        s.set_auto_title("Test title".into());
        assert_eq!(s.meta.title.as_deref(), Some("Test title"));
        assert_eq!(s.meta.title_source, TitleSource::Auto);
    }

    #[test]
    fn manual_rename_locks_title() {
        let mut s = Session::new(PathBuf::from("/tmp"));
        s.rename("My title".into());
        assert_eq!(s.meta.title_source, TitleSource::Manual);
        // Auto-title should NOT overwrite manual title
        s.set_auto_title("Auto title".into());
        assert_eq!(s.meta.title.as_deref(), Some("My title"));
    }

    #[test]
    fn archive_unarchive() {
        let mut s = Session::new(PathBuf::from("/tmp"));
        assert!(!s.meta.archived);
        s.archive();
        assert!(s.meta.archived);
        s.unarchive();
        assert!(!s.meta.archived);
    }

    #[test]
    fn generate_fallback_title_from_user_message() {
        let mut s = Session::new(PathBuf::from("/tmp"));
        s.push_message(Message::system("You are helpful."));
        s.push_message(Message::user("Help me write a Rust HTTP server"));
        let title = s.generate_fallback_title();
        assert_eq!(title.as_deref(), Some("Help me write a Rust HTTP server"));
    }

    #[test]
    fn generate_fallback_title_truncates_long() {
        let mut s = Session::new(PathBuf::from("/tmp"));
        let long_msg = "a".repeat(100);
        s.push_message(Message::user(&long_msg));
        let title = s.generate_fallback_title().unwrap();
        assert!(title.len() <= 54); // 50 chars + "..."
        assert!(title.ends_with("..."));
    }

    #[test]
    fn generate_fallback_title_none_when_empty() {
        let s = Session::new(PathBuf::from("/tmp"));
        assert!(s.generate_fallback_title().is_none());
    }

    #[test]
    fn auto_title_if_needed_sets_title() {
        let mut s = Session::new(PathBuf::from("/tmp"));
        s.push_message(Message::user("Build a CLI tool"));
        s.auto_title_if_needed();
        assert_eq!(s.meta.title.as_deref(), Some("Build a CLI tool"));
        assert_eq!(s.meta.title_source, TitleSource::Auto);
    }

    #[test]
    fn auto_title_if_needed_skips_manual() {
        let mut s = Session::new(PathBuf::from("/tmp"));
        s.push_message(Message::user("Build a CLI tool"));
        s.rename("Custom name".into());
        s.auto_title_if_needed();
        assert_eq!(s.meta.title.as_deref(), Some("Custom name"));
    }

    #[test]
    fn session_store_create_and_list() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().to_path_buf()).unwrap();
        let s1 = store.create(PathBuf::from("/tmp")).unwrap();
        let s2 = store.create(PathBuf::from("/tmp")).unwrap();
        let list = store.list(false).unwrap();
        assert_eq!(list.len(), 2);
        let ids: Vec<_> = list.iter().map(|m| m.id.as_str()).collect();
        assert!(ids.contains(&s1.meta.id.as_str()));
        assert!(ids.contains(&s2.meta.id.as_str()));
    }

    #[test]
    fn session_store_list_excludes_archived() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().to_path_buf()).unwrap();
        let mut s = store.create(PathBuf::from("/tmp")).unwrap();
        s.archive();
        store.save(&s).unwrap();
        let _ = store.create(PathBuf::from("/tmp")).unwrap();
        let active = store.list(false).unwrap();
        assert_eq!(active.len(), 1);
        let all = store.list(true).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn session_store_load_and_save() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().to_path_buf()).unwrap();
        let mut s = store.create(PathBuf::from("/tmp")).unwrap();
        s.rename("Test session".into());
        store.save(&s).unwrap();
        let loaded = store.load(&s.meta.id).unwrap();
        assert_eq!(loaded.meta.title.as_deref(), Some("Test session"));
        assert_eq!(loaded.meta.title_source, TitleSource::Manual);
    }

    #[test]
    fn session_store_delete() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().to_path_buf()).unwrap();
        let s = store.create(PathBuf::from("/tmp")).unwrap();
        store.delete(&s.meta.id).unwrap();
        assert!(store.load(&s.meta.id).is_err());
    }

    #[test]
    fn backward_compat_no_title_fields() {
        // Old-format session JSON without title/title_source/archived
        let json = r#"{
            "meta": {
                "id": "ses_old",
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z",
                "active_model": null,
                "active_skill": null,
                "working_dir": "/tmp"
            },
            "transcript": [],
            "tool_audit": []
        }"#;
        let session: Session = serde_json::from_str(json).unwrap();
        assert!(session.meta.title.is_none());
        assert_eq!(session.meta.title_source, TitleSource::Auto);
        assert!(!session.meta.archived);
        assert!(session.compaction_log.is_empty());
        assert!(session.meta.parent_session_id.is_none());
        assert!(session.meta.fork_source_index.is_none());
        assert!(session.usage.records.is_empty());
    }

    #[test]
    fn fork_creates_child_with_lineage() {
        let mut parent = Session::new(PathBuf::from("/project"));
        parent.set_active_model(Some("claude-3".into()));
        parent.set_active_skill(Some("tdd".into()));
        parent.push_message(Message::user("hello"));
        parent.push_message(Message::assistant("hi"));
        parent.push_message(Message::user("next"));

        let child = parent.fork(Some(2));

        assert_ne!(child.meta.id, parent.meta.id);
        assert_eq!(
            child.meta.parent_session_id.as_deref(),
            Some(parent.meta.id.as_str())
        );
        assert_eq!(child.meta.fork_source_index, Some(2));
        assert_eq!(child.transcript.len(), 2);
        assert_eq!(child.meta.active_model.as_deref(), Some("claude-3"));
        assert_eq!(child.meta.active_skill.as_deref(), Some("tdd"));
        assert_eq!(child.meta.working_dir, parent.meta.working_dir);
        assert!(child.tool_audit.is_empty());
        assert!(child.compaction_log.is_empty());
        assert!(child.meta.title.is_none());
    }

    #[test]
    fn fork_none_index_copies_full_transcript() {
        let mut parent = Session::new(PathBuf::from("/tmp"));
        parent.push_message(Message::user("a"));
        parent.push_message(Message::assistant("b"));

        let child = parent.fork(None);
        assert_eq!(child.transcript.len(), 2);
        assert_eq!(child.meta.fork_source_index, Some(2));
    }

    #[test]
    fn fork_index_clamped_to_transcript_len() {
        let mut parent = Session::new(PathBuf::from("/tmp"));
        parent.push_message(Message::user("a"));

        let child = parent.fork(Some(999));
        assert_eq!(child.transcript.len(), 1);
        assert_eq!(child.meta.fork_source_index, Some(1));
    }

    #[test]
    fn fork_no_state_bleed() {
        let mut parent = Session::new(PathBuf::from("/tmp"));
        parent.push_message(Message::user("hello"));
        let mut child = parent.fork(None);

        // Mutate child — parent must not change
        child.push_message(Message::user("child msg"));
        child.set_active_model(Some("gpt-4".into()));

        assert_eq!(parent.transcript.len(), 1);
        assert_eq!(child.transcript.len(), 2);
        assert!(parent.meta.active_model.is_none());
    }

    #[test]
    fn session_store_fork() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().to_path_buf()).unwrap();
        let mut parent = store.create(PathBuf::from("/tmp")).unwrap();
        parent.push_message(Message::user("hello"));
        parent.push_message(Message::assistant("world"));
        store.save(&parent).unwrap();

        let child = store.fork(&parent.meta.id, Some(1)).unwrap();
        assert_eq!(
            child.meta.parent_session_id.as_deref(),
            Some(parent.meta.id.as_str())
        );
        assert_eq!(child.transcript.len(), 1);

        // Verify child is persisted
        let loaded = store.load(&child.meta.id).unwrap();
        assert_eq!(
            loaded.meta.parent_session_id.as_deref(),
            Some(parent.meta.id.as_str())
        );
    }
}
