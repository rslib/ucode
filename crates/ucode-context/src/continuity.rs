use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::ContextError;

/// A single continuity event captured during a session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContinuityEvent {
    pub timestamp: DateTime<Utc>,
    pub event_type: ContinuityEventType,
    pub summary: String,
}

/// Types of events tracked for session continuity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ContinuityEventType {
    GoalEstablished,
    FileChanged {
        path: String,
    },
    ErrorEncountered {
        tool: String,
    },
    Decision {
        rationale: String,
    },
    GitCommit {
        hash: String,
        message: String,
    },
    ToolDiscovery {
        tool: String,
        purpose: String,
    },
    MilestoneReached,
    ContextCompacted {
        tokens_before: usize,
        tokens_after: usize,
    },
}

/// Snapshot of session state for restoration after compaction.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompactionSnapshot {
    pub created_at: DateTime<Utc>,
    pub user_goals: Vec<String>,
    pub working_set: Vec<String>,
    pub error_history: Vec<ErrorRecord>,
    pub git_state: Option<GitState>,
    pub key_decisions: Vec<String>,
    pub pending_tasks: Vec<String>,
}

/// Record of an error encountered during the session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ErrorRecord {
    pub tool: String,
    pub summary: String,
    pub timestamp: DateTime<Utc>,
}

/// Git state at time of snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GitState {
    pub last_commit_hash: String,
    pub last_commit_message: String,
    pub branch: Option<String>,
}

pub struct SessionContinuity {
    event_log: Vec<ContinuityEvent>,
    snapshot: Option<CompactionSnapshot>,
    session_dir: PathBuf,
}

impl SessionContinuity {
    pub fn new(session_dir: PathBuf) -> Self {
        Self {
            event_log: Vec::new(),
            snapshot: None,
            session_dir,
        }
    }

    /// Add an event to the log.
    pub fn capture_event(&mut self, event_type: ContinuityEventType, summary: String) {
        self.event_log.push(ContinuityEvent {
            timestamp: Utc::now(),
            event_type,
            summary,
        });
    }

    /// Create a snapshot from the current event log.
    pub fn create_snapshot(&mut self) -> CompactionSnapshot {
        let mut user_goals = Vec::new();
        let mut seen_paths = HashSet::new();
        let mut working_set = Vec::new();
        let mut error_history = Vec::new();
        let mut git_state: Option<GitState> = None;
        let mut key_decisions = Vec::new();

        for event in &self.event_log {
            match &event.event_type {
                ContinuityEventType::GoalEstablished => {
                    user_goals.push(event.summary.clone());
                }
                ContinuityEventType::FileChanged { path } => {
                    if seen_paths.insert(path.clone()) {
                        working_set.push(path.clone());
                    }
                }
                ContinuityEventType::ErrorEncountered { tool } => {
                    error_history.push(ErrorRecord {
                        tool: tool.clone(),
                        summary: event.summary.clone(),
                        timestamp: event.timestamp,
                    });
                }
                ContinuityEventType::GitCommit { hash, message } => {
                    git_state = Some(GitState {
                        last_commit_hash: hash.clone(),
                        last_commit_message: message.clone(),
                        branch: None,
                    });
                }
                ContinuityEventType::Decision { .. } => {
                    key_decisions.push(event.summary.clone());
                }
                _ => {}
            }
        }

        let snapshot = CompactionSnapshot {
            created_at: Utc::now(),
            user_goals,
            working_set,
            error_history,
            git_state,
            key_decisions,
            pending_tasks: Vec::new(),
        };

        self.snapshot = Some(snapshot.clone());
        snapshot
    }

    /// Save event log and snapshot to disk.
    pub fn save(&self) -> Result<(), ContextError> {
        std::fs::create_dir_all(&self.session_dir)?;

        let events_path = self.session_dir.join("continuity_events.json");
        let events_json = serde_json::to_string_pretty(&self.event_log)?;
        std::fs::write(events_path, events_json)?;

        if let Some(snapshot) = &self.snapshot {
            let snapshot_path = self.session_dir.join("continuity.json");
            let snapshot_json = serde_json::to_string_pretty(snapshot)?;
            std::fs::write(snapshot_path, snapshot_json)?;
        }

        Ok(())
    }

    /// Load from disk. Missing files = empty state (not an error).
    pub fn load(session_dir: &Path) -> Result<Self, ContextError> {
        let events_path = session_dir.join("continuity_events.json");
        let event_log = if events_path.exists() {
            let data = std::fs::read_to_string(&events_path)?;
            serde_json::from_str(&data)?
        } else {
            Vec::new()
        };

        let snapshot_path = session_dir.join("continuity.json");
        let snapshot = if snapshot_path.exists() {
            let data = std::fs::read_to_string(&snapshot_path)?;
            Some(serde_json::from_str(&data)?)
        } else {
            None
        };

        Ok(Self {
            event_log,
            snapshot,
            session_dir: session_dir.to_path_buf(),
        })
    }

    /// Generate a system prompt prefix from the snapshot for post-compaction restoration.
    pub fn restore_prompt(&self) -> Option<String> {
        let snapshot = self.snapshot.as_ref()?;

        let mut parts = vec!["## Session Context (restored after compaction)".to_string()];

        if !snapshot.user_goals.is_empty() {
            parts.push("\n### Goals".to_string());
            for goal in &snapshot.user_goals {
                parts.push(format!("- {goal}"));
            }
        }

        if !snapshot.working_set.is_empty() {
            parts.push("\n### Working Files".to_string());
            for file in &snapshot.working_set {
                parts.push(format!("- {file}"));
            }
        }

        if !snapshot.error_history.is_empty() {
            parts.push("\n### Recent Errors".to_string());
            for err in &snapshot.error_history {
                parts.push(format!("- [{}] {}", err.tool, err.summary));
            }
        }

        if let Some(git) = &snapshot.git_state {
            parts.push("\n### Git State".to_string());
            parts.push(format!(
                "Last commit: {} - {}",
                git.last_commit_hash, git.last_commit_message
            ));
        }

        if !snapshot.key_decisions.is_empty() {
            parts.push("\n### Key Decisions".to_string());
            for decision in &snapshot.key_decisions {
                parts.push(format!("- {decision}"));
            }
        }

        Some(parts.join("\n"))
    }

    /// Access the event log.
    pub fn events(&self) -> &[ContinuityEvent] {
        &self.event_log
    }

    /// Access the current snapshot.
    pub fn snapshot(&self) -> Option<&CompactionSnapshot> {
        self.snapshot.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn make_continuity(dir: &TempDir) -> SessionContinuity {
        SessionContinuity::new(dir.path().to_path_buf())
    }

    // --- Event capture tests ---

    #[test]
    fn capture_event_adds_to_log() {
        let dir = TempDir::new().unwrap();
        let mut sc = make_continuity(&dir);

        sc.capture_event(ContinuityEventType::GoalEstablished, "goal one".into());
        sc.capture_event(
            ContinuityEventType::FileChanged {
                path: "src/main.rs".into(),
            },
            "edited main".into(),
        );
        sc.capture_event(ContinuityEventType::MilestoneReached, "done".into());

        assert_eq!(sc.events().len(), 3);
    }

    #[test]
    fn event_types_serialize_roundtrip() {
        let variants: Vec<ContinuityEventType> = vec![
            ContinuityEventType::GoalEstablished,
            ContinuityEventType::FileChanged {
                path: "foo.rs".into(),
            },
            ContinuityEventType::ErrorEncountered {
                tool: "cargo".into(),
            },
            ContinuityEventType::Decision {
                rationale: "chose X".into(),
            },
            ContinuityEventType::GitCommit {
                hash: "abc123".into(),
                message: "init".into(),
            },
            ContinuityEventType::ToolDiscovery {
                tool: "rg".into(),
                purpose: "search".into(),
            },
            ContinuityEventType::MilestoneReached,
            ContinuityEventType::ContextCompacted {
                tokens_before: 1000,
                tokens_after: 200,
            },
        ];

        for variant in variants {
            let json = serde_json::to_string(&variant).unwrap();
            let roundtripped: ContinuityEventType = serde_json::from_str(&json).unwrap();
            assert_eq!(variant, roundtripped);
        }
    }

    // --- Snapshot tests ---

    #[test]
    fn create_snapshot_captures_goals() {
        let dir = TempDir::new().unwrap();
        let mut sc = make_continuity(&dir);

        sc.capture_event(
            ContinuityEventType::GoalEstablished,
            "implement feature A".into(),
        );
        sc.capture_event(ContinuityEventType::GoalEstablished, "fix bug B".into());

        let snapshot = sc.create_snapshot();
        assert_eq!(
            snapshot.user_goals,
            vec!["implement feature A", "fix bug B"]
        );
    }

    #[test]
    fn create_snapshot_captures_errors() {
        let dir = TempDir::new().unwrap();
        let mut sc = make_continuity(&dir);

        sc.capture_event(
            ContinuityEventType::ErrorEncountered {
                tool: "cargo".into(),
            },
            "build failed".into(),
        );
        sc.capture_event(
            ContinuityEventType::ErrorEncountered {
                tool: "rustc".into(),
            },
            "type mismatch".into(),
        );

        let snapshot = sc.create_snapshot();
        assert_eq!(snapshot.error_history.len(), 2);
        assert_eq!(snapshot.error_history[0].tool, "cargo");
        assert_eq!(snapshot.error_history[0].summary, "build failed");
        assert_eq!(snapshot.error_history[1].tool, "rustc");
        assert_eq!(snapshot.error_history[1].summary, "type mismatch");
    }

    #[test]
    fn create_snapshot_captures_git_state() {
        let dir = TempDir::new().unwrap();
        let mut sc = make_continuity(&dir);

        sc.capture_event(
            ContinuityEventType::GitCommit {
                hash: "aaa111".into(),
                message: "first commit".into(),
            },
            "committed".into(),
        );
        sc.capture_event(
            ContinuityEventType::GitCommit {
                hash: "bbb222".into(),
                message: "second commit".into(),
            },
            "committed again".into(),
        );

        let snapshot = sc.create_snapshot();
        let git = snapshot.git_state.expect("git state should be present");
        // Must be the LAST commit
        assert_eq!(git.last_commit_hash, "bbb222");
        assert_eq!(git.last_commit_message, "second commit");
    }

    #[test]
    fn create_snapshot_captures_working_set() {
        let dir = TempDir::new().unwrap();
        let mut sc = make_continuity(&dir);

        sc.capture_event(
            ContinuityEventType::FileChanged {
                path: "src/lib.rs".into(),
            },
            "edited".into(),
        );
        sc.capture_event(
            ContinuityEventType::FileChanged {
                path: "src/main.rs".into(),
            },
            "edited".into(),
        );
        // Duplicate -- should be deduplicated
        sc.capture_event(
            ContinuityEventType::FileChanged {
                path: "src/lib.rs".into(),
            },
            "edited again".into(),
        );

        let snapshot = sc.create_snapshot();
        assert_eq!(snapshot.working_set.len(), 2);
        assert!(snapshot.working_set.contains(&"src/lib.rs".to_string()));
        assert!(snapshot.working_set.contains(&"src/main.rs".to_string()));
    }

    // --- Persistence tests ---

    #[test]
    fn save_and_load_roundtrip() {
        let dir = TempDir::new().unwrap();
        let mut sc = SessionContinuity::new(dir.path().to_path_buf());

        sc.capture_event(ContinuityEventType::GoalEstablished, "goal one".into());
        sc.capture_event(
            ContinuityEventType::FileChanged {
                path: "src/lib.rs".into(),
            },
            "edited".into(),
        );
        sc.create_snapshot();
        sc.save().unwrap();

        let loaded = SessionContinuity::load(dir.path()).unwrap();
        assert_eq!(loaded.events().len(), 2);
        assert_eq!(loaded.events()[0].summary, "goal one");
        assert_eq!(loaded.events()[1].summary, "edited");

        let snap = loaded.snapshot().expect("snapshot should be present");
        assert_eq!(snap.user_goals, vec!["goal one"]);
        assert_eq!(snap.working_set, vec!["src/lib.rs"]);
    }

    #[test]
    fn load_missing_files_returns_empty() {
        let dir = TempDir::new().unwrap();
        let sc = SessionContinuity::load(dir.path()).unwrap();
        assert!(sc.events().is_empty());
        assert!(sc.snapshot().is_none());
    }

    #[test]
    fn restore_prompt_format() {
        let dir = TempDir::new().unwrap();
        let mut sc = make_continuity(&dir);

        sc.capture_event(ContinuityEventType::GoalEstablished, "implement X".into());
        sc.capture_event(
            ContinuityEventType::FileChanged {
                path: "src/lib.rs".into(),
            },
            "edited".into(),
        );
        sc.capture_event(
            ContinuityEventType::ErrorEncountered {
                tool: "cargo".into(),
            },
            "build failed".into(),
        );
        sc.capture_event(
            ContinuityEventType::GitCommit {
                hash: "abc123".into(),
                message: "initial commit".into(),
            },
            "committed".into(),
        );
        sc.capture_event(
            ContinuityEventType::Decision {
                rationale: "chose approach A".into(),
            },
            "use approach A".into(),
        );

        sc.create_snapshot();
        let prompt = sc.restore_prompt().expect("prompt should be present");

        assert!(prompt.contains("## Session Context (restored after compaction)"));
        assert!(prompt.contains("### Goals"));
        assert!(prompt.contains("- implement X"));
        assert!(prompt.contains("### Working Files"));
        assert!(prompt.contains("- src/lib.rs"));
        assert!(prompt.contains("### Recent Errors"));
        assert!(prompt.contains("- [cargo] build failed"));
        assert!(prompt.contains("### Git State"));
        assert!(prompt.contains("Last commit: abc123 - initial commit"));
        assert!(prompt.contains("### Key Decisions"));
        assert!(prompt.contains("- use approach A"));
    }

    #[test]
    fn restore_prompt_returns_none_without_snapshot() {
        let dir = TempDir::new().unwrap();
        let sc = make_continuity(&dir);
        assert!(sc.restore_prompt().is_none());
    }
}
