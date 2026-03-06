//! Guest SDK for writing ucode WASM plugins.
//!
//! Plugin authors depend on this crate and implement the generated traits.
//! Build with: `cargo build --target wasm32-wasip2`
//!
//! # Quick start
//!
//! The SDK generates guest-side bindings from the `minimal-plugin` WIT world,
//! which requires only the lifecycle interface. To handle hook events, create
//! your own `world.wit` that exports the specific hook interfaces you need,
//! and call `wit_bindgen::generate!` in your own crate.
//!
//! # Example
//!
//! ```ignore
//! use ucode_plugin_sdk::exports::ucode::plugin::lifecycle::Guest as Lifecycle;
//!
//! struct MyPlugin;
//!
//! impl Lifecycle for MyPlugin {
//!     fn handshake() -> ucode_plugin_sdk::ucode::hooks_types::types::HandshakeRequest {
//!         ucode_plugin_sdk::ucode::hooks_types::types::HandshakeRequest {
//!             plugin_id: "org.example.my-plugin".into(),
//!             plugin_version: "0.1.0".into(),
//!             min_api_version: "1.0.0".into(),
//!             required_features: vec!["hooks".into()],
//!         }
//!     }
//!
//!     fn initialize(
//!         _result: ucode_plugin_sdk::ucode::hooks_types::types::HandshakeResult,
//!     ) -> Result<(), String> {
//!         Ok(())
//!     }
//!
//!     fn shutdown() {}
//! }
//! ```

// Generate guest-side bindings from the minimal-plugin WIT world.
// This provides the lifecycle interface (mandatory) and shared types.
// Plugin authors who need hook interfaces should create their own world
// and call wit_bindgen::generate! in their crate.
wit_bindgen::generate!({
    path: "../ucode-plugins/wit",
    world: "minimal-plugin",
    pub_export_macro: true,
    generate_all,
});
