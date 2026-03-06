use serde::{Deserialize, Serialize};

/// A parsed plugin manifest from `plugin.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    /// Reverse-domain globally unique identifier (e.g., "org.acme.code-analyzer").
    pub id: Option<String>,
    /// Human-readable display name.
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub author: Option<String>,
    /// Minimum host API version required.
    pub min_api_version: Option<String>,
    /// API feature surfaces this plugin requires (e.g., ["hooks", "tools", "ui"]).
    #[serde(default)]
    pub required_features: Vec<String>,
    /// Hooks this plugin subscribes to.
    #[serde(default)]
    pub hooks: Vec<String>,
    /// Tools this plugin exports.
    #[serde(default)]
    pub tools: Vec<PluginToolDef>,
    /// Capabilities requested by this plugin.
    #[serde(default)]
    pub capabilities: PluginCapabilities,
}

/// A tool definition exported by a plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginToolDef {
    pub name: String,
    pub description: Option<String>,
    /// JSON Schema for the tool's input.
    pub input_schema: Option<serde_json::Value>,
}

/// Capabilities requested by a plugin.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginCapabilities {
    /// Whether the plugin needs filesystem access.
    #[serde(default)]
    pub filesystem: bool,
    /// Whether the plugin needs network access.
    #[serde(default)]
    pub network: bool,
    /// Whether the plugin needs to spawn processes.
    #[serde(default)]
    pub process_spawn: bool,
    /// Whether the plugin may issue guarded UI calls (modals, prompts, transcript injection).
    #[serde(default)]
    pub guarded_ui: bool,
    /// Filesystem path scopes (relative to workspace root). Empty = workspace root only.
    #[serde(default)]
    pub filesystem_paths: Vec<String>,
    /// Network domain access. Empty = all domains (when network is true).
    #[serde(default)]
    pub network_domains: Vec<String>,
    /// Hook categories this plugin wants to handle. Empty = all categories.
    #[serde(default)]
    pub hook_categories: Vec<String>,
    /// Maximum override class: "safe", "guarded", or "risky". None = "safe".
    #[serde(default)]
    pub max_override_class: Option<String>,
}

/// Errors from manifest parsing.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("manifest I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("manifest parse error: {0}")]
    Parse(String),
    #[error("manifest validation error: {0}")]
    Validation(String),
}

const KNOWN_FEATURES: &[&str] = &["hooks", "tools", "ui"];

/// Parse a plugin manifest from TOML string.
pub fn parse_manifest(toml_str: &str) -> Result<PluginManifest, ManifestError> {
    let manifest: PluginManifest =
        toml::from_str(toml_str).map_err(|e| ManifestError::Parse(e.to_string()))?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

/// Parse a plugin manifest from a file path.
pub fn parse_manifest_file(path: &std::path::Path) -> Result<PluginManifest, ManifestError> {
    let content = std::fs::read_to_string(path)?;
    parse_manifest(&content)
}

/// Validate a parsed manifest.
pub fn validate_manifest(manifest: &PluginManifest) -> Result<(), ManifestError> {
    if manifest.name.is_empty() {
        return Err(ManifestError::Validation("name must not be empty".into()));
    }
    if manifest.version.is_empty() {
        return Err(ManifestError::Validation(
            "version must not be empty".into(),
        ));
    }
    if let Some(id) = &manifest.id {
        validate_plugin_id(id)?;
    }
    for feature in &manifest.required_features {
        if !KNOWN_FEATURES.contains(&feature.as_str()) {
            return Err(ManifestError::Validation(format!(
                "unknown feature '{}'; known features: {}",
                feature,
                KNOWN_FEATURES.join(", ")
            )));
        }
    }
    for tool in &manifest.tools {
        if tool.name.is_empty() {
            return Err(ManifestError::Validation(
                "tool name must not be empty".into(),
            ));
        }
        if tool.name.contains('.') {
            return Err(ManifestError::Validation(format!(
                "tool name '{}' must not contain dots; host constructs FQN from plugin id",
                tool.name
            )));
        }
    }
    for hook in &manifest.hooks {
        if hook.is_empty() {
            return Err(ManifestError::Validation(
                "hook name must not be empty".into(),
            ));
        }
    }
    if let Some(ref class) = manifest.capabilities.max_override_class
        && !["safe", "guarded", "risky"].contains(&class.as_str())
    {
        return Err(ManifestError::Validation(format!(
            "max_override_class '{}' must be one of: safe, guarded, risky",
            class
        )));
    }
    Ok(())
}

fn validate_plugin_id(id: &str) -> Result<(), ManifestError> {
    let segments: Vec<&str> = id.split('.').collect();
    if segments.len() < 3 {
        return Err(ManifestError::Validation(format!(
            "plugin id '{}' must have at least 3 dot-separated segments (e.g., org.acme.plugin)",
            id
        )));
    }
    for segment in &segments {
        if segment.is_empty() {
            return Err(ManifestError::Validation(format!(
                "plugin id '{}' has empty segment",
                id
            )));
        }
        if !segment
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(ManifestError::Validation(format!(
                "plugin id segment '{}' must contain only lowercase letters, digits, and hyphens",
                segment
            )));
        }
        if segment.starts_with('-') {
            return Err(ManifestError::Validation(format!(
                "plugin id segment '{}' must not start with a hyphen",
                segment
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_parse_minimal_manifest() {
        let toml = r#"
            name = "my-plugin"
            version = "1.0.0"
        "#;
        let m = parse_manifest(toml).unwrap();
        assert_eq!(m.name, "my-plugin");
        assert_eq!(m.version, "1.0.0");
        assert!(m.description.is_none());
        assert!(m.author.is_none());
        assert!(m.hooks.is_empty());
        assert!(m.tools.is_empty());
    }

    #[test]
    fn test_parse_full_manifest() {
        let toml = r#"
            name = "full-plugin"
            version = "2.3.1"
            description = "A complete plugin"
            author = "Alice"
            min_api_version = "0.5.0"
        "#;
        let m = parse_manifest(toml).unwrap();
        assert_eq!(m.name, "full-plugin");
        assert_eq!(m.version, "2.3.1");
        assert_eq!(m.description.as_deref(), Some("A complete plugin"));
        assert_eq!(m.author.as_deref(), Some("Alice"));
        assert_eq!(m.min_api_version.as_deref(), Some("0.5.0"));
    }

    #[test]
    fn test_parse_manifest_with_tools() {
        let toml = r#"
            name = "tool-plugin"
            version = "1.0.0"

            [[tools]]
            name = "search"
            description = "Search the web"

            [[tools]]
            name = "fetch"
        "#;
        let m = parse_manifest(toml).unwrap();
        assert_eq!(m.tools.len(), 2);
        assert_eq!(m.tools[0].name, "search");
        assert_eq!(m.tools[0].description.as_deref(), Some("Search the web"));
        assert_eq!(m.tools[1].name, "fetch");
        assert!(m.tools[1].description.is_none());
    }

    #[test]
    fn test_parse_manifest_with_hooks() {
        let toml = r#"
            name = "hook-plugin"
            version = "1.0.0"
            hooks = ["pre-tool-use", "post-tool-use", "session-start"]
        "#;
        let m = parse_manifest(toml).unwrap();
        assert_eq!(m.hooks, ["pre-tool-use", "post-tool-use", "session-start"]);
    }

    #[test]
    fn test_parse_manifest_with_capabilities() {
        let toml = r#"
            name = "cap-plugin"
            version = "1.0.0"

            [capabilities]
            filesystem = true
            network = true
            process_spawn = false
            guarded_ui = true
        "#;
        let m = parse_manifest(toml).unwrap();
        assert!(m.capabilities.filesystem);
        assert!(m.capabilities.network);
        assert!(!m.capabilities.process_spawn);
        assert!(m.capabilities.guarded_ui);
    }

    #[test]
    fn test_validate_empty_name_fails() {
        let toml = r#"name = "" \nversion = "1.0.0""#;
        // Build directly to avoid TOML parse ambiguity
        let manifest = PluginManifest {
            id: None,
            name: String::new(),
            version: "1.0.0".into(),
            description: None,
            author: None,
            min_api_version: None,
            required_features: vec![],
            hooks: vec![],
            tools: vec![],
            capabilities: PluginCapabilities::default(),
        };
        let err = validate_manifest(&manifest).unwrap_err();
        assert!(matches!(err, ManifestError::Validation(_)));
        assert!(err.to_string().contains("name"));
        let _ = toml; // suppress unused warning
    }

    #[test]
    fn test_validate_empty_version_fails() {
        let manifest = PluginManifest {
            id: None,
            name: "ok".into(),
            version: String::new(),
            description: None,
            author: None,
            min_api_version: None,
            required_features: vec![],
            hooks: vec![],
            tools: vec![],
            capabilities: PluginCapabilities::default(),
        };
        let err = validate_manifest(&manifest).unwrap_err();
        assert!(matches!(err, ManifestError::Validation(_)));
        assert!(err.to_string().contains("version"));
    }

    #[test]
    fn test_validate_empty_tool_name_fails() {
        let manifest = PluginManifest {
            id: None,
            name: "ok".into(),
            version: "1.0.0".into(),
            description: None,
            author: None,
            min_api_version: None,
            required_features: vec![],
            hooks: vec![],
            tools: vec![PluginToolDef {
                name: String::new(),
                description: None,
                input_schema: None,
            }],
            capabilities: PluginCapabilities::default(),
        };
        let err = validate_manifest(&manifest).unwrap_err();
        assert!(matches!(err, ManifestError::Validation(_)));
        assert!(err.to_string().contains("tool name"));
    }

    #[test]
    fn test_validate_empty_hook_name_fails() {
        let manifest = PluginManifest {
            id: None,
            name: "ok".into(),
            version: "1.0.0".into(),
            description: None,
            author: None,
            min_api_version: None,
            required_features: vec![],
            hooks: vec![String::new()],
            tools: vec![],
            capabilities: PluginCapabilities::default(),
        };
        let err = validate_manifest(&manifest).unwrap_err();
        assert!(matches!(err, ManifestError::Validation(_)));
        assert!(err.to_string().contains("hook name"));
    }

    #[test]
    fn test_parse_invalid_toml_fails() {
        let bad = "name = [unclosed";
        let err = parse_manifest(bad).unwrap_err();
        assert!(matches!(err, ManifestError::Parse(_)));
    }

    #[test]
    fn test_parse_manifest_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plugin.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, r#"name = "file-plugin""#).unwrap();
        writeln!(f, r#"version = "0.1.0""#).unwrap();
        drop(f);

        let m = parse_manifest_file(&path).unwrap();
        assert_eq!(m.name, "file-plugin");
        assert_eq!(m.version, "0.1.0");
    }

    #[test]
    fn test_parse_manifest_file_not_found() {
        let err =
            parse_manifest_file(std::path::Path::new("/nonexistent/plugin.toml")).unwrap_err();
        assert!(matches!(err, ManifestError::Io(_)));
    }

    // --- New tests for id, required_features, and stricter validation ---

    #[test]
    fn test_parse_manifest_with_id() {
        let toml = r#"
            id = "org.acme.code-analyzer"
            name = "Code Analyzer"
            version = "1.0.0"
        "#;
        let m = parse_manifest(toml).unwrap();
        assert_eq!(m.id.as_deref(), Some("org.acme.code-analyzer"));
        assert_eq!(m.name, "Code Analyzer");
    }

    #[test]
    fn test_validate_id_format_valid() {
        let toml = r#"
            id = "org.acme.code-analyzer"
            name = "Code Analyzer"
            version = "1.0.0"
        "#;
        assert!(parse_manifest(toml).is_ok());
    }

    #[test]
    fn test_validate_id_format_too_few_segments() {
        let toml = r#"
            id = "acme.plugin"
            name = "Bad Plugin"
            version = "1.0.0"
        "#;
        let err = parse_manifest(toml).unwrap_err();
        assert!(
            err.to_string()
                .contains("at least 3 dot-separated segments")
        );
    }

    #[test]
    fn test_validate_id_format_invalid_chars() {
        let toml = r#"
            id = "org.Acme.Plugin"
            name = "Bad Plugin"
            version = "1.0.0"
        "#;
        let err = parse_manifest(toml).unwrap_err();
        assert!(err.to_string().contains("segment"));
    }

    #[test]
    fn test_validate_id_format_empty_segment() {
        let toml = r#"
            id = "org..plugin"
            name = "Bad Plugin"
            version = "1.0.0"
        "#;
        let err = parse_manifest(toml).unwrap_err();
        assert!(err.to_string().contains("segment"));
    }

    #[test]
    fn test_parse_required_features() {
        let toml = r#"
            id = "org.acme.logger"
            name = "Logger"
            version = "1.0.0"
            required_features = ["hooks", "tools"]
        "#;
        let m = parse_manifest(toml).unwrap();
        assert_eq!(m.required_features, vec!["hooks", "tools"]);
    }

    #[test]
    fn test_validate_unknown_feature() {
        let toml = r#"
            id = "org.acme.logger"
            name = "Logger"
            version = "1.0.0"
            required_features = ["hooks", "quantum"]
        "#;
        let err = parse_manifest(toml).unwrap_err();
        assert!(err.to_string().contains("unknown feature"));
    }

    #[test]
    fn test_validate_tool_name_no_dots() {
        let toml = r#"
            id = "org.acme.tools"
            name = "Tools"
            version = "1.0.0"

            [[tools]]
            name = "my.tool"
        "#;
        let err = parse_manifest(toml).unwrap_err();
        assert!(err.to_string().contains("must not contain dots"));
    }

    #[test]
    fn test_backward_compat_name_only() {
        // Old-style manifest without id should still parse (name used as display name)
        let toml = r#"
            name = "my-plugin"
            version = "1.0.0"
        "#;
        let m = parse_manifest(toml).unwrap();
        assert!(m.id.is_none());
        assert_eq!(m.name, "my-plugin");
    }

    #[test]
    fn test_parse_manifest_with_scoped_capabilities() {
        let toml = r#"
            name = "scoped-plugin"
            version = "1.0.0"

            [capabilities]
            filesystem = true
            network = true
            process_spawn = false
            guarded_ui = false
            filesystem_paths = ["src/", "tests/"]
            network_domains = ["api.example.com", "cdn.example.com"]
            hook_categories = ["session", "tool"]
            max_override_class = "guarded"
        "#;
        let m = parse_manifest(toml).unwrap();
        assert!(m.capabilities.filesystem);
        assert!(m.capabilities.network);
        assert_eq!(m.capabilities.filesystem_paths, vec!["src/", "tests/"]);
        assert_eq!(
            m.capabilities.network_domains,
            vec!["api.example.com", "cdn.example.com"]
        );
        assert_eq!(m.capabilities.hook_categories, vec!["session", "tool"]);
        assert_eq!(
            m.capabilities.max_override_class.as_deref(),
            Some("guarded")
        );
    }

    #[test]
    fn test_parse_manifest_scoped_caps_default_empty() {
        let toml = r#"
            name = "minimal-plugin"
            version = "1.0.0"

            [capabilities]
            filesystem = true
        "#;
        let m = parse_manifest(toml).unwrap();
        assert!(m.capabilities.filesystem);
        assert!(m.capabilities.filesystem_paths.is_empty());
        assert!(m.capabilities.network_domains.is_empty());
        assert!(m.capabilities.hook_categories.is_empty());
        assert!(m.capabilities.max_override_class.is_none());
    }

    #[test]
    fn test_validate_invalid_override_class() {
        let toml = r#"
            name = "bad-plugin"
            version = "1.0.0"

            [capabilities]
            max_override_class = "superadmin"
        "#;
        let err = parse_manifest(toml).unwrap_err();
        assert!(err.to_string().contains("max_override_class"));
    }
}
