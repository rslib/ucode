use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::agent::AgentId;

/// A message sent between agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessage {
    pub from: AgentId,
    pub to: AgentId,
    pub payload: serde_json::Value,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Policy controlling inter-agent communication.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum CommPolicy {
    #[default]
    Disabled,
    Enabled {
        /// Maximum allowed serialized payload size in bytes.
        max_message_size: usize,
    },
}

/// Errors produced by [`CommBus`] operations.
#[derive(Debug, thiserror::Error)]
pub enum CommError {
    #[error("inter-agent communication is disabled by policy")]
    Disabled,
    #[error("agent '{0}' not registered")]
    AgentNotFound(AgentId),
    #[error("message exceeds size limit ({size} > {limit} bytes)")]
    MessageTooLarge { size: usize, limit: usize },
    #[error("board operation failed: {0}")]
    BoardError(String),
}

/// Communication bus for inter-agent mailbox messaging and shared board access.
///
/// All operations are gated by [`CommPolicy`]. When the policy is
/// [`CommPolicy::Disabled`], every mutating operation returns
/// [`CommError::Disabled`]; read-only board reads also return the same error so
/// callers cannot observe stale state from a previous enabled session.
pub struct CommBus {
    policy: CommPolicy,
    mailboxes: Arc<Mutex<HashMap<AgentId, Vec<AgentMessage>>>>,
    board: Arc<Mutex<HashMap<String, serde_json::Value>>>,
    audit_log: Arc<Mutex<Vec<AgentMessage>>>,
}

impl CommBus {
    pub fn new(policy: CommPolicy) -> Self {
        Self {
            policy,
            mailboxes: Arc::new(Mutex::new(HashMap::new())),
            board: Arc::new(Mutex::new(HashMap::new())),
            audit_log: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Register a mailbox for `agent_id`. Called when an agent is spawned.
    ///
    /// Registering an already-registered agent is a no-op (idempotent).
    pub async fn register(&self, agent_id: &AgentId) {
        let mut mailboxes = self.mailboxes.lock().await;
        mailboxes.entry(agent_id.clone()).or_default();
    }

    /// Unregister a mailbox. Called when an agent completes or fails.
    ///
    /// Any pending messages in the mailbox are discarded.
    pub async fn unregister(&self, agent_id: &AgentId) {
        let mut mailboxes = self.mailboxes.lock().await;
        mailboxes.remove(agent_id);
    }

    /// Send `payload` from `from` to `to`'s mailbox.
    ///
    /// Returns [`CommError::Disabled`] when the policy is disabled,
    /// [`CommError::AgentNotFound`] when `to` has no registered mailbox, and
    /// [`CommError::MessageTooLarge`] when the serialized payload exceeds the
    /// configured limit.
    pub async fn send(
        &self,
        from: &AgentId,
        to: &AgentId,
        payload: serde_json::Value,
    ) -> Result<(), CommError> {
        let max_size = match &self.policy {
            CommPolicy::Disabled => return Err(CommError::Disabled),
            CommPolicy::Enabled { max_message_size } => *max_message_size,
        };

        let serialized =
            serde_json::to_vec(&payload).map_err(|e| CommError::BoardError(e.to_string()))?;
        if serialized.len() > max_size {
            return Err(CommError::MessageTooLarge {
                size: serialized.len(),
                limit: max_size,
            });
        }

        let msg = AgentMessage {
            from: from.clone(),
            to: to.clone(),
            payload,
            timestamp: chrono::Utc::now(),
        };

        {
            let mut mailboxes = self.mailboxes.lock().await;
            let inbox = mailboxes
                .get_mut(to)
                .ok_or_else(|| CommError::AgentNotFound(to.clone()))?;
            inbox.push(msg.clone());
        }

        self.audit_log.lock().await.push(msg);
        Ok(())
    }

    /// Drain and return all pending messages for `agent_id`.
    ///
    /// Returns [`CommError::AgentNotFound`] when the agent has no registered
    /// mailbox (regardless of policy — receiving is always allowed when
    /// registered).
    pub async fn recv(&self, agent_id: &AgentId) -> Result<Vec<AgentMessage>, CommError> {
        let mut mailboxes = self.mailboxes.lock().await;
        let inbox = mailboxes
            .get_mut(agent_id)
            .ok_or_else(|| CommError::AgentNotFound(agent_id.clone()))?;
        Ok(std::mem::take(inbox))
    }

    /// Return the number of pending messages without draining the mailbox.
    ///
    /// Returns [`CommError::AgentNotFound`] when the agent is not registered.
    pub async fn pending_count(&self, agent_id: &AgentId) -> Result<usize, CommError> {
        let mailboxes = self.mailboxes.lock().await;
        mailboxes
            .get(agent_id)
            .map(|inbox| inbox.len())
            .ok_or_else(|| CommError::AgentNotFound(agent_id.clone()))
    }

    /// Write `value` under `key` on the shared context board.
    ///
    /// Returns [`CommError::Disabled`] when the policy is disabled.
    pub async fn board_write(
        &self,
        key: String,
        value: serde_json::Value,
    ) -> Result<(), CommError> {
        if self.policy == CommPolicy::Disabled {
            return Err(CommError::Disabled);
        }
        self.board.lock().await.insert(key, value);
        Ok(())
    }

    /// Read the value stored under `key` from the shared context board.
    ///
    /// Returns `Ok(None)` when the key is absent. Returns
    /// [`CommError::Disabled`] when the policy is disabled.
    pub async fn board_read(&self, key: &str) -> Result<Option<serde_json::Value>, CommError> {
        if self.policy == CommPolicy::Disabled {
            return Err(CommError::Disabled);
        }
        Ok(self.board.lock().await.get(key).cloned())
    }

    /// Return a snapshot of the full audit log (all messages ever sent).
    pub async fn audit_log(&self) -> Vec<AgentMessage> {
        self.audit_log.lock().await.clone()
    }
}
