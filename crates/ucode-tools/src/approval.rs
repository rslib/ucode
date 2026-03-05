use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use ucode_core::CoreError;

use crate::policy::{EffectivePolicy, check_path_within_workspace};

/// How long an approval decision remains valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalScope {
    /// Valid for this single invocation only; not reused on subsequent lookups.
    Once,
    /// Valid for the remainder of the session.
    Session,
}

/// The action being approved or denied.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalAction {
    FileAccess { path: PathBuf, write: bool },
    CmdExec { cmd: String, cwd: Option<PathBuf> },
    ProcessSpawn { program: String },
    NetworkAccess,
}

/// The outcome of an approval decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Approved(ApprovalScope),
    Denied,
}

/// A single entry in the approval audit trail.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRecord {
    pub action: ApprovalAction,
    pub decision: ApprovalDecision,
    pub reason: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// In-memory store of approval decisions for the current session.
///
/// `Once`-scoped approvals are written to the audit log but are NOT indexed for
/// lookup — they are consumed on the first (and only) use by the caller that
/// recorded them, so a subsequent `lookup` for the same action will return
/// `None` and trigger a fresh prompt.
pub struct ApprovalStore {
    records: Mutex<Vec<ApprovalRecord>>,
}

impl ApprovalStore {
    pub fn new() -> Self {
        Self {
            records: Mutex::new(Vec::new()),
        }
    }

    /// Record an approval decision.
    pub fn record(&self, action: ApprovalAction, decision: ApprovalDecision, reason: &str) {
        let record = ApprovalRecord {
            action,
            decision,
            reason: reason.to_owned(),
            timestamp: chrono::Utc::now(),
        };
        // Unwrap is safe: we never poison the mutex (no panics while holding it).
        self.records
            .lock()
            .expect("approval store mutex poisoned")
            .push(record);
    }

    /// Check if a session-scoped approval already exists for the given action.
    ///
    /// Returns `Some(decision)` only for `Session`-scoped records.
    /// `Once`-scoped records are intentionally excluded so they cannot be reused.
    pub fn lookup(&self, action: &ApprovalAction) -> Option<ApprovalDecision> {
        let records = self.records.lock().expect("approval store mutex poisoned");
        records.iter().rev().find_map(|r| {
            if &r.action != action {
                return None;
            }
            match r.decision {
                ApprovalDecision::Approved(ApprovalScope::Session) => Some(r.decision),
                ApprovalDecision::Denied => Some(r.decision),
                // Once-scoped approvals are not reusable.
                ApprovalDecision::Approved(ApprovalScope::Once) => None,
            }
        })
    }

    /// Return all recorded decisions for audit purposes.
    pub fn audit_log(&self) -> Vec<ApprovalRecord> {
        self.records
            .lock()
            .expect("approval store mutex poisoned")
            .clone()
    }
}

impl Default for ApprovalStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Gate that enforces workspace boundary policy and manages out-of-workspace approvals.
///
/// For in-workspace actions the gate delegates to [`EffectivePolicy`].
/// For out-of-workspace actions it consults the [`ApprovalStore`]:
/// - Session-approved → allow.
/// - Denied → return `CoreError::Tool("denied: …")`.
/// - No prior decision → return `CoreError::Tool("approval_required:…")` so the
///   TUI layer can intercept, prompt the user, and call [`BoundaryGate::approve`].
pub struct BoundaryGate {
    policy: EffectivePolicy,
    store: ApprovalStore,
}

impl BoundaryGate {
    pub fn new(policy: EffectivePolicy) -> Self {
        Self {
            policy,
            store: ApprovalStore::new(),
        }
    }

    /// Check file access.
    ///
    /// - In-workspace: delegates to `EffectivePolicy::check_file_access`.
    /// - Out-of-workspace: checks the approval store; returns
    ///   `approval_required:file_access:<path>` if no prior decision exists.
    pub fn check_file_access(&self, path: &Path, write: bool) -> Result<PathBuf, CoreError> {
        // Capability check first — policy denial is unconditional.
        if !self.policy.capabilities.file_read {
            return Err(CoreError::Tool("file_read capability denied".into()));
        }
        if write && !self.policy.capabilities.file_write {
            return Err(CoreError::Tool("file_write capability denied".into()));
        }

        match check_path_within_workspace(path, &self.policy.workspace_root) {
            Ok(canonical) => Ok(canonical),
            Err(e) => {
                // Distinguish a hard escape (../ traversal) from a simple outside-workspace path.
                // check_path_within_workspace returns "escapes workspace" for both; we treat
                // dotdot/symlink escapes as hard denials and plain outside paths as approvable.
                let msg = e.to_string();
                if msg.contains("escapes workspace") {
                    // Check whether the path itself is a dotdot/symlink escape or just outside.
                    // We re-examine: if the path resolves cleanly (exists or parent exists) but
                    // is simply not under workspace_root, it is approvable.
                    // If it contains ".." components that resolve to an escape, deny hard.
                    let has_dotdot = path
                        .components()
                        .any(|c| c == std::path::Component::ParentDir);
                    if has_dotdot {
                        return Err(CoreError::Tool(format!(
                            "denied: path '{}' uses '..' traversal",
                            path.display()
                        )));
                    }

                    // Plain outside-workspace path — check approval store.
                    let action = ApprovalAction::FileAccess {
                        path: path.to_path_buf(),
                        write,
                    };
                    self.check_approval(&action, || {
                        format!("approval_required:file_access:{}", path.display())
                    })?;

                    // Approved — return the path as-is (best-effort canonicalization).
                    let canonical = if path.exists() {
                        path.canonicalize().map_err(|io| {
                            CoreError::Tool(format!(
                                "cannot canonicalize '{}': {io}",
                                path.display()
                            ))
                        })?
                    } else {
                        path.to_path_buf()
                    };
                    Ok(canonical)
                } else {
                    Err(e)
                }
            }
        }
    }

    /// Check command execution.
    ///
    /// - Policy denial is unconditional.
    /// - If `cwd` is outside the workspace, requires approval.
    pub fn check_cmd_exec(&self, cmd: &str, cwd: Option<&Path>) -> Result<(), CoreError> {
        self.policy.check_cmd_exec()?;

        if let Some(cwd_path) = cwd
            && check_path_within_workspace(cwd_path, &self.policy.workspace_root).is_err()
        {
            let action = ApprovalAction::CmdExec {
                cmd: cmd.to_owned(),
                cwd: Some(cwd_path.to_path_buf()),
            };
            self.check_approval(&action, || format!("approval_required:cmd_exec:{cmd}"))?;
        }

        Ok(())
    }

    /// Check process spawn.
    ///
    /// Always requires approval unless a session-scoped approval already exists.
    pub fn check_spawn(&self, program: &str) -> Result<(), CoreError> {
        self.policy.check_spawn()?;

        let action = ApprovalAction::ProcessSpawn {
            program: program.to_owned(),
        };
        self.check_approval(&action, || format!("approval_required:spawn:{program}"))
    }

    /// Check network access per policy capabilities.
    pub fn check_network(&self) -> Result<(), CoreError> {
        self.policy.check_network()
    }

    /// Record an approval decision (called after the user responds to a prompt).
    pub fn approve(&self, action: ApprovalAction, decision: ApprovalDecision, reason: &str) {
        self.store.record(action, decision, reason);
    }

    /// Consult the approval store for `action`.
    ///
    /// - Session-approved → `Ok(())`.
    /// - Denied → `Err(CoreError::Tool("denied: …"))`.
    /// - No prior decision → `Err(CoreError::Tool(<approval_required_msg>))`.
    fn check_approval(
        &self,
        action: &ApprovalAction,
        approval_required_msg: impl FnOnce() -> String,
    ) -> Result<(), CoreError> {
        match self.store.lookup(action) {
            Some(ApprovalDecision::Approved(_)) => Ok(()),
            Some(ApprovalDecision::Denied) => Err(CoreError::Tool(format!("denied: {:?}", action))),
            None => Err(CoreError::Tool(approval_required_msg())),
        }
    }
}
