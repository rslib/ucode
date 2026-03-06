# WASM Runtime Design (Task 8.4 / ISSUE 0804)

## Goal

Add wasmtime-based WASM plugin support to `ucode-plugins` behind an optional
`wasm` feature flag. Plugin authors write WASM components that export typed WIT
interfaces -- one interface per hook event, grouped into versioned category
packages. The host dynamically probes which interfaces a component exports and
builds a dispatch table.

## Architecture

```
                        WIT definitions
                        (per-event interfaces)
                              |
              +---------------+---------------+
              |                               |
     Host side (wasmtime)            Guest side (wit-bindgen)
     bindgen! for types              bindgen! for exports
     Instance API for dispatch       typed handle() impls
              |                               |
        WasmPlugin adapter             .wasm component
        (impl Plugin + HookHandler)    (wasm32-wasip2)
              |
        PluginHost integration
        (alongside native plugins)
```

Three layers:

1. **WIT contract** -- typed interfaces per hook event, shared payload types,
   lifecycle interfaces. Versioned per category.
2. **Host runtime** (`src/wasm/`) -- loads `.wasm` components, probes exports,
   dispatches events via typed functions. Adapts to existing `Plugin` +
   `HookHandler` traits.
3. **Guest SDK** (`crates/ucode-plugin-sdk/`) -- re-exports wit-bindgen
   generated types, provides ergonomic macros for plugin authors.

## WIT Package Structure

```
ucode:hooks-types@1.0.0     shared payload records, hook-response enum
ucode:hooks-session@1.0.0   /on-start, /on-end, /on-title-generated,
                             /on-title-updated, /on-config-reloaded
ucode:hooks-message@1.0.0   /on-user-message, /on-response-started,
                             /on-response-completed, /on-retry
ucode:hooks-model@1.0.0     /on-before-call, /on-after-call,
                             /on-before-select, /on-fallback,
                             /on-router-decision, /on-rate-limited,
                             /on-quota-exhausted
ucode:hooks-tool@1.0.0      /on-before-call, /on-after-call,
                             /on-error, /on-timeout
ucode:hooks-tool-fs@1.0.0   /on-before-read, /on-after-read,
                             /on-before-write, /on-after-write
ucode:hooks-tool-cmd@1.0.0  /on-before-run, /on-after-run
ucode:hooks-tool-patch@1.0.0 /on-before-apply, /on-after-apply
ucode:hooks-context@1.0.0   /on-overflow, /on-compaction,
                             /on-distilled, /on-usage-updated
ucode:hooks-agent@1.0.0     /on-spawned, /on-message, /on-completed,
                             /on-failed, /on-cancelled
ucode:hooks-approval@1.0.0  /on-required, /on-granted, /on-denied,
                             /on-sandbox-decision, /on-permission-decision
ucode:hooks-auth@1.0.0      /on-changed, /on-failed, /on-provider-switched
ucode:hooks-mcp@1.0.0       /on-connected, /on-disconnected, /on-launch,
                             /on-restart, /on-crash, /on-tool-invoked
ucode:hooks-skill@1.0.0     /on-activated, /on-deactivated
ucode:hooks-plugin@1.0.0    /on-loaded, /on-unloaded, /on-error
ucode:hooks-checkpoint@1.0.0 /on-created, /on-restored
ucode:hooks-budget@1.0.0    /on-warning, /on-reached, /on-cost-incurred
ucode:hooks-job@1.0.0       /on-state-changed
ucode:hooks-command@1.0.0   /on-invoked, /on-palette-executed
ucode:hooks-diagnostic@1.0.0 /on-unhandled-error
ucode:plugin@1.0.0           lifecycle (handshake, initialize, shutdown),
                             tool-provider interface
```

**Totals:** 20 packages, 64 event interfaces + lifecycle + tool-provider.

Each category package is versioned independently. Bumping
`ucode:hooks-tool@2.0.0` does not affect plugins that only use
`ucode:hooks-session@1.0.0`.

## WIT Interface Pattern

Every event interface follows the same pattern:

```wit
// hooks-session.wit
package ucode:hooks-session@1.0.0;

use ucode:hooks-types@1.0.0 as types;

interface on-start {
    use types.{hook-response, session-start-payload};
    handle: func(payload: session-start-payload) -> hook-response;
}

interface on-end {
    use types.{hook-response, session-end-payload};
    handle: func(payload: session-end-payload) -> hook-response;
}
```

Shared types live in `ucode:hooks-types@1.0.0`:

```wit
package ucode:hooks-types@1.0.0;

interface types {
    enum hook-response-kind {
        ok,
        modify,
        veto,
    }

    record hook-response {
        kind: hook-response-kind,
        data: option<string>,
    }

    record session-start-payload {
        session-id: string,
    }

    record session-end-payload {
        session-id: string,
        duration-secs: f64,
    }

    // ... all 64 payload records
}
```

## Plugin World

A plugin author defines a world that exports only the interfaces they handle:

```wit
package my-org:session-logger@1.0.0;

world session-logger {
    // host provides
    import ucode:plugin/host-log;

    // plugin lifecycle (required)
    export ucode:plugin/lifecycle;

    // only the events this plugin cares about
    export ucode:hooks-session/on-start;
    export ucode:hooks-session/on-end;
}
```

## Host-Side Strategy

### Why not bindgen! for instantiation

`wasmtime::component::bindgen!` generates an `instantiate()` that **fails** if
the component doesn't export everything the world declares. Since plugins export
different subsets of the 64 interfaces, we cannot use a single "maximal world"
with bindgen!'s instantiate.

### Actual approach

1. **Use `bindgen!` for type generation only.** Define a maximal world in WIT,
   run bindgen! to get Rust types for all payload records and enums. We never
   call the generated `instantiate()`.

2. **Use low-level `Instance` API for dynamic dispatch.** After instantiating
   the component with a `Linker`, probe each interface:

   ```rust
   let instance = linker.instantiate(&mut store, &component)?;

   // Probe: does this plugin export ucode:hooks-session/on-start?
   if let Some(func) = instance
       .get_export(&mut store, None, "ucode:hooks-session/on-start")
       .and_then(|idx| instance.get_export(&mut store, Some(&idx), "handle"))
   {
       // Found -- register in dispatch table
       let typed = instance.get_typed_func::<(SessionStartPayload,), (HookResponse,)>(&mut store, ...)?;
       dispatch.insert("session_start", typed);
   }
   ```

3. **Build dispatch table at load time.** A `HashMap<&'static str, TypedFunc>`
   mapping event names to their typed WASM functions. Dispatch is O(1) lookup +
   function call.

4. **WasmPlugin adapter.** Wraps the dispatch table and implements our existing
   `Plugin` + `HookHandler` traits so WASM plugins integrate seamlessly with
   `PluginHost`.

### Dispatch table generation

The 64-event probing code is mechanical. We generate it with a declarative
macro:

```rust
macro_rules! probe_hooks {
    ($instance:expr, $store:expr, $table:expr, [
        $($event_name:literal => $wit_iface:literal : ($payload:ty) -> ($response:ty)),* $(,)?
    ]) => {
        $(
            if let Ok(func) = /* probe $wit_iface */ {
                $table.insert($event_name, Box::new(move |store, payload| {
                    // deserialize payload, call func, serialize response
                }));
            }
        )*
    };
}
```

## Guest SDK

`crates/ucode-plugin-sdk/` provides:

1. **wit-bindgen generated types** -- re-exported for plugin authors
2. **Export macro** -- generates the boilerplate to wire a Rust struct to WIT exports
3. **Typed event handlers** -- trait with default (no-op) methods per event

```rust
// Plugin author writes:
use ucode_plugin_sdk::prelude::*;

struct MyPlugin;

impl SessionHooks for MyPlugin {
    fn on_start(&mut self, payload: SessionStartPayload) -> HookResponse {
        HookResponse::ok()
    }
}

// Macro generates WIT export glue
export_plugin!(MyPlugin, [SessionHooks]);
```

## File Layout

```
crates/ucode-plugins/
  Cargo.toml                    # +wasmtime optional deps, `wasm` feature
  wit/
    hooks-types.wit             # ucode:hooks-types@1.0.0
    hooks-session.wit           # ucode:hooks-session@1.0.0
    hooks-message.wit           # ucode:hooks-message@1.0.0
    hooks-model.wit             # ucode:hooks-model@1.0.0
    hooks-tool.wit              # ucode:hooks-tool@1.0.0
    hooks-tool-fs.wit           # ucode:hooks-tool-fs@1.0.0
    hooks-tool-cmd.wit          # ucode:hooks-tool-cmd@1.0.0
    hooks-tool-patch.wit        # ucode:hooks-tool-patch@1.0.0
    hooks-context.wit           # ucode:hooks-context@1.0.0
    hooks-agent.wit             # ucode:hooks-agent@1.0.0
    hooks-approval.wit          # ucode:hooks-approval@1.0.0
    hooks-auth.wit              # ucode:hooks-auth@1.0.0
    hooks-mcp.wit               # ucode:hooks-mcp@1.0.0
    hooks-skill.wit             # ucode:hooks-skill@1.0.0
    hooks-plugin.wit            # ucode:hooks-plugin@1.0.0
    hooks-checkpoint.wit        # ucode:hooks-checkpoint@1.0.0
    hooks-budget.wit            # ucode:hooks-budget@1.0.0
    hooks-job.wit               # ucode:hooks-job@1.0.0
    hooks-command.wit            # ucode:hooks-command@1.0.0
    hooks-diagnostic.wit        # ucode:hooks-diagnostic@1.0.0
    plugin.wit                  # ucode:plugin@1.0.0 (lifecycle, tools, host-log)
    world.wit                   # maximal world (for bindgen! type generation)
  src/
    wasm/
      mod.rs                    # cfg(feature = "wasm"), bindgen!, re-exports
      host.rs                   # WasmPluginHost, dispatch table, probing
      convert.rs                # HookEvent <-> WIT payload conversion

crates/ucode-plugin-sdk/
  Cargo.toml                    # wit-bindgen dep, wasm32-wasip2 target
  wit/ -> symlink or copy of ucode-plugins/wit/
  src/
    lib.rs                      # re-exports, export_plugin! macro

examples/plugins/hello-wasm/
  Cargo.toml                    # depends on ucode-plugin-sdk, cdylib
  src/lib.rs                    # minimal session-start handler
```

## Version Compatibility

- Changing a payload record's fields = breaking change for that category package
- Safe evolution: add `on-start-v2` interface to `ucode:hooks-session@1.0.0`
  (non-breaking) or bump to `ucode:hooks-session@2.0.0` (breaking)
- Host dispatch tries v2 first, falls back to v1
- In practice: payload records are stable, so this is a safety net

## Scope for v1

For the initial implementation, we implement the full WIT surface (all 64
events) but focus integration testing on a starter set:

- `ucode:hooks-session/on-start` and `/on-end`
- `ucode:hooks-tool/on-before-call` and `/on-after-call`
- `ucode:plugin/lifecycle` (handshake, initialize, shutdown)

The remaining 60 events follow the exact same pattern and are covered by the
macro-generated probing code.

## Dependencies

```toml
# workspace Cargo.toml [workspace.dependencies]
wasmtime = { version = "42", features = ["component-model"] }
wasmtime-wasi = "42"

# crates/ucode-plugins/Cargo.toml
[features]
default = []
wasm = ["dep:wasmtime", "dep:wasmtime-wasi"]

[dependencies]
wasmtime = { workspace = true, optional = true }
wasmtime-wasi = { workspace = true, optional = true }

# crates/ucode-plugin-sdk/Cargo.toml
[dependencies]
wit-bindgen = "0.53"
```

## Testing Strategy

- Unit tests: WIT payload <-> HookEvent conversion (convert.rs)
- Unit tests: dispatch table construction with mock component
- Integration test: build hello-wasm example to .wasm, load via WasmPluginHost,
  dispatch session_start, verify response
- Integration test: plugin that doesn't export lifecycle -> handshake failure
- Integration test: WASM plugin loaded alongside native plugin in PluginHost
- All tests gated behind `#[cfg(feature = "wasm")]`
