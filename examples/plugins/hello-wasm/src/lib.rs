//! Example WASM plugin that handles session_start events.

wit_bindgen::generate!({
    path: "wit",
    world: "hello-plugin",
    pub_export_macro: true,
    generate_all,
});

struct HelloPlugin;

impl exports::ucode::plugin::lifecycle::Guest for HelloPlugin {
    fn handshake() -> ucode::hooks_types::types::HandshakeRequest {
        ucode::hooks_types::types::HandshakeRequest {
            plugin_id: "org.ucode.hello-wasm".into(),
            plugin_version: "1.0.0".into(),
            min_api_version: "1.0.0".into(),
            required_features: vec!["hooks".into()],
        }
    }

    fn initialize(_result: ucode::hooks_types::types::HandshakeResult) -> Result<(), String> {
        ucode::plugin::host_log::log("hello-wasm: initialized!");
        Ok(())
    }

    fn shutdown() {
        ucode::plugin::host_log::log("hello-wasm: shutting down");
    }
}

impl exports::ucode::hooks_session::on_start::Guest for HelloPlugin {
    fn handle(
        payload: ucode::hooks_types::types::SessionStartPayload,
    ) -> ucode::hooks_types::types::HookResponse {
        ucode::plugin::host_log::log(&format!(
            "hello-wasm: session started with id={}",
            payload.session_id
        ));
        ucode::hooks_types::types::HookResponse {
            kind: ucode::hooks_types::types::HookResponseKind::Ok,
            data: None,
        }
    }
}

export!(HelloPlugin);
