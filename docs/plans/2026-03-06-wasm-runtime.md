# WASM Runtime Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add wasmtime-based WASM plugin support behind a `wasm` feature flag, with per-event WIT interfaces grouped into versioned category packages.

**Architecture:** Host uses `bindgen!` for type generation from WIT, low-level `Instance` API for dynamic export probing. Guest SDK uses `wit-bindgen` for typed exports. WASM plugins integrate with existing `PluginHost` via a `WasmPlugin` adapter.

**Tech Stack:** wasmtime 42 (component-model), wit-bindgen 0.53, wasm32-wasip2 target

**Design doc:** `docs/plans/2026-03-06-wasm-runtime-design.md`

---

### Task 1: Add wasmtime dependencies and `wasm` feature flag

**Files:**
- Modify: `Cargo.toml` (workspace deps)
- Modify: `crates/ucode-plugins/Cargo.toml` (optional deps + feature)

**Step 1: Add workspace dependencies**

Add to `Cargo.toml` `[workspace.dependencies]` section:

```toml
wasmtime = { version = "42", features = ["component-model"] }
wasmtime-wasi = "42"
```

**Step 2: Add optional deps and feature to ucode-plugins**

In `crates/ucode-plugins/Cargo.toml`:

```toml
[features]
default = []
wasm = ["dep:wasmtime", "dep:wasmtime-wasi"]

[dependencies]
# ... existing deps ...
wasmtime = { workspace = true, optional = true }
wasmtime-wasi = { workspace = true, optional = true }
```

**Step 3: Create empty wasm module**

Create `crates/ucode-plugins/src/wasm/mod.rs`:

```rust
//! WASM plugin runtime using wasmtime component model.
//!
//! Gated behind the `wasm` feature flag.
```

Add to `crates/ucode-plugins/src/lib.rs`:

```rust
#[cfg(feature = "wasm")]
pub mod wasm;
```

**Step 4: Verify compilation**

Run: `cargo check -p ucode-plugins` (without wasm -- should pass)
Run: `cargo check -p ucode-plugins --features wasm` (with wasm -- should pass)

**Step 5: Commit**

```
feat(plugins): add wasmtime deps and wasm feature flag (ISSUE 0804)
```

---

### Task 2: Write WIT shared types package

**Files:**
- Create: `crates/ucode-plugins/wit/deps/hooks-types/types.wit`

This is the foundation -- all payload records and the `hook-response` type that
every event interface depends on.

**Step 1: Create the WIT types file**

```wit
package ucode:hooks-types@1.0.0;

interface types {
    /// What a plugin returns from a hook handler.
    enum hook-response-kind {
        /// Observed, no action taken.
        ok,
        /// Propose modifications (Guarded events only).
        modify,
        /// Veto the action (Risky events only, requires approval).
        veto,
    }

    record hook-response {
        kind: hook-response-kind,
        /// JSON string for modify (changes) or veto (reason). None for ok.
        data: option<string>,
    }

    // --- Session payloads ---

    record session-start-payload {
        session-id: string,
    }

    record session-end-payload {
        session-id: string,
        duration-secs: f64,
    }

    record session-title-payload {
        session-id: string,
        title: string,
    }

    // --- Message payloads ---

    record user-message-payload {
        message-len: u32,
    }

    record response-started-payload {
        model: string,
    }

    record response-completed-payload {
        model: string,
        tokens: u32,
        duration-ms: u64,
    }

    record retry-payload {
        reason: string,
        attempt: u32,
    }

    // --- Model payloads ---

    record before-model-call-payload {
        model: string,
        message-count: u32,
    }

    record after-model-call-payload {
        model: string,
        tokens-used: u32,
        duration-ms: u64,
    }

    record before-model-select-payload {
        candidates: list<string>,
    }

    record model-fallback-payload {
        from-model: string,
        to-model: string,
        reason: string,
    }

    record router-decision-payload {
        model: string,
        reason: string,
    }

    record model-rate-limited-payload {
        model: string,
        retry-after-ms: option<u64>,
    }

    record model-quota-exhausted-payload {
        model: string,
    }

    // --- Tool payloads ---

    record before-tool-call-payload {
        tool-name: string,
        args: string,
    }

    record after-tool-call-payload {
        tool-name: string,
        result: string,
        duration-ms: u64,
    }

    record tool-error-payload {
        tool-name: string,
        error: string,
    }

    record tool-timeout-payload {
        tool-name: string,
        timeout-ms: u64,
    }

    // --- Tool FS payloads ---

    record before-file-read-payload {
        path: string,
    }

    record after-file-read-payload {
        path: string,
        size-bytes: u64,
    }

    record before-file-write-payload {
        path: string,
    }

    record after-file-write-payload {
        path: string,
        size-bytes: u64,
    }

    // --- Tool CMD payloads ---

    record before-run-cmd-payload {
        command: string,
    }

    record after-run-cmd-payload {
        command: string,
        exit-code: s32,
        duration-ms: u64,
    }

    // --- Tool Patch payloads ---

    record before-apply-patch-payload {
        file-path: string,
        patch-summary: string,
    }

    record after-apply-patch-payload {
        file-path: string,
        lines-changed: u32,
    }

    // --- Context payloads ---

    record context-overflow-payload {
        current-tokens: u32,
        max-tokens: u32,
    }

    record context-compaction-payload {
        before-tokens: u32,
        after-tokens: u32,
    }

    record context-distilled-payload {
        before-tokens: u32,
        after-tokens: u32,
    }

    record token-usage-payload {
        total-tokens: u32,
        max-tokens: u32,
    }

    // --- Agent payloads ---

    record agent-spawned-payload {
        agent-id: string,
        task: string,
    }

    record agent-message-payload {
        agent-id: string,
        message: string,
    }

    record agent-completed-payload {
        agent-id: string,
        duration-ms: u64,
    }

    record agent-failed-payload {
        agent-id: string,
        error: string,
    }

    record agent-cancelled-payload {
        agent-id: string,
        reason: string,
    }

    // --- Approval payloads ---

    record approval-required-payload {
        tool-name: string,
        risk-level: string,
    }

    record approval-granted-payload {
        tool-name: string,
    }

    record approval-denied-payload {
        tool-name: string,
        reason: string,
    }

    record sandbox-decision-payload {
        tool-name: string,
        allowed: bool,
        reason: string,
    }

    record permission-decision-payload {
        action: string,
        allowed: bool,
        reason: string,
    }

    // --- Auth payloads ---

    record auth-changed-payload {
        provider: string,
    }

    record auth-failed-payload {
        provider: string,
        error: string,
    }

    record provider-switched-payload {
        from: string,
        to: string,
    }

    // --- MCP payloads ---

    record mcp-server-payload {
        server-name: string,
    }

    record mcp-server-reason-payload {
        server-name: string,
        reason: string,
    }

    record mcp-server-error-payload {
        server-name: string,
        error: string,
    }

    record mcp-tool-invoked-payload {
        server-name: string,
        tool-name: string,
    }

    // --- Skill payloads ---

    record skill-payload {
        skill-name: string,
    }

    // --- Plugin payloads ---

    record plugin-payload {
        plugin-name: string,
    }

    record plugin-error-payload {
        plugin-name: string,
        error: string,
    }

    // --- Checkpoint payloads ---

    record checkpoint-payload {
        checkpoint-id: string,
    }

    // --- Budget payloads ---

    record budget-warning-payload {
        current-cost: f64,
        threshold: f64,
    }

    record budget-reached-payload {
        current-cost: f64,
        limit: f64,
    }

    record cost-incurred-payload {
        model: string,
        cost-usd: f64,
        tokens: u32,
    }

    // --- Job payloads ---

    record job-state-payload {
        job-id: string,
        state: string,
    }

    // --- Command payloads ---

    record command-payload {
        command: string,
    }

    // --- Diagnostic payloads ---

    record unhandled-error-payload {
        error: string,
        context: string,
    }

    // --- Plugin lifecycle payloads ---

    record handshake-request {
        plugin-id: string,
        plugin-version: string,
        min-api-version: string,
        required-features: list<string>,
    }

    variant handshake-result {
        accepted(string),
        rejected(string),
    }

    record tool-spec {
        name: string,
        description: option<string>,
        input-schema: option<string>,
    }
}
```

**Step 2: Verify WIT syntax**

Run: `wasm-tools parse crates/ucode-plugins/wit/deps/hooks-types/types.wit` (if wasm-tools installed)
Or just verify in next task when bindgen! parses it.

**Step 3: Commit**

```
feat(plugins): add WIT shared types package ucode:hooks-types@1.0.0 (ISSUE 0804)
```

---

### Task 3: Write WIT category packages (all 20)

**Files:**
- Create: `crates/ucode-plugins/wit/deps/hooks-session/hooks-session.wit`
- Create: `crates/ucode-plugins/wit/deps/hooks-message/hooks-message.wit`
- Create: `crates/ucode-plugins/wit/deps/hooks-model/hooks-model.wit`
- Create: `crates/ucode-plugins/wit/deps/hooks-tool/hooks-tool.wit`
- Create: `crates/ucode-plugins/wit/deps/hooks-tool-fs/hooks-tool-fs.wit`
- Create: `crates/ucode-plugins/wit/deps/hooks-tool-cmd/hooks-tool-cmd.wit`
- Create: `crates/ucode-plugins/wit/deps/hooks-tool-patch/hooks-tool-patch.wit`
- Create: `crates/ucode-plugins/wit/deps/hooks-context/hooks-context.wit`
- Create: `crates/ucode-plugins/wit/deps/hooks-agent/hooks-agent.wit`
- Create: `crates/ucode-plugins/wit/deps/hooks-approval/hooks-approval.wit`
- Create: `crates/ucode-plugins/wit/deps/hooks-auth/hooks-auth.wit`
- Create: `crates/ucode-plugins/wit/deps/hooks-mcp/hooks-mcp.wit`
- Create: `crates/ucode-plugins/wit/deps/hooks-skill/hooks-skill.wit`
- Create: `crates/ucode-plugins/wit/deps/hooks-plugin/hooks-plugin.wit`
- Create: `crates/ucode-plugins/wit/deps/hooks-checkpoint/hooks-checkpoint.wit`
- Create: `crates/ucode-plugins/wit/deps/hooks-budget/hooks-budget.wit`
- Create: `crates/ucode-plugins/wit/deps/hooks-job/hooks-job.wit`
- Create: `crates/ucode-plugins/wit/deps/hooks-command/hooks-command.wit`
- Create: `crates/ucode-plugins/wit/deps/hooks-diagnostic/hooks-diagnostic.wit`
- Create: `crates/ucode-plugins/wit/deps/plugin/plugin.wit`

Each category package follows the same pattern. Example for hooks-session:

```wit
package ucode:hooks-session@1.0.0;

interface on-start {
    use ucode:hooks-types/types.{hook-response, session-start-payload};
    handle: func(payload: session-start-payload) -> hook-response;
}

interface on-end {
    use ucode:hooks-types/types.{hook-response, session-end-payload};
    handle: func(payload: session-end-payload) -> hook-response;
}

interface on-title-generated {
    use ucode:hooks-types/types.{hook-response, session-title-payload};
    handle: func(payload: session-title-payload) -> hook-response;
}

interface on-title-updated {
    use ucode:hooks-types/types.{hook-response, session-title-payload};
    handle: func(payload: session-title-payload) -> hook-response;
}

interface on-config-reloaded {
    use ucode:hooks-types/types.{hook-response};
    handle: func() -> hook-response;
}
```

The plugin lifecycle package:

```wit
package ucode:plugin@1.0.0;

interface lifecycle {
    use ucode:hooks-types/types.{handshake-request, handshake-result};
    handshake: func() -> handshake-request;
    initialize: func(result: handshake-result) -> result<_, string>;
    shutdown: func();
}

interface tool-provider {
    use ucode:hooks-types/types.{tool-spec};
    tool-specs: func() -> list<tool-spec>;
    invoke-tool: func(name: string, args: string) -> result<string, string>;
}

interface host-log {
    log: func(msg: string);
}
```

**Step 1: Create all 20 WIT files**

Write each file following the pattern above. Full content for each file is
mechanical -- each interface has one `handle` function taking the appropriate
payload and returning `hook-response`.

**Step 2: Create the maximal world for bindgen! type generation**

Create `crates/ucode-plugins/wit/world.wit`:

```wit
package ucode:plugin-world@1.0.0;

world maximal-plugin {
    // Host provides
    import ucode:plugin/host-log;

    // Lifecycle (required)
    export ucode:plugin/lifecycle;

    // Tools (optional)
    export ucode:plugin/tool-provider;

    // All hook categories (all optional in practice)
    export ucode:hooks-session/on-start;
    export ucode:hooks-session/on-end;
    export ucode:hooks-session/on-title-generated;
    export ucode:hooks-session/on-title-updated;
    export ucode:hooks-session/on-config-reloaded;

    export ucode:hooks-message/on-user-message;
    export ucode:hooks-message/on-response-started;
    export ucode:hooks-message/on-response-completed;
    export ucode:hooks-message/on-retry;

    export ucode:hooks-model/on-before-call;
    export ucode:hooks-model/on-after-call;
    export ucode:hooks-model/on-before-select;
    export ucode:hooks-model/on-fallback;
    export ucode:hooks-model/on-router-decision;
    export ucode:hooks-model/on-rate-limited;
    export ucode:hooks-model/on-quota-exhausted;

    export ucode:hooks-tool/on-before-call;
    export ucode:hooks-tool/on-after-call;
    export ucode:hooks-tool/on-error;
    export ucode:hooks-tool/on-timeout;

    export ucode:hooks-tool-fs/on-before-read;
    export ucode:hooks-tool-fs/on-after-read;
    export ucode:hooks-tool-fs/on-before-write;
    export ucode:hooks-tool-fs/on-after-write;

    export ucode:hooks-tool-cmd/on-before-run;
    export ucode:hooks-tool-cmd/on-after-run;

    export ucode:hooks-tool-patch/on-before-apply;
    export ucode:hooks-tool-patch/on-after-apply;

    export ucode:hooks-context/on-overflow;
    export ucode:hooks-context/on-compaction;
    export ucode:hooks-context/on-distilled;
    export ucode:hooks-context/on-usage-updated;

    export ucode:hooks-agent/on-spawned;
    export ucode:hooks-agent/on-message;
    export ucode:hooks-agent/on-completed;
    export ucode:hooks-agent/on-failed;
    export ucode:hooks-agent/on-cancelled;

    export ucode:hooks-approval/on-required;
    export ucode:hooks-approval/on-granted;
    export ucode:hooks-approval/on-denied;
    export ucode:hooks-approval/on-sandbox-decision;
    export ucode:hooks-approval/on-permission-decision;

    export ucode:hooks-auth/on-changed;
    export ucode:hooks-auth/on-failed;
    export ucode:hooks-auth/on-provider-switched;

    export ucode:hooks-mcp/on-connected;
    export ucode:hooks-mcp/on-disconnected;
    export ucode:hooks-mcp/on-launch;
    export ucode:hooks-mcp/on-restart;
    export ucode:hooks-mcp/on-crash;
    export ucode:hooks-mcp/on-tool-invoked;

    export ucode:hooks-skill/on-activated;
    export ucode:hooks-skill/on-deactivated;

    export ucode:hooks-plugin/on-loaded;
    export ucode:hooks-plugin/on-unloaded;
    export ucode:hooks-plugin/on-error;

    export ucode:hooks-checkpoint/on-created;
    export ucode:hooks-checkpoint/on-restored;

    export ucode:hooks-budget/on-warning;
    export ucode:hooks-budget/on-reached;
    export ucode:hooks-budget/on-cost-incurred;

    export ucode:hooks-job/on-state-changed;

    export ucode:hooks-command/on-invoked;
    export ucode:hooks-command/on-palette-executed;

    export ucode:hooks-diagnostic/on-unhandled-error;
}
```

**Step 3: Commit**

```
feat(plugins): add WIT category packages for all 64 hook events (ISSUE 0804)
```

---

### Task 4: Host-side bindgen! for type generation

**Files:**
- Modify: `crates/ucode-plugins/src/wasm/mod.rs`

**Step 1: Add bindgen! invocation**

```rust
//! WASM plugin runtime using wasmtime component model.

mod convert;
mod host;

pub use host::WasmPluginHost;

// Generate Rust types from WIT definitions.
// We use these types for payload conversion but NOT the generated
// instantiate() method (which requires all exports to be present).
wasmtime::component::bindgen!({
    path: "wit",
    world: "maximal-plugin",
});
```

**Step 2: Verify bindgen! compiles**

Run: `cargo check -p ucode-plugins --features wasm`

If there are WIT parse errors, fix them in the .wit files and re-check.

**Step 3: Commit**

```
feat(plugins): host-side bindgen! type generation from WIT (ISSUE 0804)
```

---

### Task 5: HookEvent <-> WIT payload conversion

**Files:**
- Create: `crates/ucode-plugins/src/wasm/convert.rs`

This module converts between our Rust `HookEvent` enum and the WIT-generated
payload types. Each conversion is a simple field-by-field mapping.

**Step 1: Write the conversion module**

```rust
//! Conversion between HookEvent and WIT-generated payload types.

use crate::hooks::HookEvent;

// Import the WIT-generated types from bindgen!
use super::ucode::hooks_types::types as wit;

/// Convert a HookEvent to its WIT event name and serialized payload.
///
/// Returns `(wit_interface_path, payload)` where `wit_interface_path` is
/// the fully-qualified WIT interface name like
/// "ucode:hooks-session/on-start@1.0.0".
///
/// The payload is the WIT-generated struct, but since we use dynamic
/// dispatch we serialize to the component model's canonical ABI at call
/// time. Here we just produce the typed value.
pub enum WitPayload {
    SessionStart(wit::SessionStartPayload),
    SessionEnd(wit::SessionEndPayload),
    SessionTitleGenerated(wit::SessionTitlePayload),
    SessionTitleUpdated(wit::SessionTitlePayload),
    ConfigReloaded,
    UserMessage(wit::UserMessagePayload),
    ResponseStarted(wit::ResponseStartedPayload),
    ResponseCompleted(wit::ResponseCompletedPayload),
    Retry(wit::RetryPayload),
    BeforeModelCall(wit::BeforeModelCallPayload),
    AfterModelCall(wit::AfterModelCallPayload),
    BeforeModelSelect(wit::BeforeModelSelectPayload),
    ModelFallback(wit::ModelFallbackPayload),
    RouterDecision(wit::RouterDecisionPayload),
    ModelRateLimited(wit::ModelRateLimitedPayload),
    ModelQuotaExhausted(wit::ModelQuotaExhaustedPayload),
    BeforeToolCall(wit::BeforeToolCallPayload),
    AfterToolCall(wit::AfterToolCallPayload),
    ToolError(wit::ToolErrorPayload),
    ToolTimeout(wit::ToolTimeoutPayload),
    BeforeFileRead(wit::BeforeFileReadPayload),
    AfterFileRead(wit::AfterFileReadPayload),
    BeforeFileWrite(wit::BeforeFileWritePayload),
    AfterFileWrite(wit::AfterFileWritePayload),
    BeforeRunCmd(wit::BeforeRunCmdPayload),
    AfterRunCmd(wit::AfterRunCmdPayload),
    BeforeApplyPatch(wit::BeforeApplyPatchPayload),
    AfterApplyPatch(wit::AfterApplyPatchPayload),
    ContextOverflow(wit::ContextOverflowPayload),
    ContextCompaction(wit::ContextCompactionPayload),
    ContextDistilled(wit::ContextDistilledPayload),
    TokenUsageUpdated(wit::TokenUsagePayload),
    AgentSpawned(wit::AgentSpawnedPayload),
    AgentMessage(wit::AgentMessagePayload),
    AgentCompleted(wit::AgentCompletedPayload),
    AgentFailed(wit::AgentFailedPayload),
    AgentCancelled(wit::AgentCancelledPayload),
    ApprovalRequired(wit::ApprovalRequiredPayload),
    ApprovalGranted(wit::ApprovalGrantedPayload),
    ApprovalDenied(wit::ApprovalDeniedPayload),
    SandboxDecision(wit::SandboxDecisionPayload),
    PermissionDecision(wit::PermissionDecisionPayload),
    AuthChanged(wit::AuthChangedPayload),
    AuthFailed(wit::AuthFailedPayload),
    ProviderSwitched(wit::ProviderSwitchedPayload),
    McpConnected(wit::McpServerPayload),
    McpDisconnected(wit::McpServerReasonPayload),
    McpLaunch(wit::McpServerPayload),
    McpRestart(wit::McpServerReasonPayload),
    McpCrash(wit::McpServerErrorPayload),
    McpToolInvoked(wit::McpToolInvokedPayload),
    SkillActivated(wit::SkillPayload),
    SkillDeactivated(wit::SkillPayload),
    PluginLoaded(wit::PluginPayload),
    PluginUnloaded(wit::PluginPayload),
    PluginError(wit::PluginErrorPayload),
    CheckpointCreated(wit::CheckpointPayload),
    CheckpointRestored(wit::CheckpointPayload),
    BudgetWarning(wit::BudgetWarningPayload),
    BudgetReached(wit::BudgetReachedPayload),
    CostIncurred(wit::CostIncurredPayload),
    JobStateChanged(wit::JobStatePayload),
    CommandInvoked(wit::CommandPayload),
    PaletteExecuted(wit::CommandPayload),
    UnhandledError(wit::UnhandledErrorPayload),
}

/// The WIT interface path for each event, used for export probing.
impl WitPayload {
    pub fn wit_interface(&self) -> &'static str {
        match self {
            Self::SessionStart(_) => "ucode:hooks-session/on-start@1.0.0",
            Self::SessionEnd(_) => "ucode:hooks-session/on-end@1.0.0",
            Self::SessionTitleGenerated(_) => "ucode:hooks-session/on-title-generated@1.0.0",
            Self::SessionTitleUpdated(_) => "ucode:hooks-session/on-title-updated@1.0.0",
            Self::ConfigReloaded => "ucode:hooks-session/on-config-reloaded@1.0.0",
            // ... all 64 mappings
            _ => todo!(),
        }
    }
}

/// Convert HookEvent -> WitPayload.
pub fn hook_event_to_wit(event: &HookEvent) -> WitPayload {
    match event {
        HookEvent::SessionStart { session_id } => {
            WitPayload::SessionStart(wit::SessionStartPayload {
                session_id: session_id.clone(),
            })
        }
        HookEvent::SessionEnd { session_id, duration_secs } => {
            WitPayload::SessionEnd(wit::SessionEndPayload {
                session_id: session_id.clone(),
                duration_secs: *duration_secs,
            })
        }
        // ... all 64 conversions (mechanical field-by-field mapping)
        _ => todo!(),
    }
}

/// Convert WIT hook-response -> our HookResponse.
pub fn wit_response_to_hook(resp: wit::HookResponse) -> crate::api::HookResponse {
    match resp.kind {
        wit::HookResponseKind::Ok => crate::api::HookResponse::Ok,
        wit::HookResponseKind::Modify => crate::api::HookResponse::Modify {
            changes: resp.data
                .map(|d| serde_json::from_str(&d).unwrap_or(serde_json::Value::Null))
                .unwrap_or(serde_json::Value::Null),
        },
        wit::HookResponseKind::Veto => crate::api::HookResponse::Veto {
            reason: resp.data.unwrap_or_default(),
        },
    }
}
```

**Step 2: Write tests for conversion**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_start_conversion() {
        let event = HookEvent::SessionStart { session_id: "s1".into() };
        let wit = hook_event_to_wit(&event);
        assert_eq!(wit.wit_interface(), "ucode:hooks-session/on-start@1.0.0");
    }

    #[test]
    fn test_wit_response_ok() {
        let resp = wit::HookResponse {
            kind: wit::HookResponseKind::Ok,
            data: None,
        };
        assert!(matches!(wit_response_to_hook(resp), crate::api::HookResponse::Ok));
    }

    #[test]
    fn test_wit_response_veto() {
        let resp = wit::HookResponse {
            kind: wit::HookResponseKind::Veto,
            data: Some("blocked".into()),
        };
        match wit_response_to_hook(resp) {
            crate::api::HookResponse::Veto { reason } => assert_eq!(reason, "blocked"),
            _ => panic!("expected Veto"),
        }
    }
}
```

**Step 3: Verify**

Run: `cargo test -p ucode-plugins --features wasm`

**Step 4: Commit**

```
feat(plugins): HookEvent <-> WIT payload conversion (ISSUE 0804)
```

---

### Task 6: WasmPluginHost with dynamic export probing

**Files:**
- Create: `crates/ucode-plugins/src/wasm/host.rs`

This is the core: load a .wasm component, probe which interfaces it exports,
build a dispatch table, and wrap it as a `Plugin` + `HookHandler`.

**Step 1: Write WasmPluginHost**

```rust
//! WASM plugin host using wasmtime component model.

use std::collections::HashMap;
use std::path::Path;

use wasmtime::component::{Component, Linker, Val, Type};
use wasmtime::{Engine, Store, Config};

use crate::api::{HookHandler, HookResponse, Plugin, HandshakeRequest, HandshakeResponse};
use crate::hooks::HookRecord;

use super::convert;

/// Configuration for the WASM engine.
pub struct WasmEngineConfig {
    /// Maximum memory in bytes a plugin can use.
    pub max_memory_bytes: usize,
    /// Maximum execution fuel (instruction count limit).
    pub max_fuel: u64,
}

impl Default for WasmEngineConfig {
    fn default() -> Self {
        Self {
            max_memory_bytes: 64 * 1024 * 1024, // 64 MB
            max_fuel: 1_000_000_000,              // 1B instructions
        }
    }
}

/// Host state passed to WASM plugins via imports.
struct HostState {
    log_messages: Vec<String>,
}

/// A loaded WASM plugin component with its dispatch table.
pub struct WasmPlugin {
    store: Store<HostState>,
    instance: wasmtime::component::Instance,
    /// Maps event name -> ComponentExportIndex for the interface's "handle" func.
    dispatch_table: HashMap<&'static str, wasmtime::component::ComponentExportIndex>,
    /// Plugin ID from handshake.
    plugin_id: String,
}

/// Registry of known WIT interface paths and their corresponding event names.
///
/// Each entry: (event_name, wit_interface_path)
const EVENT_INTERFACES: &[(&str, &str)] = &[
    ("session_start", "ucode:hooks-session/on-start@1.0.0"),
    ("session_end", "ucode:hooks-session/on-end@1.0.0"),
    ("session_title_generated", "ucode:hooks-session/on-title-generated@1.0.0"),
    ("session_title_updated", "ucode:hooks-session/on-title-updated@1.0.0"),
    ("config_reloaded", "ucode:hooks-session/on-config-reloaded@1.0.0"),
    // ... all 64 entries
];

impl WasmPlugin {
    /// Load a WASM component from a file path.
    pub fn from_file(
        path: &Path,
        config: &WasmEngineConfig,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let mut engine_config = Config::new();
        engine_config.wasm_component_model(true);
        engine_config.consume_fuel(true);
        let engine = Engine::new(&engine_config)?;

        let component = Component::from_file(&engine, path)?;

        let mut linker = Linker::new(&engine);

        // Add host-log import
        // linker.root().func_wrap("host-log", |mut caller, msg: &str| { ... })?;

        let mut store = Store::new(
            &engine,
            HostState {
                log_messages: Vec::new(),
            },
        );
        store.set_fuel(config.max_fuel)?;

        let instance = linker.instantiate(&mut store, &component)?;

        // Probe which hook interfaces this component exports
        let mut dispatch_table = HashMap::new();
        for &(event_name, wit_iface) in EVENT_INTERFACES {
            // Try to find the interface export
            if let Some((_ty, iface_idx)) = component.get_export(None, wit_iface) {
                // Try to find the "handle" function within the interface
                if let Some((_ty, func_idx)) = component.get_export(Some(&iface_idx), "handle") {
                    dispatch_table.insert(event_name, func_idx);
                }
            }
        }

        // TODO: call lifecycle/handshake

        Ok(Self {
            store,
            instance,
            dispatch_table,
            plugin_id: String::new(), // filled by handshake
        })
    }

    /// Check if this plugin handles a given event.
    pub fn handles_event(&self, event_name: &str) -> bool {
        self.dispatch_table.contains_key(event_name)
    }

    /// List all events this plugin handles.
    pub fn subscribed_events(&self) -> Vec<&'static str> {
        self.dispatch_table.keys().copied().collect()
    }

    /// Dispatch a hook event to this plugin.
    /// Returns None if the plugin doesn't handle this event.
    pub fn dispatch(
        &mut self,
        event_name: &str,
    ) -> Option<Result<HookResponse, Box<dyn std::error::Error>>> {
        let export_idx = self.dispatch_table.get(event_name)?;
        let func = self.instance.get_func(&mut self.store, export_idx)?;

        // Call the function with the appropriate payload
        // The exact calling convention depends on the WIT signature
        // For now, use dynamic Val-based calling
        let mut results = vec![Val::Bool(false); 1]; // placeholder
        match func.call(&mut self.store, &[], &mut results) {
            Ok(()) => {
                func.post_return(&mut self.store).ok();
                // Convert results to HookResponse
                // TODO: proper result conversion
                Some(Ok(HookResponse::Ok))
            }
            Err(e) => Some(Err(e.into())),
        }
    }
}
```

Note: The exact calling convention (typed vs dynamic `Val`) will be refined
during implementation based on what bindgen! generates. The key pattern is:
1. `Component::get_export(None, wit_iface)` to find the interface
2. `Component::get_export(Some(&iface_idx), "handle")` to find the function
3. `Instance::get_func(&mut store, &export_idx)` to get the callable
4. `func.call(&mut store, &params, &mut results)` to invoke

**Step 2: Write unit tests**

Tests at this stage use inline WAT components or skip WASM loading and test
the dispatch table logic in isolation.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_interfaces_complete() {
        // Verify EVENT_INTERFACES has entries for all 64 events
        assert_eq!(EVENT_INTERFACES.len(), 64);
    }

    #[test]
    fn test_event_interfaces_unique_names() {
        let mut names: Vec<&str> = EVENT_INTERFACES.iter().map(|(n, _)| *n).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), EVENT_INTERFACES.len());
    }
}
```

**Step 3: Verify**

Run: `cargo test -p ucode-plugins --features wasm`
Run: `cargo clippy -p ucode-plugins --features wasm`

**Step 4: Commit**

```
feat(plugins): WasmPluginHost with dynamic export probing (ISSUE 0804)
```

---

### Task 7: Wire WasmPlugin into PluginHost

**Files:**
- Modify: `crates/ucode-plugins/src/host.rs`

Add a `WasmPlugin` variant to `PluginInstance` and a `load_wasm` method to
`PluginHost`. WASM plugins participate in the same dispatch loop as native
plugins.

**Step 1: Add WasmPlugin variant (behind cfg)**

```rust
#[cfg(feature = "wasm")]
use crate::wasm::WasmPlugin;

enum PluginInstance {
    WithHooks(Box<dyn PluginWithHooks>),
    WithTools(Box<dyn PluginWithTools>, Vec<PluginToolDef>),
    #[cfg(feature = "wasm")]
    Wasm(WasmPlugin),
}
```

**Step 2: Add load_wasm method**

```rust
#[cfg(feature = "wasm")]
pub fn load_wasm(&mut self, path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let config = crate::wasm::WasmEngineConfig::default();
    let plugin = WasmPlugin::from_file(path, &config)?;
    let plugin_id = plugin.plugin_id.clone();
    self.plugins.push(LoadedPlugin {
        plugin_id,
        instance: PluginInstance::Wasm(plugin),
        tool_fqns: vec![],
    });
    Ok(())
}
```

**Step 3: Update dispatch_hook to include WASM plugins**

In `dispatch_hook`, add a match arm for `PluginInstance::Wasm`:

```rust
#[cfg(feature = "wasm")]
PluginInstance::Wasm(wasm_plugin) => {
    let event_name = record.event.event_name();
    if let Some(result) = wasm_plugin.dispatch(event_name) {
        match result {
            Ok(response) => results.push(HookResult {
                plugin_id: loaded.plugin_id.clone(),
                response,
            }),
            Err(_) => { /* log error, skip */ }
        }
    }
}
```

**Step 4: Update unload for WASM variant**

```rust
#[cfg(feature = "wasm")]
PluginInstance::Wasm(_) => { /* WASM cleanup handled by Drop */ }
```

**Step 5: Verify**

Run: `cargo test -p ucode-plugins` (without wasm -- must still pass)
Run: `cargo test -p ucode-plugins --features wasm`
Run: `cargo clippy -p ucode-plugins --features wasm`

**Step 6: Commit**

```
feat(plugins): wire WasmPlugin into PluginHost dispatch (ISSUE 0804)
```

---

### Task 8: Guest SDK crate

**Files:**
- Create: `crates/ucode-plugin-sdk/Cargo.toml`
- Create: `crates/ucode-plugin-sdk/src/lib.rs`
- Modify: `Cargo.toml` (add to workspace members)

The guest SDK is a library crate that plugin authors depend on. It uses
`wit-bindgen` to generate guest-side bindings from the same WIT definitions.

**Step 1: Add workspace member**

In root `Cargo.toml`, add `"crates/ucode-plugin-sdk"` to `[workspace] members`.

Add to `[workspace.dependencies]`:
```toml
wit-bindgen = "0.53"
```

**Step 2: Create Cargo.toml**

```toml
[package]
name = "ucode-plugin-sdk"
version.workspace = true
edition.workspace = true

[dependencies]
wit-bindgen = { workspace = true }

[lib]
crate-type = ["cdylib", "rlib"]
```

**Step 3: Create src/lib.rs**

```rust
//! Guest SDK for writing ucode WASM plugins.
//!
//! Plugin authors depend on this crate and implement the generated traits.
//! Build with: `cargo build --target wasm32-wasip2`

// Generate guest-side bindings from WIT.
// The WIT files are symlinked/copied from ucode-plugins/wit/.
wit_bindgen::generate!({
    path: "../ucode-plugins/wit",
    world: "maximal-plugin",
});

// Re-export generated types for plugin authors.
pub mod prelude {
    pub use super::exports::*;
    pub use super::ucode::hooks_types::types::*;
}
```

Note: The exact `wit_bindgen::generate!` invocation may need adjustment based
on the WIT directory layout. The `path` must point to the directory containing
`world.wit` and the `deps/` folder.

**Step 4: Verify**

Run: `cargo check -p ucode-plugin-sdk` (host target, just for syntax)

For actual WASM compilation:
```
rustup target add wasm32-wasip2
cargo build -p ucode-plugin-sdk --target wasm32-wasip2
```

**Step 5: Commit**

```
feat(plugins): guest SDK crate ucode-plugin-sdk (ISSUE 0804)
```

---

### Task 9: Example WASM plugin + integration tests

**Files:**
- Create: `examples/plugins/hello-wasm/Cargo.toml`
- Create: `examples/plugins/hello-wasm/src/lib.rs`
- Create: `crates/ucode-plugins/tests/wasm_integration.rs`

**Step 1: Create example plugin**

`examples/plugins/hello-wasm/Cargo.toml`:
```toml
[package]
name = "hello-wasm"
version = "0.1.0"
edition = "2024"

[dependencies]
ucode-plugin-sdk = { path = "../../../crates/ucode-plugin-sdk" }

[lib]
crate-type = ["cdylib"]
```

`examples/plugins/hello-wasm/src/lib.rs`:
```rust
use ucode_plugin_sdk::prelude::*;

struct HelloPlugin;

// Implement lifecycle
impl /* generated lifecycle trait */ for HelloPlugin {
    fn handshake() -> HandshakeRequest {
        HandshakeRequest {
            plugin_id: "org.ucode.hello-wasm".into(),
            plugin_version: "1.0.0".into(),
            min_api_version: "1.0.0".into(),
            required_features: vec!["hooks".into()],
        }
    }

    fn initialize(result: HandshakeResult) -> Result<(), String> {
        Ok(())
    }

    fn shutdown() {}
}

// Implement session hooks
impl /* generated on-start trait */ for HelloPlugin {
    fn handle(payload: SessionStartPayload) -> HookResponse {
        HookResponse {
            kind: HookResponseKind::Ok,
            data: None,
        }
    }
}
```

Note: The exact trait names depend on what `wit_bindgen::generate!` produces.
This will be refined during implementation.

**Step 2: Build the example to .wasm**

```bash
rustup target add wasm32-wasip2
cargo build -p hello-wasm --target wasm32-wasip2 --release
```

The output will be at
`target/wasm32-wasip2/release/hello_wasm.wasm`.

**Step 3: Write integration test**

`crates/ucode-plugins/tests/wasm_integration.rs`:
```rust
#![cfg(feature = "wasm")]

use std::path::PathBuf;
use ucode_plugins::wasm::{WasmPlugin, WasmEngineConfig};

fn example_wasm_path() -> PathBuf {
    // Path to pre-built example plugin
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/wasm32-wasip2/release/hello_wasm.wasm")
}

#[test]
fn test_load_wasm_plugin() {
    let path = example_wasm_path();
    if !path.exists() {
        eprintln!("Skipping: build hello-wasm first with wasm32-wasip2 target");
        return;
    }
    let config = WasmEngineConfig::default();
    let plugin = WasmPlugin::from_file(&path, &config).expect("failed to load");
    assert!(plugin.handles_event("session_start"));
}

#[test]
fn test_dispatch_session_start() {
    let path = example_wasm_path();
    if !path.exists() {
        return;
    }
    let config = WasmEngineConfig::default();
    let mut plugin = WasmPlugin::from_file(&path, &config).unwrap();
    let result = plugin.dispatch("session_start");
    assert!(result.is_some());
    assert!(result.unwrap().is_ok());
}

#[test]
fn test_unhandled_event_returns_none() {
    let path = example_wasm_path();
    if !path.exists() {
        return;
    }
    let config = WasmEngineConfig::default();
    let mut plugin = WasmPlugin::from_file(&path, &config).unwrap();
    // hello-wasm only handles session_start, not budget_warning
    assert!(plugin.dispatch("budget_warning").is_none());
}
```

**Step 4: Run integration tests**

```bash
# Build example first
cargo build -p hello-wasm --target wasm32-wasip2 --release
# Run tests
cargo test -p ucode-plugins --features wasm
```

**Step 5: Commit**

```
feat(plugins): example WASM plugin and integration tests (ISSUE 0804)
```

---

### Task 10: Final verification and docs update

**Files:**
- Modify: `EPIC.md` (mark ISSUE 0804 done)
- Modify: `PLANS.md` (mark Task 8.4 done)

**Step 1: Full workspace check**

```bash
cargo check                                    # without wasm
cargo check -p ucode-plugins --features wasm   # with wasm
cargo test -p ucode-plugins                    # without wasm
cargo test -p ucode-plugins --features wasm    # with wasm
cargo clippy --workspace                       # 0 warnings
cargo clippy -p ucode-plugins --features wasm  # 0 warnings
```

**Step 2: Update EPIC.md and PLANS.md**

Mark ISSUE 0804 / Task 8.4 as DONE.

**Step 3: Commit**

```
docs: mark Task 8.4 / ISSUE 0804 as DONE
```
