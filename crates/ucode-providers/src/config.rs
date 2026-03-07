use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterKind {
    Openai,
    Anthropic,
    Ollama,
    Gemini,
}

impl AdapterKind {
    pub fn default_base_url(&self) -> &'static str {
        match self {
            Self::Openai => "https://api.openai.com/v1",
            Self::Anthropic => "https://api.anthropic.com",
            Self::Ollama => "http://localhost:11434",
            Self::Gemini => "https://generativelanguage.googleapis.com",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderConfig {
    #[serde(rename = "type")]
    pub adapter: AdapterKind,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

impl ProviderConfig {
    /// Returns the explicit `base_url` if set, otherwise the adapter's default.
    pub fn base_url(&self) -> &str {
        self.base_url
            .as_deref()
            .unwrap_or_else(|| self.adapter.default_base_url())
    }

    /// Reads the env var named by `api_key_env`. Returns `None` if no env var
    /// is configured or if the variable is not set in the environment.
    pub fn resolve_api_key(&self) -> Option<String> {
        let var_name = self.api_key_env.as_deref()?;
        std::env::var(var_name).ok()
    }
}

/// Top-level TOML structure: `[providers.<name>]` sections.
#[derive(Debug, Clone, Deserialize)]
pub struct ProvidersTable {
    pub providers: HashMap<String, ProviderConfig>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(toml: &str) -> ProvidersTable {
        toml::from_str(toml).expect("valid TOML")
    }

    #[test]
    fn openai_default_base_url() {
        let table = parse(
            r#"
            [providers.openai]
            type = "openai"
            api_key_env = "OPENAI_API_KEY"
            "#,
        );
        let cfg = &table.providers["openai"];
        assert_eq!(cfg.adapter, AdapterKind::Openai);
        assert_eq!(cfg.base_url(), "https://api.openai.com/v1");
        assert_eq!(cfg.api_key_env.as_deref(), Some("OPENAI_API_KEY"));
        assert!(cfg.headers.is_empty());
    }

    #[test]
    fn groq_custom_base_url_openai_type() {
        let table = parse(
            r#"
            [providers.groq]
            type = "openai"
            base_url = "https://api.groq.com/openai/v1"
            api_key_env = "GROQ_API_KEY"
            "#,
        );
        let cfg = &table.providers["groq"];
        assert_eq!(cfg.adapter, AdapterKind::Openai);
        assert_eq!(cfg.base_url(), "https://api.groq.com/openai/v1");
    }

    #[test]
    fn ollama_no_api_key() {
        let table = parse(
            r#"
            [providers.local]
            type = "ollama"
            "#,
        );
        let cfg = &table.providers["local"];
        assert_eq!(cfg.adapter, AdapterKind::Ollama);
        assert_eq!(cfg.base_url(), "http://localhost:11434");
        assert!(cfg.api_key_env.is_none());
        assert!(cfg.resolve_api_key().is_none());
    }

    #[test]
    fn azure_custom_headers() {
        let table = parse(
            r#"
            [providers.azure]
            type = "openai"
            base_url = "https://my-resource.openai.azure.com/openai/deployments/gpt-4"
            api_key_env = "AZURE_OPENAI_KEY"

            [providers.azure.headers]
            "api-version" = "2024-02-01"
            "#,
        );
        let cfg = &table.providers["azure"];
        assert_eq!(
            cfg.headers.get("api-version").map(String::as_str),
            Some("2024-02-01")
        );
    }

    #[test]
    fn gemini_config() {
        let table = parse(
            r#"
            [providers.gemini]
            type = "gemini"
            api_key_env = "GEMINI_API_KEY"
            "#,
        );
        let cfg = &table.providers["gemini"];
        assert_eq!(cfg.adapter, AdapterKind::Gemini);
        assert_eq!(cfg.base_url(), "https://generativelanguage.googleapis.com");
    }

    #[test]
    fn anthropic_config() {
        let table = parse(
            r#"
            [providers.claude]
            type = "anthropic"
            api_key_env = "ANTHROPIC_API_KEY"
            "#,
        );
        let cfg = &table.providers["claude"];
        assert_eq!(cfg.adapter, AdapterKind::Anthropic);
        assert_eq!(cfg.base_url(), "https://api.anthropic.com");
    }

    #[test]
    fn multiple_providers() {
        let table = parse(
            r#"
            [providers.openai]
            type = "openai"
            api_key_env = "OPENAI_API_KEY"

            [providers.local]
            type = "ollama"

            [providers.claude]
            type = "anthropic"
            api_key_env = "ANTHROPIC_API_KEY"
            "#,
        );
        assert_eq!(table.providers.len(), 3);
        assert!(table.providers.contains_key("openai"));
        assert!(table.providers.contains_key("local"));
        assert!(table.providers.contains_key("claude"));
    }

    #[test]
    fn default_base_urls_all_adapters() {
        assert_eq!(
            AdapterKind::Openai.default_base_url(),
            "https://api.openai.com/v1"
        );
        assert_eq!(
            AdapterKind::Anthropic.default_base_url(),
            "https://api.anthropic.com"
        );
        assert_eq!(
            AdapterKind::Ollama.default_base_url(),
            "http://localhost:11434"
        );
        assert_eq!(
            AdapterKind::Gemini.default_base_url(),
            "https://generativelanguage.googleapis.com"
        );
    }

    #[test]
    fn resolve_api_key_missing_env_var() {
        let table = parse(
            r#"
            [providers.test]
            type = "openai"
            api_key_env = "UCODE_TEST_KEY_DEFINITELY_NOT_SET_XYZ123"
            "#,
        );
        let cfg = &table.providers["test"];
        // Env var is configured but not present in environment.
        assert!(cfg.resolve_api_key().is_none());
    }

    #[test]
    fn resolve_api_key_no_env_var_configured() {
        let table = parse(
            r#"
            [providers.test]
            type = "ollama"
            "#,
        );
        let cfg = &table.providers["test"];
        assert!(cfg.resolve_api_key().is_none());
    }
}
