//! ucode-plugins: plugin manifest, discovery, registry, and hooks API

pub mod api;
pub mod hooks;
pub mod host;
pub mod loader;
pub mod manifest;
pub mod policy;
pub mod ui_api;

pub use api::{
    API_VERSION, Feature, HandshakeError, HandshakeRequest, HandshakeResponse, HookHandler,
    HookResponse, Plugin, ToolProvider, check_features_compatible, check_version_compatible,
};
pub use hooks::{HookDispatcher, HookEvent, HookRecord, HookSubscription, OverrideClass};
pub use host::{HookResult, PluginHost};
pub use loader::{PluginInfo, PluginRegistry, PluginStatus, discover_plugins};
pub use manifest::{
    ManifestError, PluginCapabilities, PluginManifest, PluginToolDef, parse_manifest,
    parse_manifest_file, validate_manifest,
};
pub use policy::{PluginNetworkPolicy, PluginPolicy, PolicyCheckResult};
pub use ui_api::{PluginUiCall, UiCallClass, UiCallDenied, check_ui_call};

#[cfg(feature = "wasm")]
pub mod wasm;
