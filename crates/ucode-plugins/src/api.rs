use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::hooks::HookRecord;
use crate::manifest::{PluginCapabilities, PluginToolDef};

/// Current host API version.
pub const API_VERSION: &str = "1.0.0";

/// API surface areas a plugin can opt into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Feature {
    Hooks,
    Tools,
    Ui,
}

/// Plugin -> Host: request to connect.
pub struct HandshakeRequest {
    pub plugin_id: String,
    pub plugin_version: semver::Version,
    pub min_api_version: semver::Version,
    pub required_features: HashSet<Feature>,
    pub capabilities: PluginCapabilities,
}

/// Host -> Plugin: handshake result.
#[derive(Debug)]
pub enum HandshakeResponse {
    Accepted {
        api_version: semver::Version,
        supported_features: HashSet<Feature>,
        granted_capabilities: PluginCapabilities,
    },
    Rejected {
        reason: HandshakeError,
    },
}

/// Handshake failure reasons.
#[derive(Debug)]
pub enum HandshakeError {
    VersionIncompatible {
        host_version: semver::Version,
        required_version: semver::Version,
    },
    UnsupportedFeatures {
        missing: Vec<Feature>,
    },
    CapabilityDenied {
        denied: Vec<String>,
    },
}

/// What a plugin returns from a hook.
#[derive(Debug)]
pub enum HookResponse {
    /// Observed, no action taken.
    Ok,
    /// Propose modifications (only valid for Guarded events).
    Modify { changes: serde_json::Value },
    /// Veto the action (only valid for Risky events, requires approval).
    Veto { reason: String },
}

/// Check semver compatibility: same major, host >= required.
pub fn check_version_compatible(
    host: &semver::Version,
    required: &semver::Version,
) -> Result<(), HandshakeError> {
    if host.major != required.major || host < required {
        return Err(HandshakeError::VersionIncompatible {
            host_version: host.clone(),
            required_version: required.clone(),
        });
    }
    Ok(())
}

/// Check that all required features are in the supported set.
pub fn check_features_compatible(
    required: &HashSet<Feature>,
    supported: &HashSet<Feature>,
) -> Result<(), HandshakeError> {
    let mut missing: Vec<Feature> = required.difference(supported).copied().collect();
    if !missing.is_empty() {
        // Sort for deterministic test output.
        missing.sort_by_key(|f| format!("{f:?}"));
        return Err(HandshakeError::UnsupportedFeatures { missing });
    }
    Ok(())
}

/// Core plugin trait. Every plugin implements this.
pub trait Plugin: Send {
    /// Return handshake request with plugin's requirements.
    fn handshake(&self) -> HandshakeRequest;

    /// Called after successful handshake. Plugin performs setup.
    fn initialize(&mut self, response: &HandshakeResponse) -> Result<(), String>;

    /// Called on shutdown. Plugin cleans up resources.
    fn shutdown(&mut self);
}

/// Optional: handle hook events. Requires Feature::Hooks.
pub trait HookHandler: Send {
    /// Process a hook event and return a response.
    fn on_event(&mut self, record: &HookRecord) -> HookResponse;
}

/// Optional: provide tools. Requires Feature::Tools.
pub trait ToolProvider: Send {
    /// Declare tool specs during initialization.
    fn tool_specs(&self) -> Vec<PluginToolDef>;

    /// Handle a tool invocation. `name` is the local tool name (not FQN).
    fn invoke_tool(
        &mut self,
        name: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, String>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feature_serde_roundtrip() {
        for (feature, expected) in [
            (Feature::Hooks, "\"hooks\""),
            (Feature::Tools, "\"tools\""),
            (Feature::Ui, "\"ui\""),
        ] {
            let json = serde_json::to_string(&feature).unwrap();
            assert_eq!(json, expected);
            let back: Feature = serde_json::from_str(&json).unwrap();
            assert_eq!(back, feature);
        }
    }

    #[test]
    fn test_handshake_version_compatible() {
        let host = semver::Version::new(1, 2, 0);
        let required = semver::Version::new(1, 0, 0);
        assert!(check_version_compatible(&host, &required).is_ok());
    }

    #[test]
    fn test_handshake_version_incompatible_major() {
        let host = semver::Version::new(2, 0, 0);
        let required = semver::Version::new(1, 0, 0);
        assert!(matches!(
            check_version_compatible(&host, &required),
            Err(HandshakeError::VersionIncompatible { .. })
        ));
    }

    #[test]
    fn test_handshake_version_host_too_old() {
        let host = semver::Version::new(1, 0, 0);
        let required = semver::Version::new(1, 2, 0);
        assert!(matches!(
            check_version_compatible(&host, &required),
            Err(HandshakeError::VersionIncompatible { .. })
        ));
    }

    #[test]
    fn test_handshake_features_compatible() {
        let required: HashSet<Feature> = [Feature::Hooks, Feature::Tools].into();
        let supported: HashSet<Feature> = [Feature::Hooks, Feature::Tools, Feature::Ui].into();
        assert!(check_features_compatible(&required, &supported).is_ok());
    }

    #[test]
    fn test_handshake_features_missing() {
        let required: HashSet<Feature> = [Feature::Hooks, Feature::Tools].into();
        let supported: HashSet<Feature> = [Feature::Hooks].into();
        let err = check_features_compatible(&required, &supported).unwrap_err();
        match err {
            HandshakeError::UnsupportedFeatures { missing } => {
                assert_eq!(missing, vec![Feature::Tools]);
            }
            _ => panic!("wrong error variant"),
        }
    }

    #[test]
    fn test_hook_response_variants() {
        let ok = HookResponse::Ok;
        assert!(matches!(ok, HookResponse::Ok));

        let modify = HookResponse::Modify {
            changes: serde_json::json!({"key": "val"}),
        };
        assert!(matches!(modify, HookResponse::Modify { .. }));

        let veto = HookResponse::Veto {
            reason: "blocked".into(),
        };
        assert!(matches!(veto, HookResponse::Veto { .. }));
    }

    #[test]
    fn test_api_version_constant() {
        let v: semver::Version = API_VERSION.parse().unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 0);
        assert_eq!(v.patch, 0);
    }
}
