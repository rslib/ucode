# Token Refresh + Expiry Management (Task 2.5) Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add token expiry checking and OAuth refresh so provider requests automatically refresh tokens before they expire, without user intervention.

**Architecture:** On-demand check-and-refresh (no background task). A `refresh_oauth_token()` async function handles the HTTP refresh. Helper methods on `AuthMaterial` check expiry. `resolve_auth_with_refresh()` wraps the existing `resolve_auth()` with automatic refresh. The chrono crate (already in workspace) parses ISO 8601 timestamps.

**Tech Stack:** Rust, reqwest, tokio, chrono, serde, serde_json

---

## Task 1: Add expiry error variant and chrono dependency

**Files:**
- Modify: `crates/ucode-auth/Cargo.toml`
- Modify: `crates/ucode-auth/src/error.rs`

**Step 1: Add chrono dependency**

```bash
cargo add chrono --features serde -p ucode-auth
```

Then edit `crates/ucode-auth/Cargo.toml` to use workspace: `chrono = { workspace = true }`

**Step 2: Add error variants**

Add to `AuthError` enum in `error.rs`:

```rust
#[error("auth token expired for provider '{provider}': {detail}")]
AuthExpired { provider: String, detail: String },
```

**Verify:** `cargo build -p ucode-auth`

**Commit:**
```
feat(auth): add AuthExpired error variant and chrono dependency
```

---

## Task 2: Add expiry helpers on AuthMaterial

**Files:**
- Modify: `crates/ucode-auth/src/credential.rs`

Add these methods to `AuthMaterial` via an `impl` block:

```rust
impl AuthMaterial {
    /// Returns the expiry timestamp string if this material type has one.
    pub fn expires_at(&self) -> Option<&str> {
        match self {
            Self::OAuth { expires_at, .. } => expires_at.as_deref(),
            Self::SessionToken { expires_at, .. } => expires_at.as_deref(),
            _ => None,
        }
    }

    /// Returns true if the token has expired.
    pub fn is_expired(&self) -> bool {
        self.expires_at()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .is_some_and(|exp| exp <= chrono::Utc::now())
    }

    /// Returns true if the token expires within the given duration.
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
```

**Tests** (add to `#[cfg(test)]` in credential.rs or as integration tests):

```rust
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
    // Token expiring in 1 minute — should be "within 5 minutes"
    let soon = (chrono::Utc::now() + chrono::Duration::seconds(60))
        .to_rfc3339();
    let mat = AuthMaterial::OAuth {
        access_token: "tok".into(),
        refresh_token: None,
        expires_at: Some(soon),
    };
    assert!(mat.expires_within(chrono::Duration::minutes(5)));
}

#[test]
fn expires_within_not_soon() {
    // Token expiring in 1 hour — should NOT be "within 5 minutes"
    let later = (chrono::Utc::now() + chrono::Duration::hours(1))
        .to_rfc3339();
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
fn refresh_token_absent() {
    let mat = AuthMaterial::ApiKey { key: "k".into() };
    assert_eq!(mat.refresh_token(), None);
}
```

**Verify:** `cargo test -p ucode-auth`

**Commit:**
```
feat(auth): add expiry helpers on AuthMaterial
```

---

## Task 3: Implement OAuth token refresh

**Files:**
- Create: `crates/ucode-auth/src/refresh.rs`
- Modify: `crates/ucode-auth/src/lib.rs`

**Implementation:**

Create `crates/ucode-auth/src/refresh.rs`:

```rust
//! OAuth token refresh and expiry-aware auth resolution.

use chrono::Duration;
use reqwest::Client;
use serde::Deserialize;

use crate::credential::{AuthMaterial, CredentialStore};
use crate::error::AuthError;

/// Configuration for token refresh.
pub struct RefreshConfig {
    pub token_url: String,
    pub client_id: String,
}

/// How soon before expiry to trigger a refresh (default: 5 minutes).
const REFRESH_MARGIN: i64 = 300;

#[derive(Deserialize)]
struct RefreshResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    #[allow(dead_code)]
    token_type: Option<String>,
}

/// Refresh an OAuth token using the refresh_token grant.
///
/// Returns a new `AuthMaterial::OAuth` with the refreshed access token.
/// The caller is responsible for storing the result.
pub async fn refresh_oauth_token(
    client: &Client,
    config: &RefreshConfig,
    refresh_token: &str,
) -> Result<AuthMaterial, AuthError> {
    let resp = client
        .post(&config.token_url)
        .header("Accept", "application/json")
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", &config.client_id),
            ("refresh_token", refresh_token),
        ])
        .send()
        .await
        .map_err(|e| AuthError::Http { message: e.to_string() })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(AuthError::AuthFlow {
            message: format!("token refresh failed ({status}): {body}"),
        });
    }

    let r: RefreshResponse = resp
        .json()
        .await
        .map_err(|e| AuthError::AuthFlow { message: e.to_string() })?;

    let expires_at = r.expires_in.map(|secs| {
        (chrono::Utc::now() + Duration::seconds(secs as i64)).to_rfc3339()
    });

    Ok(AuthMaterial::OAuth {
        access_token: r.access_token,
        refresh_token: r.refresh_token.or_else(|| Some(refresh_token.to_owned())),
        expires_at,
    })
}

/// Resolve auth material, refreshing OAuth tokens if they expire soon.
///
/// This wraps `resolve_auth()` with automatic token refresh:
/// 1. Resolve credentials via env var or store (existing logic)
/// 2. If the result is an OAuth token expiring within 5 minutes AND has a refresh_token:
///    - Attempt refresh
///    - On success: store the new credential and return it
///    - On failure: return AuthExpired error
/// 3. If expired with no refresh_token: return AuthExpired error
/// 4. Otherwise: return the credential as-is
pub async fn resolve_auth_with_refresh(
    provider: &str,
    api_key_env: Option<&str>,
    store: &dyn CredentialStore,
    refresh_config: Option<&RefreshConfig>,
) -> Result<AuthMaterial, AuthError> {
    let material = crate::resolve::resolve_auth(provider, api_key_env, store)?;

    // Only OAuth tokens can expire and be refreshed
    if !material.expires_within(Duration::seconds(REFRESH_MARGIN)) {
        return Ok(material);
    }

    // Token is expiring soon — try to refresh
    let refresh_tok = match material.refresh_token() {
        Some(rt) => rt.to_owned(),
        None => {
            return Err(AuthError::AuthExpired {
                provider: provider.to_owned(),
                detail: "token expired and no refresh_token available; run `ucode auth login` again".into(),
            });
        }
    };

    let config = match refresh_config {
        Some(c) => c,
        None => {
            return Err(AuthError::AuthExpired {
                provider: provider.to_owned(),
                detail: "token expired but no refresh endpoint configured".into(),
            });
        }
    };

    let client = Client::new();
    match refresh_oauth_token(&client, config, &refresh_tok).await {
        Ok(new_material) => {
            // Store the refreshed credential
            store.store(provider, &new_material)?;
            Ok(new_material)
        }
        Err(e) => Err(AuthError::AuthExpired {
            provider: provider.to_owned(),
            detail: format!("refresh failed: {e}"),
        }),
    }
}
```

Update `crates/ucode-auth/src/lib.rs`:
- Add `pub mod refresh;`
- Add `pub use refresh::{RefreshConfig, refresh_oauth_token, resolve_auth_with_refresh};`

**Tests** (inside `refresh.rs` as `#[cfg(test)] mod tests`):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::credential::InMemoryStore;

    #[test]
    fn refresh_config_construction() {
        let cfg = RefreshConfig {
            token_url: "https://example.com/token".into(),
            client_id: "client123".into(),
        };
        assert_eq!(cfg.token_url, "https://example.com/token");
        assert_eq!(cfg.client_id, "client123");
    }

    #[test]
    fn refresh_response_deserialization() {
        let json = r#"{"access_token":"new_tok","refresh_token":"new_ref","expires_in":3600,"token_type":"Bearer"}"#;
        let r: RefreshResponse = serde_json::from_str(json).unwrap();
        assert_eq!(r.access_token, "new_tok");
        assert_eq!(r.refresh_token, Some("new_ref".into()));
        assert_eq!(r.expires_in, Some(3600));
    }

    #[test]
    fn refresh_response_minimal() {
        let json = r#"{"access_token":"tok"}"#;
        let r: RefreshResponse = serde_json::from_str(json).unwrap();
        assert_eq!(r.access_token, "tok");
        assert!(r.refresh_token.is_none());
        assert!(r.expires_in.is_none());
    }

    #[tokio::test]
    async fn resolve_with_refresh_non_expiring_passes_through() {
        let store = InMemoryStore::new();
        store.store("test", &AuthMaterial::ApiKey { key: "k".into() }).unwrap();
        let result = resolve_auth_with_refresh("test", None, &store, None).await;
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), AuthMaterial::ApiKey { .. }));
    }

    #[tokio::test]
    async fn resolve_with_refresh_expired_no_refresh_token() {
        let store = InMemoryStore::new();
        store.store("test", &AuthMaterial::OAuth {
            access_token: "old".into(),
            refresh_token: None,
            expires_at: Some("2020-01-01T00:00:00Z".into()),
        }).unwrap();
        let result = resolve_auth_with_refresh("test", None, &store, None).await;
        assert!(matches!(result, Err(AuthError::AuthExpired { .. })));
    }

    #[tokio::test]
    async fn resolve_with_refresh_expired_no_config() {
        let store = InMemoryStore::new();
        store.store("test", &AuthMaterial::OAuth {
            access_token: "old".into(),
            refresh_token: Some("ref".into()),
            expires_at: Some("2020-01-01T00:00:00Z".into()),
        }).unwrap();
        let result = resolve_auth_with_refresh("test", None, &store, None).await;
        assert!(matches!(result, Err(AuthError::AuthExpired { .. })));
    }

    #[tokio::test]
    async fn resolve_with_refresh_not_expiring_soon() {
        let store = InMemoryStore::new();
        let far_future = (chrono::Utc::now() + Duration::hours(1)).to_rfc3339();
        store.store("test", &AuthMaterial::OAuth {
            access_token: "tok".into(),
            refresh_token: Some("ref".into()),
            expires_at: Some(far_future),
        }).unwrap();
        let result = resolve_auth_with_refresh("test", None, &store, None).await;
        assert!(result.is_ok());
    }
}
```

**Verify:** `cargo test -p ucode-auth`

**Commit:**
```
feat(auth): implement OAuth token refresh and expiry-aware resolution
```

---

## Task 4: Workspace verification

Run: `cargo build && cargo test && cargo clippy`

Verify all modules compile and existing tests still pass.

---

## Summary

| Task | What | Tests |
|------|------|-------|
| 1 | AuthExpired error + chrono dep | build check |
| 2 | Expiry helpers on AuthMaterial | 9 unit tests |
| 3 | refresh_oauth_token + resolve_auth_with_refresh | 6 unit tests |
| 4 | Verification | full suite |
