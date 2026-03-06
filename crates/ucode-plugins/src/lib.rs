//! ucode-plugins: plugin manifest, discovery, registry, and hooks API

pub mod hooks;
pub mod loader;
pub mod manifest;
pub mod ui_api;

pub use hooks::{HookDispatcher, HookEvent, HookRecord, HookSubscription, OverrideClass};
pub use loader::{PluginInfo, PluginRegistry, PluginStatus, discover_plugins};
pub use manifest::{
    ManifestError, PluginCapabilities, PluginManifest, PluginToolDef, parse_manifest,
    parse_manifest_file, validate_manifest,
};
pub use ui_api::{PluginUiCall, UiCallClass, UiCallDenied, check_ui_call};
