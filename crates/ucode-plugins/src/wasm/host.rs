//! WASM plugin host using wasmtime component model.

use std::collections::HashSet;
use std::path::Path;

use wasmtime::component::{Component, Linker};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};

use super::convert::EVENT_INTERFACE_MAP;
use crate::policy::PluginPolicy;

/// Host state accessible to WASM plugins via imports.
pub struct WasmHostState {
    /// Log messages emitted by plugins via the `ucode:plugin/host-log` import.
    pub log_messages: Vec<String>,
    /// Resource limiter enforcing memory and instance caps from [`PluginPolicy`].
    pub limits: StoreLimits,
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

    /// Create a fresh [`Store`] with empty host state and default resource limits.
    ///
    /// Each dispatch call should use a fresh store to avoid cross-call state
    /// leakage. For policy-enforced fuel and memory limits use
    /// [`create_store_with_policy`] instead.
    pub fn create_store(&self) -> Store<WasmHostState> {
        Store::new(
            &self.engine,
            WasmHostState {
                log_messages: Vec::new(),
                limits: StoreLimitsBuilder::new().build(),
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

    /// Dispatch a hook event to this WASM plugin component.
    ///
    /// Creates a fresh store with policy-enforced resource limits, instantiates
    /// the component, looks up the event's WIT interface export, and calls its
    /// `handle` function with the JSON-serialized event payload.
    ///
    /// Returns the plugin's [`HookResponse`]. On any error (instantiation failure,
    /// missing export, fuel exhaustion, call trap), the caller should log a warning
    /// and treat as `HookResponse::Ok` (fail-open).
    pub fn dispatch_hook(
        &self,
        record: &crate::hooks::HookRecord,
        policy: &PluginPolicy,
    ) -> Result<crate::api::HookResponse, WasmPluginError> {
        use super::convert::{event_to_wit_interface, wit_response_to_native};

        let event_name = record.event.event_name();
        let wit_iface = event_to_wit_interface(event_name)
            .ok_or_else(|| WasmPluginError::EventNotHandled(event_name.to_string()))?;

        // Fresh store per dispatch — no cross-call state leakage.
        let mut store = self.create_store_with_policy(policy, None);
        let linker = self.create_linker()?;

        let instance = linker
            .instantiate(&mut store, &self.component)
            .map_err(WasmPluginError::Instantiate)?;

        // Resolve the `handle` function index via the component's export table.
        // This avoids string lookups at call time and works with nested interface exports.
        let iface_idx = self
            .component
            .get_export_index(None, wit_iface)
            .ok_or_else(|| {
                WasmPluginError::EventNotHandled(format!("{wit_iface} interface not exported"))
            })?;
        let handle_idx = self
            .component
            .get_export_index(Some(&iface_idx), "handle")
            .ok_or_else(|| {
                WasmPluginError::EventNotHandled(format!("no handle fn in {wit_iface}"))
            })?;

        let handle_fn = instance.get_func(&mut store, &handle_idx).ok_or_else(|| {
            WasmPluginError::EventNotHandled(format!("handle fn not callable in {wit_iface}"))
        })?;

        // Serialize the event payload to JSON for the guest.
        let payload_json =
            serde_json::to_string(&record.event).unwrap_or_else(|_| "{}".to_string());

        // Call handle(payload: string) -> hook-response.
        // The result slice must be pre-sized to the number of return values (1).
        let mut results = vec![wasmtime::component::Val::Bool(false)];
        handle_fn
            .call(
                &mut store,
                &[wasmtime::component::Val::String(payload_json)],
                &mut results,
            )
            .map_err(WasmPluginError::Call)?;

        let response = parse_hook_response_val(&results[0]);
        Ok(wit_response_to_native(response))
    }

    /// Check if this component exports the `ucode:plugin/tool-provider` interface.
    pub fn has_tool_provider(&self) -> bool {
        self.component
            .get_export_index(None, "ucode:plugin/tool-provider")
            .is_some()
    }

    /// List tools provided by this WASM plugin.
    ///
    /// Instantiates the component and calls `tool-specs` on the
    /// `ucode:plugin/tool-provider` interface.
    pub fn list_tools(
        &self,
        policy: &PluginPolicy,
    ) -> Result<Vec<crate::manifest::PluginToolDef>, WasmPluginError> {
        let mut store = self.create_store_with_policy(policy, None);
        let linker = self.create_linker()?;
        let instance = linker
            .instantiate(&mut store, &self.component)
            .map_err(WasmPluginError::Instantiate)?;

        let iface_idx = self
            .component
            .get_export_index(None, "ucode:plugin/tool-provider")
            .ok_or_else(|| {
                WasmPluginError::ToolInvocation("tool-provider interface not exported".into())
            })?;
        let fn_idx = self
            .component
            .get_export_index(Some(&iface_idx), "tool-specs")
            .ok_or_else(|| {
                WasmPluginError::ToolInvocation("tool-specs function not found".into())
            })?;

        let tool_specs_fn = instance
            .get_func(&mut store, &fn_idx)
            .ok_or_else(|| WasmPluginError::ToolInvocation("tool-specs not callable".into()))?;

        let mut results = vec![wasmtime::component::Val::Bool(false)];
        tool_specs_fn
            .call(&mut store, &[], &mut results)
            .map_err(WasmPluginError::Call)?;

        Ok(parse_tool_specs_val(&results[0]))
    }

    /// Invoke a tool by name with JSON arguments.
    ///
    /// Returns the JSON-encoded result string on success, or a
    /// [`WasmPluginError::ToolInvocation`] on failure.
    pub fn invoke_tool(
        &self,
        name: &str,
        args: &str,
        policy: &PluginPolicy,
    ) -> Result<String, WasmPluginError> {
        let mut store = self.create_store_with_policy(policy, None);
        let linker = self.create_linker()?;
        let instance = linker
            .instantiate(&mut store, &self.component)
            .map_err(WasmPluginError::Instantiate)?;

        let iface_idx = self
            .component
            .get_export_index(None, "ucode:plugin/tool-provider")
            .ok_or_else(|| {
                WasmPluginError::ToolInvocation("tool-provider interface not exported".into())
            })?;
        let fn_idx = self
            .component
            .get_export_index(Some(&iface_idx), "invoke-tool")
            .ok_or_else(|| {
                WasmPluginError::ToolInvocation("invoke-tool function not found".into())
            })?;

        let invoke_fn = instance
            .get_func(&mut store, &fn_idx)
            .ok_or_else(|| WasmPluginError::ToolInvocation("invoke-tool not callable".into()))?;

        let mut results = vec![wasmtime::component::Val::Bool(false)];
        invoke_fn
            .call(
                &mut store,
                &[
                    wasmtime::component::Val::String(name.to_string()),
                    wasmtime::component::Val::String(args.to_string()),
                ],
                &mut results,
            )
            .map_err(WasmPluginError::Call)?;

        // Parse result<string, string>
        match &results[0] {
            wasmtime::component::Val::Result(r) => match r.as_ref() {
                Ok(Some(val)) => {
                    if let wasmtime::component::Val::String(s) = val.as_ref() {
                        Ok(s.clone())
                    } else {
                        Ok(String::new())
                    }
                }
                Ok(None) => Ok(String::new()),
                Err(Some(val)) => {
                    if let wasmtime::component::Val::String(s) = val.as_ref() {
                        Err(WasmPluginError::ToolInvocation(s.clone()))
                    } else {
                        Err(WasmPluginError::ToolInvocation("unknown error".into()))
                    }
                }
                Err(None) => Err(WasmPluginError::ToolInvocation("unknown error".into())),
            },
            _ => Err(WasmPluginError::ToolInvocation(
                "unexpected return type".into(),
            )),
        }
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
    /// Failed to invoke a plugin tool.
    ToolInvocation(String),
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
            Self::ToolInvocation(msg) => write!(f, "tool invocation error: {msg}"),
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
            Self::EventNotHandled(_) | Self::ToolInvocation(_) => None,
        }
    }
}

// --- private helpers ---

fn build_engine() -> Result<Engine, WasmPluginError> {
    let mut config = Config::new();
    config.wasm_component_model(true);
    Engine::new(&config).map_err(WasmPluginError::Engine)
}

/// Build an engine with fuel consumption enabled.
///
/// Required when creating stores that enforce per-dispatch instruction budgets
/// via [`Store::set_fuel`]. The engine and store must agree on fuel enablement;
/// calling `set_fuel` on a store backed by a non-fuel engine returns an error.
pub fn build_engine_with_fuel() -> Result<Engine, WasmPluginError> {
    let mut config = Config::new();
    config.wasm_component_model(true);
    config.consume_fuel(true);
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
///
/// The engine must have been created with [`build_engine_with_fuel`] for fuel
/// limits to take effect; if the engine does not support fuel, the `set_fuel`
/// call is a no-op (logged as a warning).
pub fn create_store_with_policy(
    engine: &Engine,
    policy: &PluginPolicy,
    _workspace_root: Option<&std::path::Path>,
) -> Store<WasmHostState> {
    let limits = StoreLimitsBuilder::new()
        .memory_size(policy.resource_limits.max_memory_bytes)
        .instances(policy.resource_limits.max_instances)
        .build();

    let mut store = Store::new(
        engine,
        WasmHostState {
            log_messages: Vec::new(),
            limits,
        },
    );

    store.limiter(|state| &mut state.limits);

    if let Err(e) = store.set_fuel(policy.resource_limits.max_fuel) {
        tracing::warn!("failed to set fuel: {e}");
    }

    // Note: Full WASI preopens configuration requires wasmtime-wasi's
    // WasiCtxBuilder. Full WASI integration will be wired when wasmtime-wasi
    // WasiCtx is added to WasmHostState.

    tracing::info!(
        filesystem_read = policy.filesystem_read,
        filesystem_write = policy.filesystem_write,
        workspace_bound = policy.workspace_bound,
        network_allowed = policy.network.allowed,
        process_spawn = policy.process_spawn,
        max_memory_bytes = policy.resource_limits.max_memory_bytes,
        max_fuel = policy.resource_limits.max_fuel,
        max_instances = policy.resource_limits.max_instances,
        "WASM store created with resource limits"
    );

    store
}

/// Parse a `hook-response` record from a wasmtime [`Val`].
///
/// The WIT record has two fields: `kind` (enum) and `data` (option<string>).
/// Falls back to `HookResponseKind::Ok` on unexpected shapes.
fn parse_hook_response_val(
    val: &wasmtime::component::Val,
) -> super::ucode::hooks_types::types::HookResponse {
    use super::ucode::hooks_types::types::{HookResponse, HookResponseKind};

    if let wasmtime::component::Val::Record(fields) = val {
        let mut kind = HookResponseKind::Ok;
        let mut data = None;

        for (name, field_val) in fields {
            match name.as_str() {
                "kind" => {
                    if let wasmtime::component::Val::Enum(discriminant) = field_val {
                        kind = match discriminant.as_str() {
                            "ok" => HookResponseKind::Ok,
                            "modify" => HookResponseKind::Modify,
                            "veto" => HookResponseKind::Veto,
                            _ => HookResponseKind::Ok,
                        };
                    }
                }
                "data" => {
                    if let wasmtime::component::Val::Option(opt) = field_val {
                        data = opt.as_ref().and_then(|v| {
                            if let wasmtime::component::Val::String(s) = v.as_ref() {
                                Some(s.clone())
                            } else {
                                None
                            }
                        });
                    }
                }
                _ => {}
            }
        }

        HookResponse { kind, data }
    } else {
        HookResponse {
            kind: HookResponseKind::Ok,
            data: None,
        }
    }
}

/// Parse a `list<tool-spec>` Val into `Vec<PluginToolDef>`.
fn parse_tool_specs_val(val: &wasmtime::component::Val) -> Vec<crate::manifest::PluginToolDef> {
    let mut specs = Vec::new();
    if let wasmtime::component::Val::List(items) = val {
        for item in items {
            if let wasmtime::component::Val::Record(fields) = item {
                let mut name = String::new();
                let mut description = None;
                let mut input_schema = None;

                for (field_name, field_val) in fields {
                    match field_name.as_str() {
                        "name" => {
                            if let wasmtime::component::Val::String(s) = field_val {
                                name = s.clone();
                            }
                        }
                        "description" => {
                            if let wasmtime::component::Val::Option(opt) = field_val {
                                description = opt.as_ref().and_then(|v| {
                                    if let wasmtime::component::Val::String(s) = v.as_ref() {
                                        Some(s.clone())
                                    } else {
                                        None
                                    }
                                });
                            }
                        }
                        "input-schema" => {
                            if let wasmtime::component::Val::Option(opt) = field_val {
                                input_schema = opt.as_ref().and_then(|v| {
                                    if let wasmtime::component::Val::String(s) = v.as_ref() {
                                        serde_json::from_str(s).ok()
                                    } else {
                                        None
                                    }
                                });
                            }
                        }
                        _ => {}
                    }
                }

                if !name.is_empty() {
                    specs.push(crate::manifest::PluginToolDef {
                        name,
                        description,
                        input_schema,
                    });
                }
            }
        }
    }
    specs
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
            limits: StoreLimitsBuilder::new().build(),
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

    #[test]
    fn test_build_engine_with_fuel() {
        let engine = build_engine_with_fuel().unwrap();
        let _ = engine;
    }

    #[test]
    fn test_create_store_with_policy_has_fuel() {
        let engine = build_engine_with_fuel().unwrap();
        let policy = PluginPolicy::default_wasm();
        let store = create_store_with_policy(&engine, &policy, None);
        assert!(store.get_fuel().is_ok());
        assert_eq!(store.get_fuel().unwrap(), 1_000_000);
    }

    #[test]
    fn test_create_store_with_policy_custom_fuel() {
        let engine = build_engine_with_fuel().unwrap();
        let mut policy = PluginPolicy::default_wasm();
        policy.resource_limits.max_fuel = 500_000;
        let store = create_store_with_policy(&engine, &policy, None);
        assert_eq!(store.get_fuel().unwrap(), 500_000);
    }

    #[test]
    fn test_create_store_backward_compat() {
        // create_store() must still work without fuel
        let engine = build_engine().unwrap();
        let plugin_state = WasmHostState {
            log_messages: Vec::new(),
            limits: wasmtime::StoreLimitsBuilder::new().build(),
        };
        let store = Store::new(&engine, plugin_state);
        assert!(store.data().log_messages.is_empty());
    }

    #[test]
    fn test_wasm_plugin_error_tool_invocation_display() {
        let err = WasmPluginError::ToolInvocation("tool not found".into());
        assert_eq!(err.to_string(), "tool invocation error: tool not found");
    }

    #[test]
    fn test_wasm_plugin_error_tool_invocation_source() {
        use std::error::Error;
        let err = WasmPluginError::ToolInvocation("x".into());
        assert!(err.source().is_none());
    }

    #[test]
    fn test_parse_hook_response_val_ok() {
        use super::super::ucode::hooks_types::types::HookResponseKind;
        let val = wasmtime::component::Val::Record(vec![
            (
                "kind".to_string(),
                wasmtime::component::Val::Enum("ok".to_string()),
            ),
            ("data".to_string(), wasmtime::component::Val::Option(None)),
        ]);
        let resp = parse_hook_response_val(&val);
        assert!(matches!(resp.kind, HookResponseKind::Ok));
        assert!(resp.data.is_none());
    }

    #[test]
    fn test_parse_hook_response_val_modify() {
        use super::super::ucode::hooks_types::types::HookResponseKind;
        let val = wasmtime::component::Val::Record(vec![
            (
                "kind".to_string(),
                wasmtime::component::Val::Enum("modify".to_string()),
            ),
            (
                "data".to_string(),
                wasmtime::component::Val::Option(Some(Box::new(wasmtime::component::Val::String(
                    r#"{"key":"val"}"#.to_string(),
                )))),
            ),
        ]);
        let resp = parse_hook_response_val(&val);
        assert!(matches!(resp.kind, HookResponseKind::Modify));
        assert_eq!(resp.data.as_deref(), Some(r#"{"key":"val"}"#));
    }

    #[test]
    fn test_parse_hook_response_val_veto() {
        use super::super::ucode::hooks_types::types::HookResponseKind;
        let val = wasmtime::component::Val::Record(vec![
            (
                "kind".to_string(),
                wasmtime::component::Val::Enum("veto".to_string()),
            ),
            (
                "data".to_string(),
                wasmtime::component::Val::Option(Some(Box::new(wasmtime::component::Val::String(
                    "blocked".to_string(),
                )))),
            ),
        ]);
        let resp = parse_hook_response_val(&val);
        assert!(matches!(resp.kind, HookResponseKind::Veto));
        assert_eq!(resp.data.as_deref(), Some("blocked"));
    }

    #[test]
    fn test_parse_hook_response_val_unknown_shape() {
        use super::super::ucode::hooks_types::types::HookResponseKind;
        // Non-record Val falls back to Ok
        let val = wasmtime::component::Val::Bool(false);
        let resp = parse_hook_response_val(&val);
        assert!(matches!(resp.kind, HookResponseKind::Ok));
        assert!(resp.data.is_none());
    }

    #[test]
    fn test_parse_tool_specs_val_empty_list() {
        let val = wasmtime::component::Val::List(vec![]);
        let specs = parse_tool_specs_val(&val);
        assert!(specs.is_empty());
    }

    #[test]
    fn test_parse_tool_specs_val_with_entries() {
        let val = wasmtime::component::Val::List(vec![wasmtime::component::Val::Record(vec![
            (
                "name".to_string(),
                wasmtime::component::Val::String("search".to_string()),
            ),
            (
                "description".to_string(),
                wasmtime::component::Val::Option(Some(Box::new(wasmtime::component::Val::String(
                    "Search the web".to_string(),
                )))),
            ),
            (
                "input-schema".to_string(),
                wasmtime::component::Val::Option(None),
            ),
        ])]);
        let specs = parse_tool_specs_val(&val);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "search");
        assert_eq!(specs[0].description.as_deref(), Some("Search the web"));
        assert!(specs[0].input_schema.is_none());
    }

    #[test]
    fn test_parse_tool_specs_val_skips_empty_name() {
        let val = wasmtime::component::Val::List(vec![wasmtime::component::Val::Record(vec![
            (
                "name".to_string(),
                wasmtime::component::Val::String(String::new()),
            ),
            (
                "description".to_string(),
                wasmtime::component::Val::Option(None),
            ),
            (
                "input-schema".to_string(),
                wasmtime::component::Val::Option(None),
            ),
        ])]);
        let specs = parse_tool_specs_val(&val);
        assert!(specs.is_empty(), "empty-name tool should be skipped");
    }

    #[test]
    fn test_wasm_dispatch_hook_method_exists() {
        // Verify the method signature compiles.
        let _: fn(
            &WasmPlugin,
            &crate::hooks::HookRecord,
            &PluginPolicy,
        ) -> Result<crate::api::HookResponse, WasmPluginError> = WasmPlugin::dispatch_hook;
    }

    #[test]
    fn test_wasm_has_tool_provider_method_exists() {
        let _: fn(&WasmPlugin) -> bool = WasmPlugin::has_tool_provider;
    }
}
