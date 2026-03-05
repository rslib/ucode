use serde::{Deserialize, Serialize};

/// Discriminates authentication failure modes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthErrorKind {
    Missing,
    Invalid,
    Expired,
}

/// Canonical error type for ucode-core operations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum CoreError {
    #[error("provider '{provider}' error: {message}")]
    Provider { provider: String, message: String },

    #[error("auth error for provider '{provider}': {auth_kind:?}")]
    Auth {
        provider: String,
        auth_kind: AuthErrorKind,
    },

    #[error("context too large: limit {limit}, actual {actual}")]
    ContextTooLarge { limit: usize, actual: usize },

    #[error("tool '{tool}' failed: {message}")]
    ToolFailed { tool: String, message: String },

    #[error("operation '{operation}' timed out after {duration_ms}ms")]
    Timeout { operation: String, duration_ms: u64 },

    #[error("internal error: {message}")]
    Internal { message: String },
}
