//! ucode-plugins: plugin manifest, discovery, registry, and hooks API

pub mod loader;
pub mod manifest;

pub use loader::{PluginInfo, PluginRegistry, PluginStatus, discover_plugins};
pub use manifest::{
    ManifestError, PluginCapabilities, PluginManifest, PluginToolDef, parse_manifest,
    parse_manifest_file, validate_manifest,
};
