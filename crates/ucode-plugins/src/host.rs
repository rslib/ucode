use std::collections::HashSet;

use crate::api::{
    API_VERSION, Feature, HandshakeError, HandshakeRequest, HandshakeResponse, HookHandler,
    HookResponse, Plugin, ToolProvider, check_features_compatible, check_version_compatible,
};
use crate::hooks::{HookEvent, HookRecord, OverrideClass};
use crate::manifest::PluginToolDef;
use crate::policy::{PluginPolicy, PolicyCheckResult, override_class_level};

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
        self.plugins.push(LoadedPlugin {
            plugin_id,
            instance: PluginInstance::WithHooks(Box::new(plugin)),
            tool_fqns: vec![],
            policy: PluginPolicy::default_native(),
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
        self.plugins.push(LoadedPlugin {
            plugin_id,
            instance: PluginInstance::WithTools(Box::new(plugin), specs),
            tool_fqns: fqns,
            policy: PluginPolicy::default_native(),
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
        self.plugins.push(LoadedPlugin {
            plugin_id,
            instance: PluginInstance::Wasm(plugin),
            tool_fqns: vec![],
            policy: PluginPolicy::default_wasm(),
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
    pub fn dispatch_hook(&mut self, event: HookEvent) -> Vec<HookResult> {
        let record = HookRecord::new(event);
        let category = record.event.hook_category();
        let event_override_class = record.event.override_class();
        let mut results = Vec::new();
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
            match &mut loaded.instance {
                PluginInstance::WithHooks(p) => {
                    let response = p.on_event(&record);
                    let response = validate_hook_response(
                        response,
                        &loaded.policy,
                        &event_override_class,
                        &loaded.plugin_id,
                    );
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

    /// Number of currently loaded plugins.
    pub fn loaded_count(&self) -> usize {
        self.plugins.len()
    }
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
}
