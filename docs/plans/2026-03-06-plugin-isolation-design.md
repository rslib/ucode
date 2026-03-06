# Plugin Runtime Isolation Model Design (Task 8.5 / ISSUE 0805)

## Goal

Implement a per-plugin policy profile system for WASM plugins that gates
filesystem, network, process, and hook capabilities. Plugin-originated actions
flow through the normal approval/sandbox/audit pipeline. Untrusted plugins
cannot exceed granted permissions.

## Architecture

```
  Plugin manifest                Host config
  (requested caps)               (allowed caps)
         \                          /
          \                        /
           +--> Handshake negotiation
                       |
                 PluginPolicy
                 (effective = requested ∩ allowed)
                       |
          +------------+------------+
          |            |            |
    Hook dispatch  Tool invoke  WASI preopens
    (category +    (fs/net/    (defense-in-depth
     override      spawn       at WASM boundary)
     ceiling)      checks)
```

Plugin policy is orthogonal to tool policy. When a plugin triggers a tool, both
apply (most-restrictive wins). The bridging happens at the application layer,
not inside `ucode-plugins`.

## Key Design Decisions

### 1. Self-contained PluginPolicy in ucode-plugins

`ucode-plugins` has no dependency on `ucode-tools`. Plugin policy types are
self-contained. The host application bridges plugin policy with tool policy
when a plugin-originated action triggers a tool.

Alternatives considered:
- Plugin as PolicyLayer in existing PolicyStack: wrong abstraction (PolicyStack
  is per-tool, not per-plugin) and creates cross-crate coupling.
- Hybrid (PluginPolicy generates PolicyLayer): over-complex for current needs.

### 2. Enforcement at two layers

- **Host dispatch layer**: PluginHost checks PluginPolicy before dispatching
  hooks, validating responses, and invoking tools.
- **WASM boundary layer**: wasmtime WASI preopens restrict filesystem access
  to allowed paths. Defense-in-depth — even if host checks have a bug, the
  WASM sandbox prevents escape.

### 3. Default policies

- **Native plugins**: all capabilities granted (backward compat; native code
  runs in-process and can bypass host checks anyway).
- **WASM plugins**: workspace-bound read-only filesystem, no network, no
  spawn, no guarded UI, all hook categories, Safe override class only.

### 4. Hook response aggregation

When multiple plugins respond to the same hook event:
- `Veto` wins over `Modify` wins over `Ok`.
- First `Veto` wins (plugin load order).
- `Modify` changes are applied in plugin load order (later plugins see
  earlier modifications).

## Types

### Extended PluginCapabilities (manifest.rs)

Backward-compatible additions to the existing struct:

```rust
pub struct PluginCapabilities {
    // Existing boolean flags
    pub filesystem: bool,
    pub network: bool,
    pub process_spawn: bool,
    pub guarded_ui: bool,
    // New: scoped declarations
    pub filesystem_paths: Vec<String>,
    pub network_domains: Vec<String>,
    pub hook_categories: Vec<String>,
    pub max_override_class: Option<String>,  // "safe" | "guarded" | "risky"
}
```

### PluginPolicy (new: policy.rs)

Per-plugin effective policy, computed at handshake time:

```rust
pub struct PluginPolicy {
    // Filesystem
    pub filesystem_read: bool,
    pub filesystem_write: bool,
    pub allowed_paths: Vec<PathBuf>,
    pub workspace_bound: bool,

    // Network
    pub network_allowed: bool,
    pub network_domain_allowlist: Vec<String>,
    pub network_domain_denylist: Vec<String>,
    pub network_port_allowlist: Vec<u16>,

    // Process
    pub process_spawn: bool,

    // UI
    pub guarded_ui: bool,

    // Hook scope
    pub allowed_hook_categories: HashSet<String>,
    pub max_override_class: OverrideClass,
}
```

### PluginPolicyConfig (new: policy.rs)

Host-side configuration for what plugins are allowed to do:

```rust
pub struct PluginPolicyConfig {
    pub default_wasm: PluginPolicy,
    pub default_native: PluginPolicy,
    pub per_plugin: HashMap<String, PluginPolicy>,  // keyed by plugin_id
}
```

Loaded from workspace config or hardcoded defaults. Per-plugin overrides
take precedence over defaults.

### PolicyCheckResult (new: policy.rs)

```rust
pub enum PolicyCheckResult {
    Allowed,
    Denied { action: String, reason: String },
}
```

## Enforcement Points

### 1. Hook dispatch (PluginHost::dispatch_hook)

Before dispatching a hook event to a plugin:
- Extract the hook category from the event name (e.g., "session" from "session_start").
- Check if the plugin's `allowed_hook_categories` includes this category.
- If not, skip the plugin (do not dispatch).

### 2. Hook response validation (PluginHost::dispatch_hook)

After receiving a HookResponse from a plugin:
- If response is `Modify` and `max_override_class < Guarded`: downgrade to
  `Ok`, log WARN.
- If response is `Veto` and `max_override_class < Risky`: downgrade to `Ok`,
  log WARN.

### 3. Hook response aggregation (PluginHost::dispatch_hook)

After collecting all plugin responses:
- If any plugin returned `Veto`: the aggregate result is `Veto` (first one wins).
- Else if any plugin returned `Modify`: aggregate is `Modify` (changes merged
  in load order).
- Else: aggregate is `Ok`.

Return both individual results (for audit) and the aggregate (for the caller).

### 4. Tool invocation (PluginHost::invoke_plugin_tool)

Before executing a plugin-provided tool:
- Check the plugin's policy for the action type (filesystem, network, spawn).
- This is a pre-check; the actual tool execution also goes through the normal
  approval pipeline.

### 5. WASI preopens (WasmPlugin::create_store)

When creating a wasmtime Store for a WASM plugin:
- Configure WASI context with filesystem preopens restricted to the plugin's
  `allowed_paths` (or workspace root if `workspace_bound` and no specific paths).
- If `network_allowed` is false, do not grant WASI network capability.
- This is defense-in-depth: even if host-level checks have a bug, the WASM
  sandbox prevents escape.

### 6. Handshake negotiation (run_handshake)

Replace the current rubber-stamp logic:
1. Load host-side `PluginPolicyConfig` (per-plugin override or default).
2. Compute effective policy: `requested ∩ host_allowed`.
3. If the plugin's minimum requirements exceed what the host allows, reject
   with `HandshakeError::CapabilityDenied`.
4. Store the computed `PluginPolicy` in `LoadedPlugin`.
5. Return granted capabilities in `HandshakeResponse`.

## Plugin Permission Query API

For UI visibility:

```rust
impl PluginHost {
    pub fn plugin_policy(&self, plugin_id: &str) -> Option<&PluginPolicy>;
    pub fn plugin_policies(&self) -> Vec<(&str, &PluginPolicy)>;
}
```

## Logging

Every policy check emits a structured `tracing` event:
- `tracing::debug!` for allowed actions (high volume, debug only).
- `tracing::warn!` for denied actions with plugin_id, action, reason.
- `tracing::info!` at plugin load time with the effective PluginPolicy.

`PluginPolicy` derives `Serialize` + `Debug` for structured logging and UI
display.

## Files Touched

- `crates/ucode-plugins/src/manifest.rs` — extend PluginCapabilities
- `crates/ucode-plugins/src/policy.rs` — new: PluginPolicy, PluginPolicyConfig, enforcement helpers
- `crates/ucode-plugins/src/host.rs` — add PluginPolicy to LoadedPlugin, enforce in dispatch/invoke, add query API, hook response aggregation
- `crates/ucode-plugins/src/api.rs` — update HandshakeResponse to carry scoped granted info
- `crates/ucode-plugins/src/wasm/host.rs` — WASI preopens configuration
- `crates/ucode-plugins/src/hooks.rs` — add hook_category() helper to HookEvent
- `crates/ucode-plugins/src/lib.rs` — add pub mod policy
- `crates/ucode-plugins/Cargo.toml` — add tracing dependency

## WASM Resource Limits

Wasmtime provides two mechanisms for bounding plugin resource consumption:

1. **Memory limits** via `StoreLimitsBuilder::memory_size(bytes)` — caps linear
   memory growth per instance. Configured on the `Store` via `store.limiter()`.
2. **Fuel metering** via `Config::consume_fuel(true)` + `Store::set_fuel(n)` —
   instruction-level CPU budget. When fuel runs out, wasmtime raises a trap.

New type in `PluginPolicy`:

```rust
pub struct ResourceLimits {
    /// Maximum linear memory in bytes (default: 16 MiB).
    pub max_memory_bytes: usize,
    /// Maximum fuel (instruction budget) per hook dispatch (default: 1_000_000).
    pub max_fuel: u64,
    /// Maximum number of WASM instances (default: 1).
    pub max_instances: usize,
}
```

Enforcement:
- Engine config: `config.consume_fuel(true)` when any plugin has fuel limits.
- Store creation: `store.limiter(|state| &mut state.limits)` with
  `StoreLimitsBuilder::memory_size(policy.resource_limits.max_memory_bytes)`.
- Before each dispatch: `store.set_fuel(policy.resource_limits.max_fuel)`.
- Trap on fuel exhaustion is caught and converted to a `HookResponse::Ok` with
  a warning log.

## Dynamic Policy Hot-Reload

Plugin policies can be updated at runtime without restarting the host:

1. **Config file**: `PluginPolicyConfig` serialized as TOML. Loaded from
   workspace config directory.
2. **Reload method**: `PluginHost::reload_policy_config(config)` updates
   host-level checks immediately for all loaded plugins.
3. **WASI preopens caveat**: WASI preopens are set at Store creation time.
   Changed filesystem paths only take effect on next plugin restart (Store
   recreation). Host-level checks update immediately.
4. **Trigger**: The host application calls `reload_policy_config()` on SIGHUP,
   CLI command, or config file change. The file watcher is the caller's
   responsibility — `ucode-plugins` only provides the reload method.

## Plugin-to-Plugin Communication Policy

Controls whether plugins can observe each other's hook responses:

```rust
pub enum PluginIsolationLevel {
    /// Plugin sees only the original event payload. No visibility into
    /// other plugins' responses.
    Full,
    /// Plugin sees the event payload as modified by prior plugins in
    /// load order. Later plugins see earlier Modify results.
    Ordered,
}
```

Default: `Full` for WASM plugins, `Ordered` for native plugins.

When `Ordered`:
- `dispatch_hook()` applies each plugin's `Modify` response to the
  `HookRecord` before passing it to the next plugin.
- This enables plugin pipelines (e.g., plugin A normalizes args, plugin B
  validates them).

When `Full`:
- Each plugin receives the original, unmodified `HookRecord`.
- Responses are aggregated after all plugins have run.

## Signed Plugin Verification

WASM plugin binaries can be signed with Ed25519 for authenticity verification:

1. **Signature format**: Detached `.wasm.sig` file alongside the `.wasm` file.
   Contains a 64-byte Ed25519 signature over the raw `.wasm` bytes.
2. **Key management**: Trusted public keys stored in `PluginPolicyConfig` as
   hex-encoded 32-byte Ed25519 verifying keys.
3. **Verification policy**:

```rust
pub enum SignaturePolicy {
    /// Reject unsigned or invalid-signature plugins.
    Required,
    /// Warn on unsigned plugins, reject invalid signatures.
    WarnUnsigned,
    /// Skip signature verification entirely.
    Disabled,
}
```

4. **Enforcement**: At `load_wasm()` time, before instantiation:
   - Read `{path}.sig` file.
   - Verify signature against all trusted keys.
   - Apply policy (reject/warn/skip).
5. **Dependency**: `ed25519-dalek` crate (optional, behind `signed-plugins`
   feature flag).

## Files Touched

- `crates/ucode-plugins/src/manifest.rs` — extend PluginCapabilities
- `crates/ucode-plugins/src/policy.rs` — new: PluginPolicy, PluginPolicyConfig, ResourceLimits, PluginIsolationLevel, SignaturePolicy, enforcement helpers
- `crates/ucode-plugins/src/host.rs` — add PluginPolicy to LoadedPlugin, enforce in dispatch/invoke, add query API, hook response aggregation, reload, ordered dispatch
- `crates/ucode-plugins/src/api.rs` — update HandshakeResponse to carry scoped granted info
- `crates/ucode-plugins/src/wasm/host.rs` — WASI preopens, resource limits, Store config
- `crates/ucode-plugins/src/wasm/signature.rs` — new: Ed25519 signature verification
- `crates/ucode-plugins/src/hooks.rs` — add hook_category() helper to HookEvent
- `crates/ucode-plugins/src/lib.rs` — add pub mod policy
- `crates/ucode-plugins/Cargo.toml` — add tracing, ed25519-dalek (optional) dependencies
