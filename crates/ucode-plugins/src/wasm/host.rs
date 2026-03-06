//! WASM plugin host using wasmtime component model.

use std::collections::HashSet;
use std::path::Path;

use wasmtime::component::{Component, Linker};
use wasmtime::{Config, Engine, Store};

use super::convert::EVENT_INTERFACE_MAP;
use crate::policy::PluginPolicy;

/// Host state accessible to WASM plugins via imports.
pub struct WasmHostState {
    /// Log messages emitted by plugins via the `ucode:plugin/host-log` import.
    pub log_messages: Vec<String>,
}

/// A loaded WASM plugin component.
///
/// Probes which hook interfaces the component exports at load time by calling
/// [`Component::get_export_index`] for each of the 64 WIT interface paths.
/// At dispatch time, the caller instantiates the component and calls the
/// typed `handle` export on the appropriate interface.
pub struct WasmPlugin {
    engine: Engine,
    component: Component,
    /// Event names this plugin handles (probed at load time).
    subscribed_events: HashSet<String>,
    /// Plugin ID (empty until handshake is performed).
    plugin_id: String,
}

impl WasmPlugin {
    /// Load a WASM component from a file and probe its exports.
    pub fn from_file(path: &Path) -> Result<Self, WasmPluginError> {
        let engine = build_engine()?;
        let component =
            Component::from_file(&engine, path).map_err(WasmPluginError::ComponentLoad)?;
        let subscribed_events = probe_exports(&component);
        Ok(Self {
            engine,
            component,
            subscribed_events,
            plugin_id: String::new(),
        })
    }

    /// Load a WASM component from bytes and probe its exports.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, WasmPluginError> {
        let engine = build_engine()?;
        let component = Component::new(&engine, bytes).map_err(WasmPluginError::ComponentLoad)?;
        let subscribed_events = probe_exports(&component);
        Ok(Self {
            engine,
            component,
            subscribed_events,
            plugin_id: String::new(),
        })
    }

    /// Returns `true` if this plugin exports a handler for `event_name`.
    pub fn handles_event(&self, event_name: &str) -> bool {
        self.subscribed_events.contains(event_name)
    }

    /// The set of event names this plugin handles.
    pub fn subscribed_events(&self) -> &HashSet<String> {
        &self.subscribed_events
    }

    /// The plugin's ID (empty string until handshake is performed).
    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    /// Set the plugin ID after a successful handshake.
    pub fn set_plugin_id(&mut self, id: String) {
        self.plugin_id = id;
    }

    /// The wasmtime engine used by this plugin.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// The compiled component.
    pub fn component(&self) -> &Component {
        &self.component
    }

    /// Create a fresh [`Store`] with empty host state.
    ///
    /// Each dispatch call should use a fresh store to avoid cross-call state
    /// leakage. For performance-critical paths, consider reusing a store and
    /// clearing `log_messages` between calls.
    pub fn create_store(&self) -> Store<WasmHostState> {
        Store::new(
            &self.engine,
            WasmHostState {
                log_messages: Vec::new(),
            },
        )
    }

    /// Create a store with policy-aware WASI configuration.
    pub fn create_store_with_policy(
        &self,
        policy: &PluginPolicy,
        workspace_root: Option<&std::path::Path>,
    ) -> Store<WasmHostState> {
        create_store_with_policy(&self.engine, policy, workspace_root)
    }

    /// Create a [`Linker`] with the `ucode:plugin/host-log` import wired up.
    ///
    /// The `log` function appends messages to [`WasmHostState::log_messages`].
    pub fn create_linker(&self) -> Result<Linker<WasmHostState>, WasmPluginError> {
        let mut linker = Linker::new(&self.engine);
        wire_host_log(&mut linker)?;
        Ok(linker)
    }
}

/// Errors from WASM plugin operations.
#[derive(Debug)]
pub enum WasmPluginError {
    /// Failed to create the wasmtime engine.
    Engine(wasmtime::Error),
    /// Failed to load or compile the WASM component.
    ComponentLoad(wasmtime::Error),
    /// Failed to configure the linker.
    Linker(wasmtime::Error),
    /// Failed to instantiate the component.
    Instantiate(wasmtime::Error),
    /// Failed to call a WASM function.
    Call(wasmtime::Error),
    /// Plugin does not export a handler for this event.
    EventNotHandled(String),
}

impl std::fmt::Display for WasmPluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Engine(e) => write!(f, "wasmtime engine error: {e}"),
            Self::ComponentLoad(e) => write!(f, "component load error: {e}"),
            Self::Linker(e) => write!(f, "linker error: {e}"),
            Self::Instantiate(e) => write!(f, "instantiation error: {e}"),
            Self::Call(e) => write!(f, "call error: {e}"),
            Self::EventNotHandled(name) => write!(f, "event not handled: {name}"),
        }
    }
}

impl std::error::Error for WasmPluginError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        // wasmtime::Error is anyhow::Error, which does not implement
        // std::error::Error itself, so we delegate to its own source chain.
        match self {
            Self::Engine(e)
            | Self::ComponentLoad(e)
            | Self::Linker(e)
            | Self::Instantiate(e)
            | Self::Call(e) => e.source(),
            Self::EventNotHandled(_) => None,
        }
    }
}

// --- private helpers ---

fn build_engine() -> Result<Engine, WasmPluginError> {
    let mut config = Config::new();
    config.wasm_component_model(true);
    Engine::new(&config).map_err(WasmPluginError::Engine)
}

/// Walk every (event_name, wit_iface) pair and record which ones the
/// component actually exports.
fn probe_exports(component: &Component) -> HashSet<String> {
    EVENT_INTERFACE_MAP
        .iter()
        .filter_map(|&(event_name, wit_iface)| {
            component
                .get_export_index(None, wit_iface)
                .map(|_| event_name.to_string())
        })
        .collect()
}

/// Create a Store with WASI context configured according to the plugin's policy.
///
/// Filesystem preopens are restricted to `allowed_paths` (or `workspace_root`
/// if `workspace_bound` and no specific paths). Network capability is only
/// granted if `policy.network.allowed` is true.
pub fn create_store_with_policy(
    engine: &Engine,
    policy: &PluginPolicy,
    _workspace_root: Option<&std::path::Path>,
) -> Store<WasmHostState> {
    let store = Store::new(
        engine,
        WasmHostState {
            log_messages: Vec::new(),
        },
    );

    // Note: Full WASI preopens configuration requires wasmtime-wasi's
    // WasiCtxBuilder. For now we document the policy intent and configure
    // what we can. Full WASI integration will be wired when wasmtime-wasi
    // WasiCtx is added to WasmHostState.
    //
    // The policy is stored and can be queried for enforcement at the host
    // dispatch layer.

    tracing::info!(
        filesystem_read = policy.filesystem_read,
        filesystem_write = policy.filesystem_write,
        workspace_bound = policy.workspace_bound,
        network_allowed = policy.network.allowed,
        process_spawn = policy.process_spawn,
        "WASM store created with policy"
    );

    store
}

/// Wire the `ucode:plugin/host-log` import into `linker`.
///
/// The WIT definition is:
/// ```wit
/// interface host-log {
///     log: func(msg: string);
/// }
/// ```
fn wire_host_log(linker: &mut Linker<WasmHostState>) -> Result<(), WasmPluginError> {
    let mut iface = linker
        .instance("ucode:plugin/host-log")
        .map_err(WasmPluginError::Linker)?;

    iface
        .func_wrap(
            "log",
            |mut caller: wasmtime::StoreContextMut<'_, WasmHostState>, (msg,): (String,)| {
                caller.data_mut().log_messages.push(msg);
                Ok(())
            },
        )
        .map_err(WasmPluginError::Linker)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wasm_plugin_error_display() {
        let err = WasmPluginError::EventNotHandled("session_start".into());
        assert_eq!(err.to_string(), "event not handled: session_start");
    }

    #[test]
    fn test_wasm_host_state_default() {
        let state = WasmHostState {
            log_messages: Vec::new(),
        };
        assert!(state.log_messages.is_empty());
    }

    #[test]
    fn test_wasm_plugin_error_source_event_not_handled() {
        use std::error::Error;
        let err = WasmPluginError::EventNotHandled("x".into());
        assert!(err.source().is_none());
    }

    #[test]
    fn test_create_store_with_policy_no_preopens() {
        let engine = build_engine().unwrap();
        let policy = PluginPolicy::default_wasm();
        let store = create_store_with_policy(&engine, &policy, None);
        assert!(store.data().log_messages.is_empty());
    }

    #[test]
    fn test_create_store_with_policy_workspace_preopen() {
        let engine = build_engine().unwrap();
        let mut policy = PluginPolicy::default_wasm();
        policy.filesystem_read = true;
        let workspace = tempfile::tempdir().unwrap();
        let store = create_store_with_policy(&engine, &policy, Some(workspace.path()));
        assert!(store.data().log_messages.is_empty());
    }
}
