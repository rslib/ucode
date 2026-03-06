//! Integration tests verifying the context-manager fixture plugin structure.
//!
//! Validates the plugin manifest and WIT file layout without requiring WASM
//! cross-compilation. Actual WASM dispatch tests require a compiled `.wasm`
//! binary (see examples/plugins/context-manager/README.md).

use std::path::Path;

fn fixture_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/plugins/context-manager")
}

#[test]
fn test_fixture_plugin_manifest_exists() {
    let path = fixture_root().join("plugin.toml");
    assert!(
        path.exists(),
        "fixture plugin manifest not found at {}",
        path.display()
    );
}

#[test]
fn test_fixture_plugin_manifest_parses() {
    let path = fixture_root().join("plugin.toml");
    let manifest =
        ucode_plugins::manifest::parse_manifest_file(&path).expect("manifest should parse");
    assert_eq!(manifest.id.as_deref(), Some("org.example.context-manager"));
    assert_eq!(manifest.name, "Context Manager");
    assert_eq!(manifest.version, "0.1.0");
    assert!(
        manifest.hooks.contains(&"session_start".to_string()),
        "hooks should contain session_start, got: {:?}",
        manifest.hooks
    );
    assert!(
        manifest.hooks.contains(&"session_end".to_string()),
        "hooks should contain session_end, got: {:?}",
        manifest.hooks
    );
    assert!(
        manifest.hooks.contains(&"transform_messages".to_string()),
        "hooks should contain transform_messages, got: {:?}",
        manifest.hooks
    );
    assert_eq!(
        manifest.tools.len(),
        1,
        "expected 1 tool, got: {:?}",
        manifest.tools
    );
    assert_eq!(manifest.tools[0].name, "context_stats");
}

#[test]
fn test_fixture_plugin_wit_world_exists() {
    let path = fixture_root().join("wit/world.wit");
    assert!(
        path.exists(),
        "fixture plugin world.wit not found at {}",
        path.display()
    );
}

#[test]
fn test_fixture_plugin_wit_world_content() {
    let path = fixture_root().join("wit/world.wit");
    let content = std::fs::read_to_string(&path).expect("world.wit should be readable");
    assert!(
        content.contains("world context-manager-plugin"),
        "world.wit should declare world context-manager-plugin"
    );
    assert!(
        content.contains("ucode:plugin/host-log"),
        "world.wit should import host-log"
    );
    assert!(
        content.contains("ucode:hooks-session/on-start"),
        "world.wit should export on-start"
    );
    assert!(
        content.contains("ucode:hooks-session/on-end"),
        "world.wit should export on-end"
    );
    assert!(
        content.contains("ucode:hooks-transform/on-transform-messages"),
        "world.wit should export on-transform-messages"
    );
    assert!(
        content.contains("ucode:plugin/tool-provider"),
        "world.wit should export tool-provider"
    );
}

#[test]
fn test_fixture_plugin_wit_deps_exist() {
    let deps = fixture_root().join("wit/deps");
    for (dir, file) in [
        ("hooks-types", "types.wit"),
        ("plugin", "plugin.wit"),
        ("hooks-session", "hooks-session.wit"),
        ("hooks-transform", "hooks-transform.wit"),
    ] {
        let path = deps.join(dir).join(file);
        assert!(path.exists(), "WIT dep not found at {}", path.display());
    }
}

#[test]
fn test_fixture_plugin_source_exists() {
    let path = fixture_root().join("src/lib.rs");
    assert!(
        path.exists(),
        "fixture plugin source not found at {}",
        path.display()
    );
}

#[test]
fn test_fixture_plugin_source_content() {
    let path = fixture_root().join("src/lib.rs");
    let content = std::fs::read_to_string(&path).expect("lib.rs should be readable");
    assert!(
        content.contains("context-manager-plugin"),
        "lib.rs should reference the WIT world name"
    );
    assert!(
        content.contains("ContextManagerPlugin"),
        "lib.rs should define ContextManagerPlugin"
    );
    assert!(
        content.contains("on_start"),
        "lib.rs should implement on_start"
    );
    assert!(content.contains("on_end"), "lib.rs should implement on_end");
    assert!(
        content.contains("on_transform_messages"),
        "lib.rs should implement on_transform_messages"
    );
    assert!(
        content.contains("tool_specs"),
        "lib.rs should implement tool_specs"
    );
    assert!(
        content.contains("invoke_tool"),
        "lib.rs should implement invoke_tool"
    );
}

#[test]
fn test_fixture_plugin_cargo_toml_exists() {
    let path = fixture_root().join("Cargo.toml");
    assert!(
        path.exists(),
        "fixture plugin Cargo.toml not found at {}",
        path.display()
    );
}

#[test]
fn test_fixture_plugin_cargo_toml_is_standalone() {
    let path = fixture_root().join("Cargo.toml");
    let content = std::fs::read_to_string(&path).expect("Cargo.toml should be readable");
    // Standalone project: no workspace = true for version/edition
    assert!(
        content.contains("cdylib"),
        "Cargo.toml should declare cdylib crate type"
    );
    assert!(
        content.contains("wit-bindgen"),
        "Cargo.toml should depend on wit-bindgen"
    );
    // Must NOT reference workspace = true (it's standalone)
    assert!(
        !content.contains("workspace = true"),
        "standalone plugin Cargo.toml must not use workspace = true"
    );
}
