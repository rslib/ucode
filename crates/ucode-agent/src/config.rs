use std::collections::HashMap;
use std::path::{Path, PathBuf};

use thiserror::Error;

use ucode_core::logging::default_config_home;
use ucode_providers::config::{AdapterKind, ProviderConfig, ProvidersTable};

// ---------------------------------------------------------------------------
// ConfigError
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("I/O error reading config: {0}")]
    Io(#[from] std::io::Error),
    #[error("TOML parse error: {0}")]
    Parse(#[from] toml::de::Error),
}

// ---------------------------------------------------------------------------
// ENV_PROVIDER_MAP
// ---------------------------------------------------------------------------

/// Maps (env_var_name, provider_name, adapter_kind).
/// TOML takes precedence: `discover_from_env` skips entries already present.
const ENV_PROVIDER_MAP: &[(&str, &str, AdapterKind)] = &[
    ("ANTHROPIC_API_KEY", "anthropic", AdapterKind::Anthropic),
    ("OPENAI_API_KEY", "openai", AdapterKind::Openai),
    ("GEMINI_API_KEY", "gemini", AdapterKind::Gemini),
    ("GOOGLE_API_KEY", "gemini", AdapterKind::Gemini),
];

// ---------------------------------------------------------------------------
// AppConfig
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub providers: HashMap<String, ProviderConfig>,
    pub config_path: Option<PathBuf>,
}

impl AppConfig {
    /// Loads config from `path`. Returns an empty config if the file does not
    /// exist; errors only on malformed TOML.
    pub fn from_file(path: &Path) -> Result<Self, ConfigError> {
        if !path.exists() {
            return Ok(Self {
                providers: HashMap::new(),
                config_path: None,
            });
        }

        let raw = std::fs::read_to_string(path)?;
        let table: ProvidersTable = toml::from_str(&raw)?;
        Ok(Self {
            providers: table.providers,
            config_path: Some(path.to_owned()),
        })
    }

    /// Adds providers discovered from environment variables.
    /// Skips any provider name already present (TOML takes precedence).
    pub fn discover_from_env(&mut self) {
        for (env_var, name, kind) in ENV_PROVIDER_MAP {
            if self.providers.contains_key(*name) {
                continue;
            }
            if std::env::var(env_var).is_ok() {
                self.providers.insert(
                    name.to_string(),
                    ProviderConfig {
                        adapter: kind.clone(),
                        base_url: None,
                        api_key_env: Some(env_var.to_string()),
                        headers: HashMap::new(),
                    },
                );
            }
        }
    }

    /// Loads from `default_config_home()/ucode.toml`, then discovers env vars.
    pub fn load_default() -> Result<Self, ConfigError> {
        let path = default_config_home().join("ucode.toml");
        let mut cfg = Self::from_file(&path)?;
        cfg.discover_from_env();
        Ok(cfg)
    }

    pub fn has_providers(&self) -> bool {
        !self.providers.is_empty()
    }

    /// Returns the preferred provider name: anthropic > openai > gemini > first available.
    pub fn default_provider(&self) -> Option<&str> {
        for preferred in ["anthropic", "openai", "gemini"] {
            if self.providers.contains_key(preferred) {
                return Some(preferred);
            }
        }
        self.providers.keys().next().map(String::as_str)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn write_temp_toml(content: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    #[test]
    fn from_file_missing_returns_empty() {
        let cfg = AppConfig::from_file(Path::new("/tmp/ucode-nonexistent-config-xyz.toml"))
            .expect("missing file should not error");
        assert!(!cfg.has_providers());
        assert!(cfg.config_path.is_none());
    }

    #[test]
    fn from_file_valid_toml() {
        let f = write_temp_toml(
            r#"
            [providers.openai]
            type = "openai"
            api_key_env = "OPENAI_API_KEY"

            [providers.anthropic]
            type = "anthropic"
            api_key_env = "ANTHROPIC_API_KEY"
            "#,
        );
        let cfg = AppConfig::from_file(f.path()).expect("valid TOML");
        assert!(cfg.has_providers());
        assert_eq!(cfg.providers.len(), 2);
        assert!(cfg.providers.contains_key("openai"));
        assert!(cfg.providers.contains_key("anthropic"));
        assert_eq!(cfg.config_path.as_deref(), Some(f.path()));
    }

    #[test]
    fn from_file_malformed_toml_errors() {
        let f = write_temp_toml("this is not valid toml ][[[");
        let result = AppConfig::from_file(f.path());
        assert!(matches!(result, Err(ConfigError::Parse(_))));
    }

    #[test]
    fn discover_env_adds_missing_providers() {
        let mut cfg = AppConfig {
            providers: HashMap::new(),
            config_path: None,
        };
        unsafe { std::env::set_var("ANTHROPIC_API_KEY", "sk-test-key") };
        cfg.discover_from_env();
        unsafe { std::env::remove_var("ANTHROPIC_API_KEY") };

        assert!(cfg.providers.contains_key("anthropic"));
        let p = &cfg.providers["anthropic"];
        assert_eq!(p.adapter, AdapterKind::Anthropic);
        assert_eq!(p.api_key_env.as_deref(), Some("ANTHROPIC_API_KEY"));
    }

    #[test]
    fn discover_env_does_not_override_toml() {
        let custom_url = "https://my-proxy.example.com/v1";
        let mut providers = HashMap::new();
        providers.insert(
            "openai".to_string(),
            ProviderConfig {
                adapter: AdapterKind::Openai,
                base_url: Some(custom_url.to_string()),
                api_key_env: Some("MY_CUSTOM_KEY".to_string()),
                headers: HashMap::new(),
            },
        );
        let mut cfg = AppConfig {
            providers,
            config_path: None,
        };

        unsafe { std::env::set_var("OPENAI_API_KEY", "sk-should-not-override") };
        cfg.discover_from_env();
        unsafe { std::env::remove_var("OPENAI_API_KEY") };

        // base_url and api_key_env must remain from the TOML-sourced entry.
        let p = &cfg.providers["openai"];
        assert_eq!(p.base_url.as_deref(), Some(custom_url));
        assert_eq!(p.api_key_env.as_deref(), Some("MY_CUSTOM_KEY"));
    }

    #[test]
    fn default_provider_prefers_anthropic() {
        let mut providers = HashMap::new();
        providers.insert(
            "openai".to_string(),
            ProviderConfig {
                adapter: AdapterKind::Openai,
                base_url: None,
                api_key_env: Some("OPENAI_API_KEY".to_string()),
                headers: HashMap::new(),
            },
        );
        providers.insert(
            "anthropic".to_string(),
            ProviderConfig {
                adapter: AdapterKind::Anthropic,
                base_url: None,
                api_key_env: Some("ANTHROPIC_API_KEY".to_string()),
                headers: HashMap::new(),
            },
        );
        let cfg = AppConfig {
            providers,
            config_path: None,
        };
        assert_eq!(cfg.default_provider(), Some("anthropic"));
    }

    #[test]
    fn has_providers_empty() {
        let cfg = AppConfig {
            providers: HashMap::new(),
            config_path: None,
        };
        assert!(!cfg.has_providers());
    }
}
