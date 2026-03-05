# Tool Registry + Invocation Runtime (ISSUE 0401) Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Create a unified tool registry that can register, list, and invoke async tool handlers, producing canonical ToolResult values.

**Architecture:** ToolSpec holds metadata (name, description, JSON Schema parameters). ToolHandler trait defines async invocation. ToolRegistry is a HashMap-backed store that registers tools, lists their specs, and invokes handlers by name. A to_tool_def() conversion bridges to the provider-facing ToolDef type.

**Tech Stack:** Rust, serde_json, tokio (async), ucode-core (ToolResult, CoreError), ucode-providers (ToolDef)

---

### Task 1: ToolSpec + ToolHandler trait + ToolRegistry types

**Files:**
- Create: `crates/ucode-tools/src/registry.rs`
- Modify: `crates/ucode-tools/src/lib.rs`
- Modify: `crates/ucode-tools/Cargo.toml`
- Test: `crates/ucode-tools/tests/registry_tests.rs`

**Step 1: Update Cargo.toml dependencies**

Add to `crates/ucode-tools/Cargo.toml`:
```toml
[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
tokio = { workspace = true }
ucode-core = { workspace = true }

[dev-dependencies]
tokio = { workspace = true, features = ["full"] }
```

**Step 2: Write failing tests**

Create `crates/ucode-tools/tests/registry_tests.rs`:
```rust
use serde_json::json;
use ucode_tools::{ToolHandler, ToolRegistry, ToolSpec};

/// A simple echo tool for testing.
struct EchoTool;

impl ToolHandler for EchoTool {
    fn invoke(
        &self,
        args: serde_json::Value,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<serde_json::Value, ucode_core::CoreError>> + Send>>
    {
        Box::pin(async move { Ok(args) })
    }
}

#[test]
fn register_and_list_tools() {
    let mut registry = ToolRegistry::new();
    let spec = ToolSpec {
        name: "echo".into(),
        description: "Echoes input back".into(),
        parameters: json!({"type": "object", "properties": {"msg": {"type": "string"}}}),
    };
    registry.register(spec, Box::new(EchoTool)).unwrap();

    let tools = registry.list();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "echo");
}

#[test]
fn register_duplicate_fails() {
    let mut registry = ToolRegistry::new();
    let spec = ToolSpec {
        name: "echo".into(),
        description: "Echoes input".into(),
        parameters: json!({}),
    };
    registry.register(spec.clone(), Box::new(EchoTool)).unwrap();
    let result = registry.register(spec, Box::new(EchoTool));
    assert!(result.is_err());
}

#[tokio::test]
async fn invoke_registered_tool() {
    let mut registry = ToolRegistry::new();
    let spec = ToolSpec {
        name: "echo".into(),
        description: "Echoes input".into(),
        parameters: json!({}),
    };
    registry.register(spec, Box::new(EchoTool)).unwrap();

    let result = registry
        .invoke("call-1", "echo", json!({"msg": "hello"}))
        .await
        .unwrap();
    assert_eq!(result.name, "echo");
    assert_eq!(result.id, "call-1");
    assert_eq!(result.result, json!({"msg": "hello"}));
    assert!(!result.is_error);
}

#[tokio::test]
async fn invoke_unknown_tool_fails() {
    let registry = ToolRegistry::new();
    let result = registry.invoke("call-1", "nonexistent", json!({})).await;
    assert!(result.is_err());
}

#[test]
fn to_tool_def_conversion() {
    let spec = ToolSpec {
        name: "read_file".into(),
        description: "Read a file".into(),
        parameters: json!({"type": "object", "properties": {"path": {"type": "string"}}}),
    };
    let def = spec.to_tool_def();
    assert_eq!(def.name, "read_file");
    assert_eq!(def.description, "Read a file");
    assert_eq!(def.parameters, spec.parameters);
}

#[test]
fn list_tool_defs() {
    let mut registry = ToolRegistry::new();
    let spec1 = ToolSpec {
        name: "tool_a".into(),
        description: "A".into(),
        parameters: json!({}),
    };
    let spec2 = ToolSpec {
        name: "tool_b".into(),
        description: "B".into(),
        parameters: json!({}),
    };
    registry.register(spec1, Box::new(EchoTool)).unwrap();
    registry.register(spec2, Box::new(EchoTool)).unwrap();

    let defs = registry.tool_defs();
    assert_eq!(defs.len(), 2);
}

#[tokio::test]
async fn invoke_handler_error_produces_error_result() {
    struct FailTool;
    impl ToolHandler for FailTool {
        fn invoke(
            &self,
            _args: serde_json::Value,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<serde_json::Value, ucode_core::CoreError>> + Send>>
        {
            Box::pin(async { Err(ucode_core::CoreError::Tool("tool broke".into())) })
        }
    }

    let mut registry = ToolRegistry::new();
    let spec = ToolSpec {
        name: "fail".into(),
        description: "Always fails".into(),
        parameters: json!({}),
    };
    registry.register(spec, Box::new(FailTool)).unwrap();

    let result = registry.invoke("call-1", "fail", json!({})).await.unwrap();
    assert!(result.is_error);
    assert!(result.result.as_str().unwrap().contains("tool broke"));
}

#[test]
fn get_tool_by_name() {
    let mut registry = ToolRegistry::new();
    let spec = ToolSpec {
        name: "echo".into(),
        description: "Echoes".into(),
        parameters: json!({}),
    };
    registry.register(spec, Box::new(EchoTool)).unwrap();

    assert!(registry.get("echo").is_some());
    assert!(registry.get("nope").is_none());
}
```

**Step 3: Run tests to verify they fail**

Run: `cargo test -p ucode-tools --test registry_tests 2>&1`
Expected: compilation errors (types don't exist yet)

**Step 4: Implement ToolSpec, ToolHandler, ToolRegistry**

Create `crates/ucode-tools/src/registry.rs`:
```rust
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};
use ucode_core::{CoreError, ToolResult};

/// Metadata describing a tool's interface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    /// Unique tool name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// JSON Schema for the tool's input parameters.
    pub parameters: serde_json::Value,
}

impl ToolSpec {
    /// Convert to provider-facing ToolDef.
    pub fn to_tool_def(&self) -> ucode_providers::ToolDef {
        ucode_providers::ToolDef {
            name: self.name.clone(),
            description: self.description.clone(),
            parameters: self.parameters.clone(),
        }
    }
}

/// Async tool invocation handler.
pub trait ToolHandler: Send + Sync {
    /// Execute the tool with the given arguments.
    fn invoke(
        &self,
        args: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, CoreError>> + Send>>;
}

/// A registered tool: spec + handler.
pub struct RegisteredTool {
    pub spec: ToolSpec,
    handler: Box<dyn ToolHandler>,
}

impl RegisteredTool {
    /// Invoke this tool's handler.
    pub async fn invoke(&self, args: serde_json::Value) -> Result<serde_json::Value, CoreError> {
        self.handler.invoke(args).await
    }
}

/// Registry of available tools.
pub struct ToolRegistry {
    tools: HashMap<String, RegisteredTool>,
}

impl ToolRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Register a tool. Fails if a tool with the same name already exists.
    pub fn register(
        &mut self,
        spec: ToolSpec,
        handler: Box<dyn ToolHandler>,
    ) -> Result<(), CoreError> {
        if self.tools.contains_key(&spec.name) {
            return Err(CoreError::Tool(format!(
                "tool '{}' already registered",
                spec.name
            )));
        }
        let name = spec.name.clone();
        self.tools.insert(name, RegisteredTool { spec, handler });
        Ok(())
    }

    /// List all registered tool specs.
    pub fn list(&self) -> Vec<&ToolSpec> {
        self.tools.values().map(|rt| &rt.spec).collect()
    }

    /// Get a registered tool by name.
    pub fn get(&self, name: &str) -> Option<&RegisteredTool> {
        self.tools.get(name)
    }

    /// List all tool definitions in provider-facing format.
    pub fn tool_defs(&self) -> Vec<ucode_providers::ToolDef> {
        self.tools
            .values()
            .map(|rt| rt.spec.to_tool_def())
            .collect()
    }

    /// Invoke a tool by name, returning a canonical ToolResult.
    ///
    /// If the handler returns an error, it is captured as an error ToolResult
    /// rather than propagated — the caller always gets a ToolResult.
    /// Only returns Err if the tool name is not found.
    pub async fn invoke(
        &self,
        call_id: &str,
        tool_name: &str,
        args: serde_json::Value,
    ) -> Result<ToolResult, CoreError> {
        let tool = self
            .tools
            .get(tool_name)
            .ok_or_else(|| CoreError::Tool(format!("unknown tool: '{}'", tool_name)))?;

        match tool.invoke(args).await {
            Ok(value) => Ok(ToolResult {
                id: call_id.to_string(),
                name: tool_name.to_string(),
                result: value,
                is_error: false,
            }),
            Err(e) => Ok(ToolResult {
                id: call_id.to_string(),
                name: tool_name.to_string(),
                result: serde_json::Value::String(e.to_string()),
                is_error: true,
            }),
        }
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
```

Update `crates/ucode-tools/src/lib.rs`:
```rust
//! ucode-tools: built-in tools, registry, permissions, sandbox policy engine

pub mod registry;

pub use registry::{RegisteredTool, ToolHandler, ToolRegistry, ToolSpec};
```

**Step 5: Run tests to verify they pass**

Run: `cargo test -p ucode-tools --test registry_tests -v 2>&1`
Expected: 8 tests pass

**Step 6: Run full workspace verification**

Run: `cargo fmt && cargo clippy --workspace -- -D warnings && cargo test --workspace`
Expected: all pass, 0 warnings

**Step 7: Commit**

```bash
git add -A
git commit -m "feat(tools): add tool registry + invocation runtime (ISSUE 0401)

- ToolSpec: name, description, JSON Schema parameters with to_tool_def()
- ToolHandler trait: async invoke(args) -> Result<Value, CoreError>
- RegisteredTool: spec + handler bundle
- ToolRegistry: register, list, get, invoke, tool_defs
- invoke captures handler errors as error ToolResult (no propagation)
- Duplicate registration returns error
- 8 new tests covering register, list, invoke, errors, conversion
- N tests passing, 0 clippy warnings"
```

### Task 2: Add CoreError::Tool variant (if missing)

**Files:**
- Check: `crates/ucode-core/src/error.rs`

Check if `CoreError::Tool(String)` variant exists. If not, add it.

---

## Acceptance Criteria (from EPIC.md)

- `list_tools` returns registered tools ✓ (test: `register_and_list_tools`)
- invoke demo tool returns ToolResult ✓ (test: `invoke_registered_tool`)
