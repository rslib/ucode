//! ucode-plugins: plugin manifest, discovery, registry, and hooks API

pub mod hooks;
pub mod loader;
pub mod manifest;

pub use hooks::{HookDispatcher, HookEvent, HookRecord, HookSubscription, OverrideClass};
pub use loader::{PluginInfo, PluginRegistry, PluginStatus, discover_plugins};
pub use manifest::{
    ManifestError, PluginCapabilities, PluginManifest, PluginToolDef, parse_manifest,
    parse_manifest_file, validate_manifest,
};
