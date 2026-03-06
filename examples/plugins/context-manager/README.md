# Context Manager Plugin

Demo WASM plugin for ucode that demonstrates:
- Hook handling (session start/end, transform messages)
- Message deduplication in the transform pipeline
- Tool registration and invocation via `tool-provider`

## Building

Requires the `wasm32-wasip2` target:

    rustup target add wasm32-wasip2
    cargo build --target wasm32-wasip2 --release

The compiled plugin will be at:

    target/wasm32-wasip2/release/context_manager_plugin.wasm

## Installation

Copy the plugin directory to `~/.ucode/plugins/context-manager/`:

    mkdir -p ~/.ucode/plugins/context-manager
    cp target/wasm32-wasip2/release/context_manager_plugin.wasm ~/.ucode/plugins/context-manager/plugin.wasm
    cp plugin.toml ~/.ucode/plugins/context-manager/

## Hooks

- `session_start` -- logs session ID on session open
- `session_end` -- logs session ID and duration on session close
- `transform_messages` -- removes consecutive duplicate assistant messages before model call

## Tools

- `context_stats` -- returns message count and total size (demo values)

## WIT structure

This plugin uses a subset of the ucode WIT world:

```
world context-manager-plugin {
    import ucode:plugin/host-log;

    export ucode:plugin/lifecycle;
    export ucode:plugin/tool-provider;
    export ucode:hooks-session/on-start;
    export ucode:hooks-session/on-end;
    export ucode:hooks-transform/on-transform-messages;
}
```

The `wit/deps/` directory contains copies of the canonical WIT definitions from
`crates/ucode-plugins/wit/deps/`. Keep them in sync when the host WIT evolves.
