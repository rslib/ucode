# External Plugin Infrastructure Implementation Plan (Task 8.6 / ISSUE 0806)

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Complete the plugin runtime so external WASM plugins can load from disk, receive hooks (including transform events with pipeline dispatch), register tools, and operate within the existing capability/policy model.

**Architecture decision (finalized):** Keep all 20 typed WIT category packages (19 existing + 1 new `hooks-transform`). Each category can version independently. Typed payloads provide compile-time safety at the WASM boundary. Transform events (`transform_messages`, `transform_system_prompt`) are dispatched through their typed `hooks-transform` WIT interfaces with pipeline composition on the host side. User controls transform ordering via `ucode.toml`.

**Tech Stack:** Rust, wasmtime (component model), WIT, serde_json, semver, TOML config

**Key files:**
- `crates/ucode-plugins/src/host.rs` — `PluginHost`, `dispatch_hook()`, `dispatch_transform()`, `PluginInstance` enum
- `crates/ucode-plugins/src/hooks.rs` — `HookEvent` enum (67 variants), `HookRecord`, `OverrideClass`, `payload_version()`
- `crates/ucode-plugins/src/wasm/host.rs` — `WasmPlugin`, `create_store_with_policy()`, `probe_exports()`
- `crates/ucode-plugins/src/wasm/convert.rs` — `EVENT_INTERFACE_MAP` (67 entries), `event_to_wit_interface()`, `wit_response_to_native()`
- `crates/ucode-plugins/wit/world.wit` — 67-export `maximal-plugin` world
- `crates/ucode-plugins/wit/deps/hooks-types/types.wit` — typed payload records (including transform payloads)
- `crates/ucode-plugins/wit/deps/hooks-transform/hooks-transform.wit` — NEW: transform hook interfaces
- `crates/ucode-plugins/wit/deps/plugin/plugin.wit` — lifecycle + tool-provider interfaces
- `crates/ucode-plugins/src/loader.rs` — `discover_plugins()`, `default_plugin_search_paths()`, `plugin_search_paths()`
- `crates/ucode-plugins/src/manifest.rs` — `PluginManifest`, `PluginCapabilities`, `min_payload_versions`

---

## Task 1: Plugin discovery paths (8.6.1) [DONE]

Wire default search paths into `discover_plugins()` at startup. The function already works — it just needs default paths.
Implemented: `default_plugin_search_paths()` and `plugin_search_paths()` in `loader.rs`.

**Files:**
- Modify: `crates/ucode-plugins/src/loader.rs`
- Test: `crates/ucode-plugins/src/loader.rs` (inline tests)

**Step 1: Write failing test for default discovery paths**

```rust
#[test]
fn test_default_plugin_paths_includes_project_and_user() {
    let paths = default_plugin_search_paths(Some(Path::new("/fake/project")));
    assert_eq!(paths[0], PathBuf::from("/fake/project/.ucode/plugins"));
    // User path depends on $HOME / $UCODE_HOME
    assert!(paths.len() >= 2);
    assert!(paths[1].ends_with("plugins"));
}
```

Run: `cargo test -p ucode-plugins test_default_plugin_paths -v`
Expected: FAIL — `default_plugin_search_paths` doesn't exist

**Step 2: Implement `default_plugin_search_paths()`**

```rust
/// Returns the default plugin search paths in priority order:
/// 1. `.ucode/plugins/` (project-local, if workspace_root provided)
/// 2. `$UCODE_HOME/plugins/` or `~/.ucode/plugins/` (user-level)
pub fn default_plugin_search_paths(workspace_root: Option<&Path>) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    // Project-local
    if let Some(root) = workspace_root {
        paths.push(root.join(".ucode/plugins"));
    }
    // User-level: $UCODE_HOME/plugins/ or ~/.ucode/plugins/
    if let Ok(ucode_home) = std::env::var("UCODE_HOME") {
        paths.push(PathBuf::from(ucode_home).join("plugins"));
    } else if let Some(home) = dirs::home_dir() {
        paths.push(home.join(".ucode/plugins"));
    }
    paths
}
```

Run: `cargo test -p ucode-plugins test_default_plugin_paths -v`
Expected: PASS

**Step 3: Add test for config-driven extra paths**

```rust
#[test]
fn test_plugin_paths_with_extras() {
    let extras = vec![PathBuf::from("/opt/ucode-plugins"), PathBuf::from("/custom")];
    let paths = plugin_search_paths(Some(Path::new("/project")), &extras);
    assert_eq!(paths.len(), 4); // project + user + 2 extras
    assert_eq!(paths[2], PathBuf::from("/opt/ucode-plugins"));
}
```

**Step 4: Implement `plugin_search_paths()` combining defaults + extras**

```rust
/// Combine default paths with config-driven extras.
pub fn plugin_search_paths(workspace_root: Option<&Path>, extras: &[PathBuf]) -> Vec<PathBuf> {
    let mut paths = default_plugin_search_paths(workspace_root);
    paths.extend(extras.iter().cloned());
    paths
}
```

Run: `cargo test -p ucode-plugins -- test_plugin_paths -v`
Expected: PASS

**Step 5: Commit**

```
git add crates/ucode-plugins/src/loader.rs
git commit -m "feat(plugins): add default plugin discovery paths (project + user + config extras)"
```

---

## Task 2: Typed WIT interface with hooks-transform category (8.6.2) [DONE]

**Decision changed:** Keep all 20 typed WIT category packages (19 existing + 1 new `hooks-transform`).
The original plan to replace with a unified interface was rejected in favor of type safety and
independent per-category versioning.

**What was done (actual implementation):**
- Created `crates/ucode-plugins/wit/deps/hooks-transform/hooks-transform.wit` with 2 typed interfaces
- Added `transform-messages-payload` and `transform-system-prompt-payload` records to `hooks-types/types.wit`
- Updated `world.wit`: `maximal-plugin` world now exports 67 hook interfaces (added 2 transform exports)
- Updated `convert.rs`: `EVENT_INTERFACE_MAP` expanded from 65 to 67 entries; `event_to_wit_interface()` updated
- All 19 existing hook category WIT dirs kept (hooks-session, hooks-message, hooks-model, etc.)
- Tests updated: `test_event_interface_map_has_64_entries` renamed to assert 67 entries

---

## Task 3: Complete WASM hook dispatch (8.6.3) [DONE]

Wire the stubbed WASM dispatch to actually call the typed per-event WIT exports (e.g., `ucode:hooks-session/on-start.handle()`).

> **Note:** The code examples below were written for the old unified `hook-handler` approach.
> The actual implementation should use typed per-event WIT exports instead. Look up the event's
> interface from `EVENT_INTERFACE_MAP` (67 entries) and call the typed export function.

**Files:**
- Modify: `crates/ucode-plugins/src/host.rs:294-302` — replace stub with real dispatch
- Modify: `crates/ucode-plugins/src/wasm/host.rs` — add `dispatch_hook()` method to `WasmPlugin`
- Test: `crates/ucode-plugins/tests/` — integration test with real WASM plugin

**Step 1: Write failing test**

```rust
#[cfg(test)]
#[cfg(feature = "wasm")]
mod wasm_dispatch_tests {
    use super::*;

    #[test]
    fn test_wasm_dispatch_returns_actual_response() {
        // Load the hello-wasm fixture plugin
        let wasm_bytes = include_bytes!("../../examples/plugins/hello-wasm/target/...");
        let plugin = WasmPlugin::from_bytes(wasm_bytes).unwrap();
        // Dispatch a session_start event
        let event = HookEvent::SessionStart { session_id: "test-123".into() };
        let record = HookRecord::new(event);
        // Should NOT return Ok unconditionally — should call the actual handler
        let response = plugin.dispatch_hook(&record, &default_policy());
        // The hello-wasm plugin returns Ok for session_start
        assert!(matches!(response, Ok(HookResponse::Ok)));
    }
}
```

Run: `cargo test -p ucode-plugins wasm_dispatch_tests -v --features wasm`
Expected: FAIL — `dispatch_hook` method doesn't exist on `WasmPlugin`

**Step 2: Implement `WasmPlugin::dispatch_hook()`**

Add to `crates/ucode-plugins/src/wasm/host.rs`:

```rust
impl WasmPlugin {
    /// Dispatch a hook event to this WASM plugin via the unified hook-handler.handle() export.
    pub fn dispatch_hook(
        &self,
        record: &HookRecord,
        policy: &PluginPolicy,
    ) -> Result<HookResponse, WasmPluginError> {
        // 1. Create store with policy (fuel + memory limits)
        let mut store = self.create_store_with_policy(policy, None);

        // 2. Create linker with host-log import
        let linker = self.create_linker()?;

        // 3. Instantiate component
        let instance = linker
            .instantiate(&mut store, &self.component)
            .map_err(WasmPluginError::Instantiate)?;

        // 4. Serialize HookRecord -> JSON, wrap in hook-event
        let payload_json = serde_json::to_string(&record.event)
            .unwrap_or_else(|_| "{}".to_string());
        let event_name = record.event.event_name().to_string();
        let payload_version = "1.0.0".to_string(); // All events start at 1.0.0

        // 5. Get the hook-handler.handle export and call it
        // (exact wasmtime component API depends on generated bindings)
        let handle_func = instance
            .get_export(&mut store, None, "ucode:plugin/hook-handler")
            .and_then(|idx| instance.get_export(&mut store, Some(&idx), "handle"))
            .ok_or_else(|| WasmPluginError::EventNotHandled(event_name.clone()))?;

        // Call with (name, payload_version, payload) -> hook-response
        // ... (exact calling convention depends on wasmtime component model API)

        // 6. Deserialize response
        // 7. On error/fuel exhaustion: log + return Ok (fail-open)
        Ok(HookResponse::Ok) // placeholder until wiring complete
    }
}
```

**Step 3: Wire into `PluginHost::dispatch_hook()`**

Replace the stub at `host.rs:294-302`:

```rust
#[cfg(feature = "wasm")]
PluginInstance::Wasm(wasm_plugin) => {
    let event_name = record.event.event_name();
    match wasm_plugin.dispatch_hook(&record, &loaded.policy) {
        Ok(response) => {
            // Handle Modify: accumulate changes like native plugins
            if let HookResponse::Modify { ref changes } = response {
                accumulated_changes = Some(match accumulated_changes {
                    Some(mut existing) => {
                        if let (Some(e), Some(c)) = (existing.as_object_mut(), changes.as_object()) {
                            for (k, v) in c {
                                e.insert(k.clone(), v.clone());
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
        Err(e) => {
            // Fail-open: log error, return Ok
            tracing::warn!(plugin_id = %loaded.plugin_id, error = %e, "WASM hook dispatch failed, treating as Ok");
            results.push(HookResult {
                plugin_id: loaded.plugin_id.clone(),
                response: HookResponse::Ok,
            });
        }
    }
}
```

**Step 4: Run tests**

Run: `cargo test -p ucode-plugins --features wasm`
Expected: PASS

**Step 5: Commit**

```
git commit -m "feat(plugins): wire WASM hook dispatch through unified hook-handler.handle()"
```

---

## Task 4: Add transform hook events + pipeline dispatch (8.6.4) [DONE]

Add `TransformMessages` and `TransformSystemPrompt` to `HookEvent`, implement pipeline dispatch on `PluginHost`.
Implemented: `dispatch_transform()`, `transform_messages()`, `transform_system_prompt()` methods on `PluginHost`.
Transform events dispatched through typed `hooks-transform` WIT interfaces with pipeline composition.

**Files:**
- Modify: `crates/ucode-plugins/src/hooks.rs` — add 2 new variants
- Modify: `crates/ucode-plugins/src/host.rs` — add `dispatch_transform()`, `transform_messages()`, `transform_system_prompt()`
- Test: inline tests in both files

**Step 1: Write failing test for new hook events**

```rust
#[test]
fn test_transform_messages_event() {
    let event = HookEvent::TransformMessages {
        messages_json: r#"[{"role":"user","content":"hello"}]"#.to_string(),
    };
    assert_eq!(event.event_name(), "transform_messages");
    assert_eq!(event.override_class(), OverrideClass::Guarded);
}

#[test]
fn test_transform_system_prompt_event() {
    let event = HookEvent::TransformSystemPrompt {
        prompt: "You are a helpful assistant.".to_string(),
    };
    assert_eq!(event.event_name(), "transform_system_prompt");
    assert_eq!(event.override_class(), OverrideClass::Guarded);
}
```

Run: `cargo test -p ucode-plugins test_transform -v`
Expected: FAIL — variants don't exist

**Step 2: Add variants to HookEvent**

In `hooks.rs`, add to the enum:

```rust
// --- Transform (pipeline dispatch) ---
TransformMessages {
    messages_json: String,
},
TransformSystemPrompt {
    prompt: String,
},
```

Add to `event_name()`:

```rust
Self::TransformMessages { .. } => "transform_messages",
Self::TransformSystemPrompt { .. } => "transform_system_prompt",
```

Add to `override_class()`:

```rust
Self::TransformMessages { .. } | Self::TransformSystemPrompt { .. } => OverrideClass::Guarded,
```

Run: `cargo test -p ucode-plugins test_transform -v`
Expected: PASS

**Step 3: Write failing test for pipeline dispatch**

```rust
#[test]
fn test_dispatch_transform_chains_modify_responses() {
    // Plugin A removes duplicates, Plugin B adds metadata
    // Pipeline: input -> A -> B -> output
    let mut host = PluginHost::new();
    // ... load two mock plugins that return Modify for transform_messages
    let input = r#"[{"role":"user","content":"hello"},{"role":"user","content":"hello"}]"#;
    let output = host.dispatch_transform("transform_messages", input.to_string());
    // Output should reflect both plugins' modifications chained
    assert_ne!(output, input);
}

#[test]
fn test_dispatch_transform_ok_passes_through() {
    let mut host = PluginHost::new();
    // Plugin returns Ok (no change)
    let input = r#"[{"role":"user","content":"hello"}]"#;
    let output = host.dispatch_transform("transform_messages", input.to_string());
    assert_eq!(output, input);
}
```

**Step 4: Implement `dispatch_transform()` on PluginHost**

```rust
/// Set of event names that use pipeline dispatch instead of fan-out.
const TRANSFORM_EVENTS: &[&str] = &["transform_messages", "transform_system_prompt"];

/// Returns true if this event uses pipeline dispatch.
pub fn is_transform_event(event_name: &str) -> bool {
    TRANSFORM_EVENTS.contains(&event_name)
}

impl PluginHost {
    /// Pipeline dispatch for transform events.
    /// Each plugin in `transform_order` sees the output of the previous plugin.
    /// Returns the final transformed payload.
    pub fn dispatch_transform(
        &mut self,
        event_name: &str,
        payload: String,
        transform_order: &[String],
    ) -> String {
        let mut current = payload;
        for plugin_id in transform_order {
            if plugin_id == "native" {
                // Reserved for built-in context management (Task 8.8)
                continue;
            }
            // Find the plugin
            let loaded = match self.plugins.iter().find(|p| p.plugin_id == *plugin_id) {
                Some(p) => p,
                None => continue, // Plugin not loaded, skip
            };
            // Build the hook event
            let event = HookEvent::TransformMessages {
                messages_json: current.clone(),
            };
            let record = HookRecord::new(event);
            // Dispatch to this single plugin
            let response = match &loaded.instance {
                PluginInstance::WithHooks(plugin) => plugin.on_event(&record),
                #[cfg(feature = "wasm")]
                PluginInstance::Wasm(wasm_plugin) => {
                    match wasm_plugin.dispatch_hook(&record, &loaded.policy) {
                        Ok(r) => r,
                        Err(_) => HookResponse::Ok, // fail-open
                    }
                }
                _ => HookResponse::Ok,
            };
            // Chain: Modify = replace, Ok = pass through, Veto = skip
            match response {
                HookResponse::Modify { changes } => {
                    if let Some(s) = changes.as_str() {
                        current = s.to_string();
                    } else {
                        current = changes.to_string();
                    }
                }
                HookResponse::Ok | HookResponse::Veto { .. } => {}
            }
        }
        current
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
}
```

Run: `cargo test -p ucode-plugins dispatch_transform -v`
Expected: PASS

**Step 5: Commit**

```
git commit -m "feat(plugins): add transform hook events with pipeline dispatch

TransformMessages and TransformSystemPrompt are regular hook events
dispatched through hook-handler.handle(). Host uses pipeline mode:
each plugin sees previous output. Modify = full replacement,
Ok = pass through, Veto = skip plugin."
```

---

## Task 5: Plugin tool registration via tool-provider (8.6.5) [DONE]

Wire WASM plugin tool calls through the `tool-provider` WIT interface (already defined in `ucode:plugin/tool-provider`).

**Files:**
- Modify: `crates/ucode-plugins/src/wasm/host.rs` — add `invoke_tool()` to `WasmPlugin`
- Modify: `crates/ucode-plugins/src/host.rs` — route tool calls to WASM plugins
- Test: integration test

**Step 1: Write failing test**

```rust
#[test]
fn test_wasm_plugin_tool_invocation() {
    // Load fixture plugin that exports a tool
    let mut host = PluginHost::new();
    // ... load WASM plugin with tool "context_stats"
    let tools = host.plugin_tools();
    assert!(tools.iter().any(|(fqn, _)| fqn.contains("context_stats")));
    // Invoke the tool
    let result = host.invoke_plugin_tool("org.example.ctx.context_stats", "{}");
    assert!(result.is_ok());
}
```

**Step 2: Implement `WasmPlugin::invoke_tool()`**

```rust
impl WasmPlugin {
    pub fn invoke_tool(
        &self,
        name: &str,
        args_json: &str,
        policy: &PluginPolicy,
    ) -> Result<String, WasmPluginError> {
        let mut store = self.create_store_with_policy(policy, None);
        let linker = self.create_linker()?;
        let instance = linker
            .instantiate(&mut store, &self.component)
            .map_err(WasmPluginError::Instantiate)?;
        // Call tool-handler.handle-tool-call(name, args) -> result<string, string>
        // ... (wasmtime component model calling convention)
        todo!("wire wasmtime component call")
    }
}
```

**Step 3: Add `invoke_plugin_tool()` to PluginHost**

```rust
impl PluginHost {
    pub fn invoke_plugin_tool(&self, fqn: &str, args_json: &str) -> Result<String, String> {
        // Parse FQN: "org.acme.plugin.tool_name" -> plugin_id="org.acme.plugin", tool="tool_name"
        let (plugin_id, tool_name) = parse_tool_fqn(fqn)?;
        let loaded = self.plugins.iter()
            .find(|p| p.plugin_id == plugin_id)
            .ok_or_else(|| format!("plugin not found: {plugin_id}"))?;
        match &loaded.instance {
            PluginInstance::WithTools(provider, _) => {
                provider.invoke_tool(tool_name, args_json)
            }
            #[cfg(feature = "wasm")]
            PluginInstance::Wasm(wasm_plugin) => {
                wasm_plugin.invoke_tool(tool_name, args_json, &loaded.policy)
                    .map_err(|e| e.to_string())
            }
            _ => Err(format!("plugin {plugin_id} does not provide tools")),
        }
    }
}
```

**Step 4: Run tests, commit**

```
git commit -m "feat(plugins): route plugin tool calls through tool-handler WIT interface"
```

---

## Task 6: Hook payload versioning (8.6.6) [DONE]

Add `payload_version()` to `HookEvent` and version-mismatch skipping in `dispatch_hook()`.
Implemented: `payload_version()` method (all events return "1.0.0"), `min_payload_versions` field on `LoadedPlugin`,
version-mismatch skipping logic in `dispatch_hook()`.

**Files:**
- Modify: `crates/ucode-plugins/src/hooks.rs` — add `PAYLOAD_VERSIONS` map, `payload_version()` method
- Modify: `crates/ucode-plugins/src/manifest.rs` — add `min_payload_version` to hook subscription
- Modify: `crates/ucode-plugins/src/host.rs` — skip dispatch on version mismatch
- Test: inline tests

**Step 1: Write failing test**

```rust
#[test]
fn test_payload_version_for_events() {
    let event = HookEvent::SessionStart { session_id: "x".into() };
    assert_eq!(event.payload_version(), "1.0.0");
}

#[test]
fn test_dispatch_skips_on_version_mismatch() {
    // Plugin requires min_payload_version "2.0.0" for session_start
    // Current payload version is "1.0.0"
    // Dispatch should skip this plugin
}
```

**Step 2: Implement `payload_version()` on HookEvent**

```rust
impl HookEvent {
    /// Returns the semver payload version for this event type.
    /// All events start at "1.0.0". Bump minor for additive fields, major for breaking.
    pub fn payload_version(&self) -> &'static str {
        "1.0.0" // All events at v1 initially
    }
}
```

**Step 3: Add version check to dispatch**

In `dispatch_hook()`, before calling the plugin:

```rust
// Check payload version compatibility
if let Some(min_version) = loaded.min_payload_versions.get(event_name) {
    let current: semver::Version = record.event.payload_version().parse().unwrap();
    let required: semver::Version = min_version.parse().unwrap_or_default();
    if current < required {
        tracing::debug!(plugin_id = %loaded.plugin_id, event = event_name,
            "skipping: payload version {current} < required {required}");
        continue;
    }
}
```

**Step 4: Run tests, commit**

```
git commit -m "feat(plugins): add hook payload versioning with per-event semver"
```

---

## Task 7: Hook payload documentation (8.6.7) [DONE]

Generate `docs/hooks/` with one markdown file per hook category.

**Files:**
- Create: `docs/hooks/session.md`, `docs/hooks/tool.md`, `docs/hooks/context.md`, etc.
- Create: `docs/hooks/README.md` — index

**Step 1: Write the docs**

Each file follows this template:

```markdown
# Session Hooks

## session_start

- **Safety tier:** Safe
- **Payload version:** 1.0.0
- **Payload schema:**
  ```json
  { "session_id": "string" }
  ```
- **Response options:** Ok, Modify, Veto
- **Version history:** 1.0.0 — initial
```

Generate for all 67 events (65 original + 2 transform events).

**Step 2: Commit**

```
git commit -m "docs: add hook payload documentation for all 67 events"
```

---

## Task 8: Fixture plugin — end-to-end contract test (8.6.8) [DONE]

Build a minimal WASM plugin that exercises all interfaces.

**Files:**
- Create: `examples/plugins/context-manager/` — Cargo.toml, src/lib.rs, plugin.toml
- Create: `examples/plugins/context-manager/wit/` — WIT deps (symlinks to main)
- Create: `crates/ucode-plugins/tests/fixture_plugin_test.rs` — integration test

**Step 1: Create the fixture plugin**

`examples/plugins/context-manager/src/lib.rs`:

```rust
// Implements typed WIT exports for session_start, session_end, transform_messages
// Implements tool-provider with context_stats tool
// Uses typed per-event WIT interfaces

struct ContextManagerPlugin;

impl hook_handler::HookHandler for ContextManagerPlugin {
    fn handle(event: HookEvent) -> HookResponse {
        match event.name.as_str() {
            "session_start" => {
                // Log session start
                host_log::log(&format!("Session started: {}", event.payload));
                HookResponse { kind: HookResponseKind::Ok, data: None }
            }
            "transform_messages" => {
                // Remove duplicate consecutive assistant messages
                let messages: Vec<serde_json::Value> = serde_json::from_str(&event.payload)
                    .unwrap_or_default();
                let deduped = dedup_messages(messages);
                let json = serde_json::to_string(&deduped).unwrap();
                HookResponse {
                    kind: HookResponseKind::Modify,
                    data: Some(json),
                }
            }
            _ => HookResponse { kind: HookResponseKind::Ok, data: None },
        }
    }
}

impl tool_handler::ToolHandler for ContextManagerPlugin {
    fn handle_tool_call(name: &str, args: &str) -> Result<String, String> {
        match name {
            "context_stats" => {
                // Return message count and total size
                Ok(r#"{"message_count": 42, "total_bytes": 12345}"#.to_string())
            }
            _ => Err(format!("unknown tool: {name}")),
        }
    }
}
```

**Step 2: Write integration test**

```rust
#[test]
#[cfg(feature = "wasm")]
fn test_fixture_plugin_full_lifecycle() {
    // 1. Discover from temp dir simulating ~/.ucode/plugins/
    let tmp = tempfile::tempdir().unwrap();
    let plugin_dir = tmp.path().join("context-manager");
    // Copy compiled WASM + plugin.toml
    // ...

    // 2. Load into PluginHost
    let mut host = PluginHost::new();
    // host.load_wasm_plugin(...)

    // 3. Dispatch session_start hook
    let results = host.dispatch_hook(&HookRecord::new(
        HookEvent::SessionStart { session_id: "test".into() }
    ));
    assert!(!results.is_empty());

    // 4. Dispatch transform_messages (pipeline)
    let input = r#"[{"role":"assistant","content":"hi"},{"role":"assistant","content":"hi"}]"#;
    let output = host.dispatch_transform("transform_messages", input.to_string(), &["context-manager".into()]);
    // Should have deduped
    let msgs: Vec<serde_json::Value> = serde_json::from_str(&output).unwrap();
    assert_eq!(msgs.len(), 1);

    // 5. Invoke tool
    let result = host.invoke_plugin_tool("org.example.context-manager.context_stats", "{}");
    assert!(result.is_ok());
}
```

**Step 3: Build fixture, run integration test**

```
cargo component build -p context-manager-plugin --target wasm32-wasip2
cargo test -p ucode-plugins --features wasm fixture_plugin -v
```

**Step 4: Commit**

```
git commit -m "feat(plugins): add context-manager fixture plugin for end-to-end contract testing

Demonstrates full lifecycle: discovery -> load -> hook dispatch ->
transform pipeline -> tool call -> response. Exercises typed WIT
interfaces (per-event hooks + tool-provider)."
```

---

## Execution order and dependencies

```
Task 1 (discovery paths)     — DONE
Task 2 (typed WIT + hooks-transform) — DONE
Task 3 (WASM dispatch)       — DONE
Task 4 (transform pipeline)  — DONE
Task 5 (tool registration)   — DONE
Task 6 (payload versioning)  — DONE
Task 7 (docs)                — DONE
Task 8 (fixture plugin)      — DONE
```

**All tasks complete.**
