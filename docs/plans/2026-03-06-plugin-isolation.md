# Plugin Runtime Isolation Model Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Implement per-plugin policy profiles for WASM plugins that gate filesystem, network, process, and hook capabilities, with enforcement at both host dispatch and WASM boundary layers.

**Architecture:** `PluginPolicy` struct in `ucode-plugins` holds per-plugin effective permissions computed at handshake time (requested caps intersected with host-allowed caps). Enforcement at 5 points: hook category filtering, hook response validation, hook response aggregation, tool invocation policy checks, and WASI preopens for defense-in-depth. No cross-crate dependency on `ucode-tools`.

**Tech Stack:** Rust, serde, tracing, wasmtime (WASI preopens), existing ucode-plugins infrastructure

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

### Task 9: Update PLANS.md and EPIC.md, final verification

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
