//! WASM plugin runtime using wasmtime component model.
//!
//! Gated behind the `wasm` feature flag.
//!
//! Uses [`wasmtime::component::bindgen!`] to generate Rust types from WIT
//! definitions. The generated `instantiate()` is NOT used -- we use the
//! low-level [`wasmtime::component::Instance`] API for dynamic export
//! probing since plugins export different subsets of the 64 hook interfaces.

pub mod convert;
pub mod host;

pub use host::{WasmHostState, WasmPlugin, WasmPluginError};

// Generate Rust types from WIT definitions.
wasmtime::component::bindgen!({
    path: "wit",
    world: "maximal-plugin",
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wit_types_generated() {
        // Verify key types from hooks-types are generated
        let _resp = ucode::hooks_types::types::HookResponse {
            kind: ucode::hooks_types::types::HookResponseKind::Ok,
            data: None,
        };

        let _payload = ucode::hooks_types::types::SessionStartPayload {
            session_id: "test".to_string(),
        };
    }
}
