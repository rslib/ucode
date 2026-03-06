# Plugin API Contract Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Implement the v1 plugin API contract with Rust traits, expand hook events to 64, add manifest `id`/`required_features` fields, tool namespacing, and an in-process example plugin.

**Architecture:** Traits-first approach. `Plugin`, `HookHandler`, `ToolProvider` traits define the contract. `PluginHost` manages lifecycle (load, handshake, dispatch, unload). WASM deferred to Task 8.4.

**Tech Stack:** Rust, semver crate, serde, serde_json, thiserror, chrono (all existing except semver)

**Design doc:** `docs/plans/2026-03-06-plugin-api-contract-design.md`

---

### Task 1: Add semver dependency

**Files:**
- Modify: `crates/ucode-plugins/Cargo.toml`

**Step 1: Add semver dependency**

```bash
cargo add semver --package ucode-plugins
```

**Step 2: Verify it compiles**

Run: `cargo check --package ucode-plugins`
Expected: compiles without errors

**Step 3: Commit**

```bash
git add crates/ucode-plugins/Cargo.toml
git commit -m "chore: add semver dependency to ucode-plugins"
```

---

### Task 2: Manifest changes — add `id`, `required_features`, update validation

**Files:**
- Modify: `crates/ucode-plugins/src/manifest.rs`

**Step 1: Write failing tests for new manifest fields**

Add these tests to the existing `mod tests` block in `manifest.rs`:

```rust
#[test]
fn test_parse_manifest_with_id() {
    let toml = r#"
        id = "org.acme.code-analyzer"
        name = "Code Analyzer"
        version = "1.0.0"
    "#;
    let m = parse_manifest(toml).unwrap();
    assert_eq!(m.id.as_deref(), Some("org.acme.code-analyzer"));
    assert_eq!(m.name, "Code Analyzer");
}

#[test]
fn test_validate_id_format_valid() {
    let toml = r#"
        id = "org.acme.code-analyzer"
        name = "Code Analyzer"
        version = "1.0.0"
    "#;
    assert!(parse_manifest(toml).is_ok());
}

#[test]
fn test_validate_id_format_too_few_segments() {
    let toml = r#"
        id = "acme.plugin"
        name = "Bad Plugin"
        version = "1.0.0"
    "#;
    let err = parse_manifest(toml).unwrap_err();
    assert!(err.to_string().contains("at least 3 dot-separated segments"));
}

#[test]
fn test_validate_id_format_invalid_chars() {
    let toml = r#"
        id = "org.Acme.Plugin"
        name = "Bad Plugin"
        version = "1.0.0"
    "#;
    let err = parse_manifest(toml).unwrap_err();
    assert!(err.to_string().contains("segment"));
}

#[test]
fn test_validate_id_format_empty_segment() {
    let toml = r#"
        id = "org..plugin"
        name = "Bad Plugin"
        version = "1.0.0"
    "#;
    let err = parse_manifest(toml).unwrap_err();
    assert!(err.to_string().contains("segment"));
}

#[test]
fn test_parse_required_features() {
    let toml = r#"
        id = "org.acme.logger"
        name = "Logger"
        version = "1.0.0"
        required_features = ["hooks", "tools"]
    "#;
    let m = parse_manifest(toml).unwrap();
    assert_eq!(m.required_features, vec!["hooks", "tools"]);
}

#[test]
fn test_validate_unknown_feature() {
    let toml = r#"
        id = "org.acme.logger"
        name = "Logger"
        version = "1.0.0"
        required_features = ["hooks", "quantum"]
    "#;
    let err = parse_manifest(toml).unwrap_err();
    assert!(err.to_string().contains("unknown feature"));
}

#[test]
fn test_validate_tool_name_no_dots() {
    let toml = r#"
        id = "org.acme.tools"
        name = "Tools"
        version = "1.0.0"

        [[tools]]
        name = "my.tool"
    "#;
    let err = parse_manifest(toml).unwrap_err();
    assert!(err.to_string().contains("must not contain dots"));
}

#[test]
fn test_backward_compat_name_only() {
    // Old-style manifest without id should still parse (name used as display name)
    let toml = r#"
        name = "my-plugin"
        version = "1.0.0"
    "#;
    let m = parse_manifest(toml).unwrap();
    assert!(m.id.is_none());
    assert_eq!(m.name, "my-plugin");
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --package ucode-plugins -- manifest`
Expected: compilation errors (fields don't exist yet)

**Step 3: Update PluginManifest struct**

Add `id` and `required_features` fields to `PluginManifest`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    /// Reverse-domain globally unique identifier (e.g., "org.acme.code-analyzer").
    /// Minimum 3 dot-separated segments. Used as marketplace ID and tool namespace.
    pub id: Option<String>,
    /// Human-readable display name.
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub author: Option<String>,
    /// Minimum host API version required (semver string).
    pub min_api_version: Option<String>,
    /// API feature surfaces this plugin requires (e.g., ["hooks", "tools", "ui"]).
    #[serde(default)]
    pub required_features: Vec<String>,
    /// Hooks this plugin subscribes to.
    #[serde(default)]
    pub hooks: Vec<String>,
    /// Tools this plugin exports.
    #[serde(default)]
    pub tools: Vec<PluginToolDef>,
    /// Capabilities requested by this plugin.
    #[serde(default)]
    pub capabilities: PluginCapabilities,
}
```

**Step 4: Update validate_manifest**

Add validation for `id` format, `required_features`, and tool name format:

```rust
/// Known feature names for required_features validation.
const KNOWN_FEATURES: &[&str] = &["hooks", "tools", "ui"];

pub fn validate_manifest(manifest: &PluginManifest) -> Result<(), ManifestError> {
    if manifest.name.is_empty() {
        return Err(ManifestError::Validation("name must not be empty".into()));
    }
    if manifest.version.is_empty() {
        return Err(ManifestError::Validation("version must not be empty".into()));
    }
    // Validate id format if present
    if let Some(id) = &manifest.id {
        validate_plugin_id(id)?;
    }
    // Validate required_features
    for feature in &manifest.required_features {
        if !KNOWN_FEATURES.contains(&feature.as_str()) {
            return Err(ManifestError::Validation(
                format!("unknown feature '{}'; known features: {}", feature, KNOWN_FEATURES.join(", "))
            ));
        }
    }
    // Validate tools
    for tool in &manifest.tools {
        if tool.name.is_empty() {
            return Err(ManifestError::Validation("tool name must not be empty".into()));
        }
        if tool.name.contains('.') {
            return Err(ManifestError::Validation(
                format!("tool name '{}' must not contain dots; host constructs FQN from plugin id", tool.name)
            ));
        }
    }
    for hook in &manifest.hooks {
        if hook.is_empty() {
            return Err(ManifestError::Validation("hook name must not be empty".into()));
        }
    }
    Ok(())
}

/// Validate plugin id: at least 3 dot-separated segments, each [a-z0-9][a-z0-9-]*
fn validate_plugin_id(id: &str) -> Result<(), ManifestError> {
    let segments: Vec<&str> = id.split('.').collect();
    if segments.len() < 3 {
        return Err(ManifestError::Validation(
            format!("plugin id '{}' must have at least 3 dot-separated segments (e.g., org.acme.plugin)", id)
        ));
    }
    for segment in &segments {
        if segment.is_empty() {
            return Err(ManifestError::Validation(
                format!("plugin id '{}' has empty segment", id)
            ));
        }
        if !segment.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
            return Err(ManifestError::Validation(
                format!("plugin id segment '{}' must contain only lowercase letters, digits, and hyphens", segment)
            ));
        }
        if segment.starts_with('-') {
            return Err(ManifestError::Validation(
                format!("plugin id segment '{}' must not start with a hyphen", segment)
            ));
        }
    }
    Ok(())
}
```

**Step 5: Run tests**

Run: `cargo test --package ucode-plugins -- manifest`
Expected: all manifest tests pass (existing + new)

**Step 6: Run clippy**

Run: `cargo clippy --package ucode-plugins --all-targets -- -D warnings`
Expected: 0 warnings

**Step 7: Commit**

```bash
git add crates/ucode-plugins/src/manifest.rs
git commit -m "feat: manifest id, required_features, and stricter validation (ISSUE 0803)"
```

---

### Task 3: Expand HookEvent from 22 to 64 variants

**Files:**
- Modify: `crates/ucode-plugins/src/hooks.rs`

**Step 1: Write failing tests for new hook events**

Add to the existing test module in `hooks.rs`:

```rust
#[test]
fn test_new_session_events() {
    assert_eq!(
        HookEvent::SessionTitleGenerated { session_id: "s".into(), title: "t".into() }.event_name(),
        "session_title_generated"
    );
    assert_eq!(
        HookEvent::SessionTitleUpdated { session_id: "s".into(), title: "t".into() }.event_name(),
        "session_title_updated"
    );
    assert_eq!(
        HookEvent::ConfigReloaded.event_name(),
        "config_reloaded"
    );
}

#[test]
fn test_message_flow_events() {
    assert_eq!(
        HookEvent::UserMessageReceived { message_len: 100 }.event_name(),
        "user_message_received"
    );
    assert_eq!(
        HookEvent::AssistantResponseStarted { model: "gpt-4".into() }.event_name(),
        "assistant_response_started"
    );
    assert_eq!(
        HookEvent::AssistantResponseCompleted { model: "gpt-4".into(), tokens: 500, duration_ms: 1200 }.event_name(),
        "assistant_response_completed"
    );
    assert_eq!(
        HookEvent::MessageRetry { reason: "rate_limit".into(), attempt: 2 }.event_name(),
        "message_retry"
    );
}

#[test]
fn test_new_model_events() {
    assert_eq!(
        HookEvent::BeforeModelSelect { candidates: vec!["a".into(), "b".into()] }.event_name(),
        "before_model_select"
    );
    assert_eq!(
        HookEvent::RouterDecision { model: "gpt-4".into(), reason: "cost".into() }.event_name(),
        "router_decision"
    );
    assert_eq!(
        HookEvent::ModelRateLimited { model: "gpt-4".into(), retry_after_ms: Some(5000) }.event_name(),
        "model_rate_limited"
    );
    assert_eq!(
        HookEvent::ModelQuotaExhausted { model: "gpt-4".into() }.event_name(),
        "model_quota_exhausted"
    );
}

#[test]
fn test_tool_specific_events() {
    assert_eq!(
        HookEvent::ToolTimeout { tool_name: "bash".into(), timeout_ms: 30000 }.event_name(),
        "tool_timeout"
    );
    assert_eq!(
        HookEvent::BeforeApplyPatch { file_path: "src/main.rs".into(), patch_summary: "add fn".into() }.event_name(),
        "before_apply_patch"
    );
    assert_eq!(
        HookEvent::AfterApplyPatch { file_path: "src/main.rs".into(), lines_changed: 10 }.event_name(),
        "after_apply_patch"
    );
    assert_eq!(
        HookEvent::BeforeRunCmd { command: "cargo test".into() }.event_name(),
        "before_run_cmd"
    );
    assert_eq!(
        HookEvent::AfterRunCmd { command: "cargo test".into(), exit_code: 0, duration_ms: 5000 }.event_name(),
        "after_run_cmd"
    );
    assert_eq!(
        HookEvent::BeforeFileRead { path: "foo.rs".into() }.event_name(),
        "before_file_read"
    );
    assert_eq!(
        HookEvent::AfterFileRead { path: "foo.rs".into(), size_bytes: 1024 }.event_name(),
        "after_file_read"
    );
    assert_eq!(
        HookEvent::BeforeFileWrite { path: "foo.rs".into() }.event_name(),
        "before_file_write"
    );
    assert_eq!(
        HookEvent::AfterFileWrite { path: "foo.rs".into(), size_bytes: 2048 }.event_name(),
        "after_file_write"
    );
}

#[test]
fn test_new_context_events() {
    assert_eq!(
        HookEvent::ContextDistilled { before_tokens: 8000, after_tokens: 3000 }.event_name(),
        "context_distilled"
    );
    assert_eq!(
        HookEvent::TokenUsageUpdated { total_tokens: 5000, max_tokens: 128000 }.event_name(),
        "token_usage_updated"
    );
}

#[test]
fn test_agent_events() {
    assert_eq!(
        HookEvent::AgentSpawned { agent_id: "a1".into(), task: "review".into() }.event_name(),
        "agent_spawned"
    );
    assert_eq!(
        HookEvent::AgentMessage { agent_id: "a1".into(), message: "done".into() }.event_name(),
        "agent_message"
    );
    assert_eq!(
        HookEvent::AgentCompleted { agent_id: "a1".into(), duration_ms: 3000 }.event_name(),
        "agent_completed"
    );
    assert_eq!(
        HookEvent::AgentFailed { agent_id: "a1".into(), error: "timeout".into() }.event_name(),
        "agent_failed"
    );
    assert_eq!(
        HookEvent::AgentCancelled { agent_id: "a1".into(), reason: "user".into() }.event_name(),
        "agent_cancelled"
    );
}

#[test]
fn test_sandbox_permission_events() {
    assert_eq!(
        HookEvent::SandboxDecision { tool_name: "bash".into(), allowed: true, reason: "policy".into() }.event_name(),
        "sandbox_decision"
    );
    assert_eq!(
        HookEvent::PermissionDecision { action: "file_write".into(), allowed: false, reason: "denied".into() }.event_name(),
        "permission_decision"
    );
}

#[test]
fn test_auth_events() {
    assert_eq!(
        HookEvent::AuthChanged { provider: "openai".into() }.event_name(),
        "auth_changed"
    );
    assert_eq!(
        HookEvent::AuthFailed { provider: "openai".into(), error: "expired".into() }.event_name(),
        "auth_failed"
    );
    assert_eq!(
        HookEvent::ProviderSwitched { from: "openai".into(), to: "anthropic".into() }.event_name(),
        "provider_switched"
    );
}

#[test]
fn test_new_mcp_events() {
    assert_eq!(
        HookEvent::McpServerLaunch { server_name: "fs".into() }.event_name(),
        "mcp_server_launch"
    );
    assert_eq!(
        HookEvent::McpServerRestart { server_name: "fs".into(), reason: "crash".into() }.event_name(),
        "mcp_server_restart"
    );
    assert_eq!(
        HookEvent::McpServerCrash { server_name: "fs".into(), error: "segfault".into() }.event_name(),
        "mcp_server_crash"
    );
    assert_eq!(
        HookEvent::McpToolInvoked { server_name: "fs".into(), tool_name: "read".into() }.event_name(),
        "mcp_tool_invoked"
    );
}

#[test]
fn test_budget_events() {
    assert_eq!(
        HookEvent::BudgetThresholdWarning { current_cost: 4.50, threshold: 5.00 }.event_name(),
        "budget_threshold_warning"
    );
    assert_eq!(
        HookEvent::BudgetThresholdReached { current_cost: 5.00, limit: 5.00 }.event_name(),
        "budget_threshold_reached"
    );
    assert_eq!(
        HookEvent::CostIncurred { model: "gpt-4".into(), cost_usd: 0.03, tokens: 1000 }.event_name(),
        "cost_incurred"
    );
}

#[test]
fn test_background_job_event() {
    assert_eq!(
        HookEvent::BackgroundJobStateChanged { job_id: "j1".into(), state: "completed".into() }.event_name(),
        "background_job_state_changed"
    );
}

#[test]
fn test_command_ui_events() {
    assert_eq!(
        HookEvent::CommandInvoked { command: "/test".into() }.event_name(),
        "command_invoked"
    );
    assert_eq!(
        HookEvent::PaletteCommandExecuted { command: "toggle_theme".into() }.event_name(),
        "palette_command_executed"
    );
}

#[test]
fn test_diagnostic_events() {
    assert_eq!(
        HookEvent::UnhandledError { error: "panic".into(), context: "tool_dispatch".into() }.event_name(),
        "unhandled_error"
    );
}

#[test]
fn test_all_new_override_classes() {
    // Message flow
    assert_eq!(HookEvent::UserMessageReceived { message_len: 1 }.override_class(), OverrideClass::Safe);
    assert_eq!(HookEvent::MessageRetry { reason: "r".into(), attempt: 1 }.override_class(), OverrideClass::Guarded);

    // Model
    assert_eq!(HookEvent::BeforeModelSelect { candidates: vec![] }.override_class(), OverrideClass::Guarded);
    assert_eq!(HookEvent::RouterDecision { model: "m".into(), reason: "r".into() }.override_class(), OverrideClass::Safe);
    assert_eq!(HookEvent::ModelRateLimited { model: "m".into(), retry_after_ms: None }.override_class(), OverrideClass::Safe);

    // Tool specific
    assert_eq!(HookEvent::BeforeApplyPatch { file_path: "f".into(), patch_summary: "s".into() }.override_class(), OverrideClass::Guarded);
    assert_eq!(HookEvent::AfterApplyPatch { file_path: "f".into(), lines_changed: 0 }.override_class(), OverrideClass::Safe);
    assert_eq!(HookEvent::BeforeRunCmd { command: "c".into() }.override_class(), OverrideClass::Guarded);
    assert_eq!(HookEvent::AfterRunCmd { command: "c".into(), exit_code: 0, duration_ms: 0 }.override_class(), OverrideClass::Safe);
    assert_eq!(HookEvent::BeforeFileRead { path: "p".into() }.override_class(), OverrideClass::Guarded);
    assert_eq!(HookEvent::BeforeFileWrite { path: "p".into() }.override_class(), OverrideClass::Guarded);

    // Budget
    assert_eq!(HookEvent::BudgetThresholdReached { current_cost: 0.0, limit: 0.0 }.override_class(), OverrideClass::Guarded);
    assert_eq!(HookEvent::BudgetThresholdWarning { current_cost: 0.0, threshold: 0.0 }.override_class(), OverrideClass::Safe);
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --package ucode-plugins -- hooks`
Expected: compilation errors (variants don't exist yet)

**Step 3: Add all 42 new HookEvent variants**

Add the new variants to the `HookEvent` enum, add arms to `event_name()` and `override_class()`. The full list of new variants (grouped by category):

Session: `SessionTitleGenerated`, `SessionTitleUpdated`, `ConfigReloaded`
Message: `UserMessageReceived`, `AssistantResponseStarted`, `AssistantResponseCompleted`, `MessageRetry`
Model: `BeforeModelSelect`, `RouterDecision`, `ModelRateLimited`, `ModelQuotaExhausted`
Tool specific: `ToolTimeout`, `BeforeApplyPatch`, `AfterApplyPatch`, `BeforeRunCmd`, `AfterRunCmd`, `BeforeFileRead`, `AfterFileRead`, `BeforeFileWrite`, `AfterFileWrite`
Context: `ContextDistilled`, `TokenUsageUpdated`
Agent: `AgentSpawned`, `AgentMessage`, `AgentCompleted`, `AgentFailed`, `AgentCancelled`
Approval: `SandboxDecision`, `PermissionDecision`
Auth: `AuthChanged`, `AuthFailed`, `ProviderSwitched`
MCP: `McpServerLaunch`, `McpServerRestart`, `McpServerCrash`, `McpToolInvoked`
Budget: `BudgetThresholdWarning`, `BudgetThresholdReached`, `CostIncurred`
Background: `BackgroundJobStateChanged`
Commands: `CommandInvoked`, `PaletteCommandExecuted`
Diagnostics: `UnhandledError`

Each variant needs fields, `event_name()` arm, and `override_class()` arm.

**Step 4: Run tests**

Run: `cargo test --package ucode-plugins -- hooks`
Expected: all hook tests pass (existing + new)

**Step 5: Run clippy**

Run: `cargo clippy --package ucode-plugins --all-targets -- -D warnings`
Expected: 0 warnings

**Step 6: Commit**

```bash
git add crates/ucode-plugins/src/hooks.rs
git commit -m "feat: expand HookEvent from 22 to 64 variants (ISSUE 0803)"
```

---

### Task 4: Plugin API traits — Feature, HandshakeRequest/Response, Plugin/HookHandler/ToolProvider

**Files:**
- Create: `crates/ucode-plugins/src/api.rs`
- Modify: `crates/ucode-plugins/src/lib.rs`

**Step 1: Write failing tests for API types**

Create `crates/ucode-plugins/src/api.rs` with test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::PluginCapabilities;

    #[test]
    fn test_feature_serde_roundtrip() {
        for (feature, expected) in [
            (Feature::Hooks, "\"hooks\""),
            (Feature::Tools, "\"tools\""),
            (Feature::Ui, "\"ui\""),
        ] {
            let json = serde_json::to_string(&feature).unwrap();
            assert_eq!(json, expected);
            let back: Feature = serde_json::from_str(&json).unwrap();
            assert_eq!(back, feature);
        }
    }

    #[test]
    fn test_handshake_version_compatible() {
        let host = semver::Version::new(1, 2, 0);
        let required = semver::Version::new(1, 0, 0);
        assert!(check_version_compatible(&host, &required).is_ok());
    }

    #[test]
    fn test_handshake_version_incompatible_major() {
        let host = semver::Version::new(2, 0, 0);
        let required = semver::Version::new(1, 0, 0);
        assert!(matches!(
            check_version_compatible(&host, &required),
            Err(HandshakeError::VersionIncompatible { .. })
        ));
    }

    #[test]
    fn test_handshake_version_host_too_old() {
        let host = semver::Version::new(1, 0, 0);
        let required = semver::Version::new(1, 2, 0);
        assert!(matches!(
            check_version_compatible(&host, &required),
            Err(HandshakeError::VersionIncompatible { .. })
        ));
    }

    #[test]
    fn test_handshake_features_compatible() {
        let required: HashSet<Feature> = [Feature::Hooks, Feature::Tools].into();
        let supported: HashSet<Feature> = [Feature::Hooks, Feature::Tools, Feature::Ui].into();
        assert!(check_features_compatible(&required, &supported).is_ok());
    }

    #[test]
    fn test_handshake_features_missing() {
        let required: HashSet<Feature> = [Feature::Hooks, Feature::Tools].into();
        let supported: HashSet<Feature> = [Feature::Hooks].into();
        let err = check_features_compatible(&required, &supported).unwrap_err();
        match err {
            HandshakeError::UnsupportedFeatures { missing } => {
                assert_eq!(missing, vec![Feature::Tools]);
            }
            _ => panic!("wrong error variant"),
        }
    }

    #[test]
    fn test_hook_response_variants() {
        let ok = HookResponse::Ok;
        assert!(matches!(ok, HookResponse::Ok));

        let modify = HookResponse::Modify { changes: serde_json::json!({"key": "val"}) };
        assert!(matches!(modify, HookResponse::Modify { .. }));

        let veto = HookResponse::Veto { reason: "blocked".into() };
        assert!(matches!(veto, HookResponse::Veto { .. }));
    }

    #[test]
    fn test_api_version_constant() {
        let v: semver::Version = API_VERSION.parse().unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 0);
        assert_eq!(v.patch, 0);
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --package ucode-plugins -- api`
Expected: compilation errors (module doesn't exist yet)

**Step 3: Implement API types and traits**

Write the full `api.rs` module with:
- `API_VERSION` constant
- `Feature` enum with serde
- `HandshakeRequest`, `HandshakeResponse`, `HandshakeError` types
- `HookResponse` enum
- `check_version_compatible()` and `check_features_compatible()` helper functions
- `Plugin` trait, `HookHandler` trait, `ToolProvider` trait

**Step 4: Update lib.rs to export new module**

Add `pub mod api;` and re-exports.

**Step 5: Run tests**

Run: `cargo test --package ucode-plugins -- api`
Expected: all API tests pass

**Step 6: Run clippy**

Run: `cargo clippy --package ucode-plugins --all-targets -- -D warnings`
Expected: 0 warnings

**Step 7: Commit**

```bash
git add crates/ucode-plugins/src/api.rs crates/ucode-plugins/src/lib.rs
git commit -m "feat: Plugin/HookHandler/ToolProvider traits and handshake protocol (ISSUE 0803)"
```

---

### Task 5: PluginHost — load, handshake, dispatch, tool registration

**Files:**
- Create: `crates/ucode-plugins/src/host.rs`
- Modify: `crates/ucode-plugins/src/lib.rs`

**Step 1: Write failing tests for PluginHost**

Create `crates/ucode-plugins/src/host.rs` with test module:

```rust
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
            Self { id: id.to_string(), events: vec![], initialized: false }
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
        assert!(host.load(Box::new(plugin)).is_ok());
        assert_eq!(host.loaded_count(), 1);
    }

    #[test]
    fn test_load_plugin_version_mismatch() {
        // Plugin requiring API 2.0.0 should fail against host 1.0.0
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
            fn initialize(&mut self, _: &HandshakeResponse) -> Result<(), String> { Ok(()) }
            fn shutdown(&mut self) {}
        }
        let mut host = PluginHost::new();
        let err = host.load(Box::new(FuturePlugin)).unwrap_err();
        assert!(matches!(err, HandshakeError::VersionIncompatible { .. }));
    }

    #[test]
    fn test_dispatch_hook_to_plugin() {
        let mut host = PluginHost::new();
        let plugin = TestPlugin::new("org.test.logger");
        host.load(Box::new(plugin)).unwrap();
        let results = host.dispatch_hook(HookEvent::SessionStart { session_id: "s1".into() });
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].plugin_id, "org.test.logger");
        assert!(matches!(results[0].response, HookResponse::Ok));
    }

    #[test]
    fn test_unload_plugin() {
        let mut host = PluginHost::new();
        let plugin = TestPlugin::new("org.test.logger");
        host.load(Box::new(plugin)).unwrap();
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
        struct ToolPlugin;
        impl Plugin for ToolPlugin {
            fn handshake(&self) -> HandshakeRequest {
                HandshakeRequest {
                    plugin_id: "org.acme.tools".into(),
                    plugin_version: semver::Version::new(1, 0, 0),
                    min_api_version: semver::Version::new(1, 0, 0),
                    required_features: [Feature::Hooks, Feature::Tools].into(),
                    capabilities: PluginCapabilities::default(),
                }
            }
            fn initialize(&mut self, _: &HandshakeResponse) -> Result<(), String> { Ok(()) }
            fn shutdown(&mut self) {}
        }
        impl HookHandler for ToolPlugin {
            fn on_event(&mut self, _: &HookRecord) -> HookResponse { HookResponse::Ok }
        }
        impl ToolProvider for ToolPlugin {
            fn tool_specs(&self) -> Vec<crate::PluginToolDef> {
                vec![crate::PluginToolDef {
                    name: "lint".into(),
                    description: Some("Run linter".into()),
                    input_schema: None,
                }]
            }
            fn invoke_tool(&mut self, name: &str, _args: serde_json::Value) -> Result<serde_json::Value, String> {
                Ok(serde_json::json!({ "tool": name, "status": "ok" }))
            }
        }
        let mut host = PluginHost::new();
        host.load_with_tools(Box::new(ToolPlugin), Box::new(ToolPlugin)).unwrap();
        let tools = host.plugin_tools();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].0, "org.acme.tools.lint");
    }

    #[test]
    fn test_invoke_plugin_tool() {
        // Similar to above but invoke the tool
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
            fn initialize(&mut self, _: &HandshakeResponse) -> Result<(), String> { Ok(()) }
            fn shutdown(&mut self) {}
        }
        impl ToolProvider for EchoPlugin {
            fn tool_specs(&self) -> Vec<crate::PluginToolDef> {
                vec![crate::PluginToolDef {
                    name: "echo".into(),
                    description: Some("Echo input".into()),
                    input_schema: None,
                }]
            }
            fn invoke_tool(&mut self, _name: &str, args: serde_json::Value) -> Result<serde_json::Value, String> {
                Ok(args)
            }
        }
        let mut host = PluginHost::new();
        host.load_with_tools(Box::new(EchoPlugin), Box::new(EchoPlugin)).unwrap();
        let result = host.invoke_plugin_tool("org.test.echo.echo", serde_json::json!({"msg": "hi"}));
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), serde_json::json!({"msg": "hi"}));
    }

    #[test]
    fn test_invoke_unknown_tool() {
        let mut host = PluginHost::new();
        let result = host.invoke_plugin_tool("org.ghost.tool", serde_json::json!({}));
        assert!(result.is_err());
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --package ucode-plugins -- host`
Expected: compilation errors (module doesn't exist yet)

**Step 3: Implement PluginHost**

Write `host.rs` with:
- `LoadedPlugin` struct (holds `Box<dyn Plugin>`, optional `Box<dyn HookHandler>`, optional tool provider, hook subscriptions)
- `PluginHost` struct with `load()`, `load_with_tools()`, `unload()`, `dispatch_hook()`, `plugin_tools()`, `invoke_plugin_tool()`, `loaded_count()`
- Handshake validation using `check_version_compatible()` and `check_features_compatible()` from `api.rs`
- Tool FQN construction: `{plugin_id}.{tool_name}`
- Hook dispatch: iterate loaded plugins with hook handlers, call `on_event()`, collect results

Note: `load()` is for plugins with hooks only. `load_with_tools()` accepts both a `Plugin + HookHandler` and a `ToolProvider`. This avoids trait object issues with multiple traits. Alternative: use a single `PluginBundle` struct that holds optional trait objects.

**Step 4: Update lib.rs**

Add `pub mod host;` and re-exports for `PluginHost`, `HookResult`, `LoadedPlugin`.

**Step 5: Run tests**

Run: `cargo test --package ucode-plugins -- host`
Expected: all host tests pass

**Step 6: Run clippy**

Run: `cargo clippy --package ucode-plugins --all-targets -- -D warnings`
Expected: 0 warnings

**Step 7: Commit**

```bash
git add crates/ucode-plugins/src/host.rs crates/ucode-plugins/src/lib.rs
git commit -m "feat: PluginHost with load/unload/dispatch/tool registration (ISSUE 0803)"
```

---

### Task 6: Full workspace verification and final commit

**Step 1: Run full workspace tests**

Run: `cargo test --workspace`
Expected: all tests pass, 0 failures

**Step 2: Run full workspace clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: 0 warnings

**Step 3: Count plugin tests**

Run: `cargo test --package ucode-plugins 2>&1 | grep "test result:"`
Expected: significant increase from current baseline

**Step 4: Update EPIC.md with completion notes**

Mark ISSUE 0803 as DONE with test count and summary.

**Step 5: Update PLANS.md Task 8.3 as DONE**

**Step 6: Commit**

```bash
git add EPIC.md PLANS.md
git commit -m "docs: mark Task 8.3 / ISSUE 0803 as DONE"
```
