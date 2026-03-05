use serde_json::json;
use ucode_tools::{ToolHandler, ToolRegistry, ToolSpec};

/// A simple echo tool for testing.
struct EchoTool;

impl ToolHandler for EchoTool {
    fn invoke(
        &self,
        args: serde_json::Value,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<serde_json::Value, ucode_core::CoreError>>
                + Send,
        >,
    > {
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
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<serde_json::Value, ucode_core::CoreError>>
                    + Send,
            >,
        > {
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
