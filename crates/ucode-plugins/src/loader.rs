use std::path::{Path, PathBuf};

use crate::manifest::{ManifestError, PluginManifest, parse_manifest_file};

/// Information about a discovered plugin.
#[derive(Debug, Clone)]
pub struct PluginInfo {
    pub manifest: PluginManifest,
    pub manifest_path: PathBuf,
    pub plugin_dir: PathBuf,
}

/// Status of a loaded plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginStatus {
    /// Discovered but not yet activated.
    Discovered,
    /// Activated and ready.
    Active,
    /// Deactivated by user or policy.
    Inactive,
    /// Failed to load.
    Failed { reason: String },
}

/// Registry of discovered and loaded plugins.
pub struct PluginRegistry {
    plugins: Vec<(PluginInfo, PluginStatus)>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
        }
    }

    /// Register a discovered plugin.
    pub fn register(&mut self, info: PluginInfo) {
        self.plugins.push((info, PluginStatus::Discovered));
    }

    /// List all plugins with their status.
    pub fn list(&self) -> Vec<(&PluginInfo, &PluginStatus)> {
        self.plugins.iter().map(|(i, s)| (i, s)).collect()
    }

    /// Find a plugin by name.
    pub fn find(&self, name: &str) -> Option<(&PluginInfo, &PluginStatus)> {
        self.plugins
            .iter()
            .find(|(i, _)| i.manifest.name == name)
            .map(|(i, s)| (i, s))
    }

    /// Activate a plugin by name. Returns false if not found.
    pub fn activate(&mut self, name: &str) -> bool {
        if let Some((_, status)) = self
            .plugins
            .iter_mut()
            .find(|(i, _)| i.manifest.name == name)
        {
            *status = PluginStatus::Active;
            true
        } else {
            false
        }
    }

    /// Deactivate a plugin by name. Returns false if not found.
    pub fn deactivate(&mut self, name: &str) -> bool {
        if let Some((_, status)) = self
            .plugins
            .iter_mut()
            .find(|(i, _)| i.manifest.name == name)
        {
            *status = PluginStatus::Inactive;
            true
        } else {
            false
        }
    }

    /// Mark a plugin as failed.
    pub fn mark_failed(&mut self, name: &str, reason: &str) -> bool {
        if let Some((_, status)) = self
            .plugins
            .iter_mut()
            .find(|(i, _)| i.manifest.name == name)
        {
            *status = PluginStatus::Failed {
                reason: reason.to_owned(),
            };
            true
        } else {
            false
        }
    }

    /// Get count of plugins by status.
    pub fn count_by_status(&self, target: &PluginStatus) -> usize {
        self.plugins.iter().filter(|(_, s)| s == target).count()
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Discover plugins from a list of search directories.
///
/// Each directory is scanned for subdirectories containing `plugin.toml`.
pub fn discover_plugins(search_dirs: &[&Path]) -> Vec<Result<PluginInfo, ManifestError>> {
    let mut results = Vec::new();
    for dir in search_dirs {
        if !dir.is_dir() {
            continue;
        }
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let manifest_path = path.join("plugin.toml");
            if manifest_path.is_file() {
                match parse_manifest_file(&manifest_path) {
                    Ok(manifest) => results.push(Ok(PluginInfo {
                        manifest,
                        manifest_path,
                        plugin_dir: path,
                    })),
                    Err(e) => results.push(Err(e)),
                }
            }
        }
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_manifest(dir: &Path, name: &str, version: &str) {
        let mut f = std::fs::File::create(dir.join("plugin.toml")).unwrap();
        writeln!(f, r#"name = "{name}""#).unwrap();
        writeln!(f, r#"version = "{version}""#).unwrap();
    }

    fn make_plugin_dir(root: &Path, plugin_name: &str) -> PathBuf {
        let dir = root.join(plugin_name);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_discover_plugins_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let results = discover_plugins(&[tmp.path()]);
        assert!(results.is_empty());
    }

    #[test]
    fn test_discover_plugins_finds_plugin() {
        let tmp = tempfile::tempdir().unwrap();
        let plugin_dir = make_plugin_dir(tmp.path(), "my-plugin");
        write_manifest(&plugin_dir, "my-plugin", "1.0.0");

        let results = discover_plugins(&[tmp.path()]);
        assert_eq!(results.len(), 1);
        let info = results[0].as_ref().unwrap();
        assert_eq!(info.manifest.name, "my-plugin");
        assert_eq!(info.plugin_dir, plugin_dir);
        assert_eq!(info.manifest_path, plugin_dir.join("plugin.toml"));
    }

    #[test]
    fn test_discover_plugins_skips_invalid() {
        let tmp = tempfile::tempdir().unwrap();
        let plugin_dir = make_plugin_dir(tmp.path(), "bad-plugin");
        // Write a broken manifest
        std::fs::write(plugin_dir.join("plugin.toml"), "name = [broken").unwrap();

        let results = discover_plugins(&[tmp.path()]);
        assert_eq!(results.len(), 1);
        assert!(results[0].is_err());
    }

    #[test]
    fn test_discover_plugins_nonexistent_dir() {
        let results = discover_plugins(&[Path::new("/nonexistent/path/xyz")]);
        assert!(results.is_empty());
    }

    fn make_info(name: &str) -> PluginInfo {
        PluginInfo {
            manifest: crate::manifest::PluginManifest {
                name: name.to_owned(),
                version: "1.0.0".into(),
                description: None,
                author: None,
                min_api_version: None,
                hooks: vec![],
                tools: vec![],
                capabilities: crate::manifest::PluginCapabilities::default(),
            },
            manifest_path: PathBuf::from(format!("/fake/{name}/plugin.toml")),
            plugin_dir: PathBuf::from(format!("/fake/{name}")),
        }
    }

    #[test]
    fn test_registry_register_and_list() {
        let mut reg = PluginRegistry::new();
        reg.register(make_info("alpha"));
        reg.register(make_info("beta"));
        let list = reg.list();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].0.manifest.name, "alpha");
        assert_eq!(list[1].0.manifest.name, "beta");
    }

    #[test]
    fn test_registry_find_by_name() {
        let mut reg = PluginRegistry::new();
        reg.register(make_info("alpha"));
        reg.register(make_info("beta"));
        let (info, status) = reg.find("beta").unwrap();
        assert_eq!(info.manifest.name, "beta");
        assert_eq!(*status, PluginStatus::Discovered);
    }

    #[test]
    fn test_registry_find_not_found() {
        let reg = PluginRegistry::new();
        assert!(reg.find("ghost").is_none());
    }

    #[test]
    fn test_registry_activate() {
        let mut reg = PluginRegistry::new();
        reg.register(make_info("alpha"));
        assert!(reg.activate("alpha"));
        let (_, status) = reg.find("alpha").unwrap();
        assert_eq!(*status, PluginStatus::Active);
    }

    #[test]
    fn test_registry_deactivate() {
        let mut reg = PluginRegistry::new();
        reg.register(make_info("alpha"));
        reg.activate("alpha");
        assert!(reg.deactivate("alpha"));
        let (_, status) = reg.find("alpha").unwrap();
        assert_eq!(*status, PluginStatus::Inactive);
    }

    #[test]
    fn test_registry_mark_failed() {
        let mut reg = PluginRegistry::new();
        reg.register(make_info("alpha"));
        assert!(reg.mark_failed("alpha", "dlopen failed"));
        let (_, status) = reg.find("alpha").unwrap();
        assert!(matches!(status, PluginStatus::Failed { reason } if reason == "dlopen failed"));
    }

    #[test]
    fn test_registry_count_by_status() {
        let mut reg = PluginRegistry::new();
        reg.register(make_info("a"));
        reg.register(make_info("b"));
        reg.register(make_info("c"));
        reg.activate("a");
        reg.activate("b");
        reg.mark_failed("c", "oops");

        assert_eq!(reg.count_by_status(&PluginStatus::Active), 2);
        assert_eq!(reg.count_by_status(&PluginStatus::Discovered), 0);
        assert_eq!(
            reg.count_by_status(&PluginStatus::Failed {
                reason: "oops".into()
            }),
            1
        );
    }
}
