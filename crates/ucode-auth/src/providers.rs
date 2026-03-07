//! Provider-specific auth metadata.
//!
//! Each known provider has a [`ProviderAuthInfo`] describing which env vars
//! to check and which auth flows are available. Unknown providers return `None`
//! from [`provider_auth_info`] — the caller can still use well-known auth or
//! manual API key entry.

use crate::flows::device_code::DeviceCodeConfig;

/// Auth methods a provider supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMethod {
    /// Direct API key entry.
    ApiKey,
    /// Device code flow (RFC 8628).
    DeviceCode,
    /// Browser-based OAuth with PKCE.
    BrowserOAuth,
    /// Well-known endpoint discovery.
    WellKnown,
    /// No auth required (e.g., local Ollama).
    None,
}

/// Auth metadata for a known provider.
#[derive(Debug, Clone)]
pub struct ProviderAuthInfo {
    /// Human-readable display name.
    pub display_name: &'static str,
    /// Environment variable(s) to check for API key. First match wins.
    pub env_vars: &'static [&'static str],
    /// Supported auth methods, in preference order.
    pub auth_methods: &'static [AuthMethod],
}

/// Look up auth metadata for a known provider.
///
/// Returns `None` for unknown providers — the caller should offer
/// well-known auth or manual API key entry as fallback.
pub fn provider_auth_info(provider: &str) -> Option<ProviderAuthInfo> {
    match provider.to_lowercase().as_str() {
        "openai" => Some(ProviderAuthInfo {
            display_name: "OpenAI",
            env_vars: &["OPENAI_API_KEY"],
            auth_methods: &[AuthMethod::ApiKey],
        }),
        "anthropic" => Some(ProviderAuthInfo {
            display_name: "Anthropic",
            env_vars: &["ANTHROPIC_API_KEY"],
            auth_methods: &[AuthMethod::ApiKey, AuthMethod::BrowserOAuth],
        }),
        "github-copilot" => Some(ProviderAuthInfo {
            display_name: "GitHub Copilot",
            env_vars: &[],
            auth_methods: &[AuthMethod::DeviceCode],
        }),
        "gemini" | "google-gemini" => Some(ProviderAuthInfo {
            display_name: "Google Gemini",
            env_vars: &["GEMINI_API_KEY", "GOOGLE_API_KEY"],
            auth_methods: &[AuthMethod::ApiKey, AuthMethod::BrowserOAuth],
        }),
        "vertex-ai" | "google-vertex" => Some(ProviderAuthInfo {
            display_name: "Google Vertex AI",
            env_vars: &["GOOGLE_APPLICATION_CREDENTIALS"],
            auth_methods: &[AuthMethod::ApiKey],
        }),
        "aws-bedrock" | "bedrock" => Some(ProviderAuthInfo {
            display_name: "AWS Bedrock",
            env_vars: &["AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY", "AWS_REGION"],
            auth_methods: &[AuthMethod::ApiKey],
        }),
        "azure-openai" | "azure" => Some(ProviderAuthInfo {
            display_name: "Azure OpenAI",
            env_vars: &["AZURE_OPENAI_API_KEY"],
            auth_methods: &[AuthMethod::ApiKey],
        }),
        "groq" => Some(ProviderAuthInfo {
            display_name: "Groq",
            env_vars: &["GROQ_API_KEY"],
            auth_methods: &[AuthMethod::ApiKey],
        }),
        "deepseek" => Some(ProviderAuthInfo {
            display_name: "DeepSeek",
            env_vars: &["DEEPSEEK_API_KEY"],
            auth_methods: &[AuthMethod::ApiKey],
        }),
        "openrouter" => Some(ProviderAuthInfo {
            display_name: "OpenRouter",
            env_vars: &["OPENROUTER_API_KEY"],
            auth_methods: &[AuthMethod::ApiKey],
        }),
        "together" => Some(ProviderAuthInfo {
            display_name: "Together",
            env_vars: &["TOGETHER_API_KEY"],
            auth_methods: &[AuthMethod::ApiKey],
        }),
        "fireworks" => Some(ProviderAuthInfo {
            display_name: "Fireworks",
            env_vars: &["FIREWORKS_API_KEY"],
            auth_methods: &[AuthMethod::ApiKey],
        }),
        "mistral" => Some(ProviderAuthInfo {
            display_name: "Mistral",
            env_vars: &["MISTRAL_API_KEY"],
            auth_methods: &[AuthMethod::ApiKey],
        }),
        "ollama" => Some(ProviderAuthInfo {
            display_name: "Ollama",
            env_vars: &[],
            auth_methods: &[AuthMethod::None],
        }),
        _ => None,
    }
}

/// Build a device code config for GitHub Copilot.
///
/// Pass `Some("github.example.com")` for enterprise instances.
pub fn github_copilot_device_config(enterprise_domain: Option<&str>) -> DeviceCodeConfig {
    let domain = enterprise_domain.unwrap_or("github.com");
    DeviceCodeConfig {
        client_id: "Ov23li8tweQw6odWQebz".into(),
        device_code_url: format!("https://{domain}/login/device/code"),
        token_url: format!("https://{domain}/login/oauth/access_token"),
        scope: "read:user".into(),
        grant_type: "urn:ietf:params:oauth:grant-type:device_code".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_provider_openai() {
        let info = provider_auth_info("openai").unwrap();
        assert_eq!(info.display_name, "OpenAI");
        assert_eq!(info.env_vars, &["OPENAI_API_KEY"]);
        assert!(info.auth_methods.contains(&AuthMethod::ApiKey));
    }

    #[test]
    fn known_provider_case_insensitive() {
        assert!(provider_auth_info("OpenAI").is_some());
        assert!(provider_auth_info("ANTHROPIC").is_some());
        assert!(provider_auth_info("GitHub-Copilot").is_some());
    }

    #[test]
    fn unknown_provider_returns_none() {
        assert!(provider_auth_info("my-custom-proxy").is_none());
        assert!(provider_auth_info("").is_none());
    }

    #[test]
    fn github_copilot_has_device_code() {
        let info = provider_auth_info("github-copilot").unwrap();
        assert!(info.auth_methods.contains(&AuthMethod::DeviceCode));
        assert!(info.env_vars.is_empty());
    }

    #[test]
    fn ollama_has_no_auth() {
        let info = provider_auth_info("ollama").unwrap();
        assert!(info.auth_methods.contains(&AuthMethod::None));
    }

    #[test]
    fn gemini_alias_works() {
        let g1 = provider_auth_info("gemini").unwrap();
        let g2 = provider_auth_info("google-gemini").unwrap();
        assert_eq!(g1.display_name, g2.display_name);
    }

    #[test]
    fn copilot_device_config_default() {
        let cfg = github_copilot_device_config(None);
        assert_eq!(cfg.client_id, "Ov23li8tweQw6odWQebz");
        assert!(cfg.device_code_url.contains("github.com"));
        assert!(cfg.token_url.contains("github.com"));
        assert_eq!(cfg.scope, "read:user");
    }

    #[test]
    fn copilot_device_config_enterprise() {
        let cfg = github_copilot_device_config(Some("github.example.com"));
        assert!(cfg.device_code_url.contains("github.example.com"));
        assert!(cfg.token_url.contains("github.example.com"));
    }

    #[test]
    fn all_api_key_providers_have_env_vars() {
        for name in [
            "openai",
            "anthropic",
            "groq",
            "deepseek",
            "openrouter",
            "together",
            "fireworks",
            "mistral",
            "azure-openai",
        ] {
            let info = provider_auth_info(name).unwrap();
            assert!(
                info.auth_methods.contains(&AuthMethod::ApiKey),
                "{name} should support ApiKey"
            );
            assert!(
                !info.env_vars.is_empty(),
                "{name} should have at least one env var"
            );
        }
    }
}
