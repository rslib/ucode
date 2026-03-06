//! Integration tests for WASM plugin loading and dispatch.
//!
//! These tests require the hello-wasm example plugin to be pre-built:
//! `cargo build -p hello-wasm --target wasm32-wasip2 --release`

#![cfg(feature = "wasm")]

use std::path::PathBuf;
use ucode_plugins::wasm::{WasmPlugin, WasmPluginError};

fn example_wasm_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/wasm32-wasip2/release/hello_wasm.wasm")
}

#[test]
fn test_load_wasm_plugin() {
    let path = example_wasm_path();
    if !path.exists() {
        eprintln!(
            "Skipping: build hello-wasm first: cargo build -p hello-wasm --target wasm32-wasip2 --release"
        );
        return;
    }
    let plugin = WasmPlugin::from_file(&path).expect("failed to load hello-wasm plugin");
    assert!(
        plugin.handles_event("session_start"),
        "plugin should handle session_start"
    );
}

#[test]
fn test_plugin_does_not_handle_unknown_event() {
    let path = example_wasm_path();
    if !path.exists() {
        return;
    }
    let plugin = WasmPlugin::from_file(&path).expect("failed to load");
    assert!(
        !plugin.handles_event("budget_warning"),
        "plugin should not handle budget_warning"
    );
}

#[test]
fn test_plugin_subscribed_events() {
    let path = example_wasm_path();
    if !path.exists() {
        return;
    }
    let plugin = WasmPlugin::from_file(&path).expect("failed to load");
    let events = plugin.subscribed_events();
    assert!(
        events.contains("session_start"),
        "subscribed events should include session_start, got: {:?}",
        events
    );
}

#[test]
fn test_load_invalid_wasm_file() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), b"not a wasm file").unwrap();
    let result = WasmPlugin::from_file(tmp.path());
    assert!(result.is_err());
    assert!(matches!(
        result.err().unwrap(),
        WasmPluginError::ComponentLoad(_)
    ));
}
