use std::env;

use crate::credential::{AuthMaterial, CredentialStore};
use crate::error::AuthError;

/// Resolve authentication material for a provider.
///
/// Precedence (first non-empty wins):
/// 1. Environment variable named by `api_key_env`
/// 2. Stored credential from the credential store
/// 3. `AuthError::MissingCredential` with a hint for the user
pub fn resolve_auth(
    provider: &str,
    api_key_env: Option<&str>,
    store: &dyn CredentialStore,
) -> Result<AuthMaterial, AuthError> {
    // 1. Check environment variable
    if let Some(env_name) = api_key_env
        && let Ok(value) = env::var(env_name)
        && !value.is_empty()
    {
        return Ok(AuthMaterial::ApiKey { key: value });
    }

    // 2. Check credential store
    match store.load(provider) {
        Ok(mat) => return Ok(mat),
        Err(AuthError::NotFound { .. }) => {}
        Err(e) => return Err(e),
    }

    // 3. Nothing found — surface an actionable error
    Err(AuthError::MissingCredential {
        provider: provider.to_owned(),
        detail: match api_key_env {
            Some(env_name) => format!("set ${env_name} or run `ucode auth login {provider}`"),
            None => format!("run `ucode auth login {provider}`"),
        },
    })
}
