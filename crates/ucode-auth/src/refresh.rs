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

/// How soon before expiry to trigger a refresh (5 minutes).
const REFRESH_MARGIN_SECS: i64 = 300;

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
/// If the server returns a new refresh_token, it replaces the old one;
/// otherwise the original refresh_token is preserved.
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
        .map_err(|e| AuthError::Http {
            message: e.to_string(),
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(AuthError::AuthFlow {
            message: format!("token refresh failed ({status}): {body}"),
        });
    }

    let r: RefreshResponse = resp.json().await.map_err(|e| AuthError::AuthFlow {
        message: e.to_string(),
    })?;

    let expires_at = r
        .expires_in
        .map(|secs| (chrono::Utc::now() + Duration::seconds(secs as i64)).to_rfc3339());

    Ok(AuthMaterial::OAuth {
        access_token: r.access_token,
        // Preserve original refresh_token if server doesn't return a new one
        refresh_token: r.refresh_token.or_else(|| Some(refresh_token.to_owned())),
        expires_at,
    })
}

/// Resolve auth material, refreshing OAuth tokens if they expire soon.
///
/// This wraps `resolve_auth()` with automatic token refresh:
/// 1. Resolve credentials via env var or store
/// 2. If the result is an OAuth token expiring within 5 minutes AND has a refresh_token:
///    - Attempt refresh via the provided RefreshConfig
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

    // Only check expiry for token types that have it
    if !material.expires_within(Duration::seconds(REFRESH_MARGIN_SECS)) {
        return Ok(material);
    }

    // Token is expiring soon — try to refresh
    let refresh_tok = match material.refresh_token() {
        Some(rt) => rt.to_owned(),
        None => {
            return Err(AuthError::AuthExpired {
                provider: provider.to_owned(),
                detail:
                    "token expired and no refresh_token available; run `ucode auth login` again"
                        .into(),
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
    fn refresh_response_full() {
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
    async fn resolve_non_expiring_passes_through() {
        let store = InMemoryStore::new();
        store
            .store("test", &AuthMaterial::ApiKey { key: "k".into() })
            .unwrap();
        let result = resolve_auth_with_refresh("test", None, &store, None).await;
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), AuthMaterial::ApiKey { .. }));
    }

    #[tokio::test]
    async fn resolve_expired_no_refresh_token() {
        let store = InMemoryStore::new();
        store
            .store(
                "test",
                &AuthMaterial::OAuth {
                    access_token: "old".into(),
                    refresh_token: None,
                    expires_at: Some("2020-01-01T00:00:00Z".into()),
                },
            )
            .unwrap();
        let result = resolve_auth_with_refresh("test", None, &store, None).await;
        assert!(matches!(result, Err(AuthError::AuthExpired { .. })));
    }

    #[tokio::test]
    async fn resolve_expired_no_refresh_config() {
        let store = InMemoryStore::new();
        store
            .store(
                "test",
                &AuthMaterial::OAuth {
                    access_token: "old".into(),
                    refresh_token: Some("ref".into()),
                    expires_at: Some("2020-01-01T00:00:00Z".into()),
                },
            )
            .unwrap();
        // Has refresh_token but no RefreshConfig
        let result = resolve_auth_with_refresh("test", None, &store, None).await;
        assert!(matches!(result, Err(AuthError::AuthExpired { .. })));
    }

    #[tokio::test]
    async fn resolve_not_expiring_soon() {
        let store = InMemoryStore::new();
        let far_future = (chrono::Utc::now() + Duration::hours(1)).to_rfc3339();
        store
            .store(
                "test",
                &AuthMaterial::OAuth {
                    access_token: "tok".into(),
                    refresh_token: Some("ref".into()),
                    expires_at: Some(far_future),
                },
            )
            .unwrap();
        let result = resolve_auth_with_refresh("test", None, &store, None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn resolve_no_expiry_field_passes_through() {
        let store = InMemoryStore::new();
        store
            .store(
                "test",
                &AuthMaterial::OAuth {
                    access_token: "tok".into(),
                    refresh_token: None,
                    expires_at: None,
                },
            )
            .unwrap();
        // No expires_at means not expiring
        let result = resolve_auth_with_refresh("test", None, &store, None).await;
        assert!(result.is_ok());
    }
}
