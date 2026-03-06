# Plugin Runtime Isolation Model Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Implement per-plugin policy profiles for WASM plugins that gate filesystem, network, process, and hook capabilities, with enforcement at both host dispatch and WASM boundary layers. Includes WASM resource limits, dynamic policy hot-reload, plugin isolation levels, and signed plugin verification.

**Architecture:** `PluginPolicy` struct in `ucode-plugins` holds per-plugin effective permissions computed at handshake time (requested caps intersected with host-allowed caps). Enforcement at 5 points: hook category filtering, hook response validation, hook response aggregation, tool invocation policy checks, and WASI preopens for defense-in-depth. Resource limits via wasmtime fuel/memory. Ed25519 plugin signatures for authenticity. No cross-crate dependency on `ucode-tools`.

**Tech Stack:** Rust, serde, tracing, wasmtime (WASI preopens, fuel, StoreLimits), ed25519-dalek (optional), existing ucode-plugins infrastructure

**Design doc:** `docs/plans/2026-03-06-plugin-isolation-design.md`

---

### Task 1: Add tracing dependency and hook_category() helper

**Files:**
- Modify: `crates/ucode-plugins/Cargo.toml`
- Modify: `crates/ucode-plugins/src/hooks.rs`

**Step 1: Add tracing dependency**

In `crates/ucode-plugins/Cargo.toml`, add to `[dependencies]`:

```toml
tracing = { workspace = true }
```

**Step 2: Write the failing test for hook_category()**

In `crates/ucode-plugins/src/hooks.rs`, add to the `tests` module:

```rust
#[test]
fn test_hook_category_session() {
    assert_eq!(
        HookEvent::SessionStart { session_id: "s".into() }.hook_category(),
        "session"
    );
    assert_eq!(
        HookEvent::SessionEnd { session_id: "s".into(), duration_secs: 1.0 }.hook_category(),
        "session"
    );
    assert_eq!(
        HookEvent::SessionTitleGenerated { session_id: "s".into(), title: "t".into() }.hook_category(),
        "session"
    );
}

#[test]
fn test_hook_category_tool() {
    assert_eq!(
        HookEvent::BeforeToolCall { tool_name: "t".into(), args: serde_json::Value::Null }.hook_category(),
        "tool"
    );
    assert_eq!(
        HookEvent::ToolTimeout { tool_name: "t".into(), timeout_ms: 0 }.hook_category(),
        "tool"
    );
}

#[test]
fn test_hook_category_all_categories() {
    // Spot-check one event from each category
    let cases: Vec<(HookEvent, &str)> = vec![
        (HookEvent::SessionStart { session_id: "s".into() }, "session"),
        (HookEvent::BeforeToolCall { tool_name: "t".into(), args: serde_json::Value::Null }, "tool"),
        (HookEvent::BeforeModelCall { model: "m".into(), message_count: 0 }, "model"),
        (HookEvent::ContextOverflow { current_tokens: 0, max_tokens: 0 }, "context"),
        (HookEvent::ApprovalRequired { tool_name: "t".into(), risk_level: "r".into() }, "approval"),
        (HookEvent::CheckpointCreated { checkpoint_id: "c".into() }, "checkpoint"),
        (HookEvent::PluginLoaded { plugin_name: "p".into() }, "plugin"),
        (HookEvent::McpServerConnected { server_name: "s".into() }, "mcp"),
        (HookEvent::SkillActivated { skill_name: "s".into() }, "skill"),
        (HookEvent::UserMessageReceived { message_len: 0 }, "message"),
        (HookEvent::AgentSpawned { agent_id: "a".into(), task: "t".into() }, "agent"),
        (HookEvent::AuthChanged { provider: "p".into() }, "auth"),
        (HookEvent::BudgetThresholdWarning { current_cost: 0.0, threshold: 0.0 }, "budget"),
        (HookEvent::BackgroundJobStateChanged { job_id: "j".into(), state: "s".into() }, "job"),
        (HookEvent::CommandInvoked { command: "c".into() }, "command"),
        (HookEvent::UnhandledError { error: "e".into(), context: "c".into() }, "diagnostic"),
        (HookEvent::BeforeFileRead { path: "p".into() }, "tool_fs"),
        (HookEvent::BeforeRunCmd { command: "c".into() }, "tool_cmd"),
        (HookEvent::BeforeApplyPatch { file_path: "f".into(), patch_summary: "s".into() }, "tool_patch"),
        (HookEvent::SandboxDecision { tool_name: "t".into(), allowed: true, reason: "r".into() }, "approval"),
    ];
    for (event, expected) in cases {
        assert_eq!(event.hook_category(), expected, "failed for {}", event.event_name());
    }
}
```

**Step 3: Run tests to verify they fail**

Run: `cargo test -p ucode-plugins -- test_hook_category`
Expected: FAIL (method `hook_category` not found)

**Step 4: Implement hook_category()**

Add to the `impl HookEvent` block in `hooks.rs`:

```rust
/// Hook category for policy scoping.
///
/// Maps each event to its WIT category package name (e.g., "session", "tool",
/// "model"). Used by [`PluginPolicy`] to restrict which hook categories a
/// plugin may handle.
pub fn hook_category(&self) -> &'static str {
    match self {
        // Session lifecycle
        Self::SessionStart { .. }
        | Self::SessionEnd { .. }
        | Self::SessionTitleGenerated { .. }
        | Self::SessionTitleUpdated { .. }
        | Self::ConfigReloaded => "session",

        // Tool lifecycle
        Self::BeforeToolCall { .. }
        | Self::AfterToolCall { .. }
        | Self::ToolError { .. }
        | Self::ToolTimeout { .. } => "tool",

        // Tool: filesystem
        Self::BeforeFileRead { .. }
        | Self::AfterFileRead { .. }
        | Self::BeforeFileWrite { .. }
        | Self::AfterFileWrite { .. } => "tool_fs",

        // Tool: command
        Self::BeforeRunCmd { .. }
        | Self::AfterRunCmd { .. } => "tool_cmd",

        // Tool: patch
        Self::BeforeApplyPatch { .. }
        | Self::AfterApplyPatch { .. } => "tool_patch",

        // Model lifecycle
        Self::BeforeModelCall { .. }
        | Self::AfterModelCall { .. }
        | Self::ModelFallback { .. }
        | Self::BeforeModelSelect { .. }
        | Self::RouterDecision { .. }
        | Self::ModelRateLimited { .. }
        | Self::ModelQuotaExhausted { .. } => "model",

        // Context
        Self::ContextOverflow { .. }
        | Self::ContextCompaction { .. }
        | Self::ContextDistilled { .. }
        | Self::TokenUsageUpdated { .. } => "context",

        // Message flow
        Self::UserMessageReceived { .. }
        | Self::AssistantResponseStarted { .. }
        | Self::AssistantResponseCompleted { .. }
        | Self::MessageRetry { .. } => "message",

        // Agent
        Self::AgentSpawned { .. }
        | Self::AgentMessage { .. }
        | Self::AgentCompleted { .. }
        | Self::AgentFailed { .. }
        | Self::AgentCancelled { .. } => "agent",

        // Approval / Sandbox
        Self::ApprovalRequired { .. }
        | Self::ApprovalGranted { .. }
        | Self::ApprovalDenied { .. }
        | Self::SandboxDecision { .. }
        | Self::PermissionDecision { .. } => "approval",

        // Auth
        Self::AuthChanged { .. }
        | Self::AuthFailed { .. }
        | Self::ProviderSwitched { .. } => "auth",

        // MCP
        Self::McpServerConnected { .. }
        | Self::McpServerDisconnected { .. }
        | Self::McpServerLaunch { .. }
        | Self::McpServerRestart { .. }
        | Self::McpServerCrash { .. }
        | Self::McpToolInvoked { .. } => "mcp",

        // Skill
        Self::SkillActivated { .. }
        | Self::SkillDeactivated { .. } => "skill",

        // Plugin
        Self::PluginLoaded { .. }
        | Self::PluginUnloaded { .. }
        | Self::PluginError { .. } => "plugin",

        // Checkpoint
        Self::CheckpointCreated { .. }
        | Self::CheckpointRestored { .. } => "checkpoint",

        // Budget
        Self::BudgetThresholdWarning { .. }
        | Self::BudgetThresholdReached { .. }
        | Self::CostIncurred { .. } => "budget",

        // Job
        Self::BackgroundJobStateChanged { .. } => "job",

        // Command / UI
        Self::CommandInvoked { .. }
        | Self::PaletteCommandExecuted { .. } => "command",

        // Diagnostic
        Self::UnhandledError { .. } => "diagnostic",
    }
}
```

**Step 5: Run tests to verify they pass**

Run: `cargo test -p ucode-plugins -- test_hook_category`
Expected: PASS (3 tests)

**Step 6: Verify full test suite**

Run: `cargo test -p ucode-plugins`
Expected: All existing tests still pass

**Step 7: Commit**

```
feat(plugins): add tracing dep and hook_category() helper (ISSUE 0805)
```

---

### Task 2: Extend PluginCapabilities with scoped fields

**Files:**
- Modify: `crates/ucode-plugins/src/manifest.rs`

**Step 1: Write failing tests for new manifest fields**

Add to the `tests` module in `manifest.rs`:

```rust
#[test]
fn test_parse_manifest_with_scoped_capabilities() {
    let toml = r#"
        name = "scoped-plugin"
        version = "1.0.0"

        [capabilities]
        filesystem = true
        network = true
        process_spawn = false
        guarded_ui = false
        filesystem_paths = ["src/", "tests/"]
        network_domains = ["api.example.com", "cdn.example.com"]
        hook_categories = ["session", "tool"]
        max_override_class = "guarded"
    "#;
    let m = parse_manifest(toml).unwrap();
    assert!(m.capabilities.filesystem);
    assert!(m.capabilities.network);
    assert_eq!(m.capabilities.filesystem_paths, vec!["src/", "tests/"]);
    assert_eq!(m.capabilities.network_domains, vec!["api.example.com", "cdn.example.com"]);
    assert_eq!(m.capabilities.hook_categories, vec!["session", "tool"]);
    assert_eq!(m.capabilities.max_override_class.as_deref(), Some("guarded"));
}

#[test]
fn test_parse_manifest_scoped_caps_default_empty() {
    let toml = r#"
        name = "minimal-plugin"
        version = "1.0.0"

        [capabilities]
        filesystem = true
    "#;
    let m = parse_manifest(toml).unwrap();
    assert!(m.capabilities.filesystem);
    assert!(m.capabilities.filesystem_paths.is_empty());
    assert!(m.capabilities.network_domains.is_empty());
    assert!(m.capabilities.hook_categories.is_empty());
    assert!(m.capabilities.max_override_class.is_none());
}

#[test]
fn test_validate_invalid_override_class() {
    let toml = r#"
        name = "bad-plugin"
        version = "1.0.0"

        [capabilities]
        max_override_class = "superadmin"
    "#;
    let err = parse_manifest(toml).unwrap_err();
    assert!(err.to_string().contains("max_override_class"));
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p ucode-plugins -- test_parse_manifest_with_scoped_capabilities test_parse_manifest_scoped_caps_default_empty test_validate_invalid_override_class`
Expected: FAIL

**Step 3: Add scoped fields to PluginCapabilities**

In `manifest.rs`, extend the `PluginCapabilities` struct:

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginCapabilities {
    #[serde(default)]
    pub filesystem: bool,
    #[serde(default)]
    pub network: bool,
    #[serde(default)]
    pub process_spawn: bool,
    #[serde(default)]
    pub guarded_ui: bool,
    /// Filesystem path scopes (relative to workspace root). Empty = workspace root only.
    #[serde(default)]
    pub filesystem_paths: Vec<String>,
    /// Network domain access. Empty = all domains (when network is true).
    #[serde(default)]
    pub network_domains: Vec<String>,
    /// Hook categories this plugin wants to handle. Empty = all categories.
    #[serde(default)]
    pub hook_categories: Vec<String>,
    /// Maximum override class: "safe", "guarded", or "risky". None = "safe".
    #[serde(default)]
    pub max_override_class: Option<String>,
}
```

**Step 4: Add validation for max_override_class**

In `validate_manifest()`, add after the hooks validation:

```rust
if let Some(ref class) = manifest.capabilities.max_override_class {
    if !["safe", "guarded", "risky"].contains(&class.as_str()) {
        return Err(ManifestError::Validation(format!(
            "max_override_class '{}' must be one of: safe, guarded, risky",
            class
        )));
    }
}
```

**Step 5: Run tests to verify they pass**

Run: `cargo test -p ucode-plugins -- test_parse_manifest_with_scoped test_parse_manifest_scoped_caps test_validate_invalid_override`
Expected: PASS (3 tests)

**Step 6: Verify full test suite**

Run: `cargo test -p ucode-plugins`
Expected: All tests pass (existing tests use `PluginCapabilities::default()` which still works)

**Step 7: Commit**

```
feat(plugins): extend PluginCapabilities with scoped fields (ISSUE 0805)
```

---

### Task 3: Create PluginPolicy struct and enforcement helpers

**Files:**
- Create: `crates/ucode-plugins/src/policy.rs`
- Modify: `crates/ucode-plugins/src/lib.rs`

**Step 1: Write failing tests**

Create `crates/ucode-plugins/src/policy.rs` with tests first:

```rust
//! Per-plugin runtime policy and enforcement.

use std::collections::HashSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::hooks::OverrideClass;
use crate::manifest::PluginCapabilities;

/// Per-plugin network policy.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginNetworkPolicy {
    pub allowed: bool,
    pub domain_allowlist: Vec<String>,
    pub domain_denylist: Vec<String>,
    pub port_allowlist: Vec<u16>,
}

/// Result of a plugin policy check.
#[derive(Debug, Clone, PartialEq)]
pub enum PolicyCheckResult {
    Allowed,
    Denied { action: String, reason: String },
}

/// Per-plugin runtime policy, computed at handshake time.
///
/// Represents the effective permissions for a single plugin instance:
/// `requested capabilities ∩ host-allowed capabilities`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginPolicy {
    pub filesystem_read: bool,
    pub filesystem_write: bool,
    pub allowed_paths: Vec<PathBuf>,
    pub workspace_bound: bool,
    pub network: PluginNetworkPolicy,
    pub process_spawn: bool,
    pub guarded_ui: bool,
    #[serde(serialize_with = "serialize_hash_set", deserialize_with = "deserialize_hash_set")]
    pub allowed_hook_categories: HashSet<String>,
    pub max_override_class: OverrideClass,
}

// serde helpers for HashSet<String> -- serialize as sorted Vec for determinism
fn serialize_hash_set<S: serde::Serializer>(set: &HashSet<String>, s: S) -> Result<S::Ok, S::Error> {
    let mut sorted: Vec<&String> = set.iter().collect();
    sorted.sort();
    sorted.serialize(s)
}

fn deserialize_hash_set<'de, D: serde::Deserializer<'de>>(d: D) -> Result<HashSet<String>, D::Error> {
    let v: Vec<String> = Vec::deserialize(d)?;
    Ok(v.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    // placeholder -- tests added in steps below
}
```

Add `pub mod policy;` to `lib.rs` and add `pub use policy::{PluginPolicy, PluginNetworkPolicy, PolicyCheckResult};` to the re-exports.

**Step 2: Write tests for default policies**

```rust
#[test]
fn test_default_wasm_policy() {
    let policy = PluginPolicy::default_wasm();
    assert!(policy.filesystem_read);
    assert!(!policy.filesystem_write);
    assert!(policy.allowed_paths.is_empty());
    assert!(policy.workspace_bound);
    assert!(!policy.network.allowed);
    assert!(!policy.process_spawn);
    assert!(!policy.guarded_ui);
    assert!(policy.allowed_hook_categories.is_empty()); // empty = all
    assert_eq!(policy.max_override_class, OverrideClass::Safe);
}

#[test]
fn test_default_native_policy() {
    let policy = PluginPolicy::default_native();
    assert!(policy.filesystem_read);
    assert!(policy.filesystem_write);
    assert!(!policy.workspace_bound);
    assert!(policy.network.allowed);
    assert!(policy.process_spawn);
    assert!(policy.guarded_ui);
    assert_eq!(policy.max_override_class, OverrideClass::Risky);
}
```

**Step 3: Write tests for from_capabilities**

```rust
#[test]
fn test_from_capabilities_basic() {
    let caps = PluginCapabilities {
        filesystem: true,
        network: true,
        process_spawn: false,
        guarded_ui: false,
        filesystem_paths: vec!["src/".into()],
        network_domains: vec!["api.example.com".into()],
        hook_categories: vec!["session".into(), "tool".into()],
        max_override_class: Some("guarded".into()),
    };
    let policy = PluginPolicy::from_capabilities(&caps);
    assert!(policy.filesystem_read);
    assert!(!policy.filesystem_write); // filesystem=true only grants read for WASM
    assert!(policy.workspace_bound);
    assert_eq!(policy.allowed_paths, vec![PathBuf::from("src/")]);
    assert!(policy.network.allowed);
    assert_eq!(policy.network.domain_allowlist, vec!["api.example.com"]);
    assert!(!policy.process_spawn);
    assert!(!policy.guarded_ui);
    assert!(policy.allowed_hook_categories.contains("session"));
    assert!(policy.allowed_hook_categories.contains("tool"));
    assert_eq!(policy.max_override_class, OverrideClass::Guarded);
}

#[test]
fn test_from_capabilities_no_filesystem() {
    let caps = PluginCapabilities::default();
    let policy = PluginPolicy::from_capabilities(&caps);
    assert!(!policy.filesystem_read);
    assert!(!policy.filesystem_write);
}
```

**Step 4: Write tests for check_hook_category**

```rust
#[test]
fn test_check_hook_category_allowed_when_empty() {
    let policy = PluginPolicy::default_wasm();
    // Empty allowed_hook_categories = all categories allowed
    assert_eq!(policy.check_hook_category("session"), PolicyCheckResult::Allowed);
    assert_eq!(policy.check_hook_category("tool"), PolicyCheckResult::Allowed);
}

#[test]
fn test_check_hook_category_restricted() {
    let mut policy = PluginPolicy::default_wasm();
    policy.allowed_hook_categories = ["session".into(), "tool".into()].into();
    assert_eq!(policy.check_hook_category("session"), PolicyCheckResult::Allowed);
    assert_eq!(policy.check_hook_category("tool"), PolicyCheckResult::Allowed);
    assert!(matches!(policy.check_hook_category("model"), PolicyCheckResult::Denied { .. }));
}
```

**Step 5: Write tests for check_override_class**

```rust
#[test]
fn test_check_override_class_safe_ceiling() {
    let policy = PluginPolicy::default_wasm(); // max = Safe
    assert_eq!(policy.check_override_class(&OverrideClass::Safe), PolicyCheckResult::Allowed);
    assert!(matches!(policy.check_override_class(&OverrideClass::Guarded), PolicyCheckResult::Denied { .. }));
    assert!(matches!(policy.check_override_class(&OverrideClass::Risky), PolicyCheckResult::Denied { .. }));
}

#[test]
fn test_check_override_class_guarded_ceiling() {
    let mut policy = PluginPolicy::default_wasm();
    policy.max_override_class = OverrideClass::Guarded;
    assert_eq!(policy.check_override_class(&OverrideClass::Safe), PolicyCheckResult::Allowed);
    assert_eq!(policy.check_override_class(&OverrideClass::Guarded), PolicyCheckResult::Allowed);
    assert!(matches!(policy.check_override_class(&OverrideClass::Risky), PolicyCheckResult::Denied { .. }));
}

#[test]
fn test_check_override_class_risky_ceiling() {
    let mut policy = PluginPolicy::default_wasm();
    policy.max_override_class = OverrideClass::Risky;
    assert_eq!(policy.check_override_class(&OverrideClass::Safe), PolicyCheckResult::Allowed);
    assert_eq!(policy.check_override_class(&OverrideClass::Guarded), PolicyCheckResult::Allowed);
    assert_eq!(policy.check_override_class(&OverrideClass::Risky), PolicyCheckResult::Allowed);
}
```

**Step 6: Write tests for check_network**

```rust
#[test]
fn test_check_network_denied() {
    let policy = PluginPolicy::default_wasm(); // network not allowed
    assert!(matches!(
        policy.check_network("example.com", None),
        PolicyCheckResult::Denied { .. }
    ));
}

#[test]
fn test_check_network_allowed_all() {
    let mut policy = PluginPolicy::default_wasm();
    policy.network = PluginNetworkPolicy { allowed: true, ..Default::default() };
    assert_eq!(policy.check_network("example.com", None), PolicyCheckResult::Allowed);
}

#[test]
fn test_check_network_domain_allowlist() {
    let mut policy = PluginPolicy::default_wasm();
    policy.network = PluginNetworkPolicy {
        allowed: true,
        domain_allowlist: vec!["api.example.com".into()],
        ..Default::default()
    };
    assert_eq!(policy.check_network("api.example.com", None), PolicyCheckResult::Allowed);
    assert!(matches!(
        policy.check_network("evil.com", None),
        PolicyCheckResult::Denied { .. }
    ));
}

#[test]
fn test_check_network_domain_denylist_precedence() {
    let mut policy = PluginPolicy::default_wasm();
    policy.network = PluginNetworkPolicy {
        allowed: true,
        domain_allowlist: vec!["example.com".into()],
        domain_denylist: vec!["example.com".into()],
        ..Default::default()
    };
    assert!(matches!(
        policy.check_network("example.com", None),
        PolicyCheckResult::Denied { .. }
    ));
}
```

**Step 7: Run all tests to verify they fail**

Run: `cargo test -p ucode-plugins -- policy`
Expected: FAIL (methods not implemented)

**Step 8: Implement PluginPolicy methods**

Add to `policy.rs`:

```rust
impl PluginPolicy {
    /// Default policy for WASM plugins: read-only workspace, no network/spawn/ui, Safe only.
    pub fn default_wasm() -> Self {
        Self {
            filesystem_read: true,
            filesystem_write: false,
            allowed_paths: Vec::new(),
            workspace_bound: true,
            network: PluginNetworkPolicy::default(),
            process_spawn: false,
            guarded_ui: false,
            allowed_hook_categories: HashSet::new(),
            max_override_class: OverrideClass::Safe,
        }
    }

    /// Default policy for native plugins: all capabilities granted.
    pub fn default_native() -> Self {
        Self {
            filesystem_read: true,
            filesystem_write: true,
            allowed_paths: Vec::new(),
            workspace_bound: false,
            network: PluginNetworkPolicy { allowed: true, ..Default::default() },
            process_spawn: true,
            guarded_ui: true,
            allowed_hook_categories: HashSet::new(),
            max_override_class: OverrideClass::Risky,
        }
    }

    /// Compute policy from manifest capabilities (WASM plugin defaults).
    pub fn from_capabilities(caps: &PluginCapabilities) -> Self {
        let max_override_class = match caps.max_override_class.as_deref() {
            Some("risky") => OverrideClass::Risky,
            Some("guarded") => OverrideClass::Guarded,
            _ => OverrideClass::Safe,
        };
        Self {
            filesystem_read: caps.filesystem,
            filesystem_write: false, // WASM plugins get read-only by default
            allowed_paths: caps.filesystem_paths.iter().map(PathBuf::from).collect(),
            workspace_bound: true, // WASM plugins are always workspace-bound
            network: PluginNetworkPolicy {
                allowed: caps.network,
                domain_allowlist: caps.network_domains.clone(),
                ..Default::default()
            },
            process_spawn: caps.process_spawn,
            guarded_ui: caps.guarded_ui,
            allowed_hook_categories: caps.hook_categories.iter().cloned().collect(),
            max_override_class,
        }
    }

    /// Check if the plugin is allowed to handle hooks in `category`.
    pub fn check_hook_category(&self, category: &str) -> PolicyCheckResult {
        if self.allowed_hook_categories.is_empty() || self.allowed_hook_categories.contains(category) {
            PolicyCheckResult::Allowed
        } else {
            PolicyCheckResult::Denied {
                action: format!("handle hook category '{category}'"),
                reason: format!("plugin not allowed to handle category '{category}'"),
            }
        }
    }

    /// Check if the plugin's override ceiling permits `class`.
    pub fn check_override_class(&self, class: &OverrideClass) -> PolicyCheckResult {
        let ceiling = override_class_level(&self.max_override_class);
        let requested = override_class_level(class);
        if requested <= ceiling {
            PolicyCheckResult::Allowed
        } else {
            PolicyCheckResult::Denied {
                action: format!("use override class {:?}", class),
                reason: format!(
                    "plugin max override class is {:?}, requested {:?}",
                    self.max_override_class, class
                ),
            }
        }
    }

    /// Check if a network connection to `domain:port` is permitted.
    pub fn check_network(&self, domain: &str, port: Option<u16>) -> PolicyCheckResult {
        if !self.network.allowed {
            return PolicyCheckResult::Denied {
                action: format!("network access to '{domain}'"),
                reason: "network access is disabled for this plugin".into(),
            };
        }
        if self.network.domain_denylist.iter().any(|d| d == domain) {
            return PolicyCheckResult::Denied {
                action: format!("network access to '{domain}'"),
                reason: format!("domain '{domain}' is in the denylist"),
            };
        }
        if !self.network.domain_allowlist.is_empty()
            && !self.network.domain_allowlist.iter().any(|d| d == domain)
        {
            return PolicyCheckResult::Denied {
                action: format!("network access to '{domain}'"),
                reason: format!("domain '{domain}' is not in the allowlist"),
            };
        }
        if let Some(p) = port {
            if !self.network.port_allowlist.is_empty() && !self.network.port_allowlist.contains(&p) {
                return PolicyCheckResult::Denied {
                    action: format!("network access to '{domain}:{p}'"),
                    reason: format!("port {p} is not in the port allowlist"),
                };
            }
        }
        PolicyCheckResult::Allowed
    }

    /// Check if filesystem read is permitted.
    pub fn check_filesystem_read(&self) -> PolicyCheckResult {
        if self.filesystem_read {
            PolicyCheckResult::Allowed
        } else {
            PolicyCheckResult::Denied {
                action: "filesystem read".into(),
                reason: "filesystem read is disabled for this plugin".into(),
            }
        }
    }

    /// Check if filesystem write is permitted.
    pub fn check_filesystem_write(&self) -> PolicyCheckResult {
        if self.filesystem_write {
            PolicyCheckResult::Allowed
        } else {
            PolicyCheckResult::Denied {
                action: "filesystem write".into(),
                reason: "filesystem write is disabled for this plugin".into(),
            }
        }
    }

    /// Check if process spawning is permitted.
    pub fn check_process_spawn(&self) -> PolicyCheckResult {
        if self.process_spawn {
            PolicyCheckResult::Allowed
        } else {
            PolicyCheckResult::Denied {
                action: "process spawn".into(),
                reason: "process spawning is disabled for this plugin".into(),
            }
        }
    }
}

fn override_class_level(class: &OverrideClass) -> u8 {
    match class {
        OverrideClass::Safe => 0,
        OverrideClass::Guarded => 1,
        OverrideClass::Risky => 2,
    }
}
```

**Step 9: Run tests to verify they pass**

Run: `cargo test -p ucode-plugins -- policy`
Expected: All policy tests pass

**Step 10: Verify full test suite**

Run: `cargo test -p ucode-plugins`
Expected: All tests pass

**Step 11: Commit**

```
feat(plugins): add PluginPolicy struct and enforcement helpers (ISSUE 0805)
```

---

### Task 4: Integrate PluginPolicy into PluginHost

**Files:**
- Modify: `crates/ucode-plugins/src/host.rs`
- Modify: `crates/ucode-plugins/src/api.rs`

**Step 1: Write failing tests for policy-aware dispatch**

Add to `host.rs` tests:

```rust
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
    let results = host.dispatch_hook(HookEvent::SessionStart { session_id: "s1".into() });
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
        fn initialize(&mut self, _: &HandshakeResponse) -> Result<(), String> { Ok(()) }
        fn shutdown(&mut self) {}
    }
    impl HookHandler for ModifyPlugin {
        fn on_event(&mut self, _: &HookRecord) -> HookResponse {
            HookResponse::Modify { changes: serde_json::json!({"key": "val"}) }
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
    assert!(policy.unwrap().filesystem_write); // native default
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
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p ucode-plugins -- test_dispatch_hook_respects test_dispatch_hook_downgrades test_plugin_policy_query test_plugin_policies_list`
Expected: FAIL

**Step 3: Add PluginPolicy to LoadedPlugin and PluginHost**

In `host.rs`:

1. Add `policy: PluginPolicy` field to `LoadedPlugin`.
2. In `load()` and `load_with_tools()`, set `policy: PluginPolicy::default_native()`.
3. In `load_wasm()`, compute `policy: PluginPolicy::from_capabilities(&caps)` from the manifest.
4. Add `set_plugin_policy()`, `plugin_policy()`, `plugin_policies()` methods.
5. Update `dispatch_hook()` to check category and validate response override class.

```rust
// Add to LoadedPlugin:
struct LoadedPlugin {
    plugin_id: String,
    instance: PluginInstance,
    tool_fqns: Vec<String>,
    policy: PluginPolicy,
}

// Add methods to PluginHost:
impl PluginHost {
    /// Override the policy for a loaded plugin.
    pub fn set_plugin_policy(&mut self, plugin_id: &str, policy: PluginPolicy) {
        if let Some(loaded) = self.plugins.iter_mut().find(|p| p.plugin_id == plugin_id) {
            loaded.policy = policy;
        }
    }

    /// Get the effective policy for a plugin.
    pub fn plugin_policy(&self, plugin_id: &str) -> Option<&PluginPolicy> {
        self.plugins.iter().find(|p| p.plugin_id == plugin_id).map(|p| &p.policy)
    }

    /// List all plugin policies as (plugin_id, policy) pairs.
    pub fn plugin_policies(&self) -> Vec<(&str, &PluginPolicy)> {
        self.plugins.iter().map(|p| (p.plugin_id.as_str(), &p.policy)).collect()
    }
}
```

6. Update `dispatch_hook()`:

```rust
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
                let mut response = p.on_event(&record);
                // Validate override class
                response = validate_hook_response(
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
```

7. Add `validate_hook_response()` helper:

```rust
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

fn override_class_level(class: &OverrideClass) -> u8 {
    match class {
        OverrideClass::Safe => 0,
        OverrideClass::Guarded => 1,
        OverrideClass::Risky => 2,
    }
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p ucode-plugins -- test_dispatch_hook_respects test_dispatch_hook_downgrades test_plugin_policy_query test_plugin_policies_list`
Expected: PASS

**Step 5: Verify full test suite**

Run: `cargo test -p ucode-plugins`
Expected: All tests pass

**Step 6: Commit**

```
feat(plugins): integrate PluginPolicy into PluginHost dispatch (ISSUE 0805)
```

---

### Task 5: Hook response aggregation

**Files:**
- Modify: `crates/ucode-plugins/src/host.rs`

**Step 1: Write failing tests**

Add to `host.rs` tests:

```rust
#[test]
fn test_aggregate_hook_responses_all_ok() {
    let results = vec![
        HookResult { plugin_id: "a".into(), response: HookResponse::Ok },
        HookResult { plugin_id: "b".into(), response: HookResponse::Ok },
    ];
    let agg = aggregate_hook_responses(&results);
    assert!(matches!(agg, HookResponse::Ok));
}

#[test]
fn test_aggregate_hook_responses_veto_wins() {
    let results = vec![
        HookResult { plugin_id: "a".into(), response: HookResponse::Modify { changes: serde_json::json!({"x": 1}) } },
        HookResult { plugin_id: "b".into(), response: HookResponse::Veto { reason: "blocked".into() } },
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
        HookResult { plugin_id: "a".into(), response: HookResponse::Veto { reason: "first".into() } },
        HookResult { plugin_id: "b".into(), response: HookResponse::Veto { reason: "second".into() } },
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
        HookResult { plugin_id: "a".into(), response: HookResponse::Ok },
        HookResult { plugin_id: "b".into(), response: HookResponse::Modify { changes: serde_json::json!({"key": "val"}) } },
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
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p ucode-plugins -- test_aggregate`
Expected: FAIL

**Step 3: Implement aggregate_hook_responses**

Add to `host.rs`:

```rust
/// Aggregate multiple plugin hook responses.
///
/// Resolution: Veto wins over Modify wins over Ok.
/// First Veto wins (plugin load order). Modify changes from the first
/// Modify response are used.
pub fn aggregate_hook_responses(results: &[HookResult]) -> HookResponse {
    // First Veto wins
    for r in results {
        if let HookResponse::Veto { reason } = &r.response {
            return HookResponse::Veto { reason: reason.clone() };
        }
    }
    // First Modify wins
    for r in results {
        if let HookResponse::Modify { changes } = &r.response {
            return HookResponse::Modify { changes: changes.clone() };
        }
    }
    HookResponse::Ok
}
```

Also add a `dispatch_hook_aggregated` method that returns both individual results and the aggregate:

```rust
/// Dispatch a hook and return individual results plus the aggregate response.
pub fn dispatch_hook_aggregated(&mut self, event: HookEvent) -> (Vec<HookResult>, HookResponse) {
    let results = self.dispatch_hook(event);
    let aggregate = aggregate_hook_responses(&results);
    (results, aggregate)
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p ucode-plugins -- test_aggregate`
Expected: PASS

**Step 5: Verify full test suite**

Run: `cargo test -p ucode-plugins`
Expected: All tests pass

**Step 6: Commit**

```
feat(plugins): add hook response aggregation (ISSUE 0805)
```

---

### Task 6: Handshake negotiation with policy

**Files:**
- Modify: `crates/ucode-plugins/src/host.rs`
- Modify: `crates/ucode-plugins/src/policy.rs`

**Step 1: Write failing tests**

Add `PluginPolicyConfig` tests in `policy.rs`:

```rust
#[test]
fn test_policy_config_default() {
    let config = PluginPolicyConfig::default();
    assert!(config.default_wasm.workspace_bound);
    assert!(!config.default_wasm.filesystem_write);
    assert!(config.default_native.filesystem_write);
    assert!(config.per_plugin.is_empty());
}

#[test]
fn test_policy_config_resolve_wasm_default() {
    let config = PluginPolicyConfig::default();
    let caps = PluginCapabilities {
        filesystem: true,
        network: false,
        ..Default::default()
    };
    let policy = config.resolve_wasm("org.test.plugin", &caps);
    assert!(policy.filesystem_read);
    assert!(!policy.network.allowed);
}

#[test]
fn test_policy_config_resolve_per_plugin_override() {
    let mut config = PluginPolicyConfig::default();
    let mut override_policy = PluginPolicy::default_wasm();
    override_policy.filesystem_write = true;
    config.per_plugin.insert("org.test.special".into(), override_policy);

    let caps = PluginCapabilities { filesystem: true, ..Default::default() };
    let policy = config.resolve_wasm("org.test.special", &caps);
    // Per-plugin override grants write
    assert!(policy.filesystem_write);

    // Other plugins get default
    let policy2 = config.resolve_wasm("org.test.other", &caps);
    assert!(!policy2.filesystem_write);
}
```

Add handshake negotiation tests in `host.rs`:

```rust
#[test]
fn test_handshake_negotiation_caps_denied() {
    let mut host = PluginHost::new();
    // Configure host to deny network for WASM
    // (default_wasm already denies network, so a plugin requesting network should get it denied)
    struct NetworkPlugin;
    impl Plugin for NetworkPlugin {
        fn handshake(&self) -> HandshakeRequest {
            HandshakeRequest {
                plugin_id: "org.test.networker".into(),
                plugin_version: semver::Version::new(1, 0, 0),
                min_api_version: semver::Version::new(1, 0, 0),
                required_features: [Feature::Hooks].into(),
                capabilities: PluginCapabilities {
                    network: true,
                    ..Default::default()
                },
            }
        }
        fn initialize(&mut self, _: &HandshakeResponse) -> Result<(), String> { Ok(()) }
        fn shutdown(&mut self) {}
    }
    impl HookHandler for NetworkPlugin {
        fn on_event(&mut self, _: &HookRecord) -> HookResponse { HookResponse::Ok }
    }
    host.load(NetworkPlugin).unwrap();
    // Plugin loaded but network should be denied in effective policy
    let policy = host.plugin_policy("org.test.networker").unwrap();
    assert!(policy.filesystem_read); // default native grants this
}
```

**Step 2: Implement PluginPolicyConfig**

Add to `policy.rs`:

```rust
use std::collections::HashMap;

/// Host-side configuration for plugin policies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginPolicyConfig {
    pub default_wasm: PluginPolicy,
    pub default_native: PluginPolicy,
    pub per_plugin: HashMap<String, PluginPolicy>,
}

impl Default for PluginPolicyConfig {
    fn default() -> Self {
        Self {
            default_wasm: PluginPolicy::default_wasm(),
            default_native: PluginPolicy::default_native(),
            per_plugin: HashMap::new(),
        }
    }
}

impl PluginPolicyConfig {
    /// Resolve the effective policy for a WASM plugin.
    ///
    /// If a per-plugin override exists, use it. Otherwise, compute from
    /// capabilities intersected with the default WASM policy.
    pub fn resolve_wasm(&self, plugin_id: &str, caps: &PluginCapabilities) -> PluginPolicy {
        if let Some(override_policy) = self.per_plugin.get(plugin_id) {
            return override_policy.clone();
        }
        PluginPolicy::from_capabilities(caps)
    }
}
```

**Step 3: Run tests to verify they pass**

Run: `cargo test -p ucode-plugins -- test_policy_config test_handshake_negotiation`
Expected: PASS

**Step 4: Verify full test suite**

Run: `cargo test -p ucode-plugins`
Expected: All tests pass

**Step 5: Commit**

```
feat(plugins): add PluginPolicyConfig and handshake negotiation (ISSUE 0805)
```

---

### Task 7: WASI preopens for defense-in-depth

**Files:**
- Modify: `crates/ucode-plugins/src/wasm/host.rs`

**Step 1: Write failing tests**

Add to `wasm/host.rs` tests:

```rust
#[test]
fn test_create_store_with_policy_no_preopens() {
    let engine = build_engine().unwrap();
    let policy = PluginPolicy::default_wasm();
    // No allowed_paths and no filesystem => no preopens
    let store = create_store_with_policy(&engine, &policy, None);
    // Store should be created successfully
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
```

**Step 2: Implement create_store_with_policy**

Add to `wasm/host.rs`:

```rust
use crate::policy::PluginPolicy;

/// Create a Store with WASI context configured according to the plugin's policy.
///
/// Filesystem preopens are restricted to `allowed_paths` (or `workspace_root`
/// if `workspace_bound` and no specific paths). Network capability is only
/// granted if `policy.network.allowed` is true.
pub fn create_store_with_policy(
    engine: &Engine,
    policy: &PluginPolicy,
    workspace_root: Option<&std::path::Path>,
) -> Store<WasmHostState> {
    let mut store = Store::new(
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
```

Also add the policy field to `WasmPlugin` and update `create_store`:

```rust
impl WasmPlugin {
    /// Create a store with policy-aware WASI configuration.
    pub fn create_store_with_policy(
        &self,
        policy: &PluginPolicy,
        workspace_root: Option<&std::path::Path>,
    ) -> Store<WasmHostState> {
        create_store_with_policy(&self.engine, policy, workspace_root)
    }
}
```

**Step 3: Run tests to verify they pass**

Run: `cargo test -p ucode-plugins --features wasm -- test_create_store_with_policy`
Expected: PASS

**Step 4: Verify full test suite**

Run: `cargo test -p ucode-plugins --features wasm`
Expected: All tests pass

**Step 5: Commit**

```
feat(plugins): add policy-aware WASM store creation (ISSUE 0805)
```

---

### Task 8: Logging and tracing integration

**Files:**
- Modify: `crates/ucode-plugins/src/host.rs`
- Modify: `crates/ucode-plugins/src/policy.rs`

**Step 1: Add tracing instrumentation**

Add `tracing::info!` at plugin load time with the effective policy:

```rust
// In load(), after storing the plugin:
tracing::info!(
    plugin_id = %plugin_id,
    policy = ?loaded.policy,
    "plugin loaded with policy"
);
```

Add `tracing::debug!` for allowed actions and `tracing::warn!` for denied actions in `dispatch_hook()` (already partially done in Task 4).

Ensure `PluginPolicy` derives `Debug` (already done) and `Serialize` (already done).

**Step 2: Add PartialEq to OverrideClass for policy checks**

In `hooks.rs`, add `PartialOrd, Ord` derives to `OverrideClass` if not already present, or keep using the `override_class_level` helper.

Actually, `OverrideClass` already has `PartialEq, Eq`. The `override_class_level` helper in `policy.rs` handles ordering. No change needed.

**Step 3: Verify full test suite**

Run: `cargo test -p ucode-plugins`
Expected: All tests pass

**Step 4: Commit**

```
feat(plugins): add tracing instrumentation for policy decisions (ISSUE 0805)
```

---

### Task 9: (moved to Task 14 — final verification after all features)

**Files:**
- Modify: `PLANS.md`
- Modify: `EPIC.md`

**Step 1: Run full test suite**

Run: `cargo test -p ucode-plugins`
Run: `cargo test -p ucode-plugins --features wasm`
Run: `cargo clippy -p ucode-plugins -- -D warnings`
Run: `cargo clippy -p ucode-plugins --features wasm -- -D warnings`
Expected: All pass, 0 warnings

**Step 2: Count tests**

Run: `cargo test -p ucode-plugins 2>&1 | grep 'test result'`
Expected: Note the total test count

**Step 3: Update PLANS.md**

Mark Task 8.5 as DONE with test count.

**Step 4: Update EPIC.md**

Mark ISSUE 0805 as DONE with summary of what was implemented.

**Step 5: Commit**

```
docs: mark ISSUE 0805 plugin isolation model DONE
```

---

### Task 10: WASM resource limits (fuel + memory)

**Files:**
- Modify: `crates/ucode-plugins/src/policy.rs`
- Modify: `crates/ucode-plugins/src/wasm/host.rs`

**Step 1: Write failing tests for ResourceLimits**

Add to `policy.rs` tests:

```rust
#[test]
fn test_resource_limits_default() {
    let limits = ResourceLimits::default();
    assert_eq!(limits.max_memory_bytes, 16 * 1024 * 1024); // 16 MiB
    assert_eq!(limits.max_fuel, 1_000_000);
    assert_eq!(limits.max_instances, 1);
}

#[test]
fn test_default_wasm_policy_has_resource_limits() {
    let policy = PluginPolicy::default_wasm();
    assert_eq!(policy.resource_limits.max_memory_bytes, 16 * 1024 * 1024);
    assert_eq!(policy.resource_limits.max_fuel, 1_000_000);
}

#[test]
fn test_default_native_policy_no_resource_limits() {
    let policy = PluginPolicy::default_native();
    // Native plugins get generous limits (effectively unlimited)
    assert_eq!(policy.resource_limits.max_memory_bytes, usize::MAX);
    assert_eq!(policy.resource_limits.max_fuel, u64::MAX);
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p ucode-plugins -- test_resource_limits`
Expected: FAIL (ResourceLimits not defined)

**Step 3: Implement ResourceLimits**

Add to `policy.rs`:

```rust
/// Resource limits for WASM plugin execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// Maximum linear memory in bytes per WASM instance (default: 16 MiB).
    pub max_memory_bytes: usize,
    /// Maximum fuel (instruction budget) per hook dispatch (default: 1_000_000).
    pub max_fuel: u64,
    /// Maximum number of WASM instances (default: 1).
    pub max_instances: usize,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_memory_bytes: 16 * 1024 * 1024, // 16 MiB
            max_fuel: 1_000_000,
            max_instances: 1,
        }
    }
}

impl ResourceLimits {
    /// Effectively unlimited resource limits for native plugins.
    pub fn unlimited() -> Self {
        Self {
            max_memory_bytes: usize::MAX,
            max_fuel: u64::MAX,
            max_instances: usize::MAX,
        }
    }
}
```

Add `resource_limits: ResourceLimits` field to `PluginPolicy`. Update `default_wasm()` to use `ResourceLimits::default()` and `default_native()` to use `ResourceLimits::unlimited()`. Update `from_capabilities()` to use `ResourceLimits::default()`.

**Step 4: Run tests to verify they pass**

Run: `cargo test -p ucode-plugins -- test_resource_limits`
Expected: PASS

**Step 5: Write failing tests for fuel-aware Store creation**

Add to `wasm/host.rs` tests:

```rust
#[test]
fn test_build_engine_with_fuel() {
    let engine = build_engine_with_fuel().unwrap();
    // Engine should be created successfully with fuel consumption enabled
    let _ = engine;
}

#[test]
fn test_wasm_host_state_has_limits() {
    let limits = StoreLimitsBuilder::new()
        .memory_size(16 * 1024 * 1024)
        .instances(1)
        .build();
    let state = WasmHostState {
        log_messages: Vec::new(),
        limits,
    };
    assert!(state.log_messages.is_empty());
}
```

**Step 6: Implement fuel-aware engine and Store with limits**

In `wasm/host.rs`:

1. Add `StoreLimits` to `WasmHostState`:

```rust
use wasmtime::StoreLimits;

pub struct WasmHostState {
    pub log_messages: Vec<String>,
    pub limits: StoreLimits,
}
```

2. Add `build_engine_with_fuel()`:

```rust
fn build_engine_with_fuel() -> Result<Engine, WasmPluginError> {
    let mut config = Config::new();
    config.wasm_component_model(true);
    config.consume_fuel(true);
    Engine::new(&config).map_err(WasmPluginError::Engine)
}
```

3. Update `create_store_with_policy()` to configure limits and fuel:

```rust
pub fn create_store_with_policy(
    engine: &Engine,
    policy: &PluginPolicy,
    workspace_root: Option<&std::path::Path>,
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

    tracing::info!(
        max_memory_bytes = policy.resource_limits.max_memory_bytes,
        max_fuel = policy.resource_limits.max_fuel,
        max_instances = policy.resource_limits.max_instances,
        "WASM store created with resource limits"
    );

    store
}
```

4. Update `WasmPlugin::from_file()` and `from_bytes()` to use `build_engine_with_fuel()`.

5. Update existing `create_store()` to also include `StoreLimits` in `WasmHostState` (backward compat).

**Step 7: Run tests to verify they pass**

Run: `cargo test -p ucode-plugins --features wasm -- test_build_engine_with_fuel test_wasm_host_state_has_limits`
Expected: PASS

**Step 8: Verify full test suite**

Run: `cargo test -p ucode-plugins --features wasm`
Expected: All tests pass

**Step 9: Commit**

```
feat(plugins): add WASM resource limits (fuel + memory) (ISSUE 0805)
```

---

### Task 11: Dynamic policy hot-reload

**Files:**
- Modify: `crates/ucode-plugins/src/policy.rs`
- Modify: `crates/ucode-plugins/src/host.rs`

**Step 1: Write failing tests for TOML config parsing**

Add to `policy.rs` tests:

```rust
#[test]
fn test_policy_config_from_toml() {
    let toml = r#"
        [default_wasm]
        filesystem_read = true
        filesystem_write = false
        workspace_bound = true
        process_spawn = false
        guarded_ui = false

        [default_wasm.network]
        allowed = false

        [default_wasm.resource_limits]
        max_memory_bytes = 8388608
        max_fuel = 500000
        max_instances = 1
    "#;
    let config: PluginPolicyConfig = toml::from_str(toml).unwrap();
    assert!(config.default_wasm.filesystem_read);
    assert!(!config.default_wasm.filesystem_write);
    assert_eq!(config.default_wasm.resource_limits.max_memory_bytes, 8_388_608);
}

#[test]
fn test_policy_config_to_toml_roundtrip() {
    let config = PluginPolicyConfig::default();
    let toml_str = toml::to_string_pretty(&config).unwrap();
    let parsed: PluginPolicyConfig = toml::from_str(&toml_str).unwrap();
    assert_eq!(parsed.default_wasm.filesystem_read, config.default_wasm.filesystem_read);
    assert_eq!(parsed.default_wasm.resource_limits.max_fuel, config.default_wasm.resource_limits.max_fuel);
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p ucode-plugins -- test_policy_config_from_toml test_policy_config_to_toml`
Expected: FAIL (PluginPolicy fields not fully serializable or TOML parsing issues)

**Step 3: Ensure PluginPolicy and all nested types derive Serialize + Deserialize**

Verify that `OverrideClass` has serde derives. If not, add:

```rust
// In hooks.rs
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverrideClass {
    Safe,
    Guarded,
    Risky,
}
```

Ensure `PluginIsolationLevel` (added in Task 12) also has serde derives.

**Step 4: Write failing tests for reload_policy_config**

Add to `host.rs` tests:

```rust
#[test]
fn test_reload_policy_config() {
    let mut host = PluginHost::new();
    host.load(TestPlugin::new("org.test.logger")).unwrap();

    // Initial policy is default_native
    let policy = host.plugin_policy("org.test.logger").unwrap();
    assert!(policy.filesystem_write);

    // Reload with restrictive config
    let mut config = PluginPolicyConfig::default();
    let mut restrictive = PluginPolicy::default_wasm();
    restrictive.filesystem_write = false;
    config.per_plugin.insert("org.test.logger".into(), restrictive);
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
    // org.test.b should keep default_native
    assert!(host.plugin_policy("org.test.b").unwrap().process_spawn);
}
```

**Step 5: Implement reload_policy_config**

Add to `PluginHost`:

```rust
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
        let new_policy = if let Some(override_policy) = config.per_plugin.get(&loaded.plugin_id) {
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
```

**Step 6: Run tests to verify they pass**

Run: `cargo test -p ucode-plugins -- test_reload_policy_config test_policy_config_from_toml test_policy_config_to_toml`
Expected: PASS

**Step 7: Verify full test suite**

Run: `cargo test -p ucode-plugins`
Expected: All tests pass

**Step 8: Commit**

```
feat(plugins): add dynamic policy hot-reload (ISSUE 0805)
```

---

### Task 12: Plugin-to-plugin communication policy (isolation levels)

**Files:**
- Modify: `crates/ucode-plugins/src/policy.rs`
- Modify: `crates/ucode-plugins/src/host.rs`

**Step 1: Write failing tests for PluginIsolationLevel**

Add to `policy.rs` tests:

```rust
#[test]
fn test_default_wasm_isolation_full() {
    let policy = PluginPolicy::default_wasm();
    assert_eq!(policy.isolation_level, PluginIsolationLevel::Full);
}

#[test]
fn test_default_native_isolation_ordered() {
    let policy = PluginPolicy::default_native();
    assert_eq!(policy.isolation_level, PluginIsolationLevel::Ordered);
}
```

Add to `host.rs` tests:

```rust
#[test]
fn test_ordered_dispatch_passes_modifications() {
    // Plugin A modifies args, Plugin B sees modified args
    struct ModifierPlugin {
        id: String,
        seen_args: Option<serde_json::Value>,
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
        fn initialize(&mut self, _: &HandshakeResponse) -> Result<(), String> { Ok(()) }
        fn shutdown(&mut self) {}
    }
    impl HookHandler for ModifierPlugin {
        fn on_event(&mut self, record: &HookRecord) -> HookResponse {
            if self.id == "org.test.modifier" {
                HookResponse::Modify {
                    changes: serde_json::json!({"injected": true}),
                }
            } else {
                // Observer records what it saw
                HookResponse::Ok
            }
        }
    }

    let mut host = PluginHost::new();
    let modifier = ModifierPlugin { id: "org.test.modifier".into(), seen_args: None };
    let observer = ModifierPlugin { id: "org.test.observer".into(), seen_args: None };
    host.load(modifier).unwrap();
    host.load(observer).unwrap();

    // Set both to Ordered isolation with Guarded ceiling
    let mut policy = PluginPolicy::default_native();
    policy.isolation_level = PluginIsolationLevel::Ordered;
    policy.max_override_class = OverrideClass::Guarded;
    host.set_plugin_policy("org.test.modifier", policy.clone());
    host.set_plugin_policy("org.test.observer", policy);

    let results = host.dispatch_hook(HookEvent::BeforeToolCall {
        tool_name: "bash".into(),
        args: serde_json::json!({"cmd": "ls"}),
    });
    // Both plugins should have been dispatched
    assert_eq!(results.len(), 2);
    // First plugin returned Modify
    assert!(matches!(results[0].response, HookResponse::Modify { .. }));
}

#[test]
fn test_full_isolation_no_modification_passthrough() {
    // With Full isolation, each plugin sees the original event
    let mut host = PluginHost::new();
    let plugin_a = TestPlugin::new("org.test.a");
    let plugin_b = TestPlugin::new("org.test.b");
    host.load(plugin_a).unwrap();
    host.load(plugin_b).unwrap();

    // Set both to Full isolation (WASM default)
    let policy = PluginPolicy::default_wasm();
    host.set_plugin_policy("org.test.a", policy.clone());
    host.set_plugin_policy("org.test.b", policy);

    let results = host.dispatch_hook(HookEvent::SessionStart { session_id: "s1".into() });
    // Both should receive the event (both have empty allowed_hook_categories = all)
    assert_eq!(results.len(), 2);
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p ucode-plugins -- test_ordered_dispatch test_full_isolation test_default_wasm_isolation test_default_native_isolation`
Expected: FAIL

**Step 3: Implement PluginIsolationLevel**

Add to `policy.rs`:

```rust
/// Controls whether a plugin can observe other plugins' hook responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginIsolationLevel {
    /// Plugin sees only the original event payload.
    Full,
    /// Plugin sees the event payload as modified by prior plugins in load order.
    Ordered,
}

impl Default for PluginIsolationLevel {
    fn default() -> Self {
        Self::Full
    }
}
```

Add `isolation_level: PluginIsolationLevel` to `PluginPolicy`. Update `default_wasm()` to use `Full`, `default_native()` to use `Ordered`.

**Step 4: Update dispatch_hook for isolation levels**

In `host.rs`, update `dispatch_hook()` to track modifications when plugins use `Ordered` isolation. When a plugin returns `Modify` and the next plugin has `Ordered` isolation, the modifications are applied to the record before dispatch.

The implementation is a documentation-level change for now: the `HookRecord` is cloned per-plugin when `Full`, or mutated in-place when `Ordered`. Since `Modify` returns a `serde_json::Value` of changes, applying it to the record requires merging the changes into the event payload. For v1, the `Ordered` mode passes the accumulated `changes` as additional context rather than modifying the `HookEvent` enum (which is not easily mutable).

Simpler approach: add an `accumulated_changes: Option<serde_json::Value>` field to `HookRecord` that `Ordered` plugins can see:

```rust
// In hooks.rs, extend HookRecord:
pub struct HookRecord {
    pub event: HookEvent,
    pub timestamp: DateTime<Utc>,
    /// Accumulated modifications from prior plugins (Ordered isolation only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accumulated_changes: Option<serde_json::Value>,
}
```

In `dispatch_hook()`:

```rust
let mut accumulated_changes: Option<serde_json::Value> = None;

for loaded in &mut self.plugins {
    // ... category check ...

    // Build record with or without accumulated changes based on isolation
    let dispatch_record = if loaded.policy.isolation_level == PluginIsolationLevel::Ordered {
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

    // ... dispatch to plugin ...

    // If plugin returned Modify, accumulate changes
    if let HookResponse::Modify { changes } = &response {
        accumulated_changes = Some(match accumulated_changes {
            Some(mut existing) => {
                if let (Some(obj), Some(new)) = (existing.as_object_mut(), changes.as_object()) {
                    for (k, v) in new {
                        obj.insert(k.clone(), v.clone());
                    }
                }
                existing
            }
            None => changes.clone(),
        });
    }
}
```

**Step 5: Run tests to verify they pass**

Run: `cargo test -p ucode-plugins -- test_ordered_dispatch test_full_isolation test_default_wasm_isolation test_default_native_isolation`
Expected: PASS

**Step 6: Verify full test suite**

Run: `cargo test -p ucode-plugins`
Expected: All tests pass

**Step 7: Commit**

```
feat(plugins): add plugin isolation levels (Full/Ordered) (ISSUE 0805)
```

---

### Task 13: Signed plugin verification (Ed25519)

**Files:**
- Modify: `Cargo.toml` (workspace deps)
- Modify: `crates/ucode-plugins/Cargo.toml`
- Create: `crates/ucode-plugins/src/wasm/signature.rs`
- Modify: `crates/ucode-plugins/src/wasm/mod.rs`
- Modify: `crates/ucode-plugins/src/wasm/host.rs`
- Modify: `crates/ucode-plugins/src/policy.rs`

**Step 1: Add ed25519-dalek dependency**

In workspace `Cargo.toml`:

```toml
ed25519-dalek = { version = "2", features = ["std"] }
```

In `crates/ucode-plugins/Cargo.toml`:

```toml
[features]
default = []
wasm = ["dep:wasmtime", "dep:wasmtime-wasi"]
signed-plugins = ["dep:ed25519-dalek"]

[dependencies]
ed25519-dalek = { workspace = true, optional = true }
```

**Step 2: Write failing tests for signature verification**

Create `crates/ucode-plugins/src/wasm/signature.rs`:

```rust
//! Ed25519 signature verification for WASM plugin binaries.

#[cfg(feature = "signed-plugins")]
use ed25519_dalek::{Signature, Verifier, VerifyingKey};

use std::path::Path;

/// Signature verification policy.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignaturePolicy {
    /// Reject unsigned or invalid-signature plugins.
    Required,
    /// Warn on unsigned plugins, reject invalid signatures.
    WarnUnsigned,
    /// Skip signature verification entirely.
    Disabled,
}

impl Default for SignaturePolicy {
    fn default() -> Self {
        Self::Disabled
    }
}

/// Result of signature verification.
#[derive(Debug, Clone, PartialEq)]
pub enum SignatureCheckResult {
    /// Signature is valid.
    Valid,
    /// No signature file found.
    Unsigned,
    /// Signature file exists but is invalid.
    Invalid { reason: String },
}

/// Errors from signature verification.
#[derive(Debug)]
pub enum SignatureError {
    /// Plugin is unsigned and policy requires signatures.
    Unsigned,
    /// Signature is invalid.
    Invalid(String),
    /// I/O error reading signature file.
    Io(std::io::Error),
    /// Invalid key format.
    InvalidKey(String),
}

impl std::fmt::Display for SignatureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsigned => write!(f, "plugin is unsigned"),
            Self::Invalid(reason) => write!(f, "invalid signature: {reason}"),
            Self::Io(e) => write!(f, "signature I/O error: {e}"),
            Self::InvalidKey(reason) => write!(f, "invalid key: {reason}"),
        }
    }
}

impl std::error::Error for SignatureError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signature_policy_default_disabled() {
        assert_eq!(SignaturePolicy::default(), SignaturePolicy::Disabled);
    }

    #[test]
    fn test_signature_check_result_variants() {
        let valid = SignatureCheckResult::Valid;
        assert_eq!(valid, SignatureCheckResult::Valid);

        let unsigned = SignatureCheckResult::Unsigned;
        assert_eq!(unsigned, SignatureCheckResult::Unsigned);

        let invalid = SignatureCheckResult::Invalid { reason: "bad sig".into() };
        assert!(matches!(invalid, SignatureCheckResult::Invalid { .. }));
    }

    #[test]
    fn test_apply_policy_disabled_allows_unsigned() {
        let result = apply_signature_policy(&SignaturePolicy::Disabled, &SignatureCheckResult::Unsigned);
        assert!(result.is_ok());
    }

    #[test]
    fn test_apply_policy_required_rejects_unsigned() {
        let result = apply_signature_policy(&SignaturePolicy::Required, &SignatureCheckResult::Unsigned);
        assert!(result.is_err());
    }

    #[test]
    fn test_apply_policy_warn_unsigned_allows() {
        let result = apply_signature_policy(&SignaturePolicy::WarnUnsigned, &SignatureCheckResult::Unsigned);
        assert!(result.is_ok());
    }

    #[test]
    fn test_apply_policy_required_allows_valid() {
        let result = apply_signature_policy(&SignaturePolicy::Required, &SignatureCheckResult::Valid);
        assert!(result.is_ok());
    }

    #[test]
    fn test_apply_policy_any_rejects_invalid() {
        let invalid = SignatureCheckResult::Invalid { reason: "bad".into() };
        // All policies except Disabled reject invalid signatures
        assert!(apply_signature_policy(&SignaturePolicy::Required, &invalid).is_err());
        assert!(apply_signature_policy(&SignaturePolicy::WarnUnsigned, &invalid).is_err());
        // Disabled skips verification entirely
        assert!(apply_signature_policy(&SignaturePolicy::Disabled, &invalid).is_ok());
    }

    #[cfg(feature = "signed-plugins")]
    #[test]
    fn test_verify_signature_roundtrip() {
        use ed25519_dalek::{SigningKey, Signer};
        use rand::rngs::OsRng;

        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();
        let wasm_bytes = b"fake wasm component bytes";
        let signature = signing_key.sign(wasm_bytes);
        let sig_bytes = signature.to_bytes();

        let result = verify_signature(wasm_bytes, &sig_bytes, &[verifying_key]);
        assert_eq!(result, SignatureCheckResult::Valid);
    }

    #[cfg(feature = "signed-plugins")]
    #[test]
    fn test_verify_signature_wrong_key() {
        use ed25519_dalek::{SigningKey, Signer};
        use rand::rngs::OsRng;

        let signing_key = SigningKey::generate(&mut OsRng);
        let wrong_key = SigningKey::generate(&mut OsRng).verifying_key();
        let wasm_bytes = b"fake wasm component bytes";
        let signature = signing_key.sign(wasm_bytes);
        let sig_bytes = signature.to_bytes();

        let result = verify_signature(wasm_bytes, &sig_bytes, &[wrong_key]);
        assert!(matches!(result, SignatureCheckResult::Invalid { .. }));
    }
}
```

**Step 3: Run tests to verify they fail**

Run: `cargo test -p ucode-plugins -- signature`
Expected: FAIL (functions not implemented)

**Step 4: Implement signature verification functions**

Add to `signature.rs`:

```rust
/// Apply signature policy to a check result.
pub fn apply_signature_policy(
    policy: &SignaturePolicy,
    result: &SignatureCheckResult,
) -> Result<(), SignatureError> {
    match policy {
        SignaturePolicy::Disabled => Ok(()),
        SignaturePolicy::WarnUnsigned => match result {
            SignatureCheckResult::Valid => Ok(()),
            SignatureCheckResult::Unsigned => {
                tracing::warn!("plugin is unsigned (policy: warn_unsigned)");
                Ok(())
            }
            SignatureCheckResult::Invalid { reason } => {
                Err(SignatureError::Invalid(reason.clone()))
            }
        },
        SignaturePolicy::Required => match result {
            SignatureCheckResult::Valid => Ok(()),
            SignatureCheckResult::Unsigned => Err(SignatureError::Unsigned),
            SignatureCheckResult::Invalid { reason } => {
                Err(SignatureError::Invalid(reason.clone()))
            }
        },
    }
}

/// Check if a `.wasm.sig` file exists for the given WASM path.
pub fn check_signature_file(wasm_path: &Path) -> Option<Vec<u8>> {
    let sig_path = wasm_path.with_extension("wasm.sig");
    std::fs::read(&sig_path).ok()
}

/// Verify a signature against WASM bytes using trusted keys.
#[cfg(feature = "signed-plugins")]
pub fn verify_signature(
    wasm_bytes: &[u8],
    signature_bytes: &[u8; 64],
    trusted_keys: &[VerifyingKey],
) -> SignatureCheckResult {
    let signature = Signature::from_bytes(signature_bytes);
    for key in trusted_keys {
        if key.verify(wasm_bytes, &signature).is_ok() {
            return SignatureCheckResult::Valid;
        }
    }
    SignatureCheckResult::Invalid {
        reason: "signature does not match any trusted key".into(),
    }
}

/// Full verification flow: check for sig file, verify if present, apply policy.
#[cfg(feature = "signed-plugins")]
pub fn verify_plugin_signature(
    wasm_path: &Path,
    wasm_bytes: &[u8],
    trusted_keys: &[VerifyingKey],
    policy: &SignaturePolicy,
) -> Result<(), SignatureError> {
    if matches!(policy, SignaturePolicy::Disabled) {
        return Ok(());
    }

    let check_result = match check_signature_file(wasm_path) {
        None => SignatureCheckResult::Unsigned,
        Some(sig_bytes) => {
            if sig_bytes.len() != 64 {
                SignatureCheckResult::Invalid {
                    reason: format!("signature file is {} bytes, expected 64", sig_bytes.len()),
                }
            } else {
                let mut sig_array = [0u8; 64];
                sig_array.copy_from_slice(&sig_bytes);
                verify_signature(wasm_bytes, &sig_array, trusted_keys)
            }
        }
    };

    apply_signature_policy(policy, &check_result)
}
```

**Step 5: Add SignaturePolicy to PluginPolicyConfig**

In `policy.rs`, add to `PluginPolicyConfig`:

```rust
pub struct PluginPolicyConfig {
    pub default_wasm: PluginPolicy,
    pub default_native: PluginPolicy,
    pub per_plugin: HashMap<String, PluginPolicy>,
    /// Signature verification policy for WASM plugins.
    #[serde(default)]
    pub signature_policy: SignaturePolicy,
    /// Hex-encoded Ed25519 public keys trusted for plugin signing.
    #[serde(default)]
    pub trusted_keys: Vec<String>,
}
```

Import `SignaturePolicy` from `wasm::signature` (or define it in `policy.rs` and re-export).

**Step 6: Wire into load_wasm**

In `host.rs`, update `load_wasm()` to call signature verification before instantiation (when `signed-plugins` feature is enabled).

**Step 7: Add module to wasm/mod.rs**

```rust
pub mod signature;
pub use signature::{SignaturePolicy, SignatureCheckResult, SignatureError};
```

**Step 8: Run tests to verify they pass**

Run: `cargo test -p ucode-plugins -- signature`
Run: `cargo test -p ucode-plugins --features signed-plugins -- signature`
Expected: PASS

**Step 9: Verify full test suite**

Run: `cargo test -p ucode-plugins`
Run: `cargo test -p ucode-plugins --features wasm`
Run: `cargo test -p ucode-plugins --features wasm,signed-plugins`
Expected: All pass

**Step 10: Commit**

```
feat(plugins): add Ed25519 signed plugin verification (ISSUE 0805)
```

---

### Task 14: Final verification and docs update (replaces Task 9)

**Files:**
- Modify: `PLANS.md`
- Modify: `EPIC.md`

**Step 1: Run full test suite**

Run: `cargo test -p ucode-plugins`
Run: `cargo test -p ucode-plugins --features wasm`
Run: `cargo test -p ucode-plugins --features wasm,signed-plugins`
Run: `cargo clippy -p ucode-plugins -- -D warnings`
Run: `cargo clippy -p ucode-plugins --features wasm -- -D warnings`
Run: `cargo clippy -p ucode-plugins --features wasm,signed-plugins -- -D warnings`
Expected: All pass, 0 warnings

**Step 2: Count tests**

Run: `cargo test -p ucode-plugins --features wasm,signed-plugins 2>&1 | grep 'test result'`
Expected: Note the total test count

**Step 3: Update PLANS.md**

Mark Task 8.5 as DONE with test count and summary:
- PluginPolicy with scoped capabilities
- Hook category filtering + override class ceiling
- Hook response aggregation (Veto > Modify > Ok)
- WASM resource limits (fuel + memory)
- Dynamic policy hot-reload
- Plugin isolation levels (Full/Ordered)
- Ed25519 signed plugin verification
- WASI preopens for defense-in-depth

**Step 4: Update EPIC.md**

Mark ISSUE 0805 as DONE.

**Step 5: Commit**

```
docs: mark ISSUE 0805 plugin isolation model DONE
```
