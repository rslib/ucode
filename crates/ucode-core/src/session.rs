use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::budget::CompactionRecord;
use crate::{CoreError, Message};

/// Unique session identifier.
pub type SessionId = String;

/// Metadata about the active session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: SessionId,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub active_model: Option<String>,
    pub active_skill: Option<String>,
    pub working_dir: PathBuf,
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
            },
            transcript: Vec::new(),
            tool_audit: Vec::new(),
            compaction_log: Vec::new(),
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
