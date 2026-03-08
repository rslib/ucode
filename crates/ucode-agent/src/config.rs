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
// DEFAULT_CONFIG_TEMPLATE
// ---------------------------------------------------------------------------

const DEFAULT_CONFIG_TEMPLATE: &str = r#"# ucode configuration
# https://github.com/rayandrew/ucode
#
# Config home: $UCODE_HOME or $XDG_CONFIG_HOME/ucode or ~/.config/ucode
# Credentials are stored separately in the system keyring + ~/.local/share/ucode/auth.json
#
# Precedence: defaults < this file < environment variables < keyring

# ── Providers ──────────────────────────────────────────────────────────────
#
# Each [providers.<name>] section configures a model provider.
# Supported types: anthropic, openai, ollama, gemini, copilot
#
# API keys can be set via:
#   1. Environment variable (api_key_env below)
#   2. System keyring (via `ucode auth set-key <provider>` or `/connect` in TUI)
#   3. OAuth/device-code login (via `/connect` in TUI)

# [providers.anthropic]
# type = "anthropic"
# api_key_env = "ANTHROPIC_API_KEY"

# [providers.openai]
# type = "openai"
# api_key_env = "OPENAI_API_KEY"

# [providers.gemini]
# type = "gemini"
# api_key_env = "GEMINI_API_KEY"

# [providers.ollama]
# type = "ollama"
# base_url = "http://localhost:11434"

# [providers.copilot]
# type = "copilot"

# ── Custom providers (OpenAI-compatible) ───────────────────────────────────
#
# Any OpenAI-compatible API can be configured with type = "openai" and a
# custom base_url.

# [providers.groq]
# type = "openai"
# base_url = "https://api.groq.com/openai/v1"
# api_key_env = "GROQ_API_KEY"

# [providers.deepseek]
# type = "openai"
# base_url = "https://api.deepseek.com/v1"
# api_key_env = "DEEPSEEK_API_KEY"

# [providers.azure]
# type = "openai"
# base_url = "https://YOUR-RESOURCE.openai.azure.com/openai/deployments/YOUR-DEPLOYMENT"
# api_key_env = "AZURE_OPENAI_KEY"
#
# [providers.azure.headers]
# "api-version" = "2024-02-01"

# ── Agents ─────────────────────────────────────────────────────────────────
#
# Override built-in agent settings. Available agents: coder, explore, planner, orchestrator
# User agents: place markdown files in ~/.config/ucode/agents/
#
# [agents.explore]
# model = "anthropic/claude-haiku-4-5"
# enabled = true
#
# [agents.oracle]
# enabled = false

# ── Theme ──────────────────────────────────────────────────────────────────
#
# Built-in themes: ucode, tokyonight, catppuccin-mocha, gruvbox-dark, nord, dracula
# Cycle themes in the TUI with the theme toggle keybind.
#
# Custom themes: place TOML files in ~/.config/ucode/themes/
# A custom theme can inherit from a built-in with `base = "ucode"` and
# override individual colors or syntax highlighting tokens.

# theme = "ucode"
"#;

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

/// Known providers that store credentials in the credential store (not env vars).
/// Maps (store_provider_id, config_name, adapter_kind).
const KEYRING_PROVIDER_MAP: &[(&str, &str, AdapterKind)] = &[
    ("github-copilot", "github-copilot", AdapterKind::Copilot),
    ("anthropic", "anthropic", AdapterKind::Anthropic),
    ("openai", "openai", AdapterKind::Openai),
    ("gemini", "gemini", AdapterKind::Gemini),
];

// ---------------------------------------------------------------------------
// AppConfig
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub providers: HashMap<String, ProviderConfig>,
    pub agent_overrides: HashMap<String, ucode_core::agent_registry::AgentConfigOverride>,
    pub config_path: Option<PathBuf>,
}

impl AppConfig {
    /// Loads config from `path`. Returns an empty config if the file does not
    /// exist; errors only on malformed TOML.
    pub fn from_file(path: &Path) -> Result<Self, ConfigError> {
        if !path.exists() {
            return Ok(Self {
                providers: HashMap::new(),
                agent_overrides: HashMap::new(),
                config_path: None,
            });
        }

        let raw = std::fs::read_to_string(path)?;
        let table: ProvidersTable = toml::from_str(&raw)?;
        Ok(Self {
            providers: table.providers,
            agent_overrides: table.agents,
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

    /// Adds providers discovered from the credential store (keyring or file fallback).
    /// Skips any provider name already present (TOML/env takes precedence).
    pub fn discover_from_store(&mut self, store: &dyn ucode_auth::CredentialStore) {
        for (store_id, name, kind) in KEYRING_PROVIDER_MAP {
            if self.providers.contains_key(*name) {
                continue;
            }
            if store.load(store_id).is_ok() {
                self.providers.insert(
                    name.to_string(),
                    ProviderConfig {
                        adapter: kind.clone(),
                        base_url: None,
                        api_key_env: None,
                        headers: HashMap::new(),
                    },
                );
            }
        }
    }

    /// Loads from `default_config_home()/ucode.toml`, then discovers env vars.
    /// Call `discover_from_store()` separately if a credential store is available.
    pub fn load_default() -> Result<Self, ConfigError> {
        let path = default_config_home().join("ucode.toml");
        let mut cfg = Self::from_file(&path)?;
        cfg.discover_from_env();
        Ok(cfg)
    }

    pub fn has_providers(&self) -> bool {
        !self.providers.is_empty()
    }

    /// Returns the preferred provider name: anthropic > openai > github-copilot > gemini > first available.
    pub fn default_provider(&self) -> Option<&str> {
        for preferred in ["anthropic", "openai", "github-copilot", "gemini"] {
            if self.providers.contains_key(preferred) {
                return Some(preferred);
            }
        }
        self.providers.keys().next().map(String::as_str)
    }
}

// ---------------------------------------------------------------------------
// ensure_config_file
// ---------------------------------------------------------------------------

/// Creates a default `ucode.toml` with commented-out examples if the file
/// does not already exist. Returns the path to the config file.
///
/// The parent directory is created if missing. On subsequent runs the file is
/// left untouched so user edits are preserved.
pub fn ensure_config_file() -> Result<PathBuf, ConfigError> {
    let path = default_config_home().join("ucode.toml");
    if path.exists() {
        return Ok(path);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, DEFAULT_CONFIG_TEMPLATE)?;
    Ok(path)
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
            agent_overrides: HashMap::new(),
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
            agent_overrides: HashMap::new(),
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
            agent_overrides: HashMap::new(),
            config_path: None,
        };
        assert_eq!(cfg.default_provider(), Some("anthropic"));
    }

    #[test]
    fn has_providers_empty() {
        let cfg = AppConfig {
            providers: HashMap::new(),
            agent_overrides: HashMap::new(),
            config_path: None,
        };
        assert!(!cfg.has_providers());
    }

    #[test]
    fn ensure_config_file_creates_file_in_temp_dir() {
        let dir = tempfile::tempdir().unwrap();
        // Point UCODE_HOME at the temp dir so ensure_config_file writes there.
        unsafe { std::env::set_var("UCODE_HOME", dir.path()) };
        let result = ensure_config_file();
        unsafe { std::env::remove_var("UCODE_HOME") };

        let path = result.expect("should create config file");
        assert!(path.exists(), "config file must exist after creation");
        assert_eq!(path, dir.path().join("ucode.toml"));

        let contents = std::fs::read_to_string(&path).unwrap();
        // Every non-blank line must start with '#' — all entries are commented out.
        for line in contents.lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                assert!(
                    trimmed.starts_with('#'),
                    "non-blank line must be a comment: {line:?}"
                );
            }
        }
    }

    #[test]
    fn from_file_with_agent_overrides() {
        let f = write_temp_toml(
            r#"
            [agents.explore]
            model = "anthropic/claude-haiku-4-5"
            enabled = false

            [agents.coder]
            model = "anthropic/claude-sonnet-4-6"
            "#,
        );
        let cfg = AppConfig::from_file(f.path()).expect("valid TOML");
        assert_eq!(cfg.agent_overrides.len(), 2);
        assert_eq!(cfg.agent_overrides["explore"].enabled, Some(false));
        assert_eq!(
            cfg.agent_overrides["coder"].model.as_deref(),
            Some("anthropic/claude-sonnet-4-6")
        );
    }

    #[test]
    fn from_file_empty_has_no_agent_overrides() {
        let f = write_temp_toml("# just a comment\n");
        let cfg = AppConfig::from_file(f.path()).expect("valid TOML");
        assert!(cfg.agent_overrides.is_empty());
    }

    #[test]
    fn ensure_config_file_does_not_overwrite_existing() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("ucode.toml");
        std::fs::write(&config_path, "# user content").unwrap();

        unsafe { std::env::set_var("UCODE_HOME", dir.path()) };
        let result = ensure_config_file();
        unsafe { std::env::remove_var("UCODE_HOME") };

        result.expect("should succeed when file already exists");
        let contents = std::fs::read_to_string(&config_path).unwrap();
        assert_eq!(
            contents, "# user content",
            "existing file must not be modified"
        );
    }
}
