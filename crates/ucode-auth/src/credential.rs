use std::collections::HashMap;
use std::sync::Mutex;

use clap::ValueEnum;
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
}

/// Known provider identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum ProviderId {
    OpenAi,
    Anthropic,
    Ollama,
}

impl ProviderId {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Ollama => "ollama",
        }
    }
}

impl std::fmt::Display for ProviderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ProviderId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "openai" => Ok(Self::OpenAi),
            "anthropic" => Ok(Self::Anthropic),
            "ollama" => Ok(Self::Ollama),
            other => Err(format!("unknown provider: {other}")),
        }
    }
}

/// Status of a provider's credentials.
#[derive(Debug, Clone, PartialEq)]
pub enum CredentialStatus {
    Configured { provider: ProviderId, kind: String },
    NotConfigured { provider: ProviderId },
}

/// Backend for storing and retrieving credentials.
pub trait CredentialStore: Send + Sync {
    fn store(&self, provider: ProviderId, material: &AuthMaterial) -> Result<(), AuthError>;
    fn load(&self, provider: ProviderId) -> Result<AuthMaterial, AuthError>;
    fn delete(&self, provider: ProviderId) -> Result<(), AuthError>;
    fn status(&self, provider: ProviderId) -> CredentialStatus;
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
    data: Mutex<HashMap<ProviderId, AuthMaterial>>,
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
    fn store(&self, provider: ProviderId, material: &AuthMaterial) -> Result<(), AuthError> {
        self.data.lock().unwrap().insert(provider, material.clone());
        Ok(())
    }

    fn load(&self, provider: ProviderId) -> Result<AuthMaterial, AuthError> {
        self.data
            .lock()
            .unwrap()
            .get(&provider)
            .cloned()
            .ok_or_else(|| AuthError::NotFound {
                provider: provider.to_string(),
            })
    }

    fn delete(&self, provider: ProviderId) -> Result<(), AuthError> {
        let removed = self.data.lock().unwrap().remove(&provider);
        removed.map(|_| ()).ok_or_else(|| AuthError::NotFound {
            provider: provider.to_string(),
        })
    }

    fn status(&self, provider: ProviderId) -> CredentialStatus {
        match self.load(provider) {
            Ok(mat) => {
                let kind = material_kind(&mat);
                CredentialStatus::Configured {
                    provider,
                    kind: kind.into(),
                }
            }
            Err(_) => CredentialStatus::NotConfigured { provider },
        }
    }

    fn list_configured(&self) -> Vec<CredentialStatus> {
        all_providers().map(|p| self.status(p)).collect()
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

    fn entry(&self, provider: ProviderId) -> Result<keyring::Entry, AuthError> {
        keyring::Entry::new(&self.service_name, provider.as_str()).map_err(|e| AuthError::Keyring {
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
    fn store(&self, provider: ProviderId, material: &AuthMaterial) -> Result<(), AuthError> {
        let json = serde_json::to_string(material).map_err(|e| AuthError::Serialization {
            message: e.to_string(),
        })?;
        self.entry(provider)?
            .set_password(&json)
            .map_err(|e| AuthError::Keyring {
                message: e.to_string(),
            })
    }

    fn load(&self, provider: ProviderId) -> Result<AuthMaterial, AuthError> {
        let json = self.entry(provider)?.get_password().map_err(|e| match e {
            keyring::Error::NoEntry => AuthError::NotFound {
                provider: provider.to_string(),
            },
            other => AuthError::Keyring {
                message: other.to_string(),
            },
        })?;
        serde_json::from_str(&json).map_err(|e| AuthError::Serialization {
            message: e.to_string(),
        })
    }

    fn delete(&self, provider: ProviderId) -> Result<(), AuthError> {
        self.entry(provider)?
            .delete_credential()
            .map_err(|e| match e {
                keyring::Error::NoEntry => AuthError::NotFound {
                    provider: provider.to_string(),
                },
                other => AuthError::Keyring {
                    message: other.to_string(),
                },
            })
    }

    fn status(&self, provider: ProviderId) -> CredentialStatus {
        match self.load(provider) {
            Ok(mat) => CredentialStatus::Configured {
                provider,
                kind: material_kind(&mat).into(),
            },
            Err(_) => CredentialStatus::NotConfigured { provider },
        }
    }

    fn list_configured(&self) -> Vec<CredentialStatus> {
        all_providers().map(|p| self.status(p)).collect()
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn material_kind(mat: &AuthMaterial) -> &'static str {
    match mat {
        AuthMaterial::ApiKey { .. } => "api_key",
        AuthMaterial::OAuth { .. } => "oauth",
        AuthMaterial::SessionToken { .. } => "session_token",
    }
}

pub fn all_providers() -> impl Iterator<Item = ProviderId> {
    [
        ProviderId::OpenAi,
        ProviderId::Anthropic,
        ProviderId::Ollama,
    ]
    .into_iter()
}
