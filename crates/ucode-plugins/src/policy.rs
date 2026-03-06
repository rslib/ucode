//! Per-plugin runtime policy and enforcement.

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::hooks::OverrideClass;
use crate::manifest::PluginCapabilities;

/// Per-plugin network policy.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginNetworkPolicy {
    pub allowed: bool,
    pub domain_allowlist: Vec<String>,
    pub domain_denylist: Vec<String>,
    pub port_allowlist: Vec<u16>,
}

/// Result of a plugin policy check.
#[derive(Debug, Clone, PartialEq)]
pub enum PolicyCheckResult {
    Allowed,
    Denied { action: String, reason: String },
}

/// Per-plugin runtime policy, computed at handshake time.
///
/// Represents the effective permissions for a single plugin instance:
/// `requested capabilities intersection host-allowed capabilities`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginPolicy {
    pub filesystem_read: bool,
    pub filesystem_write: bool,
    pub allowed_paths: Vec<PathBuf>,
    pub workspace_bound: bool,
    pub network: PluginNetworkPolicy,
    pub process_spawn: bool,
    pub guarded_ui: bool,
    #[serde(
        serialize_with = "serialize_hash_set",
        deserialize_with = "deserialize_hash_set"
    )]
    pub allowed_hook_categories: HashSet<String>,
    pub max_override_class: OverrideClass,
}

// serde helpers for HashSet<String> -- serialize as sorted Vec for determinism
fn serialize_hash_set<S: serde::Serializer>(
    set: &HashSet<String>,
    s: S,
) -> Result<S::Ok, S::Error> {
    let mut sorted: Vec<&String> = set.iter().collect();
    sorted.sort();
    sorted.serialize(s)
}

fn deserialize_hash_set<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<HashSet<String>, D::Error> {
    let v: Vec<String> = Vec::deserialize(d)?;
    Ok(v.into_iter().collect())
}

impl PluginPolicy {
    /// Default policy for WASM plugins: read-only workspace, no network/spawn/ui, Safe only.
    pub fn default_wasm() -> Self {
        Self {
            filesystem_read: true,
            filesystem_write: false,
            allowed_paths: Vec::new(),
            workspace_bound: true,
            network: PluginNetworkPolicy::default(),
            process_spawn: false,
            guarded_ui: false,
            allowed_hook_categories: HashSet::new(),
            max_override_class: OverrideClass::Safe,
        }
    }

    /// Default policy for native plugins: all capabilities granted.
    pub fn default_native() -> Self {
        Self {
            filesystem_read: true,
            filesystem_write: true,
            allowed_paths: Vec::new(),
            workspace_bound: false,
            network: PluginNetworkPolicy {
                allowed: true,
                ..Default::default()
            },
            process_spawn: true,
            guarded_ui: true,
            allowed_hook_categories: HashSet::new(),
            max_override_class: OverrideClass::Risky,
        }
    }

    /// Compute policy from manifest capabilities (WASM plugin defaults).
    pub fn from_capabilities(caps: &PluginCapabilities) -> Self {
        let max_override_class = match caps.max_override_class.as_deref() {
            Some("risky") => OverrideClass::Risky,
            Some("guarded") => OverrideClass::Guarded,
            _ => OverrideClass::Safe,
        };
        Self {
            filesystem_read: caps.filesystem,
            filesystem_write: false, // WASM plugins get read-only by default
            allowed_paths: caps.filesystem_paths.iter().map(PathBuf::from).collect(),
            workspace_bound: true, // WASM plugins are always workspace-bound
            network: PluginNetworkPolicy {
                allowed: caps.network,
                domain_allowlist: caps.network_domains.clone(),
                ..Default::default()
            },
            process_spawn: caps.process_spawn,
            guarded_ui: caps.guarded_ui,
            allowed_hook_categories: caps.hook_categories.iter().cloned().collect(),
            max_override_class,
        }
    }

    /// Check if the plugin is allowed to handle hooks in `category`.
    pub fn check_hook_category(&self, category: &str) -> PolicyCheckResult {
        if self.allowed_hook_categories.is_empty()
            || self.allowed_hook_categories.contains(category)
        {
            PolicyCheckResult::Allowed
        } else {
            PolicyCheckResult::Denied {
                action: format!("handle hook category '{category}'"),
                reason: format!("plugin not allowed to handle category '{category}'"),
            }
        }
    }

    /// Check if the plugin's override ceiling permits `class`.
    pub fn check_override_class(&self, class: &OverrideClass) -> PolicyCheckResult {
        let ceiling = override_class_level(&self.max_override_class);
        let requested = override_class_level(class);
        if requested <= ceiling {
            PolicyCheckResult::Allowed
        } else {
            PolicyCheckResult::Denied {
                action: format!("use override class {class:?}"),
                reason: format!(
                    "plugin max override class is {:?}, requested {class:?}",
                    self.max_override_class
                ),
            }
        }
    }

    /// Check if a network connection to `domain:port` is permitted.
    pub fn check_network(&self, domain: &str, port: Option<u16>) -> PolicyCheckResult {
        if !self.network.allowed {
            return PolicyCheckResult::Denied {
                action: format!("network access to '{domain}'"),
                reason: "network access is disabled for this plugin".into(),
            };
        }
        if self.network.domain_denylist.iter().any(|d| d == domain) {
            return PolicyCheckResult::Denied {
                action: format!("network access to '{domain}'"),
                reason: format!("domain '{domain}' is in the denylist"),
            };
        }
        if !self.network.domain_allowlist.is_empty()
            && !self.network.domain_allowlist.iter().any(|d| d == domain)
        {
            return PolicyCheckResult::Denied {
                action: format!("network access to '{domain}'"),
                reason: format!("domain '{domain}' is not in the allowlist"),
            };
        }
        if let Some(p) = port
            && !self.network.port_allowlist.is_empty()
            && !self.network.port_allowlist.contains(&p)
        {
            return PolicyCheckResult::Denied {
                action: format!("network access to '{domain}:{p}'"),
                reason: format!("port {p} is not in the port allowlist"),
            };
        }
        PolicyCheckResult::Allowed
    }

    /// Check if filesystem read is permitted.
    pub fn check_filesystem_read(&self) -> PolicyCheckResult {
        if self.filesystem_read {
            PolicyCheckResult::Allowed
        } else {
            PolicyCheckResult::Denied {
                action: "filesystem read".into(),
                reason: "filesystem read is disabled for this plugin".into(),
            }
        }
    }

    /// Check if filesystem write is permitted.
    pub fn check_filesystem_write(&self) -> PolicyCheckResult {
        if self.filesystem_write {
            PolicyCheckResult::Allowed
        } else {
            PolicyCheckResult::Denied {
                action: "filesystem write".into(),
                reason: "filesystem write is disabled for this plugin".into(),
            }
        }
    }

    /// Check if process spawning is permitted.
    pub fn check_process_spawn(&self) -> PolicyCheckResult {
        if self.process_spawn {
            PolicyCheckResult::Allowed
        } else {
            PolicyCheckResult::Denied {
                action: "process spawn".into(),
                reason: "process spawning is disabled for this plugin".into(),
            }
        }
    }
}

pub(crate) fn override_class_level(class: &OverrideClass) -> u8 {
    match class {
        OverrideClass::Safe => 0,
        OverrideClass::Guarded => 1,
        OverrideClass::Risky => 2,
    }
}

/// Host-side configuration for plugin policies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginPolicyConfig {
    pub default_wasm: PluginPolicy,
    pub default_native: PluginPolicy,
    #[serde(default)]
    pub per_plugin: HashMap<String, PluginPolicy>,
}

impl Default for PluginPolicyConfig {
    fn default() -> Self {
        Self {
            default_wasm: PluginPolicy::default_wasm(),
            default_native: PluginPolicy::default_native(),
            per_plugin: HashMap::new(),
        }
    }
}

impl PluginPolicyConfig {
    /// Resolve the effective policy for a WASM plugin.
    ///
    /// If a per-plugin override exists, use it. Otherwise, compute from
    /// capabilities intersected with the default WASM policy.
    pub fn resolve_wasm(&self, plugin_id: &str, caps: &PluginCapabilities) -> PluginPolicy {
        if let Some(override_policy) = self.per_plugin.get(plugin_id) {
            return override_policy.clone();
        }
        PluginPolicy::from_capabilities(caps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_wasm_policy() {
        let policy = PluginPolicy::default_wasm();
        assert!(policy.filesystem_read);
        assert!(!policy.filesystem_write);
        assert!(policy.allowed_paths.is_empty());
        assert!(policy.workspace_bound);
        assert!(!policy.network.allowed);
        assert!(!policy.process_spawn);
        assert!(!policy.guarded_ui);
        assert!(policy.allowed_hook_categories.is_empty());
        assert_eq!(policy.max_override_class, OverrideClass::Safe);
    }

    #[test]
    fn test_default_native_policy() {
        let policy = PluginPolicy::default_native();
        assert!(policy.filesystem_read);
        assert!(policy.filesystem_write);
        assert!(!policy.workspace_bound);
        assert!(policy.network.allowed);
        assert!(policy.process_spawn);
        assert!(policy.guarded_ui);
        assert_eq!(policy.max_override_class, OverrideClass::Risky);
    }

    #[test]
    fn test_from_capabilities_basic() {
        let caps = PluginCapabilities {
            filesystem: true,
            network: true,
            process_spawn: false,
            guarded_ui: false,
            filesystem_paths: vec!["src/".into()],
            network_domains: vec!["api.example.com".into()],
            hook_categories: vec!["session".into(), "tool".into()],
            max_override_class: Some("guarded".into()),
        };
        let policy = PluginPolicy::from_capabilities(&caps);
        assert!(policy.filesystem_read);
        assert!(!policy.filesystem_write);
        assert!(policy.workspace_bound);
        assert_eq!(policy.allowed_paths, vec![PathBuf::from("src/")]);
        assert!(policy.network.allowed);
        assert_eq!(policy.network.domain_allowlist, vec!["api.example.com"]);
        assert!(!policy.process_spawn);
        assert!(!policy.guarded_ui);
        assert!(policy.allowed_hook_categories.contains("session"));
        assert!(policy.allowed_hook_categories.contains("tool"));
        assert_eq!(policy.max_override_class, OverrideClass::Guarded);
    }

    #[test]
    fn test_from_capabilities_no_filesystem() {
        let caps = PluginCapabilities::default();
        let policy = PluginPolicy::from_capabilities(&caps);
        assert!(!policy.filesystem_read);
        assert!(!policy.filesystem_write);
    }

    #[test]
    fn test_check_hook_category_allowed_when_empty() {
        let policy = PluginPolicy::default_wasm();
        assert_eq!(
            policy.check_hook_category("session"),
            PolicyCheckResult::Allowed
        );
        assert_eq!(
            policy.check_hook_category("tool"),
            PolicyCheckResult::Allowed
        );
    }

    #[test]
    fn test_check_hook_category_restricted() {
        let mut policy = PluginPolicy::default_wasm();
        policy.allowed_hook_categories = ["session".into(), "tool".into()].into();
        assert_eq!(
            policy.check_hook_category("session"),
            PolicyCheckResult::Allowed
        );
        assert_eq!(
            policy.check_hook_category("tool"),
            PolicyCheckResult::Allowed
        );
        assert!(matches!(
            policy.check_hook_category("model"),
            PolicyCheckResult::Denied { .. }
        ));
    }

    #[test]
    fn test_check_override_class_safe_ceiling() {
        let policy = PluginPolicy::default_wasm();
        assert_eq!(
            policy.check_override_class(&OverrideClass::Safe),
            PolicyCheckResult::Allowed
        );
        assert!(matches!(
            policy.check_override_class(&OverrideClass::Guarded),
            PolicyCheckResult::Denied { .. }
        ));
        assert!(matches!(
            policy.check_override_class(&OverrideClass::Risky),
            PolicyCheckResult::Denied { .. }
        ));
    }

    #[test]
    fn test_check_override_class_guarded_ceiling() {
        let mut policy = PluginPolicy::default_wasm();
        policy.max_override_class = OverrideClass::Guarded;
        assert_eq!(
            policy.check_override_class(&OverrideClass::Safe),
            PolicyCheckResult::Allowed
        );
        assert_eq!(
            policy.check_override_class(&OverrideClass::Guarded),
            PolicyCheckResult::Allowed
        );
        assert!(matches!(
            policy.check_override_class(&OverrideClass::Risky),
            PolicyCheckResult::Denied { .. }
        ));
    }

    #[test]
    fn test_check_override_class_risky_ceiling() {
        let mut policy = PluginPolicy::default_wasm();
        policy.max_override_class = OverrideClass::Risky;
        assert_eq!(
            policy.check_override_class(&OverrideClass::Safe),
            PolicyCheckResult::Allowed
        );
        assert_eq!(
            policy.check_override_class(&OverrideClass::Guarded),
            PolicyCheckResult::Allowed
        );
        assert_eq!(
            policy.check_override_class(&OverrideClass::Risky),
            PolicyCheckResult::Allowed
        );
    }

    #[test]
    fn test_check_network_denied() {
        let policy = PluginPolicy::default_wasm();
        assert!(matches!(
            policy.check_network("example.com", None),
            PolicyCheckResult::Denied { .. }
        ));
    }

    #[test]
    fn test_check_network_allowed_all() {
        let mut policy = PluginPolicy::default_wasm();
        policy.network = PluginNetworkPolicy {
            allowed: true,
            ..Default::default()
        };
        assert_eq!(
            policy.check_network("example.com", None),
            PolicyCheckResult::Allowed
        );
    }

    #[test]
    fn test_check_network_domain_allowlist() {
        let mut policy = PluginPolicy::default_wasm();
        policy.network = PluginNetworkPolicy {
            allowed: true,
            domain_allowlist: vec!["api.example.com".into()],
            ..Default::default()
        };
        assert_eq!(
            policy.check_network("api.example.com", None),
            PolicyCheckResult::Allowed
        );
        assert!(matches!(
            policy.check_network("evil.com", None),
            PolicyCheckResult::Denied { .. }
        ));
    }

    #[test]
    fn test_check_network_domain_denylist_precedence() {
        let mut policy = PluginPolicy::default_wasm();
        policy.network = PluginNetworkPolicy {
            allowed: true,
            domain_allowlist: vec!["example.com".into()],
            domain_denylist: vec!["example.com".into()],
            ..Default::default()
        };
        assert!(matches!(
            policy.check_network("example.com", None),
            PolicyCheckResult::Denied { .. }
        ));
    }

    #[test]
    fn test_policy_config_default() {
        let config = PluginPolicyConfig::default();
        assert!(config.default_wasm.workspace_bound);
        assert!(!config.default_wasm.filesystem_write);
        assert!(config.default_native.filesystem_write);
        assert!(config.per_plugin.is_empty());
    }

    #[test]
    fn test_policy_config_resolve_wasm_default() {
        let config = PluginPolicyConfig::default();
        let caps = PluginCapabilities {
            filesystem: true,
            network: false,
            ..Default::default()
        };
        let policy = config.resolve_wasm("org.test.plugin", &caps);
        assert!(policy.filesystem_read);
        assert!(!policy.network.allowed);
    }

    #[test]
    fn test_policy_config_resolve_per_plugin_override() {
        let mut config = PluginPolicyConfig::default();
        let mut override_policy = PluginPolicy::default_wasm();
        override_policy.filesystem_write = true;
        config
            .per_plugin
            .insert("org.test.special".into(), override_policy);

        let caps = PluginCapabilities {
            filesystem: true,
            ..Default::default()
        };
        let policy = config.resolve_wasm("org.test.special", &caps);
        // Per-plugin override grants write
        assert!(policy.filesystem_write);

        // Other plugins get default
        let policy2 = config.resolve_wasm("org.test.other", &caps);
        assert!(!policy2.filesystem_write);
    }
}
