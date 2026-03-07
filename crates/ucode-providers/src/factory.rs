use std::sync::Arc;

use ucode_auth::CredentialStore;

use crate::config::{AdapterKind, ProviderConfig};
use crate::provider::Provider;
use ucode_core::CoreError;

/// Named result type for a single provider creation attempt.
pub type ProviderResult = (String, Result<Box<dyn Provider>, CoreError>);

/// Create a provider instance from config.
///
/// Resolves the API key from the environment variable named by `api_key_env`.
/// If `credential_store` is provided, missing env vars are not an error for
/// Anthropic and Gemini — the store will be consulted at request time.
pub fn create_provider(
    name: &str,
    config: &ProviderConfig,
    credential_store: Option<Arc<dyn CredentialStore>>,
) -> Result<Box<dyn Provider>, CoreError> {
    let api_key = config.resolve_api_key();

    match config.adapter {
        AdapterKind::Openai => Ok(Box::new(crate::openai::OpenAiCompatProvider::from_config(
            name,
            config,
            api_key,
            credential_store,
        ))),
        AdapterKind::Anthropic => {
            if api_key.is_none() && config.api_key_env.is_some() && credential_store.is_none() {
                return Err(CoreError::Auth {
                    provider: name.to_owned(),
                    auth_kind: ucode_core::AuthErrorKind::Missing,
                });
            }
            Ok(Box::new(
                crate::anthropic::AnthropicCompatProvider::from_config(
                    name,
                    config,
                    api_key,
                    credential_store,
                ),
            ))
        }
        AdapterKind::Ollama => Ok(Box::new(crate::ollama::OllamaProvider::from_config(
            name,
            config,
            api_key,
            credential_store,
        ))),
        AdapterKind::Gemini => {
            if api_key.is_none() && config.api_key_env.is_some() && credential_store.is_none() {
                return Err(CoreError::Auth {
                    provider: name.to_owned(),
                    auth_kind: ucode_core::AuthErrorKind::Missing,
                });
            }
            Ok(Box::new(crate::gemini::GeminiProvider::from_config(
                name,
                config,
                api_key,
                credential_store,
            )))
        }
    }
}

/// Create all providers from a providers table.
///
/// Returns a vec of (name, result) pairs. Providers that fail to create
/// (e.g., missing API key) return errors but don't prevent other providers
/// from being created.
pub fn create_all_providers(
    configs: &std::collections::HashMap<String, ProviderConfig>,
    credential_store: Option<Arc<dyn CredentialStore>>,
) -> Vec<ProviderResult> {
    configs
        .iter()
        .map(|(name, config)| {
            (
                name.clone(),
                create_provider(name, config, credential_store.clone()),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn create_ollama_no_api_key() {
        let config = ProviderConfig {
            adapter: AdapterKind::Ollama,
            base_url: None,
            api_key_env: None,
            headers: HashMap::new(),
        };
        let provider = create_provider("ollama", &config, None).unwrap();
        assert_eq!(provider.name(), "ollama");
    }

    #[test]
    fn create_openai_compat_as_groq() {
        let config = ProviderConfig {
            adapter: AdapterKind::Openai,
            base_url: Some("https://api.groq.com/openai/v1".into()),
            api_key_env: None,
            headers: HashMap::new(),
        };
        let provider = create_provider("groq", &config, None).unwrap();
        assert_eq!(provider.name(), "groq");
    }

    #[test]
    fn create_openai_no_key_ok() {
        // OpenAI adapter allows no key (e.g., local vLLM)
        let config = ProviderConfig {
            adapter: AdapterKind::Openai,
            base_url: Some("http://localhost:8000/v1".into()),
            api_key_env: None,
            headers: HashMap::new(),
        };
        let provider = create_provider("local-vllm", &config, None).unwrap();
        assert_eq!(provider.name(), "local-vllm");
    }

    #[test]
    fn create_anthropic_missing_key_with_env_configured() {
        let config = ProviderConfig {
            adapter: AdapterKind::Anthropic,
            base_url: None,
            api_key_env: Some("UCODE_TEST_NONEXISTENT_ANTHROPIC_KEY_XYZ".into()),
            headers: HashMap::new(),
        };
        let result = create_provider("anthropic", &config, None);
        assert!(result.is_err());
        match result.err().unwrap() {
            CoreError::Auth {
                provider,
                auth_kind,
            } => {
                assert_eq!(provider, "anthropic");
                assert_eq!(auth_kind, ucode_core::AuthErrorKind::Missing);
            }
            other => panic!("expected Auth error, got {other:?}"),
        }
    }

    #[test]
    fn create_anthropic_no_env_configured_ok() {
        // No api_key_env set — provider created without key (e.g., proxy handles auth)
        let config = ProviderConfig {
            adapter: AdapterKind::Anthropic,
            base_url: None,
            api_key_env: None,
            headers: HashMap::new(),
        };
        let provider = create_provider("anthropic-proxy", &config, None).unwrap();
        assert_eq!(provider.name(), "anthropic-proxy");
    }

    #[test]
    fn create_gemini_missing_key_with_env_configured() {
        let config = ProviderConfig {
            adapter: AdapterKind::Gemini,
            base_url: None,
            api_key_env: Some("UCODE_TEST_NONEXISTENT_GEMINI_KEY_XYZ".into()),
            headers: HashMap::new(),
        };
        let result = create_provider("gemini", &config, None);
        assert!(result.is_err());
    }

    #[test]
    fn create_gemini_no_env_configured_ok() {
        let config = ProviderConfig {
            adapter: AdapterKind::Gemini,
            base_url: None,
            api_key_env: None,
            headers: HashMap::new(),
        };
        let provider = create_provider("gemini-proxy", &config, None).unwrap();
        assert_eq!(provider.name(), "gemini-proxy");
    }

    #[test]
    fn create_all_providers_mixed() {
        let mut configs = HashMap::new();
        configs.insert(
            "ollama".into(),
            ProviderConfig {
                adapter: AdapterKind::Ollama,
                base_url: None,
                api_key_env: None,
                headers: HashMap::new(),
            },
        );
        configs.insert(
            "openai".into(),
            ProviderConfig {
                adapter: AdapterKind::Openai,
                base_url: None,
                api_key_env: None,
                headers: HashMap::new(),
            },
        );
        let results = create_all_providers(&configs, None);
        assert_eq!(results.len(), 2);
        for (_, result) in &results {
            assert!(result.is_ok());
        }
    }

    #[test]
    fn create_all_providers_partial_failure() {
        let mut configs = HashMap::new();
        configs.insert(
            "ollama".into(),
            ProviderConfig {
                adapter: AdapterKind::Ollama,
                base_url: None,
                api_key_env: None,
                headers: HashMap::new(),
            },
        );
        configs.insert(
            "anthropic".into(),
            ProviderConfig {
                adapter: AdapterKind::Anthropic,
                base_url: None,
                api_key_env: Some("UCODE_TEST_NONEXISTENT_KEY_XYZ".into()),
                headers: HashMap::new(),
            },
        );
        let results = create_all_providers(&configs, None);
        assert_eq!(results.len(), 2);
        // One should succeed, one should fail
        let successes: Vec<_> = results.iter().filter(|(_, r)| r.is_ok()).collect();
        let failures: Vec<_> = results.iter().filter(|(_, r)| r.is_err()).collect();
        assert_eq!(successes.len(), 1);
        assert_eq!(failures.len(), 1);
    }

    #[test]
    fn create_provider_with_credential_store() {
        use ucode_auth::{AuthMaterial, InMemoryStore};
        let store = Arc::new(InMemoryStore::new());
        store
            .store(
                "openai",
                &AuthMaterial::ApiKey {
                    key: "sk-from-store".into(),
                },
            )
            .unwrap();
        let config = ProviderConfig {
            adapter: AdapterKind::Openai,
            base_url: None,
            api_key_env: None,
            headers: HashMap::new(),
        };
        let provider = create_provider("openai", &config, Some(store)).unwrap();
        assert_eq!(provider.name(), "openai");
    }

    #[test]
    fn create_anthropic_with_store_no_env_var_ok() {
        use ucode_auth::InMemoryStore;
        let store = Arc::new(InMemoryStore::new());
        let config = ProviderConfig {
            adapter: AdapterKind::Anthropic,
            base_url: None,
            api_key_env: Some("UCODE_TEST_NONEXISTENT_KEY_XYZ".into()),
            headers: HashMap::new(),
        };
        let result = create_provider("anthropic", &config, Some(store));
        assert!(result.is_ok());
    }
}
