//! Auth resolution helpers for provider adapters.

use std::sync::Arc;

use ucode_auth::{AuthMaterial, CredentialStore};
use ucode_core::{AuthErrorKind, CoreError};

/// Resolve a bearer token for a provider request.
///
/// Precedence:
/// 1. If `credential_store` is `Some`, call `resolve_auth()` and extract the token
/// 2. If `credential_store` is `None`, return `fallback_api_key`
/// 3. If both are `None`, return `None` (provider may work without auth, e.g. Ollama)
pub fn resolve_provider_auth(
    provider: &str,
    api_key_env: Option<&str>,
    credential_store: Option<&dyn CredentialStore>,
    fallback_api_key: Option<&str>,
) -> Result<Option<String>, CoreError> {
    // Try credential store first
    if let Some(store) = credential_store {
        match ucode_auth::resolve_auth(provider, api_key_env, store) {
            Ok(material) => return Ok(Some(auth_material_to_bearer(&material))),
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
        return Ok(Some(key.to_owned()));
    }

    Ok(None)
}

/// Extract a bearer token string from auth material.
fn auth_material_to_bearer(material: &AuthMaterial) -> String {
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

    #[test]
    fn resolve_from_store_api_key() {
        let store = InMemoryStore::new();
        store
            .store(
                "openai",
                &AuthMaterial::ApiKey {
                    key: "sk-test".into(),
                },
            )
            .unwrap();
        let result = resolve_provider_auth("openai", None, Some(&store), None).unwrap();
        assert_eq!(result, Some("sk-test".into()));
    }

    #[test]
    fn resolve_from_store_oauth() {
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
        let result = resolve_provider_auth("copilot", None, Some(&store), None).unwrap();
        assert_eq!(result, Some("gho_abc".into()));
    }

    #[test]
    fn resolve_fallback_api_key_when_store_empty() {
        let store = InMemoryStore::new();
        let result =
            resolve_provider_auth("openai", None, Some(&store), Some("sk-fallback")).unwrap();
        assert_eq!(result, Some("sk-fallback".into()));
    }

    #[test]
    fn resolve_fallback_api_key_no_store() {
        let result = resolve_provider_auth("openai", None, None, Some("sk-direct")).unwrap();
        assert_eq!(result, Some("sk-direct".into()));
    }

    #[test]
    fn resolve_none_when_no_store_no_key() {
        let result = resolve_provider_auth("ollama", None, None, None).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn resolve_env_var_via_store() {
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
        )
        .unwrap();
        assert_eq!(result, Some("from-store".into()));
    }

    #[test]
    fn bearer_from_session_token() {
        let mat = AuthMaterial::SessionToken {
            token: "sess-123".into(),
            expires_at: None,
        };
        assert_eq!(auth_material_to_bearer(&mat), "sess-123");
    }

    #[test]
    fn bearer_from_wellknown() {
        let mat = AuthMaterial::WellKnown {
            env_key: "CUSTOM_KEY".into(),
            token: "wk-tok".into(),
        };
        assert_eq!(auth_material_to_bearer(&mat), "wk-tok");
    }
}
