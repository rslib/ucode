use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::error::AuthError;

/// The kind of credential material stored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum AuthMaterial {
    ApiKey {
        key: String,
    },
    OAuth {
        access_token: String,
        refresh_token: Option<String>,
        /// ISO 8601 timestamp.
        expires_at: Option<String>,
    },
    SessionToken {
        token: String,
        /// ISO 8601 timestamp.
        expires_at: Option<String>,
    },
    WellKnown {
        /// Env var name the provider expects (e.g., "CUSTOM_API_KEY").
        env_key: String,
        /// The actual token value.
        token: String,
    },
    AwsCredentials {
        access_key_id: String,
        secret_access_key: String,
        session_token: Option<String>,
        region: String,
    },
}

/// The protocol adapter type. Selected by the `type` field in TOML config.
/// This is NOT the provider identity — provider IDs are arbitrary strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    OpenAi,
    Anthropic,
    Ollama,
    Gemini,
}

impl ProviderType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Ollama => "ollama",
            Self::Gemini => "gemini",
        }
    }
}

impl std::fmt::Display for ProviderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ProviderType {
    type Err = AuthError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "openai" => Ok(Self::OpenAi),
            "anthropic" => Ok(Self::Anthropic),
            "ollama" => Ok(Self::Ollama),
            "gemini" => Ok(Self::Gemini),
            other => Err(AuthError::InvalidProvider {
                name: other.to_owned(),
            }),
        }
    }
}

/// Status of a provider's credentials.
#[derive(Debug, Clone, PartialEq)]
pub enum CredentialStatus {
    Configured { provider: String, kind: String },
    NotConfigured { provider: String },
}

/// Backend for storing and retrieving credentials.
pub trait CredentialStore: Send + Sync {
    fn store(&self, provider: &str, material: &AuthMaterial) -> Result<(), AuthError>;
    fn load(&self, provider: &str) -> Result<AuthMaterial, AuthError>;
    fn delete(&self, provider: &str) -> Result<(), AuthError>;
    fn status(&self, provider: &str) -> CredentialStatus;
    fn list_configured(&self) -> Vec<CredentialStatus>;
}

/// Redact a secret string for safe logging. Shows first 4 and last 4 chars.
pub fn redact(secret: &str) -> String {
    if secret.len() <= 12 {
        return "****".into();
    }
    format!("{}...{}", &secret[..4], &secret[secret.len() - 4..])
}

// ── InMemoryStore ─────────────────────────────────────────────────────────────

/// In-memory credential store for tests and fallback environments.
pub struct InMemoryStore {
    data: Mutex<HashMap<String, AuthMaterial>>,
}

impl InMemoryStore {
    pub fn new() -> Self {
        Self {
            data: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl CredentialStore for InMemoryStore {
    fn store(&self, provider: &str, material: &AuthMaterial) -> Result<(), AuthError> {
        self.data
            .lock()
            .unwrap()
            .insert(provider.to_owned(), material.clone());
        Ok(())
    }

    fn load(&self, provider: &str) -> Result<AuthMaterial, AuthError> {
        self.data
            .lock()
            .unwrap()
            .get(provider)
            .cloned()
            .ok_or_else(|| AuthError::NotFound {
                provider: provider.to_owned(),
            })
    }

    fn delete(&self, provider: &str) -> Result<(), AuthError> {
        let removed = self.data.lock().unwrap().remove(provider);
        removed.map(|_| ()).ok_or_else(|| AuthError::NotFound {
            provider: provider.to_owned(),
        })
    }

    fn status(&self, provider: &str) -> CredentialStatus {
        match self.load(provider) {
            Ok(mat) => CredentialStatus::Configured {
                provider: provider.to_owned(),
                kind: material_kind(&mat).into(),
            },
            Err(_) => CredentialStatus::NotConfigured {
                provider: provider.to_owned(),
            },
        }
    }

    fn list_configured(&self) -> Vec<CredentialStatus> {
        self.data
            .lock()
            .unwrap()
            .iter()
            .map(|(id, mat)| CredentialStatus::Configured {
                provider: id.clone(),
                kind: material_kind(mat).into(),
            })
            .collect()
    }
}

// ── KeyringStore ──────────────────────────────────────────────────────────────

/// Production credential store backed by the OS keychain.
pub struct KeyringStore {
    service_name: String,
}

impl KeyringStore {
    pub fn new() -> Self {
        Self {
            service_name: "ucode".into(),
        }
    }

    fn entry(&self, provider: &str) -> Result<keyring::Entry, AuthError> {
        keyring::Entry::new(&self.service_name, provider).map_err(|e| AuthError::Keyring {
            message: e.to_string(),
        })
    }
}

impl Default for KeyringStore {
    fn default() -> Self {
        Self::new()
    }
}

impl CredentialStore for KeyringStore {
    fn store(&self, provider: &str, material: &AuthMaterial) -> Result<(), AuthError> {
        let json = serde_json::to_string(material).map_err(|e| AuthError::Serialization {
            message: e.to_string(),
        })?;
        self.entry(provider)?
            .set_password(&json)
            .map_err(|e| AuthError::Keyring {
                message: e.to_string(),
            })
    }

    fn load(&self, provider: &str) -> Result<AuthMaterial, AuthError> {
        let json = self.entry(provider)?.get_password().map_err(|e| match e {
            keyring::Error::NoEntry => AuthError::NotFound {
                provider: provider.to_owned(),
            },
            other => AuthError::Keyring {
                message: other.to_string(),
            },
        })?;
        serde_json::from_str(&json).map_err(|e| AuthError::Serialization {
            message: e.to_string(),
        })
    }

    fn delete(&self, provider: &str) -> Result<(), AuthError> {
        self.entry(provider)?
            .delete_credential()
            .map_err(|e| match e {
                keyring::Error::NoEntry => AuthError::NotFound {
                    provider: provider.to_owned(),
                },
                other => AuthError::Keyring {
                    message: other.to_string(),
                },
            })
    }

    fn status(&self, provider: &str) -> CredentialStatus {
        match self.load(provider) {
            Ok(mat) => CredentialStatus::Configured {
                provider: provider.to_owned(),
                kind: material_kind(&mat).into(),
            },
            Err(_) => CredentialStatus::NotConfigured {
                provider: provider.to_owned(),
            },
        }
    }

    fn list_configured(&self) -> Vec<CredentialStatus> {
        // KeyringStore cannot enumerate — return empty.
        // Use ChainStore or config-driven listing instead.
        Vec::new()
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

pub(crate) fn material_kind(mat: &AuthMaterial) -> &'static str {
    match mat {
        AuthMaterial::ApiKey { .. } => "api_key",
        AuthMaterial::OAuth { .. } => "oauth",
        AuthMaterial::SessionToken { .. } => "session_token",
        AuthMaterial::WellKnown { .. } => "wellknown",
        AuthMaterial::AwsCredentials { .. } => "aws_credentials",
    }
}

// ── AuthMaterial expiry helpers ───────────────────────────────────────────────

impl AuthMaterial {
    /// Returns the expiry timestamp string if this material type has one.
    pub fn expires_at(&self) -> Option<&str> {
        match self {
            Self::OAuth { expires_at, .. } => expires_at.as_deref(),
            Self::SessionToken { expires_at, .. } => expires_at.as_deref(),
            _ => None,
        }
    }

    /// Returns true if the token has expired (past the expiry time).
    /// Returns false if there is no expiry or the timestamp can't be parsed.
    pub fn is_expired(&self) -> bool {
        self.expires_at()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .is_some_and(|exp| exp <= chrono::Utc::now())
    }

    /// Returns true if the token expires within the given duration.
    /// Returns false if there is no expiry or the timestamp can't be parsed.
    pub fn expires_within(&self, duration: chrono::Duration) -> bool {
        self.expires_at()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .is_some_and(|exp| exp <= chrono::Utc::now() + duration)
    }

    /// Returns the refresh token if this is an OAuth credential.
    pub fn refresh_token(&self) -> Option<&str> {
        match self {
            Self::OAuth { refresh_token, .. } => refresh_token.as_deref(),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expires_at_returns_oauth_expiry() {
        let mat = AuthMaterial::OAuth {
            access_token: "tok".into(),
            refresh_token: None,
            expires_at: Some("2026-01-01T00:00:00Z".into()),
        };
        assert_eq!(mat.expires_at(), Some("2026-01-01T00:00:00Z"));
    }

    #[test]
    fn expires_at_returns_session_expiry() {
        let mat = AuthMaterial::SessionToken {
            token: "tok".into(),
            expires_at: Some("2026-06-01T00:00:00Z".into()),
        };
        assert_eq!(mat.expires_at(), Some("2026-06-01T00:00:00Z"));
    }

    #[test]
    fn expires_at_returns_none_for_api_key() {
        let mat = AuthMaterial::ApiKey { key: "k".into() };
        assert_eq!(mat.expires_at(), None);
    }

    #[test]
    fn is_expired_past_date() {
        let mat = AuthMaterial::OAuth {
            access_token: "tok".into(),
            refresh_token: None,
            expires_at: Some("2020-01-01T00:00:00Z".into()),
        };
        assert!(mat.is_expired());
    }

    #[test]
    fn is_expired_future_date() {
        let mat = AuthMaterial::OAuth {
            access_token: "tok".into(),
            refresh_token: None,
            expires_at: Some("2099-01-01T00:00:00Z".into()),
        };
        assert!(!mat.is_expired());
    }

    #[test]
    fn is_expired_no_expiry() {
        let mat = AuthMaterial::OAuth {
            access_token: "tok".into(),
            refresh_token: None,
            expires_at: None,
        };
        assert!(!mat.is_expired());
    }

    #[test]
    fn expires_within_soon() {
        let soon = (chrono::Utc::now() + chrono::Duration::seconds(60)).to_rfc3339();
        let mat = AuthMaterial::OAuth {
            access_token: "tok".into(),
            refresh_token: None,
            expires_at: Some(soon),
        };
        assert!(mat.expires_within(chrono::Duration::minutes(5)));
    }

    #[test]
    fn expires_within_not_soon() {
        let later = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
        let mat = AuthMaterial::OAuth {
            access_token: "tok".into(),
            refresh_token: None,
            expires_at: Some(later),
        };
        assert!(!mat.expires_within(chrono::Duration::minutes(5)));
    }

    #[test]
    fn refresh_token_present() {
        let mat = AuthMaterial::OAuth {
            access_token: "tok".into(),
            refresh_token: Some("ref".into()),
            expires_at: None,
        };
        assert_eq!(mat.refresh_token(), Some("ref"));
    }

    #[test]
    fn refresh_token_absent_on_api_key() {
        let mat = AuthMaterial::ApiKey { key: "k".into() };
        assert_eq!(mat.refresh_token(), None);
    }
}
