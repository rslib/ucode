use std::collections::HashSet;

use crate::api::{
    API_VERSION, Feature, HandshakeError, HandshakeRequest, HandshakeResponse, HookHandler,
    HookResponse, Plugin, ToolProvider, check_features_compatible, check_version_compatible,
};
use crate::hooks::{HookEvent, HookRecord, OverrideClass};
use crate::manifest::PluginToolDef;
use crate::policy::{
    PluginIsolationLevel, PluginPolicy, PluginPolicyConfig, PolicyCheckResult, override_class_level,
};

/// Result of dispatching a hook to a single plugin.
pub struct HookResult {
    pub plugin_id: String,
    pub response: HookResponse,
}

/// Combined trait for plugins that handle hooks. Blanket impl covers all T: Plugin + HookHandler.
pub trait PluginWithHooks: Plugin + HookHandler {}
impl<T: Plugin + HookHandler> PluginWithHooks for T {}

/// Combined trait for plugins that provide tools. Blanket impl covers all T: Plugin + ToolProvider.
pub trait PluginWithTools: Plugin + ToolProvider {}
impl<T: Plugin + ToolProvider> PluginWithTools for T {}

/// Storage for a loaded plugin's instance and registered tool FQNs.
enum PluginInstance {
    /// Plugin with hook handling capability.
    WithHooks(Box<dyn PluginWithHooks>),
    /// Plugin with tool provision capability.
    WithTools(Box<dyn PluginWithTools>, Vec<PluginToolDef>),
    /// WASM component plugin.
    #[cfg(feature = "wasm")]
    Wasm(crate::wasm::WasmPlugin),
}

struct LoadedPlugin {
    plugin_id: String,
    instance: PluginInstance,
    /// FQNs for tools: `{plugin_id}.{tool_name}`.
    tool_fqns: Vec<String>,
    policy: PluginPolicy,
    /// Minimum payload versions required by this plugin per event name.
    min_payload_versions: std::collections::HashMap<String, String>,
}

/// Host runtime that manages plugin lifecycle: load, handshake, dispatch, unload.
pub struct PluginHost {
    plugins: Vec<LoadedPlugin>,
    supported_features: HashSet<Feature>,
}

fn run_handshake(
    req: HandshakeRequest,
    supported_features: &HashSet<Feature>,
) -> Result<HandshakeResponse, HandshakeError> {
    let host_version: semver::Version = API_VERSION.parse().expect("API_VERSION is valid semver");
    check_version_compatible(&host_version, &req.min_api_version)?;
    check_features_compatible(&req.required_features, supported_features)?;
    Ok(HandshakeResponse::Accepted {
        api_version: host_version,
        supported_features: supported_features.clone(),
        granted_capabilities: req.capabilities,
    })
}

impl PluginHost {
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
            supported_features: [Feature::Hooks, Feature::Tools, Feature::Ui].into(),
        }
    }

    /// Load a plugin that handles hook events.
    ///
    /// Performs handshake, version + feature checks, calls `initialize`, then stores the plugin.
    /// The plugin must implement both `Plugin` and `HookHandler`.
    pub fn load<P: PluginWithHooks + 'static>(
        &mut self,
        mut plugin: P,
    ) -> Result<(), HandshakeError> {
        let req = plugin.handshake();
        let plugin_id = req.plugin_id.clone();
        let resp = run_handshake(req, &self.supported_features)?;
        plugin
            .initialize(&resp)
            .map_err(|e| HandshakeError::CapabilityDenied { denied: vec![e] })?;
        tracing::info!(plugin_id = %plugin_id, "plugin loaded with native policy");
        self.plugins.push(LoadedPlugin {
            plugin_id,
            instance: PluginInstance::WithHooks(Box::new(plugin)),
            tool_fqns: vec![],
            policy: PluginPolicy::default_native(),
            min_payload_versions: std::collections::HashMap::new(),
        });
        Ok(())
    }

    /// Load a plugin that provides tools (and optionally handles hooks).
    ///
    /// Registers each tool with a fully-qualified name `{plugin_id}.{tool_name}`.
    pub fn load_with_tools<P: PluginWithTools + 'static>(
        &mut self,
        mut plugin: P,
    ) -> Result<(), HandshakeError> {
        let req = plugin.handshake();
        let plugin_id = req.plugin_id.clone();
        let resp = run_handshake(req, &self.supported_features)?;
        plugin
            .initialize(&resp)
            .map_err(|e| HandshakeError::CapabilityDenied { denied: vec![e] })?;
        let specs = plugin.tool_specs();
        let fqns: Vec<String> = specs
            .iter()
            .map(|t| format!("{}.{}", plugin_id, t.name))
            .collect();
        tracing::info!(plugin_id = %plugin_id, "plugin loaded with native policy");
        self.plugins.push(LoadedPlugin {
            plugin_id,
            instance: PluginInstance::WithTools(Box::new(plugin), specs),
            tool_fqns: fqns,
            policy: PluginPolicy::default_native(),
            min_payload_versions: std::collections::HashMap::new(),
        });
        Ok(())
    }

    /// Load a WASM component plugin from a `.wasm` file.
    ///
    /// The plugin ID is taken from the component after handshake; until then the
    /// file stem is used as a fallback so the plugin can be referenced by `unload`.
    #[cfg(feature = "wasm")]
    pub fn load_wasm(
        &mut self,
        path: &std::path::Path,
    ) -> Result<(), crate::wasm::WasmPluginError> {
        let plugin = crate::wasm::WasmPlugin::from_file(path)?;
        let plugin_id = if plugin.plugin_id().is_empty() {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown-wasm-plugin")
                .to_string()
        } else {
            plugin.plugin_id().to_string()
        };
        tracing::info!(plugin_id = %plugin_id, "WASM plugin loaded with default policy");
        self.plugins.push(LoadedPlugin {
            plugin_id,
            instance: PluginInstance::Wasm(plugin),
            tool_fqns: vec![],
            policy: PluginPolicy::default_wasm(),
            min_payload_versions: std::collections::HashMap::new(),
        });
        Ok(())
    }

    /// Call `shutdown` on the plugin and remove it. Returns `false` if not found.
    pub fn unload(&mut self, plugin_id: &str) -> bool {
        let Some(pos) = self.plugins.iter().position(|p| p.plugin_id == plugin_id) else {
            return false;
        };
        let mut loaded = self.plugins.remove(pos);
        match &mut loaded.instance {
            PluginInstance::WithHooks(p) => p.shutdown(),
            PluginInstance::WithTools(p, _) => p.shutdown(),
            #[cfg(feature = "wasm")]
            PluginInstance::Wasm(_) => { /* cleanup handled by Drop */ }
        }
        true
    }

    /// Reload plugin policies from a new config.
    ///
    /// Updates host-level policy checks immediately for all loaded plugins.
    /// Per-plugin overrides take precedence; plugins without overrides get
    /// the appropriate default (native or WASM).
    ///
    /// Note: WASI preopens are set at Store creation time and only take
    /// effect on next plugin restart.
    pub fn reload_policy_config(&mut self, config: &PluginPolicyConfig) {
        for loaded in &mut self.plugins {
            let new_policy = if let Some(override_policy) = config.per_plugin.get(&loaded.plugin_id)
            {
                override_policy.clone()
            } else {
                match &loaded.instance {
                    #[cfg(feature = "wasm")]
                    PluginInstance::Wasm(_) => config.default_wasm.clone(),
                    _ => config.default_native.clone(),
                }
            };
            tracing::info!(
                plugin_id = %loaded.plugin_id,
                "reloaded plugin policy"
            );
            loaded.policy = new_policy;
        }
    }

    /// Override the policy for a loaded plugin.
    pub fn set_plugin_policy(&mut self, plugin_id: &str, policy: PluginPolicy) {
        if let Some(loaded) = self.plugins.iter_mut().find(|p| p.plugin_id == plugin_id) {
            loaded.policy = policy;
        }
    }

    /// Get the effective policy for a plugin.
    pub fn plugin_policy(&self, plugin_id: &str) -> Option<&PluginPolicy> {
        self.plugins
            .iter()
            .find(|p| p.plugin_id == plugin_id)
            .map(|p| &p.policy)
    }

    /// List all plugin policies as (plugin_id, policy) pairs.
    pub fn plugin_policies(&self) -> Vec<(&str, &PluginPolicy)> {
        self.plugins
            .iter()
            .map(|p| (p.plugin_id.as_str(), &p.policy))
            .collect()
    }

    /// Dispatch a hook event to all loaded plugins that handle hooks.
    ///
    /// Respects per-plugin policy: skips plugins whose category is not allowed,
    /// and downgrades responses that exceed the plugin's override ceiling.
    ///
    /// Isolation levels control what each plugin sees:
    /// - `Full`: plugin receives only the original event (no accumulated changes).
    /// - `Ordered`: plugin receives accumulated changes from prior plugins in load order.
    pub fn dispatch_hook(&mut self, event: HookEvent) -> Vec<HookResult> {
        let record = HookRecord::new(event);
        let category = record.event.hook_category();
        let event_override_class = record.event.override_class();
        let mut results = Vec::new();
        let mut accumulated_changes: Option<serde_json::Value> = None;

        for loaded in &mut self.plugins {
            // Check hook category policy
            if loaded.policy.check_hook_category(category) != PolicyCheckResult::Allowed {
                tracing::debug!(
                    plugin_id = %loaded.plugin_id,
                    category = category,
                    "skipping plugin: hook category not allowed"
                );
                continue;
            }
            // Check payload version compatibility
            let event_name = record.event.event_name();
            if let Some(min_version) = loaded.min_payload_versions.get(event_name) {
                let current = record.event.payload_version();
                if let (Ok(current_ver), Ok(min_ver)) = (
                    semver::Version::parse(current),
                    semver::Version::parse(min_version),
                ) && current_ver < min_ver
                {
                    tracing::debug!(
                        plugin_id = %loaded.plugin_id,
                        event = event_name,
                        current = current,
                        required = %min_version,
                        "skipping: payload version too old"
                    );
                    continue;
                }
            }
            match &mut loaded.instance {
                PluginInstance::WithHooks(p) => {
                    let dispatch_record =
                        if loaded.policy.isolation_level == PluginIsolationLevel::Ordered {
                            HookRecord {
                                event: record.event.clone(),
                                timestamp: record.timestamp,
                                accumulated_changes: accumulated_changes.clone(),
                            }
                        } else {
                            HookRecord {
                                event: record.event.clone(),
                                timestamp: record.timestamp,
                                accumulated_changes: None,
                            }
                        };

                    let response = p.on_event(&dispatch_record);
                    let response = validate_hook_response(
                        response,
                        &loaded.policy,
                        &event_override_class,
                        &loaded.plugin_id,
                    );

                    // Accumulate changes from Modify responses for downstream Ordered plugins.
                    if let HookResponse::Modify { changes } = &response {
                        accumulated_changes = Some(match accumulated_changes.take() {
                            Some(mut existing) => {
                                if let (Some(obj), Some(new_obj)) =
                                    (existing.as_object_mut(), changes.as_object())
                                {
                                    for (k, v) in new_obj {
                                        obj.insert(k.clone(), v.clone());
                                    }
                                }
                                existing
                            }
                            None => changes.clone(),
                        });
                    }

                    results.push(HookResult {
                        plugin_id: loaded.plugin_id.clone(),
                        response,
                    });
                }
                PluginInstance::WithTools(_, _) => {}
                #[cfg(feature = "wasm")]
                PluginInstance::Wasm(wasm_plugin) => {
                    let event_name = record.event.event_name();
                    if wasm_plugin.handles_event(event_name) {
                        results.push(HookResult {
                            plugin_id: loaded.plugin_id.clone(),
                            response: HookResponse::Ok,
                        });
                    }
                }
            }
        }
        results
    }

    /// All registered tools as `(fqn, &PluginToolDef)` pairs.
    pub fn plugin_tools(&self) -> Vec<(String, &PluginToolDef)> {
        let mut out = Vec::new();
        for loaded in &self.plugins {
            if let PluginInstance::WithTools(_, specs) = &loaded.instance {
                for (fqn, spec) in loaded.tool_fqns.iter().zip(specs.iter()) {
                    out.push((fqn.clone(), spec));
                }
            }
        }
        out
    }

    /// Invoke a tool by its fully-qualified name `{plugin_id}.{tool_name}`.
    pub fn invoke_plugin_tool(
        &mut self,
        fqn: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        for loaded in &mut self.plugins {
            if let PluginInstance::WithTools(provider, _) = &mut loaded.instance
                && loaded.tool_fqns.iter().any(|f| f == fqn)
            {
                // Strip the `{plugin_id}.` prefix to get the local tool name.
                let local_name = fqn
                    .strip_prefix(&format!("{}.", loaded.plugin_id))
                    .unwrap_or(fqn);
                return provider.invoke_tool(local_name, args);
            }
        }
        Err(format!("unknown tool: {fqn}"))
    }

    /// Dispatch a hook and return individual results plus the aggregate response.
    pub fn dispatch_hook_aggregated(
        &mut self,
        event: HookEvent,
    ) -> (Vec<HookResult>, HookResponse) {
        let results = self.dispatch_hook(event);
        let aggregate = aggregate_hook_responses(&results);
        (results, aggregate)
    }

    /// Pipeline dispatch for transform events.
    ///
    /// Each plugin in `transform_order` sees the output of the previous plugin.
    /// `Modify` response = full replacement of the payload.
    /// `Ok` response = pass through unchanged.
    /// `Veto` response = skip this plugin (payload unchanged).
    ///
    /// Returns the final transformed payload.
    pub fn dispatch_transform(
        &mut self,
        event_name: &str,
        mut payload: String,
        transform_order: &[String],
    ) -> String {
        for plugin_id in transform_order {
            let Some(loaded) = self.plugins.iter_mut().find(|p| p.plugin_id == *plugin_id) else {
                continue;
            };

            // Build the appropriate hook event
            let event = match event_name {
                "transform_messages" => HookEvent::TransformMessages {
                    messages_json: payload.clone(),
                },
                "transform_system_prompt" => HookEvent::TransformSystemPrompt {
                    prompt: payload.clone(),
                },
                _ => continue,
            };
            let record = HookRecord::new(event);

            let response = match &mut loaded.instance {
                PluginInstance::WithHooks(p) => {
                    let resp = p.on_event(&record);
                    validate_hook_response(
                        resp,
                        &loaded.policy,
                        &record.event.override_class(),
                        &loaded.plugin_id,
                    )
                }
                PluginInstance::WithTools(_, _) => HookResponse::Ok,
                #[cfg(feature = "wasm")]
                PluginInstance::Wasm(_wasm_plugin) => {
                    // WASM dispatch will be wired in Task 3
                    HookResponse::Ok
                }
            };

            // Pipeline: Modify = replace payload, Ok/Veto = pass through
            if let HookResponse::Modify { changes } = response {
                if let Some(s) = changes.as_str() {
                    payload = s.to_string();
                } else {
                    payload = changes.to_string();
                }
            }
        }
        payload
    }

    /// Convenience: transform messages before LLM call.
    pub fn transform_messages(
        &mut self,
        messages_json: String,
        transform_order: &[String],
    ) -> String {
        self.dispatch_transform("transform_messages", messages_json, transform_order)
    }

    /// Convenience: transform system prompt before LLM call.
    pub fn transform_system_prompt(
        &mut self,
        prompt: String,
        transform_order: &[String],
    ) -> String {
        self.dispatch_transform("transform_system_prompt", prompt, transform_order)
    }

    /// Number of currently loaded plugins.
    pub fn loaded_count(&self) -> usize {
        self.plugins.len()
    }
}

/// Aggregate multiple plugin hook responses.
///
/// Resolution: Veto wins over Modify wins over Ok.
/// First Veto wins (plugin load order). Modify changes from the first
/// Modify response are used.
pub fn aggregate_hook_responses(results: &[HookResult]) -> HookResponse {
    // First Veto wins
    for r in results {
        if let HookResponse::Veto { reason } = &r.response {
            return HookResponse::Veto {
                reason: reason.clone(),
            };
        }
    }
    // First Modify wins
    for r in results {
        if let HookResponse::Modify { changes } = &r.response {
            return HookResponse::Modify {
                changes: changes.clone(),
            };
        }
    }
    HookResponse::Ok
}

fn validate_hook_response(
    response: HookResponse,
    policy: &PluginPolicy,
    event_class: &OverrideClass,
    plugin_id: &str,
) -> HookResponse {
    match &response {
        HookResponse::Ok => response,
        HookResponse::Modify { .. } => {
            let response_class = OverrideClass::Guarded;
            if policy.check_override_class(&response_class) != PolicyCheckResult::Allowed {
                tracing::warn!(
                    plugin_id = plugin_id,
                    "downgrading Modify to Ok: plugin override ceiling is {:?}",
                    policy.max_override_class
                );
                HookResponse::Ok
            } else if override_class_level(event_class) < override_class_level(&response_class) {
                tracing::warn!(
                    plugin_id = plugin_id,
                    "downgrading Modify to Ok: event class {:?} does not permit Modify",
                    event_class
                );
                HookResponse::Ok
            } else {
                response
            }
        }
        HookResponse::Veto { .. } => {
            let response_class = OverrideClass::Risky;
            if policy.check_override_class(&response_class) != PolicyCheckResult::Allowed {
                tracing::warn!(
                    plugin_id = plugin_id,
                    "downgrading Veto to Ok: plugin override ceiling is {:?}",
                    policy.max_override_class
                );
                HookResponse::Ok
            } else if override_class_level(event_class) < override_class_level(&response_class) {
                tracing::warn!(
                    plugin_id = plugin_id,
                    "downgrading Veto to Ok: event class {:?} does not permit Veto",
                    event_class
                );
                HookResponse::Ok
            } else {
                response
            }
        }
    }
}

/// Event names that use pipeline dispatch instead of fan-out.
const TRANSFORM_EVENTS: &[&str] = &["transform_messages", "transform_system_prompt"];

/// Returns true if this event uses pipeline dispatch.
pub fn is_transform_event(event_name: &str) -> bool {
    TRANSFORM_EVENTS.contains(&event_name)
}

impl Default for PluginHost {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::*;
    use crate::hooks::*;
    use crate::manifest::*;

    /// Minimal test plugin that implements Plugin + HookHandler.
    struct TestPlugin {
        id: String,
        events: Vec<String>,
        initialized: bool,
    }

    impl TestPlugin {
        fn new(id: &str) -> Self {
            Self {
                id: id.to_string(),
                events: vec![],
                initialized: false,
            }
        }
    }

    impl Plugin for TestPlugin {
        fn handshake(&self) -> HandshakeRequest {
            HandshakeRequest {
                plugin_id: self.id.clone(),
                plugin_version: semver::Version::new(1, 0, 0),
                min_api_version: semver::Version::new(1, 0, 0),
                required_features: [Feature::Hooks].into(),
                capabilities: PluginCapabilities::default(),
            }
        }
        fn initialize(&mut self, _resp: &HandshakeResponse) -> Result<(), String> {
            self.initialized = true;
            Ok(())
        }
        fn shutdown(&mut self) {
            self.initialized = false;
        }
    }

    impl HookHandler for TestPlugin {
        fn on_event(&mut self, record: &HookRecord) -> HookResponse {
            self.events.push(record.event.event_name().to_string());
            HookResponse::Ok
        }
    }

    #[test]
    fn test_load_plugin_success() {
        let mut host = PluginHost::new();
        let plugin = TestPlugin::new("org.test.logger");
        assert!(host.load(plugin).is_ok());
        assert_eq!(host.loaded_count(), 1);
    }

    #[test]
    fn test_load_plugin_version_mismatch() {
        struct FuturePlugin;
        impl Plugin for FuturePlugin {
            fn handshake(&self) -> HandshakeRequest {
                HandshakeRequest {
                    plugin_id: "org.test.future".into(),
                    plugin_version: semver::Version::new(1, 0, 0),
                    min_api_version: semver::Version::new(2, 0, 0),
                    required_features: [Feature::Hooks].into(),
                    capabilities: PluginCapabilities::default(),
                }
            }
            fn initialize(&mut self, _: &HandshakeResponse) -> Result<(), String> {
                Ok(())
            }
            fn shutdown(&mut self) {}
        }
        impl HookHandler for FuturePlugin {
            fn on_event(&mut self, _: &HookRecord) -> HookResponse {
                HookResponse::Ok
            }
        }
        let mut host = PluginHost::new();
        let err = host.load(FuturePlugin).unwrap_err();
        assert!(matches!(err, HandshakeError::VersionIncompatible { .. }));
    }

    #[test]
    fn test_dispatch_hook_to_plugin() {
        let mut host = PluginHost::new();
        let plugin = TestPlugin::new("org.test.logger");
        host.load(plugin).unwrap();
        let results = host.dispatch_hook(HookEvent::SessionStart {
            session_id: "s1".into(),
        });
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].plugin_id, "org.test.logger");
        assert!(matches!(results[0].response, HookResponse::Ok));
    }

    #[test]
    fn test_unload_plugin() {
        let mut host = PluginHost::new();
        let plugin = TestPlugin::new("org.test.logger");
        host.load(plugin).unwrap();
        assert!(host.unload("org.test.logger"));
        assert_eq!(host.loaded_count(), 0);
    }

    #[test]
    fn test_unload_nonexistent() {
        let mut host = PluginHost::new();
        assert!(!host.unload("org.test.ghost"));
    }

    #[test]
    fn test_plugin_tool_registration() {
        struct ToolPlugin {
            id: String,
        }
        impl Plugin for ToolPlugin {
            fn handshake(&self) -> HandshakeRequest {
                HandshakeRequest {
                    plugin_id: self.id.clone(),
                    plugin_version: semver::Version::new(1, 0, 0),
                    min_api_version: semver::Version::new(1, 0, 0),
                    required_features: [Feature::Hooks, Feature::Tools].into(),
                    capabilities: PluginCapabilities::default(),
                }
            }
            fn initialize(&mut self, _: &HandshakeResponse) -> Result<(), String> {
                Ok(())
            }
            fn shutdown(&mut self) {}
        }
        impl ToolProvider for ToolPlugin {
            fn tool_specs(&self) -> Vec<PluginToolDef> {
                vec![PluginToolDef {
                    name: "lint".into(),
                    description: Some("Run linter".into()),
                    input_schema: None,
                }]
            }
            fn invoke_tool(
                &mut self,
                name: &str,
                _args: serde_json::Value,
            ) -> Result<serde_json::Value, String> {
                Ok(serde_json::json!({ "tool": name, "status": "ok" }))
            }
        }
        let mut host = PluginHost::new();
        let plugin = ToolPlugin {
            id: "org.acme.tools".into(),
        };
        host.load_with_tools(plugin).unwrap();
        let tools = host.plugin_tools();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].0, "org.acme.tools.lint");
    }

    #[test]
    fn test_invoke_plugin_tool() {
        struct EchoPlugin;
        impl Plugin for EchoPlugin {
            fn handshake(&self) -> HandshakeRequest {
                HandshakeRequest {
                    plugin_id: "org.test.echo".into(),
                    plugin_version: semver::Version::new(1, 0, 0),
                    min_api_version: semver::Version::new(1, 0, 0),
                    required_features: [Feature::Tools].into(),
                    capabilities: PluginCapabilities::default(),
                }
            }
            fn initialize(&mut self, _: &HandshakeResponse) -> Result<(), String> {
                Ok(())
            }
            fn shutdown(&mut self) {}
        }
        impl ToolProvider for EchoPlugin {
            fn tool_specs(&self) -> Vec<PluginToolDef> {
                vec![PluginToolDef {
                    name: "echo".into(),
                    description: Some("Echo input".into()),
                    input_schema: None,
                }]
            }
            fn invoke_tool(
                &mut self,
                _name: &str,
                args: serde_json::Value,
            ) -> Result<serde_json::Value, String> {
                Ok(args)
            }
        }
        let mut host = PluginHost::new();
        host.load_with_tools(EchoPlugin).unwrap();
        let result =
            host.invoke_plugin_tool("org.test.echo.echo", serde_json::json!({"msg": "hi"}));
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), serde_json::json!({"msg": "hi"}));
    }

    #[test]
    fn test_invoke_unknown_tool() {
        let mut host = PluginHost::new();
        let result = host.invoke_plugin_tool("org.ghost.tool", serde_json::json!({}));
        assert!(result.is_err());
    }

    #[cfg(feature = "wasm")]
    #[test]
    fn test_load_wasm_nonexistent_file_errors() {
        let mut host = PluginHost::new();
        let path = std::path::Path::new("/nonexistent/plugin.wasm");
        let err = host.load_wasm(path).unwrap_err();
        // ComponentLoad wraps the wasmtime I/O error; just verify we get an error.
        assert!(matches!(
            err,
            crate::wasm::WasmPluginError::ComponentLoad(_)
        ));
    }

    #[test]
    fn test_dispatch_hook_respects_category_policy() {
        let mut host = PluginHost::new();
        let plugin = TestPlugin::new("org.test.session-only");
        host.load(plugin).unwrap();
        // Restrict to session category only
        host.set_plugin_policy("org.test.session-only", {
            let mut p = PluginPolicy::default_native();
            p.allowed_hook_categories = ["session".into()].into();
            p
        });
        // Session event should be dispatched
        let results = host.dispatch_hook(HookEvent::SessionStart {
            session_id: "s1".into(),
        });
        assert_eq!(results.len(), 1);
        // Tool event should be skipped
        let results = host.dispatch_hook(HookEvent::BeforeToolCall {
            tool_name: "bash".into(),
            args: serde_json::Value::Null,
        });
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_dispatch_hook_downgrades_modify_when_ceiling_safe() {
        struct ModifyPlugin;
        impl Plugin for ModifyPlugin {
            fn handshake(&self) -> HandshakeRequest {
                HandshakeRequest {
                    plugin_id: "org.test.modifier".into(),
                    plugin_version: semver::Version::new(1, 0, 0),
                    min_api_version: semver::Version::new(1, 0, 0),
                    required_features: [Feature::Hooks].into(),
                    capabilities: PluginCapabilities::default(),
                }
            }
            fn initialize(&mut self, _: &HandshakeResponse) -> Result<(), String> {
                Ok(())
            }
            fn shutdown(&mut self) {}
        }
        impl HookHandler for ModifyPlugin {
            fn on_event(&mut self, _: &HookRecord) -> HookResponse {
                HookResponse::Modify {
                    changes: serde_json::json!({"key": "val"}),
                }
            }
        }
        let mut host = PluginHost::new();
        host.load(ModifyPlugin).unwrap();
        // Set Safe ceiling -- Modify should be downgraded to Ok
        host.set_plugin_policy("org.test.modifier", PluginPolicy::default_wasm());
        let results = host.dispatch_hook(HookEvent::BeforeToolCall {
            tool_name: "bash".into(),
            args: serde_json::Value::Null,
        });
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].response, HookResponse::Ok));
    }

    #[test]
    fn test_plugin_policy_query() {
        let mut host = PluginHost::new();
        let plugin = TestPlugin::new("org.test.logger");
        host.load(plugin).unwrap();
        // Native plugins get default_native policy
        let policy = host.plugin_policy("org.test.logger");
        assert!(policy.is_some());
        assert!(policy.unwrap().filesystem_write);
        assert!(host.plugin_policy("org.test.ghost").is_none());
    }

    #[test]
    fn test_plugin_policies_list() {
        let mut host = PluginHost::new();
        host.load(TestPlugin::new("org.test.a")).unwrap();
        host.load(TestPlugin::new("org.test.b")).unwrap();
        let policies = host.plugin_policies();
        assert_eq!(policies.len(), 2);
    }

    #[test]
    fn test_aggregate_hook_responses_all_ok() {
        let results = vec![
            HookResult {
                plugin_id: "a".into(),
                response: HookResponse::Ok,
            },
            HookResult {
                plugin_id: "b".into(),
                response: HookResponse::Ok,
            },
        ];
        let agg = aggregate_hook_responses(&results);
        assert!(matches!(agg, HookResponse::Ok));
    }

    #[test]
    fn test_aggregate_hook_responses_veto_wins() {
        let results = vec![
            HookResult {
                plugin_id: "a".into(),
                response: HookResponse::Modify {
                    changes: serde_json::json!({"x": 1}),
                },
            },
            HookResult {
                plugin_id: "b".into(),
                response: HookResponse::Veto {
                    reason: "blocked".into(),
                },
            },
        ];
        let agg = aggregate_hook_responses(&results);
        assert!(matches!(agg, HookResponse::Veto { .. }));
        if let HookResponse::Veto { reason } = agg {
            assert_eq!(reason, "blocked");
        }
    }

    #[test]
    fn test_aggregate_hook_responses_first_veto_wins() {
        let results = vec![
            HookResult {
                plugin_id: "a".into(),
                response: HookResponse::Veto {
                    reason: "first".into(),
                },
            },
            HookResult {
                plugin_id: "b".into(),
                response: HookResponse::Veto {
                    reason: "second".into(),
                },
            },
        ];
        let agg = aggregate_hook_responses(&results);
        if let HookResponse::Veto { reason } = agg {
            assert_eq!(reason, "first");
        } else {
            panic!("expected Veto");
        }
    }

    #[test]
    fn test_aggregate_hook_responses_modify_over_ok() {
        let results = vec![
            HookResult {
                plugin_id: "a".into(),
                response: HookResponse::Ok,
            },
            HookResult {
                plugin_id: "b".into(),
                response: HookResponse::Modify {
                    changes: serde_json::json!({"key": "val"}),
                },
            },
        ];
        let agg = aggregate_hook_responses(&results);
        assert!(matches!(agg, HookResponse::Modify { .. }));
    }

    #[test]
    fn test_aggregate_hook_responses_empty() {
        let results: Vec<HookResult> = vec![];
        let agg = aggregate_hook_responses(&results);
        assert!(matches!(agg, HookResponse::Ok));
    }

    #[test]
    fn test_full_isolation_no_accumulated_changes() {
        let mut host = PluginHost::new();
        host.load(TestPlugin::new("org.test.a")).unwrap();
        host.load(TestPlugin::new("org.test.b")).unwrap();

        let policy = PluginPolicy::default_wasm();
        host.set_plugin_policy("org.test.a", policy.clone());
        host.set_plugin_policy("org.test.b", policy);

        let results = host.dispatch_hook(HookEvent::SessionStart {
            session_id: "s1".into(),
        });
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_ordered_dispatch_accumulates_changes() {
        struct ModifierPlugin {
            id: String,
        }
        impl Plugin for ModifierPlugin {
            fn handshake(&self) -> HandshakeRequest {
                HandshakeRequest {
                    plugin_id: self.id.clone(),
                    plugin_version: semver::Version::new(1, 0, 0),
                    min_api_version: semver::Version::new(1, 0, 0),
                    required_features: [Feature::Hooks].into(),
                    capabilities: PluginCapabilities::default(),
                }
            }
            fn initialize(&mut self, _: &HandshakeResponse) -> Result<(), String> {
                Ok(())
            }
            fn shutdown(&mut self) {}
        }
        impl HookHandler for ModifierPlugin {
            fn on_event(&mut self, _record: &HookRecord) -> HookResponse {
                if self.id == "org.test.modifier" {
                    HookResponse::Modify {
                        changes: serde_json::json!({"injected": true}),
                    }
                } else {
                    HookResponse::Ok
                }
            }
        }

        let mut host = PluginHost::new();
        host.load(ModifierPlugin {
            id: "org.test.modifier".into(),
        })
        .unwrap();
        host.load(ModifierPlugin {
            id: "org.test.observer".into(),
        })
        .unwrap();

        let mut policy = PluginPolicy::default_native();
        policy.isolation_level = crate::policy::PluginIsolationLevel::Ordered;
        policy.max_override_class = OverrideClass::Guarded;
        host.set_plugin_policy("org.test.modifier", policy.clone());
        host.set_plugin_policy("org.test.observer", policy);

        let results = host.dispatch_hook(HookEvent::BeforeToolCall {
            tool_name: "bash".into(),
            args: serde_json::json!({"cmd": "ls"}),
        });
        assert_eq!(results.len(), 2);
        assert!(matches!(results[0].response, HookResponse::Modify { .. }));
        assert!(matches!(results[1].response, HookResponse::Ok));
    }

    #[test]
    fn test_accumulated_changes_merge() {
        struct ChangePlugin {
            id: String,
            change_key: String,
        }
        impl Plugin for ChangePlugin {
            fn handshake(&self) -> HandshakeRequest {
                HandshakeRequest {
                    plugin_id: self.id.clone(),
                    plugin_version: semver::Version::new(1, 0, 0),
                    min_api_version: semver::Version::new(1, 0, 0),
                    required_features: [Feature::Hooks].into(),
                    capabilities: PluginCapabilities::default(),
                }
            }
            fn initialize(&mut self, _: &HandshakeResponse) -> Result<(), String> {
                Ok(())
            }
            fn shutdown(&mut self) {}
        }
        impl HookHandler for ChangePlugin {
            fn on_event(&mut self, _record: &HookRecord) -> HookResponse {
                HookResponse::Modify {
                    changes: serde_json::json!({ &self.change_key: true }),
                }
            }
        }

        let mut host = PluginHost::new();
        host.load(ChangePlugin {
            id: "org.test.a".into(),
            change_key: "key_a".into(),
        })
        .unwrap();
        host.load(ChangePlugin {
            id: "org.test.b".into(),
            change_key: "key_b".into(),
        })
        .unwrap();

        let mut policy = PluginPolicy::default_native();
        policy.isolation_level = crate::policy::PluginIsolationLevel::Ordered;
        policy.max_override_class = OverrideClass::Guarded;
        host.set_plugin_policy("org.test.a", policy.clone());
        host.set_plugin_policy("org.test.b", policy);

        let results = host.dispatch_hook(HookEvent::BeforeToolCall {
            tool_name: "bash".into(),
            args: serde_json::json!({}),
        });
        assert_eq!(results.len(), 2);
        assert!(matches!(results[0].response, HookResponse::Modify { .. }));
        assert!(matches!(results[1].response, HookResponse::Modify { .. }));
    }

    #[test]
    fn test_reload_policy_config() {
        let mut host = PluginHost::new();
        host.load(TestPlugin::new("org.test.logger")).unwrap();

        // Initial policy is default_native
        let policy = host.plugin_policy("org.test.logger").unwrap();
        assert!(policy.filesystem_write);

        // Reload with restrictive per-plugin config
        let mut config = PluginPolicyConfig::default();
        let mut restrictive = PluginPolicy::default_wasm();
        restrictive.filesystem_write = false;
        config
            .per_plugin
            .insert("org.test.logger".into(), restrictive);
        host.reload_policy_config(&config);

        // Policy should be updated
        let policy = host.plugin_policy("org.test.logger").unwrap();
        assert!(!policy.filesystem_write);
    }

    #[test]
    fn test_reload_policy_config_unaffected_plugins() {
        let mut host = PluginHost::new();
        host.load(TestPlugin::new("org.test.a")).unwrap();
        host.load(TestPlugin::new("org.test.b")).unwrap();

        let mut config = PluginPolicyConfig::default();
        let mut restrictive = PluginPolicy::default_wasm();
        restrictive.process_spawn = false;
        config.per_plugin.insert("org.test.a".into(), restrictive);
        host.reload_policy_config(&config);

        // org.test.a should be restricted
        assert!(!host.plugin_policy("org.test.a").unwrap().process_spawn);
        // org.test.b should get default_native (native plugin with no override)
        assert!(host.plugin_policy("org.test.b").unwrap().process_spawn);
    }

    #[test]
    fn test_reload_policy_config_updates_isolation_level() {
        let mut host = PluginHost::new();
        host.load(TestPlugin::new("org.test.plugin")).unwrap();

        // Initially native default = Ordered
        assert_eq!(
            host.plugin_policy("org.test.plugin")
                .unwrap()
                .isolation_level,
            PluginIsolationLevel::Ordered
        );

        // Reload with Full isolation override
        let mut config = PluginPolicyConfig::default();
        let mut override_policy = PluginPolicy::default_wasm();
        override_policy.isolation_level = PluginIsolationLevel::Full;
        config
            .per_plugin
            .insert("org.test.plugin".into(), override_policy);
        host.reload_policy_config(&config);

        assert_eq!(
            host.plugin_policy("org.test.plugin")
                .unwrap()
                .isolation_level,
            PluginIsolationLevel::Full
        );
    }

    #[test]
    fn test_is_transform_event() {
        assert!(is_transform_event("transform_messages"));
        assert!(is_transform_event("transform_system_prompt"));
        assert!(!is_transform_event("session_start"));
    }

    #[test]
    fn test_dispatch_transform_ok_passes_through() {
        let mut host = PluginHost::new();
        host.load(TestPlugin::new("org.test.noop")).unwrap();
        let input = r#"[{"role":"user","content":"hello"}]"#.to_string();
        let output = host.dispatch_transform(
            "transform_messages",
            input.clone(),
            &["org.test.noop".to_string()],
        );
        // TestPlugin returns Ok, so payload should be unchanged
        assert_eq!(output, input);
    }

    #[test]
    fn test_dispatch_transform_modify_replaces_payload() {
        struct TransformPlugin;
        impl Plugin for TransformPlugin {
            fn handshake(&self) -> HandshakeRequest {
                HandshakeRequest {
                    plugin_id: "org.test.transformer".into(),
                    plugin_version: semver::Version::new(1, 0, 0),
                    min_api_version: semver::Version::new(1, 0, 0),
                    required_features: [Feature::Hooks].into(),
                    capabilities: PluginCapabilities::default(),
                }
            }
            fn initialize(&mut self, _: &HandshakeResponse) -> Result<(), String> {
                Ok(())
            }
            fn shutdown(&mut self) {}
        }
        impl HookHandler for TransformPlugin {
            fn on_event(&mut self, record: &HookRecord) -> HookResponse {
                if record.event.event_name() == "transform_messages" {
                    HookResponse::Modify {
                        changes: serde_json::json!(
                            "[{\"role\":\"user\",\"content\":\"transformed\"}]"
                        ),
                    }
                } else {
                    HookResponse::Ok
                }
            }
        }
        let mut host = PluginHost::new();
        host.load(TransformPlugin).unwrap();
        // Set policy to allow Modify
        let mut policy = PluginPolicy::default_native();
        policy.max_override_class = OverrideClass::Guarded;
        host.set_plugin_policy("org.test.transformer", policy);

        let input = r#"[{"role":"user","content":"hello"}]"#.to_string();
        let output = host.dispatch_transform(
            "transform_messages",
            input,
            &["org.test.transformer".to_string()],
        );
        assert!(output.contains("transformed"));
    }

    #[test]
    fn test_dispatch_transform_chains_plugins() {
        struct AppendPlugin {
            id: String,
            suffix: String,
        }
        impl Plugin for AppendPlugin {
            fn handshake(&self) -> HandshakeRequest {
                HandshakeRequest {
                    plugin_id: self.id.clone(),
                    plugin_version: semver::Version::new(1, 0, 0),
                    min_api_version: semver::Version::new(1, 0, 0),
                    required_features: [Feature::Hooks].into(),
                    capabilities: PluginCapabilities::default(),
                }
            }
            fn initialize(&mut self, _: &HandshakeResponse) -> Result<(), String> {
                Ok(())
            }
            fn shutdown(&mut self) {}
        }
        impl HookHandler for AppendPlugin {
            fn on_event(&mut self, record: &HookRecord) -> HookResponse {
                if record.event.event_name() == "transform_system_prompt"
                    && let HookEvent::TransformSystemPrompt { ref prompt } = record.event
                {
                    return HookResponse::Modify {
                        changes: serde_json::Value::String(format!("{}{}", prompt, self.suffix)),
                    };
                }
                HookResponse::Ok
            }
        }
        let mut host = PluginHost::new();
        host.load(AppendPlugin {
            id: "org.test.a".into(),
            suffix: " [A]".into(),
        })
        .unwrap();
        host.load(AppendPlugin {
            id: "org.test.b".into(),
            suffix: " [B]".into(),
        })
        .unwrap();

        // Set policies to allow Modify
        let mut policy = PluginPolicy::default_native();
        policy.max_override_class = OverrideClass::Guarded;
        host.set_plugin_policy("org.test.a", policy.clone());
        host.set_plugin_policy("org.test.b", policy);

        let output = host.dispatch_transform(
            "transform_system_prompt",
            "Base prompt".to_string(),
            &["org.test.a".to_string(), "org.test.b".to_string()],
        );
        // A appends " [A]", B sees "Base prompt [A]" and appends " [B]"
        assert_eq!(output, "Base prompt [A] [B]");
    }

    #[test]
    fn test_dispatch_transform_skips_missing_plugin() {
        let mut host = PluginHost::new();
        let input = "original".to_string();
        let output = host.dispatch_transform(
            "transform_system_prompt",
            input.clone(),
            &["org.nonexistent".to_string()],
        );
        assert_eq!(output, input);
    }

    #[test]
    fn test_dispatch_skips_on_payload_version_mismatch() {
        let mut host = PluginHost::new();
        let plugin = TestPlugin::new("org.test.future");
        host.load(plugin).unwrap();

        // Set min_payload_versions to require 2.0.0 for session_start
        if let Some(loaded) = host
            .plugins
            .iter_mut()
            .find(|p| p.plugin_id == "org.test.future")
        {
            loaded
                .min_payload_versions
                .insert("session_start".to_string(), "2.0.0".to_string());
        }

        // Current payload version is 1.0.0, so this should be skipped
        let results = host.dispatch_hook(HookEvent::SessionStart {
            session_id: "s1".into(),
        });
        assert_eq!(
            results.len(),
            0,
            "plugin should be skipped due to version mismatch"
        );
    }

    #[test]
    fn test_dispatch_allows_matching_payload_version() {
        let mut host = PluginHost::new();
        let plugin = TestPlugin::new("org.test.current");
        host.load(plugin).unwrap();

        // Set min_payload_versions to require 1.0.0 (matches current)
        if let Some(loaded) = host
            .plugins
            .iter_mut()
            .find(|p| p.plugin_id == "org.test.current")
        {
            loaded
                .min_payload_versions
                .insert("session_start".to_string(), "1.0.0".to_string());
        }

        let results = host.dispatch_hook(HookEvent::SessionStart {
            session_id: "s1".into(),
        });
        assert_eq!(
            results.len(),
            1,
            "plugin should be dispatched with matching version"
        );
    }
}
