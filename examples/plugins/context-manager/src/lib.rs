//! Context Manager — demo WASM plugin for ucode.
//!
//! Subscribes to session lifecycle and transform hooks, and provides
//! a `context_stats` tool. Demonstrates:
//! - Hook handling (session start/end, transform messages)
//! - Message deduplication in the transform pipeline
//! - Tool registration and invocation

wit_bindgen::generate!({
    path: "wit",
    world: "context-manager-plugin",
    pub_export_macro: true,
    generate_all,
});

struct ContextManagerPlugin;

impl exports::ucode::plugin::lifecycle::Guest for ContextManagerPlugin {
    fn handshake() -> ucode::hooks_types::types::HandshakeRequest {
        ucode::hooks_types::types::HandshakeRequest {
            plugin_id: "org.example.context-manager".into(),
            plugin_version: "0.1.0".into(),
            min_api_version: "1.0.0".into(),
            required_features: vec!["hooks".into(), "tools".into()],
        }
    }

    fn initialize(_result: ucode::hooks_types::types::HandshakeResult) -> Result<(), String> {
        ucode::plugin::host_log::log("context-manager: initialized");
        Ok(())
    }

    fn shutdown() {
        ucode::plugin::host_log::log("context-manager: shutting down");
    }
}

impl exports::ucode::hooks_session::on_start::Guest for ContextManagerPlugin {
    fn handle(
        payload: ucode::hooks_types::types::SessionStartPayload,
    ) -> ucode::hooks_types::types::HookResponse {
        ucode::plugin::host_log::log(&format!(
            "context-manager: session started ({})",
            payload.session_id
        ));
        ucode::hooks_types::types::HookResponse {
            kind: ucode::hooks_types::types::HookResponseKind::Ok,
            data: None,
        }
    }
}

impl exports::ucode::hooks_session::on_end::Guest for ContextManagerPlugin {
    fn handle(
        payload: ucode::hooks_types::types::SessionEndPayload,
    ) -> ucode::hooks_types::types::HookResponse {
        ucode::plugin::host_log::log(&format!(
            "context-manager: session ended ({}, {:.1}s)",
            payload.session_id, payload.duration_secs
        ));
        ucode::hooks_types::types::HookResponse {
            kind: ucode::hooks_types::types::HookResponseKind::Ok,
            data: None,
        }
    }
}

impl exports::ucode::hooks_transform::on_transform_messages::Guest for ContextManagerPlugin {
    fn handle(
        payload: ucode::hooks_types::types::TransformMessagesPayload,
    ) -> ucode::hooks_types::types::HookResponse {
        // Parse messages, remove consecutive duplicate assistant messages.
        let messages: Vec<serde_json::Value> =
            serde_json::from_str(&payload.messages_json).unwrap_or_default();

        let mut deduped: Vec<serde_json::Value> = Vec::new();
        for msg in &messages {
            let is_dup = if let (Some(prev), Some(curr_role)) =
                (deduped.last(), msg.get("role").and_then(|r| r.as_str()))
            {
                curr_role == "assistant"
                    && prev.get("role").and_then(|r| r.as_str()) == Some("assistant")
                    && prev.get("content") == msg.get("content")
            } else {
                false
            };
            if !is_dup {
                deduped.push(msg.clone());
            }
        }

        if deduped.len() < messages.len() {
            let deduped_json = serde_json::to_string(&deduped).unwrap_or_default();
            ucode::hooks_types::types::HookResponse {
                kind: ucode::hooks_types::types::HookResponseKind::Modify,
                data: Some(deduped_json),
            }
        } else {
            ucode::hooks_types::types::HookResponse {
                kind: ucode::hooks_types::types::HookResponseKind::Ok,
                data: None,
            }
        }
    }
}

impl exports::ucode::plugin::tool_provider::Guest for ContextManagerPlugin {
    fn tool_specs() -> Vec<ucode::hooks_types::types::ToolSpec> {
        vec![ucode::hooks_types::types::ToolSpec {
            name: "context_stats".to_string(),
            description: Some("Returns message count and total size".to_string()),
            input_schema: None,
        }]
    }

    fn invoke_tool(name: String, _args: String) -> Result<String, String> {
        match name.as_str() {
            "context_stats" => Ok(r#"{"message_count":42,"total_bytes":12345}"#.to_string()),
            _ => Err(format!("unknown tool: {name}")),
        }
    }
}

export!(ContextManagerPlugin);
