# Plugin API Contract + SDK Design (Task 8.3 / ISSUE 0803)

## Goal

Define the v1 plugin API as Rust traits with handshake/capability negotiation,
expand the hook event surface to 64 events, implement tool registration with
reverse-domain namespacing, and provide an in-process example plugin.

Traits-first approach: WASM/WIT deferred to Task 8.4.

## Architecture

The plugin system has three layers:

1. **Manifest** (existing, extended) -- static plugin metadata parsed from `plugin.toml`
2. **API contract** (new) -- Rust traits that plugins implement, handshake protocol
3. **Host runtime** (new) -- loads plugins, performs handshake, dispatches hooks, registers tools

Plugins are currently discovered via `discover_plugins()` and tracked in
`PluginRegistry`. This design adds the runtime bridge: after discovery, the host
loads the plugin, performs a handshake, and wires it into the hook dispatcher and
tool registry.

## Manifest Changes

### Before (current)

```toml
name = "my-plugin"
version = "1.0.0"
description = "A plugin"
author = "Alice"
min_api_version = "0.5.0"
hooks = ["before_tool_call"]

[[tools]]
name = "search"
description = "Search the web"

[capabilities]
filesystem = true
network = false
process_spawn = false
guarded_ui = false
```

### After (v1)

```toml
id = "org.acme.code-analyzer"        # reverse-domain, globally unique, marketplace ID
name = "Code Analyzer"                # human-readable display name
version = "1.0.0"
description = "Static analysis tools for code quality"
author = "Acme Corp"
min_api_version = "1.0.0"            # semver floor
required_features = ["hooks", "tools"] # API surfaces this plugin uses

hooks = ["before_tool_call", "after_tool_call", "before_apply_patch"]

[[tools]]
name = "lint"                         # local name; FQN = org.acme.code-analyzer.lint
description = "Run linter on file"
input_schema = { type = "object", properties = { path = { type = "string" } } }

[capabilities]
filesystem = true
network = false
process_spawn = false
guarded_ui = false
```

### Validation Rules

- `id`: minimum 3 dot-separated segments, each segment matches `[a-z0-9][a-z0-9-]*`
- `name`: non-empty string (display name, no format constraints)
- `version`: valid semver
- `min_api_version`: valid semver (optional, defaults to "1.0.0")
- `required_features`: each element must be a known feature string
- Tool names: `[a-z0-9_-]+`, no dots (host constructs FQN)
- Hook names: must match a known `HookEvent::event_name()` value

### Migration

Existing manifests using `name` as identifier get a deprecation warning.
The `id` field becomes the canonical identifier. If `id` is absent but `name`
is present and contains dots, treat `name` as `id` with a warning.

## Handshake Protocol

```
Plugin                              Host
  |                                   |
  |--- HandshakeRequest ------------->|
  |    plugin_id                      |
  |    min_api_version                |  1. Semver check: host >= plugin (same major)
  |    required_features              |  2. Feature check: required subset of supported
  |    capabilities                   |  3. Capability check: grant subset of requested
  |                                   |
  |<-- HandshakeResponse -------------|
  |    Accepted { api_version,        |
  |      supported_features,          |
  |      granted_capabilities }       |
  |    OR Rejected { reason }         |
  |                                   |
  |--- initialize(response) --------->|  Plugin does setup
  |                                   |
  |    ... hook dispatch loop ...     |
  |                                   |
  |--- shutdown() ------------------->|  Cleanup on unload
```

### HandshakeError variants

- `VersionIncompatible { host, required }` -- major version mismatch or host too old
- `UnsupportedFeatures { missing }` -- plugin needs features host doesn't support
- `CapabilityDenied { denied }` -- policy blocks requested capabilities

## Plugin Traits (v1 Contract)

```rust
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

/// Core plugin trait. Every plugin implements this.
pub trait Plugin: Send + Sync {
    /// Return handshake request with plugin's requirements.
    fn handshake(&self) -> HandshakeRequest;

    /// Called after successful handshake. Plugin performs setup.
    fn initialize(&mut self, response: &HandshakeResponse) -> Result<(), String>;

    /// Called on shutdown. Plugin cleans up resources.
    fn shutdown(&mut self);
}

/// Optional: handle hook events. Requires `Feature::Hooks`.
pub trait HookHandler: Plugin {
    /// Process a hook event and return a response.
    fn on_event(&mut self, record: &HookRecord) -> HookResponse;
}

/// What a plugin returns from a hook.
pub enum HookResponse {
    /// Observed, no action taken.
    Ok,
    /// Propose modifications (only valid for Guarded events).
    Modify { changes: serde_json::Value },
    /// Veto the action (only valid for Risky events, requires approval).
    Veto { reason: String },
}

/// Optional: provide tools. Requires `Feature::Tools`.
pub trait ToolProvider: Plugin {
    /// Declare tool specs during initialization.
    fn tool_specs(&self) -> Vec<ToolSpec>;

    /// Handle a tool invocation. `name` is the local tool name (not FQN).
    fn invoke_tool(
        &mut self,
        name: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, String>;
}
```

## Tool Registration Model

1. Plugin implements `ToolProvider` and returns `Vec<ToolSpec>` from `tool_specs()`
2. Host validates each tool spec:
   - Name matches `[a-z0-9_-]+`
   - `input_schema` is valid JSON Schema (if provided)
   - Required capabilities are granted
3. Host registers each tool with FQN: `{plugin_id}.{tool_name}`
   - Example: `org.acme.code-analyzer.lint`
4. Plugin tool invocations flow through the same sandbox/approval/audit pipeline
5. When plugin is unloaded, all its tools are deregistered

## Version Negotiation

Uses the `semver` crate for parsing and comparison.

```
host_version = 1.2.0
plugin.min_api_version = 1.0.0

Check: host_version.major == plugin.min_api_version.major
       AND host_version >= plugin.min_api_version
Result: compatible
```

```
host_version = 2.0.0
plugin.min_api_version = 1.0.0

Check: host_version.major != plugin.min_api_version.major
Result: VersionIncompatible
```

Feature negotiation is a simple set-subset check:

```
plugin.required_features = {Hooks, Tools}
host.supported_features = {Hooks, Tools, Ui}

Check: {Hooks, Tools} is subset of {Hooks, Tools, Ui}
Result: compatible
```

## Hook Event Surface (64 events)

Expanding from 22 to 64 events. Full list with override class:

### Session lifecycle (5 events)
- `session_start` (Safe)
- `session_end` (Safe)
- `session_title_generated` (Safe)
- `session_title_updated` (Safe)
- `config_reloaded` (Safe)

### Message flow (4 events)
- `user_message_received` (Safe)
- `assistant_response_started` (Safe)
- `assistant_response_completed` (Safe)
- `message_retry` (Guarded)

### Model selection & routing (7 events)
- `before_model_call` (Guarded)
- `after_model_call` (Safe)
- `before_model_select` (Guarded)
- `model_fallback` (Risky)
- `router_decision` (Safe)
- `model_rate_limited` (Safe)
- `model_quota_exhausted` (Safe)

### Tool calls -- generic (4 events)
- `before_tool_call` (Guarded)
- `after_tool_call` (Safe)
- `tool_error` (Safe)
- `tool_timeout` (Safe)

### Tool calls -- specific (8 events)
- `before_apply_patch` (Guarded)
- `after_apply_patch` (Safe)
- `before_run_cmd` (Guarded)
- `after_run_cmd` (Safe)
- `before_file_read` (Guarded)
- `after_file_read` (Safe)
- `before_file_write` (Guarded)
- `after_file_write` (Safe)

### Context management (4 events)
- `context_overflow` (Guarded)
- `context_compaction` (Guarded)
- `context_distilled` (Safe)
- `token_usage_updated` (Safe)

### Agent / Sub-agent (5 events)
- `agent_spawned` (Safe)
- `agent_message` (Safe)
- `agent_completed` (Safe)
- `agent_failed` (Safe)
- `agent_cancelled` (Safe)

### Approval / Permission / Sandbox (5 events)
- `approval_required` (Guarded)
- `approval_granted` (Safe)
- `approval_denied` (Safe)
- `sandbox_decision` (Safe)
- `permission_decision` (Safe)

### Auth & Provider (3 events)
- `auth_changed` (Safe)
- `auth_failed` (Safe)
- `provider_switched` (Safe)

### MCP servers (6 events)
- `mcp_server_connected` (Safe)
- `mcp_server_disconnected` (Safe)
- `mcp_server_launch` (Safe)
- `mcp_server_restart` (Safe)
- `mcp_server_crash` (Safe)
- `mcp_tool_invoked` (Safe)

### Skills (2 events)
- `skill_activated` (Safe)
- `skill_deactivated` (Safe)

### Plugins (3 events)
- `plugin_loaded` (Safe)
- `plugin_unloaded` (Safe)
- `plugin_error` (Safe)

### Checkpoints (2 events)
- `checkpoint_created` (Guarded)
- `checkpoint_restored` (Risky)

### Budget / Cost (3 events)
- `budget_threshold_warning` (Safe)
- `budget_threshold_reached` (Guarded)
- `cost_incurred` (Safe)

### Background jobs (1 event)
- `background_job_state_changed` (Safe)

### Commands / UI (2 events)
- `command_invoked` (Safe)
- `palette_command_executed` (Safe)

### Diagnostics (1 event)
- `unhandled_error` (Safe)

**Totals:** 64 events = 44 Safe + 18 Guarded + 2 Risky

## Host Runtime (PluginHost)

```rust
pub struct PluginHost {
    plugins: Vec<LoadedPlugin>,
    dispatcher: HookDispatcher,
}

struct LoadedPlugin {
    info: PluginInfo,
    instance: Box<dyn Plugin>,
    hook_handler: Option<Box<dyn HookHandler>>,
    tool_provider: Option<Box<dyn ToolProvider>>,
    granted_capabilities: PluginCapabilities,
    status: PluginStatus,
}

impl PluginHost {
    /// Load and handshake a plugin. Returns error if handshake fails.
    pub fn load(&mut self, info: PluginInfo, plugin: Box<dyn Plugin>) -> Result<(), HandshakeError>;

    /// Unload a plugin by ID. Calls shutdown(), deregisters tools and hooks.
    pub fn unload(&mut self, plugin_id: &str) -> bool;

    /// Dispatch a hook event to all subscribed plugins.
    pub fn dispatch_hook(&mut self, event: HookEvent) -> Vec<HookResult>;

    /// List all tools from all loaded plugins (FQN).
    pub fn plugin_tools(&self) -> Vec<(String, &ToolSpec)>;

    /// Invoke a plugin tool by FQN.
    pub fn invoke_plugin_tool(
        &mut self,
        fqn: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, String>;
}

/// Result of dispatching a hook to a single plugin.
pub struct HookResult {
    pub plugin_id: String,
    pub response: HookResponse,
}
```

## Example Plugin (in-process)

```rust
pub struct LoggerPlugin {
    plugin_id: String,
    events_seen: Vec<String>,
}

impl Plugin for LoggerPlugin {
    fn handshake(&self) -> HandshakeRequest {
        HandshakeRequest {
            plugin_id: "org.ucode.logger".into(),
            plugin_version: semver::Version::new(1, 0, 0),
            min_api_version: semver::Version::new(1, 0, 0),
            required_features: [Feature::Hooks].into(),
            capabilities: PluginCapabilities::default(),
        }
    }

    fn initialize(&mut self, _response: &HandshakeResponse) -> Result<(), String> {
        Ok(())
    }

    fn shutdown(&mut self) {}
}

impl HookHandler for LoggerPlugin {
    fn on_event(&mut self, record: &HookRecord) -> HookResponse {
        self.events_seen.push(record.event.event_name().to_string());
        HookResponse::Ok
    }
}
```

## Testing Strategy

- Unit tests for manifest validation (id format, required_features, tool names)
- Unit tests for handshake logic (version compat, feature subset, capability grant)
- Unit tests for all 64 hook events (event_name, override_class)
- Integration test: load example plugin, handshake, dispatch hooks, verify receipt
- Integration test: plugin tool registration, FQN construction, invocation
- Integration test: version mismatch rejection, feature mismatch rejection

## Files to Create/Modify

- `crates/ucode-plugins/src/manifest.rs` -- add `id`, `required_features`, update validation
- `crates/ucode-plugins/src/hooks.rs` -- expand HookEvent from 22 to 64 variants
- `crates/ucode-plugins/src/api.rs` -- new: Plugin/HookHandler/ToolProvider traits, handshake types
- `crates/ucode-plugins/src/host.rs` -- new: PluginHost runtime
- `crates/ucode-plugins/src/lib.rs` -- re-export new modules
- `crates/ucode-plugins/Cargo.toml` -- add `semver` dependency
