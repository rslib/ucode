//! Auth resolution helpers for provider adapters.

use std::sync::Arc;

use ucode_auth::{AuthMaterial, CredentialStore, RefreshConfig, resolve_auth_with_refresh};
use ucode_core::{AuthErrorKind, CoreError};

/// Resolve auth material for a provider request.
///
/// Precedence:
/// 1. If `credential_store` is `Some`, resolve via the store (with optional token refresh)
/// 2. If `credential_store` is `None`, wrap `fallback_api_key` as `AuthMaterial::ApiKey`
/// 3. If both are `None`, return `None` (provider may work without auth, e.g. Ollama)
///
/// If `refresh_config` is provided and the resolved material is an OAuth token expiring
/// within 5 minutes, an automatic refresh is attempted before returning.
pub async fn resolve_provider_auth(
    provider: &str,
    api_key_env: Option<&str>,
    credential_store: Option<&dyn CredentialStore>,
    fallback_api_key: Option<&str>,
    refresh_config: Option<&RefreshConfig>,
) -> Result<Option<AuthMaterial>, CoreError> {
    // Try credential store first
    if let Some(store) = credential_store {
        match resolve_auth_with_refresh(provider, api_key_env, store, refresh_config).await {
            Ok(material) => return Ok(Some(material)),
            Err(ucode_auth::AuthError::MissingCredential { .. })
            | Err(ucode_auth::AuthError::NotFound { .. }) => {
                // Fall through to fallback_api_key
            }
            Err(ucode_auth::AuthError::AuthExpired { .. }) => {
                return Err(CoreError::Auth {
                    provider: provider.to_owned(),
                    auth_kind: AuthErrorKind::Expired,
                });
            }
            Err(_) => {
                return Err(CoreError::Auth {
                    provider: provider.to_owned(),
                    auth_kind: AuthErrorKind::Invalid,
                });
            }
        }
    }

    // Fallback to stored api_key
    if let Some(key) = fallback_api_key {
        return Ok(Some(AuthMaterial::ApiKey {
            key: key.to_owned(),
        }));
    }

    Ok(None)
}

/// Extract a bearer token string from auth material.
pub fn bearer_token(material: &AuthMaterial) -> String {
    match material {
        AuthMaterial::ApiKey { key } => key.clone(),
        AuthMaterial::OAuth { access_token, .. } => access_token.clone(),
        AuthMaterial::SessionToken { token, .. } => token.clone(),
        AuthMaterial::WellKnown { token, .. } => token.clone(),
        AuthMaterial::AwsCredentials { session_token, .. } => {
            // For AWS, use session_token if available, otherwise empty
            session_token.clone().unwrap_or_default()
        }
    }
}

// Arc is used by consumers that hold a shared credential store across provider instances.
const _: fn() = || {
    let _: Arc<dyn CredentialStore>;
};

#[cfg(test)]
mod tests {
    use super::*;
    use ucode_auth::InMemoryStore;

    #[tokio::test]
    async fn resolve_from_store_api_key() {
        let store = InMemoryStore::new();
        store
            .store(
                "openai",
                &AuthMaterial::ApiKey {
                    key: "sk-test".into(),
                },
            )
            .unwrap();
        let result = resolve_provider_auth("openai", None, Some(&store), None, None)
            .await
            .unwrap();
        assert!(matches!(result, Some(AuthMaterial::ApiKey { key }) if key == "sk-test"));
    }

    #[tokio::test]
    async fn resolve_from_store_oauth() {
        let store = InMemoryStore::new();
        store
            .store(
                "copilot",
                &AuthMaterial::OAuth {
                    access_token: "gho_abc".into(),
                    refresh_token: None,
                    expires_at: None,
                },
            )
            .unwrap();
        let result = resolve_provider_auth("copilot", None, Some(&store), None, None)
            .await
            .unwrap();
        assert!(
            matches!(result, Some(AuthMaterial::OAuth { access_token, .. }) if access_token == "gho_abc")
        );
    }

    #[tokio::test]
    async fn resolve_fallback_api_key_when_store_empty() {
        let store = InMemoryStore::new();
        let result = resolve_provider_auth("openai", None, Some(&store), Some("sk-fallback"), None)
            .await
            .unwrap();
        assert!(matches!(result, Some(AuthMaterial::ApiKey { key }) if key == "sk-fallback"));
    }

    #[tokio::test]
    async fn resolve_fallback_api_key_no_store() {
        let result = resolve_provider_auth("openai", None, None, Some("sk-direct"), None)
            .await
            .unwrap();
        assert!(matches!(result, Some(AuthMaterial::ApiKey { key }) if key == "sk-direct"));
    }

    #[tokio::test]
    async fn resolve_none_when_no_store_no_key() {
        let result = resolve_provider_auth("ollama", None, None, None, None)
            .await
            .unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn resolve_env_var_via_store() {
        let store = InMemoryStore::new();
        store
            .store(
                "test",
                &AuthMaterial::ApiKey {
                    key: "from-store".into(),
                },
            )
            .unwrap();
        let result = resolve_provider_auth(
            "test",
            Some("UCODE_TEST_NONEXISTENT_ENV_VAR_XYZ"),
            Some(&store),
            None,
            None,
        )
        .await
        .unwrap();
        assert!(matches!(result, Some(AuthMaterial::ApiKey { key }) if key == "from-store"));
    }

    #[test]
    fn bearer_from_session_token() {
        let mat = AuthMaterial::SessionToken {
            token: "sess-123".into(),
            expires_at: None,
        };
        assert_eq!(bearer_token(&mat), "sess-123");
    }

    #[test]
    fn bearer_from_wellknown() {
        let mat = AuthMaterial::WellKnown {
            env_key: "CUSTOM_KEY".into(),
            token: "wk-tok".into(),
        };
        assert_eq!(bearer_token(&mat), "wk-tok");
    }
}
