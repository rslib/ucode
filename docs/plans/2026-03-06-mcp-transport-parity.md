# MCP Transport Parity (Task 5.5) Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add SSE and Streamable HTTP transports to the MCP client, extract a `Transport` trait, add reconnect logic, and support configurable server connections via TOML.

**Architecture:** Extract `Transport` trait from existing `StdioTransport`. Add `SseTransport` (legacy 2024-11-05 spec) and `HttpTransport` (streamable HTTP 2025-03-26 spec) using `reqwest`. Reconnect logic wraps each transport with configurable retry strategies. `McpClient` becomes generic over `Box<dyn Transport>`. Server config parsed from TOML with env var expansion for headers.

**Tech Stack:** reqwest 0.12 (HTTP client with SSE streaming), tokio (async runtime), serde/serde_json (serialization), tracing (diagnostics)

---

## Task 1: Add reqwest to workspace dependencies

**Files:**
- Modify: `Cargo.toml` (workspace root, line ~83, after `regex = "1"`)
- Modify: `crates/ucode-mcp/Cargo.toml`

**Step 1: Add reqwest to workspace Cargo.toml**

In `Cargo.toml` (workspace root), add to `[workspace.dependencies]` after the `regex` entry:

```toml
# HTTP client (MCP SSE + HTTP transports)
reqwest = { version = "0.12", default-features = false, features = ["json", "stream", "rustls-tls"] }
```

**Step 2: Add reqwest + futures-util to ucode-mcp Cargo.toml**

In `crates/ucode-mcp/Cargo.toml`, add to `[dependencies]`:

```toml
reqwest = { workspace = true }
futures-util = { workspace = true }
```

**Step 3: Verify it compiles**

Run: `cargo check -p ucode-mcp`
Expected: PASS (no code uses reqwest yet, just dependency resolution)

**Step 4: Commit**

```
feat(mcp): add reqwest dependency for HTTP transports
```

---

## Task 2: Extract Transport trait and refactor StdioTransport

**Files:**
- Modify: `crates/ucode-mcp/src/transport.rs` — add `Transport` trait, implement for `StdioTransport`
- Modify: `crates/ucode-mcp/src/lib.rs` — export `Transport` trait

**Step 1: Write the Transport trait and implement it for StdioTransport**

At the top of `transport.rs`, add the `Transport` trait. Then implement it for `StdioTransport` by delegating to the existing methods. The existing inherent methods stay (they're used by the trait impl).

```rust
use serde_json::Value;

/// Transport abstraction for MCP communication.
///
/// All MCP transports (stdio, SSE, HTTP) implement this trait.
/// The client uses `Box<dyn Transport>` for runtime polymorphism.
pub trait Transport: Send {
    /// Send a JSON-RPC request and wait for the matching response.
    fn request(
        &mut self,
        method: &str,
        params: Option<Value>,
    ) -> impl std::future::Future<Output = Result<Value, McpError>> + Send;

    /// Send a JSON-RPC notification (no response expected).
    fn notify(
        &mut self,
        method: &str,
        params: Option<Value>,
    ) -> impl std::future::Future<Output = Result<(), McpError>> + Send;

    /// Shut down the transport and release resources.
    fn shutdown(
        &mut self,
    ) -> impl std::future::Future<Output = Result<(), McpError>> + Send;
}
```

Note: We use `impl Future` (RPITIT) instead of `async_trait` since we're on Rust 2024 edition. However, `Box<dyn Transport>` requires object safety. Since RPITIT is not object-safe, we need to use a different approach. Use a boxed future approach:

```rust
use std::future::Future;
use std::pin::Pin;

pub trait Transport: Send {
    fn request(
        &mut self,
        method: &str,
        params: Option<Value>,
    ) -> Pin<Box<dyn Future<Output = Result<Value, McpError>> + Send + '_>>;

    fn notify(
        &mut self,
        method: &str,
        params: Option<Value>,
    ) -> Pin<Box<dyn Future<Output = Result<(), McpError>> + Send + '_>>;

    fn shutdown(
        &mut self,
    ) -> Pin<Box<dyn Future<Output = Result<(), McpError>> + Send + '_>>;
}
```

Implement `Transport` for `StdioTransport`:

```rust
impl Transport for StdioTransport {
    fn request(
        &mut self,
        method: &str,
        params: Option<Value>,
    ) -> Pin<Box<dyn Future<Output = Result<Value, McpError>> + Send + '_>> {
        let method = method.to_owned();
        Box::pin(async move { self.send_request(&method, params).await })
    }

    fn notify(
        &mut self,
        method: &str,
        params: Option<Value>,
    ) -> Pin<Box<dyn Future<Output = Result<(), McpError>> + Send + '_>> {
        let method = method.to_owned();
        Box::pin(async move { self.send_notify(&method, params).await })
    }

    fn shutdown(
        &mut self,
    ) -> Pin<Box<dyn Future<Output = Result<(), McpError>> + Send + '_>> {
        Box::pin(async move { self.shutdown_process().await })
    }
}
```

Rename the existing inherent methods to avoid name collision:
- `request` -> `send_request`
- `notify` -> `send_notify`
- `shutdown` -> `shutdown_process`

**Step 2: Verify it compiles**

Run: `cargo check -p ucode-mcp`
Expected: PASS

**Step 3: Run existing tests**

Run: `cargo test -p ucode-mcp`
Expected: All existing tests pass (jsonrpc, types, etc.)

**Step 4: Commit**

```
refactor(mcp): extract Transport trait from StdioTransport
```

---

## Task 3: Refactor McpClient to use Box<dyn Transport>

**Files:**
- Modify: `crates/ucode-mcp/src/client.rs`
- Modify: `crates/ucode-mcp/src/lib.rs`

**Step 1: Add ClientInfo struct and refactor McpClient**

In `client.rs`:

```rust
use crate::transport::Transport;

/// Client identity sent during the MCP initialize handshake.
#[derive(Debug, Clone)]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
}

impl Default for ClientInfo {
    fn default() -> Self {
        Self {
            name: "ucode".into(),
            version: "0.1.0".into(),
        }
    }
}

pub struct McpClient {
    transport: Box<dyn Transport>,
    server_info: Option<ServerInfo>,
    server_capabilities: Option<ServerCapabilities>,
    client_info: ClientInfo,
}
```

**Step 2: Update McpClient methods**

- Keep `connect(command, args)` as a convenience that creates `StdioTransport` + wraps in `Box`
- Add `from_transport(transport: Box<dyn Transport>, client_info: ClientInfo)` constructor
- Update `initialize()` to use `self.client_info` instead of hardcoded values
- All method bodies stay the same — they already call `self.transport.request/notify/shutdown`

```rust
impl McpClient {
    /// Connect to an MCP server by spawning the given command (stdio transport).
    pub async fn connect(command: &str, args: &[&str]) -> Result<Self, McpError> {
        let transport = StdioTransport::spawn(command, args).await?;
        Ok(Self {
            transport: Box::new(transport),
            server_info: None,
            server_capabilities: None,
            client_info: ClientInfo::default(),
        })
    }

    /// Create a client from an existing transport.
    pub fn from_transport(transport: Box<dyn Transport>, client_info: ClientInfo) -> Self {
        Self {
            transport,
            server_info: None,
            server_capabilities: None,
            client_info,
        }
    }

    pub async fn initialize(&mut self) -> Result<&ServerInfo, McpError> {
        let params = json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {
                "name": self.client_info.name,
                "version": self.client_info.version
            }
        });
        // ... rest unchanged
    }
}
```

**Step 3: Export ClientInfo from lib.rs**

Add `ClientInfo` to the `pub use client::` line.

**Step 4: Verify it compiles and tests pass**

Run: `cargo test -p ucode-mcp`
Expected: All tests pass

**Step 5: Commit**

```
refactor(mcp): make McpClient generic over Transport trait
```

---

## Task 4: Add new error variants

**Files:**
- Modify: `crates/ucode-mcp/src/error.rs`

**Step 1: Add HTTP/SSE/reconnect/config error variants**

```rust
#[derive(Debug, thiserror::Error)]
pub enum McpError {
    // ... existing variants ...

    #[error("HTTP error {status}: {body}")]
    Http { status: u16, body: String },

    #[error("SSE connection error: {0}")]
    SseConnection(String),

    #[error("reconnect exhausted after {attempts} attempts")]
    ReconnectExhausted { attempts: usize },

    #[error("invalid transport config: {0}")]
    InvalidConfig(String),
}
```

**Step 2: Verify it compiles**

Run: `cargo check -p ucode-mcp`
Expected: PASS

**Step 3: Commit**

```
feat(mcp): add error variants for HTTP/SSE/reconnect transports
```

---

## Task 5: Implement ReconnectConfig and reconnect wrapper

**Files:**
- Create: `crates/ucode-mcp/src/reconnect.rs`
- Modify: `crates/ucode-mcp/src/lib.rs` — add `pub mod reconnect;`

**Step 1: Write tests for reconnect logic**

In `reconnect.rs`, add tests at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_defaults() {
        let config = ReconnectConfig::simple();
        assert_eq!(config.max_retries, Some(3));
        assert_eq!(config.backoff_base_ms, 1000);
        assert_eq!(config.backoff_cap_ms, 30_000);
    }

    #[test]
    fn persistent_no_max() {
        let config = ReconnectConfig::persistent();
        assert!(config.max_retries.is_none());
    }

    #[test]
    fn backoff_exponential_with_cap() {
        let config = ReconnectConfig::simple();
        assert_eq!(config.backoff_ms(0), 1000);
        assert_eq!(config.backoff_ms(1), 2000);
        assert_eq!(config.backoff_ms(2), 4000);
        // After cap
        assert_eq!(config.backoff_ms(10), 30_000);
    }

    #[test]
    fn should_retry_simple() {
        let config = ReconnectConfig::simple();
        assert!(config.should_retry(0));
        assert!(config.should_retry(2));
        assert!(!config.should_retry(3));
    }

    #[test]
    fn should_retry_persistent() {
        let config = ReconnectConfig::persistent();
        assert!(config.should_retry(0));
        assert!(config.should_retry(100));
        assert!(config.should_retry(10_000));
    }

    #[test]
    fn is_permanent_error() {
        assert!(super::is_permanent_error(401));
        assert!(super::is_permanent_error(403));
        assert!(super::is_permanent_error(404));
        assert!(!super::is_permanent_error(500));
        assert!(!super::is_permanent_error(502));
        assert!(!super::is_permanent_error(503));
        assert!(!super::is_permanent_error(429));
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p ucode-mcp reconnect`
Expected: FAIL (module doesn't exist yet)

**Step 3: Implement reconnect module**

```rust
//! Reconnect strategies for MCP HTTP transports.

use serde::{Deserialize, Serialize};

/// Reconnect strategy for transient transport failures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReconnectStrategy {
    /// 3 retries with exponential backoff, then fail.
    Simple,
    /// Retry indefinitely with capped exponential backoff.
    Persistent,
    /// User-defined max_retries and backoff parameters.
    Configurable,
}

/// Configuration for reconnect behavior on transient failures.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconnectConfig {
    pub strategy: ReconnectStrategy,
    /// Maximum retry attempts. `None` means unlimited (Persistent).
    pub max_retries: Option<usize>,
    /// Base backoff duration in milliseconds (default: 1000).
    pub backoff_base_ms: u64,
    /// Maximum backoff duration in milliseconds (default: 30000).
    pub backoff_cap_ms: u64,
}

impl ReconnectConfig {
    /// Simple: 3 retries, exponential backoff 1s base, 30s cap.
    pub fn simple() -> Self {
        Self {
            strategy: ReconnectStrategy::Simple,
            max_retries: Some(3),
            backoff_base_ms: 1000,
            backoff_cap_ms: 30_000,
        }
    }

    /// Persistent: unlimited retries, exponential backoff 1s base, 30s cap.
    pub fn persistent() -> Self {
        Self {
            strategy: ReconnectStrategy::Persistent,
            max_retries: None,
            backoff_base_ms: 1000,
            backoff_cap_ms: 30_000,
        }
    }

    /// Whether another retry should be attempted given the current attempt count.
    pub fn should_retry(&self, attempt: usize) -> bool {
        match self.max_retries {
            Some(max) => attempt < max,
            None => true,
        }
    }

    /// Compute backoff duration in milliseconds for the given attempt (0-indexed).
    pub fn backoff_ms(&self, attempt: usize) -> u64 {
        let exp = self.backoff_base_ms.saturating_mul(1u64 << attempt.min(31));
        exp.min(self.backoff_cap_ms)
    }
}

impl Default for ReconnectConfig {
    fn default() -> Self {
        Self::simple()
    }
}

/// Returns true if the HTTP status code indicates a permanent error
/// that should NOT be retried (4xx except 429 Too Many Requests).
pub fn is_permanent_error(status: u16) -> bool {
    (400..500).contains(&status) && status != 429
}
```

**Step 4: Add module to lib.rs**

Add `pub mod reconnect;` to `lib.rs`.

**Step 5: Run tests**

Run: `cargo test -p ucode-mcp reconnect`
Expected: All 6 tests pass

**Step 6: Commit**

```
feat(mcp): add reconnect strategies for HTTP transports
```

---

## Task 6: Implement ServerConfig with TOML parsing and env var expansion

**Files:**
- Create: `crates/ucode-mcp/src/server_config.rs`
- Modify: `crates/ucode-mcp/src/lib.rs` — add `pub mod server_config;`

**Step 1: Write tests for server config**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_stdio_config() {
        let toml = r#"
            name = "test-server"
            transport = "stdio"
            command = "node"
            args = ["server.js", "--port", "3000"]
        "#;
        let config: ServerConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.name, "test-server");
        match &config.transport_type {
            TransportType::Stdio { command, args, .. } => {
                assert_eq!(command, "node");
                assert_eq!(args, &["server.js", "--port", "3000"]);
            }
            _ => panic!("expected Stdio"),
        }
    }

    #[test]
    fn parse_sse_config() {
        let toml = r#"
            name = "remote-sse"
            transport = "sse"
            url = "https://example.com/mcp/sse"
            reconnect = "persistent"
        "#;
        let config: ServerConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.name, "remote-sse");
        match &config.transport_type {
            TransportType::Sse { url } => {
                assert_eq!(url, "https://example.com/mcp/sse");
            }
            _ => panic!("expected Sse"),
        }
        assert_eq!(config.reconnect.strategy, ReconnectStrategy::Persistent);
    }

    #[test]
    fn parse_http_config() {
        let toml = r#"
            name = "remote-http"
            transport = "streamable-http"
            url = "https://example.com/mcp"
            client_name = "kimi-code"
            client_version = "2.0.0"
        "#;
        let config: ServerConfig = toml::from_str(toml).unwrap();
        match &config.transport_type {
            TransportType::StreamableHttp { url } => {
                assert_eq!(url, "https://example.com/mcp");
            }
            _ => panic!("expected StreamableHttp"),
        }
        assert_eq!(config.client_name.as_deref(), Some("kimi-code"));
        assert_eq!(config.client_version.as_deref(), Some("2.0.0"));
    }

    #[test]
    fn env_var_expansion() {
        std::env::set_var("TEST_MCP_TOKEN", "secret123");
        let input = "Bearer ${TEST_MCP_TOKEN}";
        let result = expand_env_vars(input);
        assert_eq!(result, "Bearer secret123");
        std::env::remove_var("TEST_MCP_TOKEN");
    }

    #[test]
    fn env_var_expansion_missing() {
        let input = "Bearer ${NONEXISTENT_VAR_12345}";
        let result = expand_env_vars(input);
        assert_eq!(result, "Bearer ");
    }

    #[test]
    fn parse_headers() {
        std::env::set_var("TEST_AUTH_TOKEN", "tok_abc");
        let toml = r#"
            name = "with-headers"
            transport = "sse"
            url = "https://example.com/mcp"

            [headers]
            Authorization = "Bearer ${TEST_AUTH_TOKEN}"
            X-Custom = "static-value"
        "#;
        let config: ServerConfig = toml::from_str(toml).unwrap();
        let headers = config.expanded_headers();
        assert_eq!(headers.get("Authorization").unwrap(), "Bearer tok_abc");
        assert_eq!(headers.get("X-Custom").unwrap(), "static-value");
        std::env::remove_var("TEST_AUTH_TOKEN");
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p ucode-mcp server_config`
Expected: FAIL

**Step 3: Add toml dependency**

`toml` is needed for parsing. Check if it's in workspace — if not, add to `crates/ucode-mcp/Cargo.toml`:

```toml
toml = "0.8"
```

**Step 4: Implement server_config module**

```rust
//! MCP server configuration types with TOML parsing and env var expansion.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::reconnect::{ReconnectConfig, ReconnectStrategy};

/// Transport type for an MCP server connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "transport", rename_all = "kebab-case")]
pub enum TransportType {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: HashMap<String, String>,
    },
    Sse {
        url: String,
    },
    StreamableHttp {
        url: String,
    },
}

/// Configuration for a single MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub name: String,
    #[serde(flatten)]
    pub transport_type: TransportType,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub reconnect: ReconnectConfig,
    pub client_name: Option<String>,
    pub client_version: Option<String>,
}

impl ServerConfig {
    /// Return headers with `${VAR}` patterns expanded from environment variables.
    pub fn expanded_headers(&self) -> HashMap<String, String> {
        self.headers
            .iter()
            .map(|(k, v)| (k.clone(), expand_env_vars(v)))
            .collect()
    }
}

/// Expand `${VAR}` patterns in a string using environment variables.
/// Missing variables are replaced with empty string.
pub fn expand_env_vars(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$' && chars.peek() == Some(&'{') {
            chars.next(); // consume '{'
            let mut var_name = String::new();
            for c in chars.by_ref() {
                if c == '}' {
                    break;
                }
                var_name.push(c);
            }
            match std::env::var(&var_name) {
                Ok(val) => result.push_str(&val),
                Err(_) => {} // missing var -> empty string
            }
        } else {
            result.push(c);
        }
    }
    result
}
```

We also need to implement `Default` for `ReconnectConfig` deserialization from a string shorthand. Update the `ReconnectConfig` serde to support both string (`"simple"`) and struct forms. This requires a custom deserializer or we handle it in `ServerConfig` deserialization.

Actually, looking at the TOML config format:
```toml
reconnect = "simple"
```

This means `reconnect` is a string, but `ReconnectConfig` is a struct. We need a custom deserializer that accepts either a string shorthand or a full struct. Add this to `reconnect.rs`:

```rust
impl ReconnectConfig {
    /// Parse from a strategy name string shorthand.
    pub fn from_strategy_name(name: &str) -> Result<Self, String> {
        match name {
            "simple" => Ok(Self::simple()),
            "persistent" => Ok(Self::persistent()),
            _ => Err(format!("unknown reconnect strategy: {name}")),
        }
    }
}
```

And implement a custom `Deserialize` that handles both string and struct:

```rust
use serde::de;

impl<'de> Deserialize<'de> for ReconnectConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        // Try string first, then struct
        struct ReconnectVisitor;

        impl<'de> de::Visitor<'de> for ReconnectVisitor {
            type Value = ReconnectConfig;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a reconnect strategy name or config object")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                ReconnectConfig::from_strategy_name(v).map_err(E::custom)
            }

            fn visit_map<M: de::MapAccess<'de>>(self, map: M) -> Result<Self::Value, M::Error> {
                #[derive(Deserialize)]
                struct Inner {
                    strategy: ReconnectStrategy,
                    max_retries: Option<usize>,
                    #[serde(default = "default_backoff_base")]
                    backoff_base_ms: u64,
                    #[serde(default = "default_backoff_cap")]
                    backoff_cap_ms: u64,
                }
                fn default_backoff_base() -> u64 { 1000 }
                fn default_backoff_cap() -> u64 { 30_000 }

                let inner = Inner::deserialize(de::value::MapAccessDeserializer::new(map))?;
                Ok(ReconnectConfig {
                    strategy: inner.strategy,
                    max_retries: inner.max_retries,
                    backoff_base_ms: inner.backoff_base_ms,
                    backoff_cap_ms: inner.backoff_cap_ms,
                })
            }
        }

        deserializer.deserialize_any(ReconnectVisitor)
    }
}
```

Remove the `#[derive(Deserialize)]` from `ReconnectConfig` since we have a manual impl.

**Step 5: Add module to lib.rs, export types**

Add `pub mod server_config;` to `lib.rs` and add exports.

**Step 6: Run tests**

Run: `cargo test -p ucode-mcp server_config`
Expected: All 6 tests pass

**Step 7: Commit**

```
feat(mcp): add ServerConfig with TOML parsing and env var expansion
```

---

## Task 7: Implement HttpTransport (Streamable HTTP, 2025-03-26 spec)

**Files:**
- Create: `crates/ucode-mcp/src/transport_http.rs`
- Modify: `crates/ucode-mcp/src/lib.rs` — add `pub mod transport_http;`

**Step 1: Write tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_request_headers() {
        let mut headers = HashMap::new();
        headers.insert("Authorization".into(), "Bearer tok123".into());
        headers.insert("X-Custom".into(), "value".into());
        let header_map = build_header_map(&headers).unwrap();
        assert_eq!(
            header_map.get("Authorization").unwrap().to_str().unwrap(),
            "Bearer tok123"
        );
        assert_eq!(
            header_map.get("X-Custom").unwrap().to_str().unwrap(),
            "value"
        );
        // Content-Type always set
        assert_eq!(
            header_map.get("Content-Type").unwrap().to_str().unwrap(),
            "application/json"
        );
    }

    #[test]
    fn parse_json_response() {
        let body = r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[]}}"#;
        let resp: JsonRpcResponse = serde_json::from_str(body).unwrap();
        assert_eq!(resp.id, Some(1));
        assert!(resp.result.is_some());
    }

    #[test]
    fn parse_sse_data_line() {
        let line = "data: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"tools\":[]}}";
        let data = extract_sse_data(line);
        assert!(data.is_some());
        let resp: JsonRpcResponse = serde_json::from_str(data.unwrap()).unwrap();
        assert_eq!(resp.id, Some(1));
    }

    #[test]
    fn parse_sse_non_data_line() {
        assert!(extract_sse_data("event: message").is_none());
        assert!(extract_sse_data(": comment").is_none());
        assert!(extract_sse_data("").is_none());
    }
}
```

**Step 2: Implement HttpTransport**

The Streamable HTTP transport (2025-03-26 spec):
- Single POST endpoint
- Client sends JSON-RPC request as POST body
- Server responds with either:
  - `Content-Type: application/json` — direct JSON response
  - `Content-Type: text/event-stream` — SSE stream with `data:` lines containing JSON-RPC responses
- Session management via `Mcp-Session-Id` header (server sends in response, client echoes back)

```rust
//! Streamable HTTP transport for MCP (2025-03-26 spec).
//!
//! Single POST endpoint. Server responds with JSON or SSE stream.
//! Supports session management via `Mcp-Session-Id` header.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

use crate::error::McpError;
use crate::jsonrpc::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use crate::reconnect::{self, ReconnectConfig};
use crate::transport::Transport;

pub struct HttpTransport {
    url: String,
    client: reqwest::Client,
    headers: HeaderMap,
    next_id: u64,
    session_id: Option<String>,
    reconnect: ReconnectConfig,
}

impl HttpTransport {
    pub fn new(
        url: String,
        custom_headers: &HashMap<String, String>,
        reconnect: ReconnectConfig,
    ) -> Result<Self, McpError> {
        let headers = build_header_map(custom_headers)?;
        Ok(Self {
            url,
            client: reqwest::Client::new(),
            headers,
            next_id: 1,
            session_id: None,
            reconnect,
        })
    }

    async fn post_request(&mut self, body: String) -> Result<reqwest::Response, McpError> {
        let mut attempt = 0;
        loop {
            let mut req = self.client.post(&self.url)
                .headers(self.headers.clone())
                .body(body.clone());

            if let Some(ref sid) = self.session_id {
                req = req.header("Mcp-Session-Id", sid);
            }

            match req.send().await {
                Ok(resp) => {
                    // Capture session id from response
                    if let Some(sid) = resp.headers().get("mcp-session-id") {
                        if let Ok(s) = sid.to_str() {
                            self.session_id = Some(s.to_owned());
                        }
                    }

                    let status = resp.status().as_u16();
                    if status >= 400 {
                        if reconnect::is_permanent_error(status) || !self.reconnect.should_retry(attempt) {
                            let body_text = resp.text().await.unwrap_or_default();
                            return Err(McpError::Http { status, body: body_text });
                        }
                        // Transient error — retry
                    } else {
                        return Ok(resp);
                    }
                }
                Err(e) => {
                    if !self.reconnect.should_retry(attempt) {
                        return Err(McpError::SseConnection(e.to_string()));
                    }
                }
            }

            let backoff = self.reconnect.backoff_ms(attempt);
            tracing::warn!(
                transport = "streamable-http",
                url = %self.url,
                attempt = attempt + 1,
                backoff_ms = backoff,
                "retrying after transient error"
            );
            tokio::time::sleep(std::time::Duration::from_millis(backoff)).await;
            attempt += 1;
        }
    }

    async fn send_and_parse(&mut self, body: String, expected_id: u64) -> Result<serde_json::Value, McpError> {
        let resp = self.post_request(body).await?;
        let content_type = resp.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned();

        if content_type.contains("text/event-stream") {
            // SSE streaming response
            let mut stream = resp.bytes_stream();
            let mut buffer = String::new();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| McpError::SseConnection(e.to_string()))?;
                buffer.push_str(&String::from_utf8_lossy(&chunk));

                // Process complete lines
                while let Some(pos) = buffer.find('\n') {
                    let line = buffer[..pos].trim_end_matches('\r').to_owned();
                    buffer = buffer[pos + 1..].to_owned();

                    if let Some(data) = extract_sse_data(&line) {
                        let resp: JsonRpcResponse = serde_json::from_str(data)?;
                        if resp.id == Some(expected_id) {
                            if let Some(err) = resp.error {
                                return Err(McpError::JsonRpc {
                                    code: err.code,
                                    message: err.message,
                                });
                            }
                            return resp.result.ok_or_else(|| {
                                McpError::Protocol("response has neither result nor error".into())
                            });
                        }
                    }
                }
            }
            Err(McpError::Protocol("SSE stream ended without matching response".into()))
        } else {
            // Direct JSON response
            let text = resp.text().await.map_err(|e| McpError::SseConnection(e.to_string()))?;
            let resp: JsonRpcResponse = serde_json::from_str(&text)?;
            if let Some(err) = resp.error {
                return Err(McpError::JsonRpc {
                    code: err.code,
                    message: err.message,
                });
            }
            resp.result.ok_or_else(|| {
                McpError::Protocol("response has neither result nor error".into())
            })
        }
    }
}

impl Transport for HttpTransport {
    fn request(
        &mut self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, McpError>> + Send + '_>> {
        let id = self.next_id;
        self.next_id += 1;
        let req = JsonRpcRequest::new(id, method, params);
        let body = serde_json::to_string(&req).unwrap();
        Box::pin(async move { self.send_and_parse(body, id).await })
    }

    fn notify(
        &mut self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Pin<Box<dyn Future<Output = Result<(), McpError>> + Send + '_>> {
        let notif = JsonRpcNotification::new(method, params);
        let body = serde_json::to_string(&notif).unwrap();
        Box::pin(async move {
            self.post_request(body).await?;
            Ok(())
        })
    }

    fn shutdown(
        &mut self,
    ) -> Pin<Box<dyn Future<Output = Result<(), McpError>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }
}

/// Build a `HeaderMap` from user-provided headers, always including Content-Type.
fn build_header_map(custom: &HashMap<String, String>) -> Result<HeaderMap, McpError> {
    let mut map = HeaderMap::new();
    map.insert("Content-Type", HeaderValue::from_static("application/json"));
    for (k, v) in custom {
        let name = HeaderName::from_bytes(k.as_bytes())
            .map_err(|e| McpError::InvalidConfig(format!("invalid header name '{k}': {e}")))?;
        let value = HeaderValue::from_str(v)
            .map_err(|e| McpError::InvalidConfig(format!("invalid header value for '{k}': {e}")))?;
        map.insert(name, value);
    }
    Ok(map)
}

/// Extract the data payload from an SSE `data:` line.
pub fn extract_sse_data(line: &str) -> Option<&str> {
    line.strip_prefix("data: ").or_else(|| line.strip_prefix("data:"))
}
```

**Step 3: Add module to lib.rs**

**Step 4: Run tests**

Run: `cargo test -p ucode-mcp transport_http`
Expected: All 4 tests pass

**Step 5: Commit**

```
feat(mcp): implement Streamable HTTP transport (2025-03-26 spec)
```

---

## Task 8: Implement SseTransport (legacy SSE, 2024-11-05 spec)

**Files:**
- Create: `crates/ucode-mcp/src/transport_sse.rs`
- Modify: `crates/ucode-mcp/src/lib.rs` — add `pub mod transport_sse;`

**Step 1: Write tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_endpoint_event() {
        let line = "data: /messages?session_id=abc123";
        let endpoint = extract_endpoint_url(line, "https://example.com");
        assert_eq!(endpoint, Some("https://example.com/messages?session_id=abc123".to_string()));
    }

    #[test]
    fn parse_endpoint_event_absolute() {
        let line = "data: https://other.com/messages";
        let endpoint = extract_endpoint_url(line, "https://example.com");
        assert_eq!(endpoint, Some("https://other.com/messages".to_string()));
    }

    #[test]
    fn parse_message_event_data() {
        let data = r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[]}}"#;
        let resp: JsonRpcResponse = serde_json::from_str(data).unwrap();
        assert_eq!(resp.id, Some(1));
    }
}
```

**Step 2: Implement SseTransport**

The legacy SSE transport (2024-11-05 spec):
- Client opens GET `/sse` — long-lived SSE connection
- Server sends `event: endpoint` with `data: <POST URL>` (relative or absolute)
- Server sends `event: message` with `data: <JSON-RPC response>` for responses
- Client sends requests via POST to the endpoint URL

```rust
//! Legacy SSE transport for MCP (2024-11-05 spec).
//!
//! Server→client: GET /sse (long-lived SSE stream).
//! Client→server: POST to endpoint URL received via SSE `endpoint` event.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use tokio::sync::mpsc;

use crate::error::McpError;
use crate::jsonrpc::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use crate::reconnect::{self, ReconnectConfig};
use crate::transport::Transport;
use crate::transport_http::build_header_map;

pub struct SseTransport {
    base_url: String,
    post_url: Option<String>,
    client: reqwest::Client,
    headers: HeaderMap,
    next_id: u64,
    reconnect: ReconnectConfig,
    /// Channel receiving SSE message events from the background reader.
    message_rx: Option<mpsc::UnboundedReceiver<String>>,
    /// Handle for the background SSE reader task.
    sse_task: Option<tokio::task::JoinHandle<()>>,
}

impl SseTransport {
    /// Create and connect an SSE transport.
    ///
    /// Opens GET `{base_url}/sse`, waits for the `endpoint` event,
    /// then spawns a background task to read `message` events.
    pub async fn connect(
        base_url: String,
        custom_headers: &HashMap<String, String>,
        reconnect: ReconnectConfig,
    ) -> Result<Self, McpError> {
        let headers = build_header_map(custom_headers)?;
        let client = reqwest::Client::new();

        let mut transport = Self {
            base_url,
            post_url: None,
            client,
            headers,
            next_id: 1,
            reconnect,
            message_rx: None,
            sse_task: None,
        };

        transport.open_sse_stream().await?;
        Ok(transport)
    }

    async fn open_sse_stream(&mut self) -> Result<(), McpError> {
        let sse_url = format!("{}/sse", self.base_url.trim_end_matches('/'));

        let resp = self.client.get(&sse_url)
            .headers(self.headers.clone())
            .send()
            .await
            .map_err(|e| McpError::SseConnection(format!("GET {sse_url}: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(McpError::Http { status, body });
        }

        let (tx, rx) = mpsc::unbounded_channel();
        self.message_rx = Some(rx);

        let base_url = self.base_url.clone();
        let mut stream = resp.bytes_stream();
        let mut buffer = String::new();
        let mut current_event = String::new();
        let mut post_url: Option<String> = None;

        // Read until we get the endpoint event
        'outer: while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| McpError::SseConnection(e.to_string()))?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].trim_end_matches('\r').to_owned();
                buffer = buffer[pos + 1..].to_owned();

                if line.is_empty() {
                    // End of event
                    current_event.clear();
                    continue;
                }

                if let Some(event_type) = line.strip_prefix("event: ") {
                    current_event = event_type.to_owned();
                } else if let Some(data) = line.strip_prefix("data: ").or_else(|| line.strip_prefix("data:")) {
                    if current_event == "endpoint" {
                        post_url = extract_endpoint_url(&format!("data: {data}"), &base_url);
                        break 'outer;
                    }
                }
            }
        }

        let post_url = post_url.ok_or_else(|| {
            McpError::SseConnection("SSE stream closed without sending endpoint event".into())
        })?;
        self.post_url = Some(post_url);

        // Spawn background task to read message events
        let task = tokio::spawn(async move {
            let mut buf = buffer; // carry over remaining buffer
            while let Some(chunk) = stream.next().await {
                let Ok(chunk) = chunk else { break };
                buf.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(pos) = buf.find('\n') {
                    let line = buf[..pos].trim_end_matches('\r').to_owned();
                    buf = buf[pos + 1..].to_owned();

                    if let Some(data) = line.strip_prefix("data: ").or_else(|| line.strip_prefix("data:")) {
                        let _ = tx.send(data.to_owned());
                    }
                }
            }
        });
        self.sse_task = Some(task);

        Ok(())
    }

    async fn post_to_endpoint(&mut self, body: String) -> Result<(), McpError> {
        let url = self.post_url.as_ref().ok_or_else(|| {
            McpError::SseConnection("no endpoint URL — SSE not connected".into())
        })?;

        let mut attempt = 0;
        loop {
            let result = self.client.post(url)
                .headers(self.headers.clone())
                .body(body.clone())
                .send()
                .await;

            match result {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    if status >= 400 {
                        if reconnect::is_permanent_error(status) || !self.reconnect.should_retry(attempt) {
                            let body_text = resp.text().await.unwrap_or_default();
                            return Err(McpError::Http { status, body: body_text });
                        }
                    } else {
                        return Ok(());
                    }
                }
                Err(e) => {
                    if !self.reconnect.should_retry(attempt) {
                        return Err(McpError::SseConnection(e.to_string()));
                    }
                }
            }

            let backoff = self.reconnect.backoff_ms(attempt);
            tracing::warn!(
                transport = "sse",
                url = %url,
                attempt = attempt + 1,
                backoff_ms = backoff,
                "retrying POST after transient error"
            );
            tokio::time::sleep(std::time::Duration::from_millis(backoff)).await;
            attempt += 1;
        }
    }

    async fn recv_response(&mut self, expected_id: u64) -> Result<serde_json::Value, McpError> {
        let rx = self.message_rx.as_mut().ok_or_else(|| {
            McpError::SseConnection("SSE not connected".into())
        })?;

        while let Some(data) = rx.recv().await {
            let resp: JsonRpcResponse = serde_json::from_str(&data)?;
            if resp.id == Some(expected_id) {
                if let Some(err) = resp.error {
                    return Err(McpError::JsonRpc {
                        code: err.code,
                        message: err.message,
                    });
                }
                return resp.result.ok_or_else(|| {
                    McpError::Protocol("response has neither result nor error".into())
                });
            }
            // Skip non-matching responses
        }

        Err(McpError::ServerExited)
    }
}

impl Transport for SseTransport {
    fn request(
        &mut self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, McpError>> + Send + '_>> {
        let id = self.next_id;
        self.next_id += 1;
        let req = JsonRpcRequest::new(id, method, params);
        let body = serde_json::to_string(&req).unwrap();
        Box::pin(async move {
            self.post_to_endpoint(body).await?;
            self.recv_response(id).await
        })
    }

    fn notify(
        &mut self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Pin<Box<dyn Future<Output = Result<(), McpError>> + Send + '_>> {
        let notif = JsonRpcNotification::new(method, params);
        let body = serde_json::to_string(&notif).unwrap();
        Box::pin(async move { self.post_to_endpoint(body).await })
    }

    fn shutdown(
        &mut self,
    ) -> Pin<Box<dyn Future<Output = Result<(), McpError>> + Send + '_>> {
        Box::pin(async {
            if let Some(task) = self.sse_task.take() {
                task.abort();
            }
            self.message_rx = None;
            Ok(())
        })
    }
}

/// Extract endpoint URL from an SSE `data:` line.
/// If the data is a relative path, resolve against the base URL.
pub fn extract_endpoint_url(line: &str, base_url: &str) -> Option<String> {
    let data = line.strip_prefix("data: ").or_else(|| line.strip_prefix("data:"))?;
    let data = data.trim();
    if data.starts_with("http://") || data.starts_with("https://") {
        Some(data.to_owned())
    } else {
        // Relative path — resolve against base URL
        let base = base_url.trim_end_matches('/');
        Some(format!("{base}{data}"))
    }
}
```

**Step 3: Make `build_header_map` and `extract_sse_data` public in transport_http.rs**

So `transport_sse.rs` can reuse them.

**Step 4: Add module to lib.rs**

**Step 5: Run tests**

Run: `cargo test -p ucode-mcp transport_sse`
Expected: All 3 tests pass

**Step 6: Commit**

```
feat(mcp): implement legacy SSE transport (2024-11-05 spec)
```

---

## Task 9: Add factory function to McpClient

**Files:**
- Modify: `crates/ucode-mcp/src/client.rs`

**Step 1: Write test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_info_default() {
        let info = ClientInfo::default();
        assert_eq!(info.name, "ucode");
        assert_eq!(info.version, "0.1.0");
    }

    #[test]
    fn client_info_custom() {
        let config = ServerConfig {
            name: "test".into(),
            transport_type: TransportType::Stdio {
                command: "echo".into(),
                args: vec![],
                env: Default::default(),
            },
            headers: Default::default(),
            reconnect: ReconnectConfig::default(),
            client_name: Some("kimi-code".into()),
            client_version: Some("2.0.0".into()),
        };
        let info = ClientInfo::from_config(&config);
        assert_eq!(info.name, "kimi-code");
        assert_eq!(info.version, "2.0.0");
    }

    #[test]
    fn client_info_partial_override() {
        let config = ServerConfig {
            name: "test".into(),
            transport_type: TransportType::Stdio {
                command: "echo".into(),
                args: vec![],
                env: Default::default(),
            },
            headers: Default::default(),
            reconnect: ReconnectConfig::default(),
            client_name: Some("custom".into()),
            client_version: None,
        };
        let info = ClientInfo::from_config(&config);
        assert_eq!(info.name, "custom");
        assert_eq!(info.version, "0.1.0"); // default
    }
}
```

**Step 2: Add `ClientInfo::from_config` and `McpClient::connect_with_config`**

```rust
impl ClientInfo {
    pub fn from_config(config: &ServerConfig) -> Self {
        Self {
            name: config.client_name.clone().unwrap_or_else(|| "ucode".into()),
            version: config.client_version.clone().unwrap_or_else(|| "0.1.0".into()),
        }
    }
}

impl McpClient {
    /// Create a client from a ServerConfig, selecting the appropriate transport.
    pub async fn connect_with_config(config: &ServerConfig) -> Result<Self, McpError> {
        let headers = config.expanded_headers();
        let client_info = ClientInfo::from_config(config);

        let transport: Box<dyn Transport> = match &config.transport_type {
            TransportType::Stdio { command, args, env: _ } => {
                let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                Box::new(StdioTransport::spawn(command, &args_refs).await?)
            }
            TransportType::Sse { url } => {
                Box::new(SseTransport::connect(
                    url.clone(),
                    &headers,
                    config.reconnect.clone(),
                ).await?)
            }
            TransportType::StreamableHttp { url } => {
                Box::new(HttpTransport::new(
                    url.clone(),
                    &headers,
                    config.reconnect.clone(),
                )?)
            }
        };

        Ok(Self::from_transport(transport, client_info))
    }
}
```

**Step 3: Run tests**

Run: `cargo test -p ucode-mcp client`
Expected: All 3 tests pass

**Step 4: Commit**

```
feat(mcp): add factory connect_with_config for transport selection
```

---

## Task 10: Update lib.rs exports and verify full build

**Files:**
- Modify: `crates/ucode-mcp/src/lib.rs`

**Step 1: Update lib.rs with all new modules and exports**

```rust
pub mod reconnect;
pub mod server_config;
pub mod transport_http;
pub mod transport_sse;

// Add to existing pub use blocks:
pub use client::ClientInfo;
pub use reconnect::{ReconnectConfig, ReconnectStrategy};
pub use server_config::{ServerConfig, TransportType, expand_env_vars};
pub use transport::Transport;
pub use transport_http::HttpTransport;
pub use transport_sse::SseTransport;
```

**Step 2: Full build check**

Run: `cargo build -p ucode-mcp`
Expected: PASS

**Step 3: Full test suite**

Run: `cargo test -p ucode-mcp`
Expected: All tests pass (existing + new)

**Step 4: Clippy**

Run: `cargo clippy -p ucode-mcp -- -D warnings`
Expected: No warnings

**Step 5: Full workspace check**

Run: `cargo check --workspace`
Expected: PASS (no downstream breakage)

**Step 6: Commit**

```
feat(mcp): complete transport parity — export all new types
```

---

## Task 11: Update PLANS.md and EPIC.md

**Files:**
- Modify: `PLANS.md` — update Task 5.5 with design details and mark status
- Modify: `EPIC.md` — update ISSUE 0505 with design details and mark status

**Step 1: Update both files with implementation details**

**Step 2: Commit**

```
docs: update Task 5.5 / ISSUE 0505 with transport parity design
```

---

## Summary

| Task | Description | New files | Tests |
|------|-------------|-----------|-------|
| 1 | Add reqwest dependency | — | — |
| 2 | Extract Transport trait | — | 0 (existing pass) |
| 3 | Refactor McpClient to Box<dyn Transport> | — | 0 (existing pass) |
| 4 | Add error variants | — | 0 |
| 5 | Reconnect config + strategies | `reconnect.rs` | 6 |
| 6 | ServerConfig + TOML + env vars | `server_config.rs` | 6 |
| 7 | HttpTransport (streamable HTTP) | `transport_http.rs` | 4 |
| 8 | SseTransport (legacy SSE) | `transport_sse.rs` | 3 |
| 9 | Factory connect_with_config | — | 3 |
| 10 | Exports + full verification | — | — |
| 11 | PLANS.md + EPIC.md updates | — | — |

**Total new tests:** ~22
**Total new files:** 4 (`reconnect.rs`, `server_config.rs`, `transport_http.rs`, `transport_sse.rs`)
**Modified files:** 5 (`transport.rs`, `client.rs`, `error.rs`, `lib.rs`, `Cargo.toml`)
