use serde::{Deserialize, Serialize};

/// A parsed plugin manifest from `plugin.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub author: Option<String>,
    /// Minimum host API version required.
    pub min_api_version: Option<String>,
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
    for tool in &manifest.tools {
        if tool.name.is_empty() {
            return Err(ManifestError::Validation(
                "tool name must not be empty".into(),
            ));
        }
    }
    for hook in &manifest.hooks {
        if hook.is_empty() {
            return Err(ManifestError::Validation(
                "hook name must not be empty".into(),
            ));
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
            name: String::new(),
            version: "1.0.0".into(),
            description: None,
            author: None,
            min_api_version: None,
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
            name: "ok".into(),
            version: String::new(),
            description: None,
            author: None,
            min_api_version: None,
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
            name: "ok".into(),
            version: "1.0.0".into(),
            description: None,
            author: None,
            min_api_version: None,
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
            name: "ok".into(),
            version: "1.0.0".into(),
            description: None,
            author: None,
            min_api_version: None,
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
}
