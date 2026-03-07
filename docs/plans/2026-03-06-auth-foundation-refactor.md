# Auth Foundation Refactor (Task 2.2) Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Refactor ucode-auth to support arbitrary string-based provider IDs, expand AuthMaterial variants, add FileStore + ChainStore, and implement auth precedence resolution.

**Architecture:** Replace the fixed `ProviderId` enum with `&str`-based identifiers throughout the `CredentialStore` trait and all implementations. Add `ProviderType` enum for adapter selection (separate concern from provider identity). Add `FileStore` (JSON file at `~/.local/share/ucode/auth.json` with 0o600 permissions) and `ChainStore` (keyring-first, file-fallback). Implement `resolve_auth()` with env var > stored credential > prompt precedence.

**Tech Stack:** Rust, serde_json, keyring 3, thiserror, clap 4, std::fs (for FileStore), dirs (for XDG paths)

---

## Task 1: Expand AuthMaterial enum with new variants

**Files:**
- Modify: `crates/ucode-auth/src/credential.rs:10-27`
- Modify: `crates/ucode-auth/src/credential.rs:237-243` (material_kind helper)
- Test: `crates/ucode-auth/tests/credential_tests.rs`

**Step 1: Write failing tests for new AuthMaterial variants**

Add to `crates/ucode-auth/tests/credential_tests.rs`:

```rust
fn wellknown(env_key: &str, token: &str) -> AuthMaterial {
    AuthMaterial::WellKnown {
        env_key: env_key.into(),
        token: token.into(),
    }
}

fn aws_creds() -> AuthMaterial {
    AuthMaterial::AwsCredentials {
        access_key_id: "AKIAIOSFODNN7EXAMPLE".into(),
        secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
        session_token: Some("FwoGZXIvYXdzEBY...".into()),
        region: "us-east-1".into(),
    }
}
```

Add these test functions:

```rust
#[test]
fn store_and_load_wellknown() {
    let store = InMemoryStore::new();
    let mat = wellknown("CUSTOM_API_KEY", "tok-abc-123");
    store.store("custom-provider", &mat).unwrap();
    assert_eq!(store.load("custom-provider").unwrap(), mat);
}

#[test]
fn store_and_load_aws_credentials() {
    let store = InMemoryStore::new();
    let mat = aws_creds();
    store.store("aws-bedrock", &mat).unwrap();
    assert_eq!(store.load("aws-bedrock").unwrap(), mat);
}

#[test]
fn auth_material_serde_roundtrip_new_variants() {
    let cases = [
        wellknown("MY_KEY", "secret-value"),
        aws_creds(),
        AuthMaterial::AwsCredentials {
            access_key_id: "AKIA".into(),
            secret_access_key: "secret".into(),
            session_token: None,
            region: "eu-west-1".into(),
        },
    ];

    for mat in &cases {
        let json = serde_json::to_string(mat).expect("serialize");
        let back: AuthMaterial = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(&back, mat);
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p ucode-auth`
Expected: FAIL — `WellKnown` and `AwsCredentials` variants don't exist yet, and store/load signatures don't accept `&str`.

**Step 3: Add WellKnown and AwsCredentials variants to AuthMaterial**

In `crates/ucode-auth/src/credential.rs`, update the `AuthMaterial` enum:

```rust
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
```

Update `material_kind` helper:

```rust
fn material_kind(mat: &AuthMaterial) -> &'static str {
    match mat {
        AuthMaterial::ApiKey { .. } => "api_key",
        AuthMaterial::OAuth { .. } => "oauth",
        AuthMaterial::SessionToken { .. } => "session_token",
        AuthMaterial::WellKnown { .. } => "wellknown",
        AuthMaterial::AwsCredentials { .. } => "aws_credentials",
    }
}
```

**Step 4: Run tests to verify new variants serialize/deserialize**

Note: Tests will still fail because of `&str` vs `ProviderId` mismatch — that's Task 2. But the serde roundtrip test should pass if run in isolation.

Run: `cargo test -p ucode-auth auth_material_serde_roundtrip_new_variants` (will fail until Task 2 completes — that's expected)

**Step 5: Do NOT commit yet — continue to Task 2 (these changes are coupled)**

---

## Task 2: Replace ProviderId enum with String-based identifiers

**Files:**
- Modify: `crates/ucode-auth/src/credential.rs` (entire file — trait, InMemoryStore, KeyringStore)
- Modify: `crates/ucode-auth/src/lib.rs` (exports)
- Modify: `crates/ucode-auth/tests/credential_tests.rs` (all tests)
- Modify: `crates/ucode-cli/src/cmd_auth.rs` (CLI argument type)
- Modify: `crates/ucode-cli/src/auth_handler.rs` (handler signatures + tests)

**Step 1: Refactor CredentialStore trait to use `&str`**

Replace the entire `CredentialStore` trait in `crates/ucode-auth/src/credential.rs`:

```rust
/// Backend for storing and retrieving credentials.
pub trait CredentialStore: Send + Sync {
    fn store(&self, provider: &str, material: &AuthMaterial) -> Result<(), AuthError>;
    fn load(&self, provider: &str) -> Result<AuthMaterial, AuthError>;
    fn delete(&self, provider: &str) -> Result<(), AuthError>;
    fn status(&self, provider: &str) -> CredentialStatus;
    fn list_configured(&self) -> Vec<CredentialStatus>;
}
```

**Step 2: Update CredentialStatus to use String**

```rust
/// Status of a provider's credentials.
#[derive(Debug, Clone, PartialEq)]
pub enum CredentialStatus {
    Configured { provider: String, kind: String },
    NotConfigured { provider: String },
}
```

**Step 3: Refactor InMemoryStore to use String keys**

```rust
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
```

**Step 4: Refactor KeyringStore to use `&str`**

```rust
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
```

**Step 5: Remove ProviderId enum and all_providers function**

Delete the `ProviderId` enum, its `impl` blocks (`as_str`, `Display`, `FromStr`), and the `all_providers()` function from `credential.rs`.

**Step 6: Add ProviderType enum**

Add to `crates/ucode-auth/src/credential.rs`:

```rust
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
```

**Step 7: Update lib.rs exports**

```rust
//! ucode-auth: keychain, login flows, token refresh

pub mod credential;
pub mod error;

pub use credential::{
    AuthMaterial, CredentialStatus, CredentialStore, InMemoryStore, KeyringStore, ProviderType,
    redact,
};
pub use error::AuthError;
```

**Step 8: Update error.rs with new variants**

```rust
/// Errors produced by credential store operations.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("no credentials found for provider '{provider}'")]
    NotFound { provider: String },

    #[error("keyring error: {message}")]
    Keyring { message: String },

    #[error("serialization error: {message}")]
    Serialization { message: String },

    #[error("unknown provider type: '{name}'")]
    InvalidProvider { name: String },

    #[error("file store error: {message}")]
    FileStore { message: String },

    #[error("missing credential for provider '{provider}': {detail}")]
    MissingCredential { provider: String, detail: String },
}
```

**Step 9: Update CLI cmd_auth.rs — provider is now a String argument**

```rust
use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum AuthCommand {
    /// Show credential status for all providers.
    Status,

    /// Store an API key for a provider (reads from stdin).
    SetKey {
        /// Provider name (e.g., "openai", "anthropic", "my-custom-proxy").
        provider: String,
    },

    /// Delete stored credentials for a provider.
    Logout {
        /// Provider name.
        provider: String,
    },

    /// Initiate a login flow for a provider (stub).
    Login {
        /// Provider name.
        provider: String,

        /// Use device-code flow.
        #[arg(long)]
        device: bool,

        /// Use subscription-based login.
        #[arg(long)]
        subscription: bool,
    },
}
```

**Step 10: Update CLI auth_handler.rs — use `&str` for provider**

```rust
use std::io::{self, BufRead, Write};

use anyhow::Result;
use ucode_auth::{AuthError, AuthMaterial, CredentialStatus, CredentialStore};

pub fn handle_status(store: &dyn CredentialStore) -> Result<()> {
    for status in store.list_configured() {
        match status {
            CredentialStatus::Configured { provider, kind } => {
                println!("{provider}: configured ({kind})");
            }
            CredentialStatus::NotConfigured { provider } => {
                println!("{provider}: not configured");
            }
        }
    }
    Ok(())
}

pub fn handle_set_key(store: &dyn CredentialStore, provider: &str) -> Result<()> {
    print!("Enter API key for {provider}: ");
    io::stdout().flush()?;

    let mut key = String::new();
    io::stdin().lock().read_line(&mut key)?;
    let key = key.trim().to_owned();

    store.store(provider, &AuthMaterial::ApiKey { key })?;
    println!("API key for {provider} stored successfully.");
    Ok(())
}

pub fn handle_logout(store: &dyn CredentialStore, provider: &str) -> Result<()> {
    match store.delete(provider) {
        Ok(()) => println!("Logged out from {provider}."),
        Err(AuthError::NotFound { .. }) => {
            println!("No credentials found for {provider}.");
        }
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

pub fn handle_login(
    _store: &dyn CredentialStore,
    provider: &str,
    device: bool,
    subscription: bool,
) -> Result<()> {
    println!("Login flow for {provider} is not yet implemented.");
    if device {
        println!("  (device-code flow requested)");
    }
    if subscription {
        println!("  (subscription login requested)");
    }
    Ok(())
}
```

**Step 11: Update all tests in credential_tests.rs**

Replace the entire test file. Key changes:
- Remove `ProviderId` import
- Use string literals `"openai"`, `"anthropic"`, `"ollama"` etc.
- Remove `provider_id_display`, `provider_id_from_str_valid`, `provider_id_from_str_invalid` tests
- Add `provider_type_from_str` tests instead
- Update `list_configured` test (no longer returns NotConfigured for unknown providers)
- Add tests for arbitrary provider names like `"my-custom-proxy"`

```rust
use ucode_auth::{
    AuthError, AuthMaterial, CredentialStatus, CredentialStore, InMemoryStore, ProviderType, redact,
};

fn api_key(key: &str) -> AuthMaterial {
    AuthMaterial::ApiKey { key: key.into() }
}

fn oauth(access: &str) -> AuthMaterial {
    AuthMaterial::OAuth {
        access_token: access.into(),
        refresh_token: Some("refresh-xyz".into()),
        expires_at: Some("2026-01-01T00:00:00Z".into()),
    }
}

fn session(token: &str) -> AuthMaterial {
    AuthMaterial::SessionToken {
        token: token.into(),
        expires_at: Some("2026-06-01T00:00:00Z".into()),
    }
}

fn wellknown(env_key: &str, token: &str) -> AuthMaterial {
    AuthMaterial::WellKnown {
        env_key: env_key.into(),
        token: token.into(),
    }
}

fn aws_creds() -> AuthMaterial {
    AuthMaterial::AwsCredentials {
        access_key_id: "AKIAIOSFODNN7EXAMPLE".into(),
        secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
        session_token: Some("FwoGZXIvYXdzEBY...".into()),
        region: "us-east-1".into(),
    }
}

// ── Store / Load ─────────────────────────────────────────────────────────────

#[test]
fn store_and_load_api_key() {
    let store = InMemoryStore::new();
    let mat = api_key("sk-test-key");
    store.store("openai", &mat).unwrap();
    assert_eq!(store.load("openai").unwrap(), mat);
}

#[test]
fn store_and_load_oauth() {
    let store = InMemoryStore::new();
    let mat = oauth("access-token-abc");
    store.store("anthropic", &mat).unwrap();
    assert_eq!(store.load("anthropic").unwrap(), mat);
}

#[test]
fn store_and_load_session_token() {
    let store = InMemoryStore::new();
    let mat = session("sess-token-xyz");
    store.store("ollama", &mat).unwrap();
    assert_eq!(store.load("ollama").unwrap(), mat);
}

#[test]
fn store_and_load_wellknown() {
    let store = InMemoryStore::new();
    let mat = wellknown("CUSTOM_API_KEY", "tok-abc-123");
    store.store("custom-provider", &mat).unwrap();
    assert_eq!(store.load("custom-provider").unwrap(), mat);
}

#[test]
fn store_and_load_aws_credentials() {
    let store = InMemoryStore::new();
    let mat = aws_creds();
    store.store("aws-bedrock", &mat).unwrap();
    assert_eq!(store.load("aws-bedrock").unwrap(), mat);
}

#[test]
fn arbitrary_provider_name() {
    let store = InMemoryStore::new();
    let mat = api_key("sk-custom");
    store.store("my-custom-proxy", &mat).unwrap();
    assert_eq!(store.load("my-custom-proxy").unwrap(), mat);
}

#[test]
fn load_not_found() {
    let store = InMemoryStore::new();
    let err = store.load("openai").unwrap_err();
    assert!(matches!(err, AuthError::NotFound { .. }));
}

#[test]
fn delete_credential() {
    let store = InMemoryStore::new();
    store.store("openai", &api_key("key")).unwrap();
    store.delete("openai").unwrap();
    let err = store.load("openai").unwrap_err();
    assert!(matches!(err, AuthError::NotFound { .. }));
}

#[test]
fn delete_not_found() {
    let store = InMemoryStore::new();
    let err = store.delete("openai").unwrap_err();
    assert!(matches!(err, AuthError::NotFound { .. }));
}

#[test]
fn status_configured() {
    let store = InMemoryStore::new();
    store.store("openai", &api_key("key")).unwrap();
    let s = store.status("openai");
    assert!(
        matches!(s, CredentialStatus::Configured { ref provider, ref kind } if provider == "openai" && kind == "api_key")
    );
}

#[test]
fn status_not_configured() {
    let store = InMemoryStore::new();
    assert_eq!(
        store.status("anthropic"),
        CredentialStatus::NotConfigured {
            provider: "anthropic".into()
        }
    );
}

#[test]
fn list_configured_returns_only_stored() {
    let store = InMemoryStore::new();
    store.store("openai", &api_key("k1")).unwrap();
    store.store("anthropic", &api_key("k2")).unwrap();

    let statuses = store.list_configured();
    assert_eq!(statuses.len(), 2);

    // All should be Configured (list_configured only returns stored entries)
    for s in &statuses {
        assert!(matches!(s, CredentialStatus::Configured { .. }));
    }
}

#[test]
fn overwrite_credential() {
    let store = InMemoryStore::new();
    store.store("openai", &api_key("old-key")).unwrap();
    let new_mat = oauth("new-access-token");
    store.store("openai", &new_mat).unwrap();
    assert_eq!(store.load("openai").unwrap(), new_mat);
}

// ── Redact ───────────────────────────────────────────────────────────────────

#[test]
fn redact_short() {
    assert_eq!(redact("abc"), "****");
}

#[test]
fn redact_long() {
    assert_eq!(redact("sk-1234567890abcdef"), "sk-1...cdef");
}

// ── Serde ────────────────────────────────────────────────────────────────────

#[test]
fn auth_material_serde_roundtrip() {
    let cases = [
        api_key("my-api-key"),
        oauth("access"),
        AuthMaterial::OAuth {
            access_token: "tok".into(),
            refresh_token: None,
            expires_at: None,
        },
        session("sess"),
        AuthMaterial::SessionToken {
            token: "t".into(),
            expires_at: None,
        },
        wellknown("MY_KEY", "secret-value"),
        aws_creds(),
        AuthMaterial::AwsCredentials {
            access_key_id: "AKIA".into(),
            secret_access_key: "secret".into(),
            session_token: None,
            region: "eu-west-1".into(),
        },
    ];

    for mat in &cases {
        let json = serde_json::to_string(mat).expect("serialize");
        let back: AuthMaterial = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(&back, mat);
    }
}

// ── ProviderType ─────────────────────────────────────────────────────────────

#[test]
fn provider_type_from_str_valid() {
    assert_eq!("openai".parse::<ProviderType>().unwrap(), ProviderType::OpenAi);
    assert_eq!("anthropic".parse::<ProviderType>().unwrap(), ProviderType::Anthropic);
    assert_eq!("ollama".parse::<ProviderType>().unwrap(), ProviderType::Ollama);
    assert_eq!("gemini".parse::<ProviderType>().unwrap(), ProviderType::Gemini);
    // case-insensitive
    assert_eq!("OpenAI".parse::<ProviderType>().unwrap(), ProviderType::OpenAi);
    assert_eq!("ANTHROPIC".parse::<ProviderType>().unwrap(), ProviderType::Anthropic);
}

#[test]
fn provider_type_from_str_invalid() {
    assert!("unknown".parse::<ProviderType>().is_err());
}

#[test]
fn provider_type_display() {
    assert_eq!(ProviderType::OpenAi.to_string(), "openai");
    assert_eq!(ProviderType::Anthropic.to_string(), "anthropic");
    assert_eq!(ProviderType::Ollama.to_string(), "ollama");
    assert_eq!(ProviderType::Gemini.to_string(), "gemini");
}
```

**Step 12: Update auth_handler.rs tests**

```rust
#[cfg(test)]
mod tests {
    use ucode_auth::InMemoryStore;

    use super::*;

    #[test]
    fn status_shows_all_providers() {
        let store = InMemoryStore::new();
        // No credentials stored — list_configured returns empty.
        handle_status(&store).unwrap();
    }

    #[test]
    fn logout_not_found_is_not_an_error() {
        let store = InMemoryStore::new();
        handle_logout(&store, "anthropic").unwrap();
    }

    #[test]
    fn logout_removes_existing_credential() {
        let store = InMemoryStore::new();
        store
            .store("openai", &AuthMaterial::ApiKey {
                key: "sk-test".into(),
            })
            .unwrap();
        handle_logout(&store, "openai").unwrap();
        assert!(store.load("openai").is_err());
    }

    #[test]
    fn login_stub_returns_ok() {
        let store = InMemoryStore::new();
        handle_login(&store, "ollama", true, false).unwrap();
    }

    #[test]
    fn set_key_then_status_shows_configured() {
        let store = InMemoryStore::new();
        store
            .store("anthropic", &AuthMaterial::ApiKey {
                key: "sk-ant-test".into(),
            })
            .unwrap();
        let statuses = store.list_configured();
        let anthropic = statuses
            .iter()
            .find(|s| matches!(s, CredentialStatus::Configured { provider, .. } if provider == "anthropic"));
        assert!(anthropic.is_some());
    }
}
```

**Step 13: Check for any other consumers of ProviderId in the workspace**

Run: `cargo build --workspace 2>&1 | head -50`

Fix any remaining references. The main consumer is `crates/ucode-cli/src/main.rs` which dispatches `AuthCommand` — update the match arms to pass `&provider` instead of `provider`.

**Step 14: Run all tests**

Run: `cargo test -p ucode-auth && cargo test -p ucode-cli`
Expected: ALL PASS

**Step 15: Commit**

```bash
git add crates/ucode-auth/ crates/ucode-cli/src/cmd_auth.rs crates/ucode-cli/src/auth_handler.rs
git commit -m "refactor(auth): replace ProviderId enum with string-based identifiers

Replace fixed ProviderId enum with &str throughout CredentialStore trait.
Add ProviderType enum for adapter selection (OpenAi/Anthropic/Ollama/Gemini).
Expand AuthMaterial with WellKnown and AwsCredentials variants.
Add InvalidProvider, FileStore, MissingCredential error variants.
InMemoryStore.list_configured() now returns only stored entries.
KeyringStore.list_configured() returns empty (cannot enumerate keyring).
CLI auth commands accept arbitrary provider name strings."
```

---

## Task 3: Add FileStore implementation

**Files:**
- Create: `crates/ucode-auth/src/file_store.rs`
- Modify: `crates/ucode-auth/src/lib.rs` (add module + export)
- Modify: `crates/ucode-auth/Cargo.toml` (add `dirs` dependency)
- Test: `crates/ucode-auth/tests/file_store_tests.rs`

**Step 1: Add `dirs` dependency**

Run: `cargo add dirs -p ucode-auth`

**Step 2: Write failing tests**

Create `crates/ucode-auth/tests/file_store_tests.rs`:

```rust
use std::path::PathBuf;

use ucode_auth::{AuthError, AuthMaterial, CredentialStatus, CredentialStore, FileStore};

fn api_key(key: &str) -> AuthMaterial {
    AuthMaterial::ApiKey { key: key.into() }
}

fn oauth(access: &str) -> AuthMaterial {
    AuthMaterial::OAuth {
        access_token: access.into(),
        refresh_token: Some("refresh-xyz".into()),
        expires_at: Some("2026-01-01T00:00:00Z".into()),
    }
}

fn temp_store() -> (FileStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("auth.json");
    let store = FileStore::with_path(path);
    (store, dir)
}

#[test]
fn store_and_load() {
    let (store, _dir) = temp_store();
    let mat = api_key("sk-test");
    store.store("openai", &mat).unwrap();
    assert_eq!(store.load("openai").unwrap(), mat);
}

#[test]
fn load_not_found() {
    let (store, _dir) = temp_store();
    let err = store.load("openai").unwrap_err();
    assert!(matches!(err, AuthError::NotFound { .. }));
}

#[test]
fn delete_credential() {
    let (store, _dir) = temp_store();
    store.store("openai", &api_key("key")).unwrap();
    store.delete("openai").unwrap();
    assert!(store.load("openai").is_err());
}

#[test]
fn delete_not_found() {
    let (store, _dir) = temp_store();
    let err = store.delete("openai").unwrap_err();
    assert!(matches!(err, AuthError::NotFound { .. }));
}

#[test]
fn multiple_providers() {
    let (store, _dir) = temp_store();
    store.store("openai", &api_key("k1")).unwrap();
    store.store("anthropic", &oauth("tok")).unwrap();
    store.store("my-proxy", &api_key("k3")).unwrap();

    assert_eq!(store.load("openai").unwrap(), api_key("k1"));
    assert_eq!(store.load("my-proxy").unwrap(), api_key("k3"));

    let statuses = store.list_configured();
    assert_eq!(statuses.len(), 3);
}

#[test]
fn overwrite_credential() {
    let (store, _dir) = temp_store();
    store.store("openai", &api_key("old")).unwrap();
    store.store("openai", &api_key("new")).unwrap();
    assert_eq!(store.load("openai").unwrap(), api_key("new"));
}

#[test]
fn persists_across_instances() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("auth.json");

    // Store with first instance
    let store1 = FileStore::with_path(path.clone());
    store1.store("openai", &api_key("persistent")).unwrap();

    // Load with second instance
    let store2 = FileStore::with_path(path);
    assert_eq!(store2.load("openai").unwrap(), api_key("persistent"));
}

#[test]
fn status_configured() {
    let (store, _dir) = temp_store();
    store.store("openai", &api_key("key")).unwrap();
    let s = store.status("openai");
    assert!(matches!(
        s,
        CredentialStatus::Configured { ref provider, ref kind }
        if provider == "openai" && kind == "api_key"
    ));
}

#[test]
fn status_not_configured() {
    let (store, _dir) = temp_store();
    assert_eq!(
        store.status("openai"),
        CredentialStatus::NotConfigured {
            provider: "openai".into()
        }
    );
}
```

**Step 3: Run tests to verify they fail**

Run: `cargo test -p ucode-auth file_store`
Expected: FAIL — `FileStore` doesn't exist

**Step 4: Implement FileStore**

Create `crates/ucode-auth/src/file_store.rs`:

```rust
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use crate::credential::{AuthMaterial, CredentialStatus, CredentialStore, material_kind};
use crate::error::AuthError;

/// Credential store backed by a JSON file.
///
/// File format: `{ "provider-id": { "type": "api_key", "key": "..." }, ... }`
///
/// File permissions are set to 0o600 on Unix.
pub struct FileStore {
    path: PathBuf,
}

impl FileStore {
    /// Create a FileStore at the default path (`~/.local/share/ucode/auth.json`).
    pub fn new() -> Result<Self, AuthError> {
        let dir = dirs::data_local_dir()
            .ok_or_else(|| AuthError::FileStore {
                message: "cannot determine local data directory".into(),
            })?
            .join("ucode");
        Ok(Self {
            path: dir.join("auth.json"),
        })
    }

    /// Create a FileStore at a specific path (for testing).
    pub fn with_path(path: PathBuf) -> Self {
        Self { path }
    }

    fn read_all(&self) -> Result<HashMap<String, AuthMaterial>, AuthError> {
        if !self.path.exists() {
            return Ok(HashMap::new());
        }
        let data = fs::read_to_string(&self.path).map_err(|e| AuthError::FileStore {
            message: format!("read {}: {e}", self.path.display()),
        })?;
        serde_json::from_str(&data).map_err(|e| AuthError::Serialization {
            message: format!("parse {}: {e}", self.path.display()),
        })
    }

    fn write_all(&self, data: &HashMap<String, AuthMaterial>) -> Result<(), AuthError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| AuthError::FileStore {
                message: format!("create dir {}: {e}", parent.display()),
            })?;
        }
        let json = serde_json::to_string_pretty(data).map_err(|e| AuthError::Serialization {
            message: e.to_string(),
        })?;
        fs::write(&self.path, &json).map_err(|e| AuthError::FileStore {
            message: format!("write {}: {e}", self.path.display()),
        })?;

        // Set file permissions to 0o600 on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::Permissions::from_mode(0o600);
            fs::set_permissions(&self.path, perms).map_err(|e| AuthError::FileStore {
                message: format!("chmod {}: {e}", self.path.display()),
            })?;
        }

        Ok(())
    }
}

impl CredentialStore for FileStore {
    fn store(&self, provider: &str, material: &AuthMaterial) -> Result<(), AuthError> {
        let mut data = self.read_all()?;
        data.insert(provider.to_owned(), material.clone());
        self.write_all(&data)
    }

    fn load(&self, provider: &str) -> Result<AuthMaterial, AuthError> {
        let data = self.read_all()?;
        data.get(provider).cloned().ok_or_else(|| AuthError::NotFound {
            provider: provider.to_owned(),
        })
    }

    fn delete(&self, provider: &str) -> Result<(), AuthError> {
        let mut data = self.read_all()?;
        if data.remove(provider).is_none() {
            return Err(AuthError::NotFound {
                provider: provider.to_owned(),
            });
        }
        self.write_all(&data)
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
        match self.read_all() {
            Ok(data) => data
                .iter()
                .map(|(id, mat)| CredentialStatus::Configured {
                    provider: id.clone(),
                    kind: material_kind(mat).into(),
                })
                .collect(),
            Err(_) => Vec::new(),
        }
    }
}
```

Note: `material_kind` needs to be made `pub(crate)` in `credential.rs` so `file_store.rs` can use it.

**Step 5: Update lib.rs**

Add `pub mod file_store;` and export `FileStore`.

**Step 6: Add tempfile dev-dependency**

Run: `cargo add tempfile --dev -p ucode-auth`

**Step 7: Run tests**

Run: `cargo test -p ucode-auth file_store`
Expected: ALL PASS

**Step 8: Commit**

```bash
git add crates/ucode-auth/
git commit -m "feat(auth): add FileStore for JSON-based credential persistence

FileStore reads/writes ~/.local/share/ucode/auth.json with 0o600
permissions on Unix. Supports with_path() for testing with tempdir.
Implements full CredentialStore trait including list_configured()."
```

---

## Task 4: Add ChainStore implementation

**Files:**
- Create: `crates/ucode-auth/src/chain_store.rs`
- Modify: `crates/ucode-auth/src/lib.rs` (add module + export)
- Test: `crates/ucode-auth/tests/chain_store_tests.rs`

**Step 1: Write failing tests**

Create `crates/ucode-auth/tests/chain_store_tests.rs`:

```rust
use ucode_auth::{AuthError, AuthMaterial, CredentialStore, ChainStore, InMemoryStore};

fn api_key(key: &str) -> AuthMaterial {
    AuthMaterial::ApiKey { key: key.into() }
}

#[test]
fn load_from_primary() {
    let primary = InMemoryStore::new();
    let fallback = InMemoryStore::new();
    primary.store("openai", &api_key("primary-key")).unwrap();
    fallback.store("openai", &api_key("fallback-key")).unwrap();

    let chain = ChainStore::new(Box::new(primary), Box::new(fallback));
    // Primary wins
    assert_eq!(chain.load("openai").unwrap(), api_key("primary-key"));
}

#[test]
fn load_falls_back() {
    let primary = InMemoryStore::new();
    let fallback = InMemoryStore::new();
    fallback.store("openai", &api_key("fallback-key")).unwrap();

    let chain = ChainStore::new(Box::new(primary), Box::new(fallback));
    assert_eq!(chain.load("openai").unwrap(), api_key("fallback-key"));
}

#[test]
fn load_not_found_in_either() {
    let primary = InMemoryStore::new();
    let fallback = InMemoryStore::new();
    let chain = ChainStore::new(Box::new(primary), Box::new(fallback));
    assert!(matches!(chain.load("openai").unwrap_err(), AuthError::NotFound { .. }));
}

#[test]
fn store_writes_to_primary() {
    let primary = InMemoryStore::new();
    let fallback = InMemoryStore::new();
    let chain = ChainStore::new(Box::new(primary), Box::new(fallback));

    chain.store("openai", &api_key("new-key")).unwrap();
    assert_eq!(chain.load("openai").unwrap(), api_key("new-key"));
}

#[test]
fn delete_from_both() {
    let primary = InMemoryStore::new();
    let fallback = InMemoryStore::new();
    primary.store("openai", &api_key("k1")).unwrap();
    fallback.store("openai", &api_key("k2")).unwrap();

    let chain = ChainStore::new(Box::new(primary), Box::new(fallback));
    chain.delete("openai").unwrap();
    assert!(chain.load("openai").is_err());
}

#[test]
fn list_configured_merges() {
    let primary = InMemoryStore::new();
    let fallback = InMemoryStore::new();
    primary.store("openai", &api_key("k1")).unwrap();
    fallback.store("anthropic", &api_key("k2")).unwrap();

    let chain = ChainStore::new(Box::new(primary), Box::new(fallback));
    let statuses = chain.list_configured();
    assert_eq!(statuses.len(), 2);
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p ucode-auth chain_store`
Expected: FAIL — `ChainStore` doesn't exist

**Step 3: Implement ChainStore**

Create `crates/ucode-auth/src/chain_store.rs`:

```rust
use std::collections::HashSet;

use crate::credential::{AuthMaterial, CredentialStatus, CredentialStore};
use crate::error::AuthError;

/// Credential store that tries a primary store first, falling back to a secondary.
///
/// Writes always go to the primary store. Reads try primary first, then fallback.
/// Deletes attempt both stores (best-effort on fallback).
pub struct ChainStore {
    primary: Box<dyn CredentialStore>,
    fallback: Box<dyn CredentialStore>,
}

impl ChainStore {
    pub fn new(primary: Box<dyn CredentialStore>, fallback: Box<dyn CredentialStore>) -> Self {
        Self { primary, fallback }
    }
}

impl CredentialStore for ChainStore {
    fn store(&self, provider: &str, material: &AuthMaterial) -> Result<(), AuthError> {
        self.primary.store(provider, material)
    }

    fn load(&self, provider: &str) -> Result<AuthMaterial, AuthError> {
        match self.primary.load(provider) {
            Ok(mat) => Ok(mat),
            Err(AuthError::NotFound { .. }) => self.fallback.load(provider),
            Err(e) => {
                // Primary had a real error (keyring failure, etc.) — try fallback
                match self.fallback.load(provider) {
                    Ok(mat) => Ok(mat),
                    Err(AuthError::NotFound { .. }) => Err(e), // Return original error
                    Err(fallback_err) => Err(fallback_err),
                }
            }
        }
    }

    fn delete(&self, provider: &str) -> Result<(), AuthError> {
        let primary_result = self.primary.delete(provider);
        // Best-effort delete from fallback
        let _ = self.fallback.delete(provider);
        primary_result
    }

    fn status(&self, provider: &str) -> CredentialStatus {
        match self.load(provider) {
            Ok(mat) => CredentialStatus::Configured {
                provider: provider.to_owned(),
                kind: crate::credential::material_kind(&mat).into(),
            },
            Err(_) => CredentialStatus::NotConfigured {
                provider: provider.to_owned(),
            },
        }
    }

    fn list_configured(&self) -> Vec<CredentialStatus> {
        let mut seen = HashSet::new();
        let mut result = Vec::new();

        // Primary entries first
        for status in self.primary.list_configured() {
            if let CredentialStatus::Configured { ref provider, .. } = status {
                seen.insert(provider.clone());
                result.push(status);
            }
        }

        // Fallback entries not already in primary
        for status in self.fallback.list_configured() {
            if let CredentialStatus::Configured { ref provider, .. } = status {
                if !seen.contains(provider) {
                    result.push(status);
                }
            }
        }

        result
    }
}
```

**Step 4: Update lib.rs**

Add `pub mod chain_store;` and export `ChainStore`.

**Step 5: Run tests**

Run: `cargo test -p ucode-auth chain_store`
Expected: ALL PASS

**Step 6: Commit**

```bash
git add crates/ucode-auth/
git commit -m "feat(auth): add ChainStore for keyring-first, file-fallback credential resolution

ChainStore tries primary store (keyring) first, falls back to secondary
(file store). Writes go to primary. Deletes attempt both. list_configured
merges both stores, deduplicating by provider ID."
```

---

## Task 5: Implement resolve_auth() precedence resolver

**Files:**
- Create: `crates/ucode-auth/src/resolve.rs`
- Modify: `crates/ucode-auth/src/lib.rs` (add module + export)
- Test: `crates/ucode-auth/tests/resolve_tests.rs`

**Step 1: Write failing tests**

Create `crates/ucode-auth/tests/resolve_tests.rs`:

```rust
use ucode_auth::{AuthError, AuthMaterial, CredentialStore, InMemoryStore, resolve_auth};

fn api_key(key: &str) -> AuthMaterial {
    AuthMaterial::ApiKey { key: key.into() }
}

#[test]
fn env_var_takes_precedence() {
    let store = InMemoryStore::new();
    store.store("test-provider", &api_key("stored-key")).unwrap();

    // Set env var
    std::env::set_var("TEST_PROVIDER_API_KEY", "env-key");
    let result = resolve_auth("test-provider", Some("TEST_PROVIDER_API_KEY"), &store);
    std::env::remove_var("TEST_PROVIDER_API_KEY");

    assert_eq!(result.unwrap(), api_key("env-key"));
}

#[test]
fn falls_back_to_store() {
    let store = InMemoryStore::new();
    store.store("test-provider", &api_key("stored-key")).unwrap();

    let result = resolve_auth("test-provider", None, &store);
    assert_eq!(result.unwrap(), api_key("stored-key"));
}

#[test]
fn env_var_empty_falls_through() {
    let store = InMemoryStore::new();
    store.store("test-provider", &api_key("stored-key")).unwrap();

    std::env::set_var("EMPTY_KEY", "");
    let result = resolve_auth("test-provider", Some("EMPTY_KEY"), &store);
    std::env::remove_var("EMPTY_KEY");

    // Empty env var should fall through to store
    assert_eq!(result.unwrap(), api_key("stored-key"));
}

#[test]
fn missing_everywhere_returns_error() {
    let store = InMemoryStore::new();
    let result = resolve_auth("test-provider", Some("NONEXISTENT_VAR_12345"), &store);
    assert!(matches!(result.unwrap_err(), AuthError::MissingCredential { .. }));
}

#[test]
fn no_env_var_name_and_no_store_returns_error() {
    let store = InMemoryStore::new();
    let result = resolve_auth("test-provider", None, &store);
    assert!(matches!(result.unwrap_err(), AuthError::MissingCredential { .. }));
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p ucode-auth resolve`
Expected: FAIL — `resolve_auth` doesn't exist

**Step 3: Implement resolve_auth**

Create `crates/ucode-auth/src/resolve.rs`:

```rust
use std::env;

use crate::credential::{AuthMaterial, CredentialStore};
use crate::error::AuthError;

/// Resolve authentication material for a provider.
///
/// Precedence (first non-empty wins):
/// 1. Environment variable named by `api_key_env`
/// 2. Stored credential from the credential store
/// 3. Error (caller should prompt interactively or fail)
pub fn resolve_auth(
    provider: &str,
    api_key_env: Option<&str>,
    store: &dyn CredentialStore,
) -> Result<AuthMaterial, AuthError> {
    // 1. Check environment variable
    if let Some(env_name) = api_key_env {
        if let Ok(value) = env::var(env_name) {
            if !value.is_empty() {
                return Ok(AuthMaterial::ApiKey { key: value });
            }
        }
    }

    // 2. Check credential store
    match store.load(provider) {
        Ok(mat) => return Ok(mat),
        Err(AuthError::NotFound { .. }) => {}
        Err(e) => return Err(e),
    }

    // 3. Nothing found
    Err(AuthError::MissingCredential {
        provider: provider.to_owned(),
        detail: match api_key_env {
            Some(env_name) => format!(
                "set ${env_name} or run `ucode auth login {provider}`"
            ),
            None => format!("run `ucode auth login {provider}`"),
        },
    })
}
```

**Step 4: Update lib.rs**

Add `pub mod resolve;` and `pub use resolve::resolve_auth;`.

**Step 5: Run tests**

Run: `cargo test -p ucode-auth resolve`
Expected: ALL PASS

**Step 6: Run full test suite**

Run: `cargo test -p ucode-auth && cargo test -p ucode-cli`
Expected: ALL PASS

**Step 7: Commit**

```bash
git add crates/ucode-auth/
git commit -m "feat(auth): add resolve_auth() with env var > store > error precedence

resolve_auth(provider, api_key_env, store) checks env var first, then
credential store, then returns MissingCredential error with actionable
message. Empty env vars fall through to store lookup."
```

---

## Task 6: Verify workspace builds and all tests pass

**Step 1: Build entire workspace**

Run: `cargo build --workspace`
Expected: SUCCESS

**Step 2: Run all tests**

Run: `cargo test --workspace`
Expected: ALL PASS

**Step 3: Check for any remaining ProviderId references**

Run: `grep -r "ProviderId" crates/ --include="*.rs"`
Expected: No matches (all replaced)

**Step 4: Final commit if any fixups needed**

Only if Step 1-3 revealed issues.

---

## Summary

| Task | What | Files | Tests |
|------|------|-------|-------|
| 1-2 | AuthMaterial expansion + ProviderId -> String + ProviderType + error variants + CLI update | credential.rs, error.rs, lib.rs, cmd_auth.rs, auth_handler.rs | credential_tests.rs (rewritten) |
| 3 | FileStore (JSON file, 0o600) | file_store.rs | file_store_tests.rs |
| 4 | ChainStore (primary + fallback) | chain_store.rs | chain_store_tests.rs |
| 5 | resolve_auth() precedence | resolve.rs | resolve_tests.rs |
| 6 | Workspace verification | — | full suite |
