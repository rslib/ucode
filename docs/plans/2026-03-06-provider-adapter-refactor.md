# Provider Adapter Refactor (Task 3.6) Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace hardcoded single-endpoint providers with four generic, configurable adapters (OpenAI-compat, Anthropic-compat, Ollama native, Gemini) that cover the entire LLM API landscape via TOML config.

**Architecture:** Each adapter is protocol-generic. Users configure named provider instances in TOML with `type`, `base_url`, `api_key_env`, and optional `headers`. A factory function maps config to the correct adapter. SSE byte-stream-to-event-stream logic is extracted into a shared helper. Ollama is rewritten from OpenAI-compat `/v1/chat/completions` to native `/api/chat` (NDJSON streaming). Gemini is new.

**Tech Stack:** Rust, reqwest, tokio, serde, serde_json, toml, futures-util, futures-core, ucode-core

---

## Task 1: ProviderConfig struct and toml dependency

**Files:**
- Modify: `crates/ucode-providers/Cargo.toml`
- Create: `crates/ucode-providers/src/config.rs`
- Modify: `crates/ucode-providers/src/lib.rs`

**Step 1: Add toml dependency**

```bash
cargo add toml -p ucode-providers
```

Then edit `crates/ucode-providers/Cargo.toml` — change the toml line to: `toml = "0.8"` (match workspace style, but toml is not in workspace deps so keep it direct).

**Step 2: Write tests for ProviderConfig**

Create `crates/ucode-providers/src/config.rs` with tests first:

```rust
use std::collections::HashMap;

use serde::Deserialize;

/// Which protocol adapter to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterKind {
    Openai,
    Anthropic,
    Ollama,
    Gemini,
}

impl AdapterKind {
    /// Default base URL for this adapter kind.
    pub fn default_base_url(&self) -> &'static str {
        match self {
            Self::Openai => "https://api.openai.com/v1",
            Self::Anthropic => "https://api.anthropic.com",
            Self::Ollama => "http://localhost:11434",
            Self::Gemini => "https://generativelanguage.googleapis.com",
        }
    }
}

/// Configuration for a single named provider instance.
///
/// Parsed from TOML:
/// ```toml
/// [providers.groq]
/// type = "openai"
/// base_url = "https://api.groq.com/openai/v1"
/// api_key_env = "GROQ_API_KEY"
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct ProviderConfig {
    /// Protocol adapter type.
    #[serde(rename = "type")]
    pub adapter: AdapterKind,

    /// Base URL override. Defaults per adapter kind if absent.
    #[serde(default)]
    pub base_url: Option<String>,

    /// Environment variable name holding the API key (not the key itself).
    #[serde(default)]
    pub api_key_env: Option<String>,

    /// Extra HTTP headers (e.g., Azure `api-version`, proxy auth).
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

impl ProviderConfig {
    /// Resolved base URL: explicit override or adapter default.
    pub fn base_url(&self) -> &str {
        self.base_url
            .as_deref()
            .unwrap_or_else(|| self.adapter.default_base_url())
    }

    /// Read the API key from the environment variable named by `api_key_env`.
    /// Returns `None` if `api_key_env` is not set or the env var is missing.
    pub fn resolve_api_key(&self) -> Option<String> {
        self.api_key_env
            .as_ref()
            .and_then(|var| std::env::var(var).ok())
    }
}

/// Top-level providers table: `[providers]` in TOML.
#[derive(Debug, Clone, Deserialize)]
pub struct ProvidersTable {
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_openai_config() {
        let toml_str = r#"
            [providers.openai]
            type = "openai"
            api_key_env = "OPENAI_API_KEY"
        "#;
        let table: ProvidersTable = toml::from_str(toml_str).unwrap();
        let cfg = &table.providers["openai"];
        assert_eq!(cfg.adapter, AdapterKind::Openai);
        assert_eq!(cfg.base_url(), "https://api.openai.com/v1");
        assert_eq!(cfg.api_key_env.as_deref(), Some("OPENAI_API_KEY"));
        assert!(cfg.headers.is_empty());
    }

    #[test]
    fn parse_groq_config_with_base_url() {
        let toml_str = r#"
            [providers.groq]
            type = "openai"
            base_url = "https://api.groq.com/openai/v1"
            api_key_env = "GROQ_API_KEY"
        "#;
        let table: ProvidersTable = toml::from_str(toml_str).unwrap();
        let cfg = &table.providers["groq"];
        assert_eq!(cfg.adapter, AdapterKind::Openai);
        assert_eq!(cfg.base_url(), "https://api.groq.com/openai/v1");
    }

    #[test]
    fn parse_ollama_no_api_key() {
        let toml_str = r#"
            [providers.ollama]
            type = "ollama"
        "#;
        let table: ProvidersTable = toml::from_str(toml_str).unwrap();
        let cfg = &table.providers["ollama"];
        assert_eq!(cfg.adapter, AdapterKind::Ollama);
        assert_eq!(cfg.base_url(), "http://localhost:11434");
        assert!(cfg.api_key_env.is_none());
    }

    #[test]
    fn parse_azure_with_custom_headers() {
        let toml_str = r#"
            [providers.azure]
            type = "openai"
            base_url = "https://myresource.openai.azure.com/openai/deployments/gpt-4o"
            api_key_env = "AZURE_OPENAI_API_KEY"
            [providers.azure.headers]
            api-version = "2024-10-21"
        "#;
        let table: ProvidersTable = toml::from_str(toml_str).unwrap();
        let cfg = &table.providers["azure"];
        assert_eq!(cfg.adapter, AdapterKind::Openai);
        assert_eq!(cfg.headers.get("api-version").unwrap(), "2024-10-21");
    }

    #[test]
    fn parse_gemini_config() {
        let toml_str = r#"
            [providers.gemini]
            type = "gemini"
            api_key_env = "GEMINI_API_KEY"
        "#;
        let table: ProvidersTable = toml::from_str(toml_str).unwrap();
        let cfg = &table.providers["gemini"];
        assert_eq!(cfg.adapter, AdapterKind::Gemini);
        assert_eq!(
            cfg.base_url(),
            "https://generativelanguage.googleapis.com"
        );
    }

    #[test]
    fn parse_anthropic_config() {
        let toml_str = r#"
            [providers.anthropic]
            type = "anthropic"
            api_key_env = "ANTHROPIC_API_KEY"
        "#;
        let table: ProvidersTable = toml::from_str(toml_str).unwrap();
        let cfg = &table.providers["anthropic"];
        assert_eq!(cfg.adapter, AdapterKind::Anthropic);
        assert_eq!(cfg.base_url(), "https://api.anthropic.com");
    }

    #[test]
    fn parse_multiple_providers() {
        let toml_str = r#"
            [providers.openai]
            type = "openai"
            api_key_env = "OPENAI_API_KEY"

            [providers.groq]
            type = "openai"
            base_url = "https://api.groq.com/openai/v1"
            api_key_env = "GROQ_API_KEY"

            [providers.ollama]
            type = "ollama"
        "#;
        let table: ProvidersTable = toml::from_str(toml_str).unwrap();
        assert_eq!(table.providers.len(), 3);
        assert!(table.providers.contains_key("openai"));
        assert!(table.providers.contains_key("groq"));
        assert!(table.providers.contains_key("ollama"));
    }

    #[test]
    fn default_base_urls() {
        assert_eq!(
            AdapterKind::Openai.default_base_url(),
            "https://api.openai.com/v1"
        );
        assert_eq!(
            AdapterKind::Anthropic.default_base_url(),
            "https://api.anthropic.com"
        );
        assert_eq!(
            AdapterKind::Ollama.default_base_url(),
            "http://localhost:11434"
        );
        assert_eq!(
            AdapterKind::Gemini.default_base_url(),
            "https://generativelanguage.googleapis.com"
        );
    }

    #[test]
    fn resolve_api_key_missing_env() {
        let cfg = ProviderConfig {
            adapter: AdapterKind::Openai,
            base_url: None,
            api_key_env: Some("UCODE_TEST_NONEXISTENT_KEY_XYZ".into()),
            headers: HashMap::new(),
        };
        assert!(cfg.resolve_api_key().is_none());
    }

    #[test]
    fn resolve_api_key_no_env_var_configured() {
        let cfg = ProviderConfig {
            adapter: AdapterKind::Ollama,
            base_url: None,
            api_key_env: None,
            headers: HashMap::new(),
        };
        assert!(cfg.resolve_api_key().is_none());
    }
}
```

**Step 3: Register module in lib.rs**

Add `pub mod config;` to `crates/ucode-providers/src/lib.rs` and add to the re-exports:
```rust
pub use config::{AdapterKind, ProviderConfig, ProvidersTable};
```

**Step 4: Verify**

```bash
cargo build -p ucode-providers && cargo test -p ucode-providers && cargo clippy -p ucode-providers
```

Expected: all tests pass, no clippy warnings.

**Step 5: Commit**

```
feat(providers): add ProviderConfig with TOML parsing and adapter kind defaults
```

---

## Task 2: Extract SSE byte-stream-to-events helper

**Files:**
- Create: `crates/ucode-providers/src/sse.rs`
- Modify: `crates/ucode-providers/src/lib.rs`
- Modify: `crates/ucode-providers/src/openai.rs` (use shared helper)
- Modify: `crates/ucode-providers/src/anthropic.rs` (use shared helper)

**Step 1: Create sse.rs with the shared stream helper**

The pattern is identical across OpenAI and Anthropic: read bytes from a `reqwest` response byte stream, split into lines, feed each line to a parser function, flatten events. Extract this into a generic function.

```rust
use futures_core::Stream;
use futures_util::stream;
use ucode_core::{Event, EventStream};

/// Convert a byte stream into an `EventStream` by splitting into lines and
/// feeding each line to `parse_line`. The parser receives a mutable accumulator
/// `A` that carries state across lines (e.g., tool call fragments).
///
/// This is the shared SSE/NDJSON streaming core used by all provider adapters.
pub fn byte_stream_to_events<S, A, F>(
    byte_stream: S,
    accumulator: A,
    parse_line: F,
) -> EventStream
where
    S: Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send + Unpin + 'static,
    A: Send + 'static,
    F: Fn(&str, &mut A) -> Vec<Event> + Send + 'static,
{
    let event_stream = stream::unfold(
        (byte_stream, accumulator, String::new()),
        move |(mut byte_stream, mut acc, mut buffer)| {
            let parse = &parse_line;
            // We need to move parse_line into the async block.
            // Since unfold takes FnMut, we need a different approach.
            // Actually, unfold's closure is called repeatedly, so we can't
            // reference parse_line by ref. Let's use a different pattern.
            async { None::<(Vec<Event>, (S, A, String))> }
        },
    );
    // The above won't work due to lifetime issues with closures.
    // Instead, use the concrete pattern but as a function.
    // Let me use a simpler approach: just provide the unfold body as a helper.

    // Actually, the cleanest approach is to keep the unfold inline in each
    // provider but extract the line-splitting logic. Let me reconsider.
    todo!()
}
```

Wait — Rust's async closures and `Stream::unfold` make it hard to abstract the full unfold with a generic parser closure due to lifetime/ownership issues. The simpler and more practical approach: **extract just the line-splitting buffer logic** and keep the unfold in each provider.

Revised approach: Create a `LineBuffer` struct that handles the byte-to-line splitting, and each provider uses it in their unfold.

```rust
/// Accumulates bytes and yields complete lines.
#[derive(Debug, Default)]
pub struct LineBuffer {
    buffer: String,
}

impl LineBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Push raw bytes into the buffer.
    pub fn push_bytes(&mut self, bytes: &[u8]) {
        self.buffer.push_str(&String::from_utf8_lossy(bytes));
    }

    /// Extract the next complete line (up to `\n`), if available.
    pub fn next_line(&mut self) -> Option<String> {
        if let Some(pos) = self.buffer.find('\n') {
            let line = self.buffer[..pos].to_string();
            self.buffer = self.buffer[pos + 1..].to_string();
            Some(line)
        } else {
            None
        }
    }

    /// Drain any remaining content as a final line (for stream end).
    pub fn drain(&mut self) -> Option<String> {
        if self.buffer.trim().is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.buffer))
        }
    }
}
```

Then each provider's unfold becomes:

```rust
let event_stream = futures_util::stream::unfold(
    (byte_stream, accumulator, LineBuffer::new()),
    |(mut byte_stream, mut acc, mut line_buf)| async move {
        use futures_util::StreamExt;
        loop {
            while let Some(line) = line_buf.next_line() {
                let events = parse_sse_line(&line, &mut acc);
                if !events.is_empty() {
                    return Some((events, (byte_stream, acc, line_buf)));
                }
            }
            match byte_stream.next().await {
                Some(Ok(bytes)) => line_buf.push_bytes(&bytes),
                Some(Err(_)) | None => {
                    if let Some(remaining) = line_buf.drain() {
                        let events = parse_sse_line(&remaining, &mut acc);
                        if !events.is_empty() {
                            return Some((events, (byte_stream, acc, line_buf)));
                        }
                    }
                    return None;
                }
            }
        }
    },
);
```

This is cleaner and eliminates the duplicated buffer management across providers.

Also provide a convenience function that does the full unfold + flatten for the common case:

```rust
/// Build a flat `EventStream` from a byte stream using the given line parser.
///
/// This is the standard streaming pattern used by SSE and NDJSON providers.
pub fn stream_lines<S, A, F>(byte_stream: S, accumulator: A, parse_line: F) -> EventStream
where
    S: Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send + Unpin + 'static,
    A: Send + 'static,
    F: Fn(&str, &mut A) -> Vec<Event> + Send + 'static,
{
    let event_stream = futures_util::stream::unfold(
        (byte_stream, accumulator, LineBuffer::new(), parse_line),
        |(mut byte_stream, mut acc, mut line_buf, parse)| async move {
            use futures_util::StreamExt;
            loop {
                while let Some(line) = line_buf.next_line() {
                    let events = parse(&line, &mut acc);
                    if !events.is_empty() {
                        return Some((events, (byte_stream, acc, line_buf, parse)));
                    }
                }
                match byte_stream.next().await {
                    Some(Ok(bytes)) => line_buf.push_bytes(&bytes),
                    Some(Err(_)) | None => {
                        if let Some(remaining) = line_buf.drain() {
                            let events = parse(&remaining, &mut acc);
                            if !events.is_empty() {
                                return Some((events, (byte_stream, acc, line_buf, parse)));
                            }
                        }
                        return None;
                    }
                }
            }
        },
    );

    let flat = futures_util::stream::StreamExt::flat_map(event_stream, |events| {
        futures_util::stream::iter(events)
    });

    Box::pin(flat) as EventStream
}
```

By moving the parse function into the unfold state tuple, we avoid lifetime issues. This works because `F: Send + 'static`.

**Step 2: Write tests for LineBuffer**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_buffer_single_line() {
        let mut buf = LineBuffer::new();
        buf.push_bytes(b"hello world\n");
        assert_eq!(buf.next_line(), Some("hello world".into()));
        assert_eq!(buf.next_line(), None);
    }

    #[test]
    fn line_buffer_multiple_lines() {
        let mut buf = LineBuffer::new();
        buf.push_bytes(b"line1\nline2\nline3\n");
        assert_eq!(buf.next_line(), Some("line1".into()));
        assert_eq!(buf.next_line(), Some("line2".into()));
        assert_eq!(buf.next_line(), Some("line3".into()));
        assert_eq!(buf.next_line(), None);
    }

    #[test]
    fn line_buffer_partial_then_complete() {
        let mut buf = LineBuffer::new();
        buf.push_bytes(b"hel");
        assert_eq!(buf.next_line(), None);
        buf.push_bytes(b"lo\n");
        assert_eq!(buf.next_line(), Some("hello".into()));
    }

    #[test]
    fn line_buffer_drain_remaining() {
        let mut buf = LineBuffer::new();
        buf.push_bytes(b"no newline");
        assert_eq!(buf.next_line(), None);
        assert_eq!(buf.drain(), Some("no newline".into()));
    }

    #[test]
    fn line_buffer_drain_empty() {
        let mut buf = LineBuffer::new();
        assert_eq!(buf.drain(), None);
    }

    #[test]
    fn line_buffer_drain_whitespace_only() {
        let mut buf = LineBuffer::new();
        buf.push_bytes(b"   \n");
        // The newline produces an empty-ish line
        assert_eq!(buf.next_line(), Some("   ".into()));
        // After consuming, drain should return None (empty buffer)
        assert_eq!(buf.drain(), None);
    }
}
```

**Step 3: Register module in lib.rs**

Add `pub mod sse;` to `crates/ucode-providers/src/lib.rs`.

**Step 4: Refactor OpenAI to use `stream_lines`**

In `openai.rs`, replace the manual unfold + flat_map in `stream_chat` with:

```rust
use crate::sse::stream_lines;

// In stream_chat, replace everything after `let byte_stream = resp.bytes_stream();`:
Ok(stream_lines(byte_stream, ToolCallAccumulator::default(), parse_sse_line))
```

Make `ToolCallAccumulator` and `parse_sse_line` public (they already are).

**Step 5: Refactor Anthropic to use `stream_lines`**

Same pattern in `anthropic.rs`:

```rust
use crate::sse::stream_lines;

// Replace the unfold + flat_map:
Ok(stream_lines(byte_stream, AnthropicToolAccumulator::default(), parse_anthropic_sse_line))
```

**Step 6: Verify**

```bash
cargo build -p ucode-providers && cargo test -p ucode-providers && cargo clippy -p ucode-providers
```

All existing tests must still pass.

**Step 7: Commit**

```
refactor(providers): extract SSE line-buffer and stream_lines helper

Eliminates ~80 lines of duplicated byte-stream-to-event unfold logic
across OpenAI and Anthropic adapters. New providers (Ollama native,
Gemini) will also use stream_lines.
```

---

## Task 3: Refactor OpenAI to OpenAiCompatProvider

**Files:**
- Modify: `crates/ucode-providers/src/openai.rs`
- Modify: `crates/ucode-providers/src/lib.rs`
- Modify: `crates/ucode-providers/src/ollama.rs` (update import — Ollama still uses OpenAI SSE parser until Task 5)

**Step 1: Rename and add config-based constructor**

Rename `OpenaiProvider` to `OpenAiCompatProvider`. Add fields for provider name and custom headers. Make `api_key` optional.

```rust
use std::collections::HashMap;
use crate::config::ProviderConfig;

/// OpenAI-compatible chat provider.
///
/// Works with OpenAI, Groq, Together, Fireworks, DeepSeek, Mistral,
/// OpenRouter, vLLM, LiteLLM, Azure OpenAI, and any endpoint that
/// implements the `/v1/chat/completions` streaming SSE protocol.
pub struct OpenAiCompatProvider {
    client: reqwest::Client,
    /// Provider instance name (from TOML key, e.g., "groq", "openai").
    provider_name: String,
    api_key: Option<String>,
    base_url: String,
    /// Extra headers sent with every request.
    headers: HashMap<String, String>,
}

impl OpenAiCompatProvider {
    /// Create from a provider config and resolved API key.
    pub fn from_config(name: &str, config: &ProviderConfig, api_key: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            provider_name: name.to_owned(),
            api_key,
            base_url: config.base_url().to_owned(),
            headers: config.headers.clone(),
        }
    }

    /// Create with just an API key (backward compat, defaults to OpenAI endpoint).
    pub fn new(api_key: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            provider_name: "openai".into(),
            api_key: Some(api_key),
            base_url: "https://api.openai.com/v1".into(),
            headers: HashMap::new(),
        }
    }

    /// Override the base URL.
    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url;
        self
    }
}
```

**Step 2: Update Provider impl**

- `name()` returns `&self.provider_name`
- `stream_chat` uses `self.provider_name` in error messages
- `stream_chat` adds custom headers from `self.headers`
- `stream_chat` only adds `Authorization: Bearer` if `api_key` is `Some`
- Use `stream_lines` from sse.rs

```rust
impl Provider for OpenAiCompatProvider {
    fn name(&self) -> &str {
        &self.provider_name
    }

    // capabilities() unchanged

    fn stream_chat(&self, req: ChatRequest) -> ProviderFuture<Result<EventStream, CoreError>> {
        let client = self.client.clone();
        let api_key = self.api_key.clone();
        let url = format!("{}/chat/completions", self.base_url);
        let provider_name = self.provider_name.clone();
        let custom_headers = self.headers.clone();

        let body = OpenAiRequest { /* same as before */ };

        Box::pin(async move {
            let mut request = client
                .post(&url)
                .header("Content-Type", "application/json");

            if let Some(ref key) = api_key {
                request = request.header("Authorization", format!("Bearer {key}"));
            }

            for (k, v) in &custom_headers {
                request = request.header(k.as_str(), v.as_str());
            }

            let resp = request
                .json(&body)
                .send()
                .await
                .map_err(|e| CoreError::Provider {
                    provider: provider_name.clone(),
                    message: format!("HTTP request failed: {e}"),
                })?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body_text = resp.text().await.unwrap_or_default();
                if status.as_u16() == 401 || status.as_u16() == 403 {
                    return Err(CoreError::Auth {
                        provider: provider_name,
                        auth_kind: ucode_core::AuthErrorKind::Invalid,
                    });
                }
                return Err(CoreError::Provider {
                    provider: provider_name,
                    message: format!("HTTP {status}: {body_text}"),
                });
            }

            Ok(crate::sse::stream_lines(
                resp.bytes_stream(),
                ToolCallAccumulator::default(),
                parse_sse_line,
            ))
        })
    }
}
```

**Step 3: Update lib.rs exports**

```rust
pub use openai::{OpenAiCompatProvider, parse_sse_line};
// Keep old name as alias for backward compat:
pub type OpenaiProvider = OpenAiCompatProvider;
```

**Step 4: Update ollama.rs import**

Change `use crate::openai::{ToolCallAccumulator, parse_sse_line};` — `ToolCallAccumulator` needs to remain public in openai.rs.

**Step 5: Update existing tests**

Update any test that references `OpenaiProvider` to use `OpenAiCompatProvider`. Add tests for:

```rust
#[test]
fn from_config_uses_provider_name() {
    let config = ProviderConfig {
        adapter: AdapterKind::Openai,
        base_url: Some("https://api.groq.com/openai/v1".into()),
        api_key_env: None,
        headers: HashMap::new(),
    };
    let provider = OpenAiCompatProvider::from_config("groq", &config, Some("key".into()));
    assert_eq!(provider.name(), "groq");
    assert_eq!(provider.base_url, "https://api.groq.com/openai/v1");
}

#[test]
fn from_config_with_custom_headers() {
    let mut headers = HashMap::new();
    headers.insert("api-version".into(), "2024-10-21".into());
    let config = ProviderConfig {
        adapter: AdapterKind::Openai,
        base_url: Some("https://azure.example.com".into()),
        api_key_env: None,
        headers,
    };
    let provider = OpenAiCompatProvider::from_config("azure", &config, Some("key".into()));
    assert_eq!(provider.headers.get("api-version").unwrap(), "2024-10-21");
}

#[test]
fn new_defaults_to_openai() {
    let provider = OpenAiCompatProvider::new("test-key".into());
    assert_eq!(provider.name(), "openai");
    assert_eq!(provider.base_url, "https://api.openai.com/v1");
    assert_eq!(provider.api_key, Some("test-key".into()));
}

#[test]
fn no_api_key_allowed() {
    let config = ProviderConfig {
        adapter: AdapterKind::Openai,
        base_url: None,
        api_key_env: None,
        headers: HashMap::new(),
    };
    let provider = OpenAiCompatProvider::from_config("local-vllm", &config, None);
    assert!(provider.api_key.is_none());
}
```

**Step 6: Verify**

```bash
cargo build && cargo test && cargo clippy
```

All workspace tests must pass (including any crate that imports `OpenaiProvider`).

**Step 7: Commit**

```
feat(providers): rename OpenaiProvider to OpenAiCompatProvider with config support

Adds from_config() constructor, configurable provider name, optional
API key, and custom headers. Works with OpenAI, Groq, Azure, vLLM,
and any OpenAI-compatible endpoint. Old name kept as type alias.
```

---

## Task 4: Refactor Anthropic to AnthropicCompatProvider

**Files:**
- Modify: `crates/ucode-providers/src/anthropic.rs`
- Modify: `crates/ucode-providers/src/lib.rs`

**Step 1: Rename and add config-based constructor**

Same pattern as Task 3. Rename `AnthropicProvider` to `AnthropicCompatProvider`.

```rust
use std::collections::HashMap;
use crate::config::ProviderConfig;

pub struct AnthropicCompatProvider {
    client: reqwest::Client,
    provider_name: String,
    api_key: Option<String>,
    base_url: String,
    headers: HashMap<String, String>,
}

impl AnthropicCompatProvider {
    pub fn from_config(name: &str, config: &ProviderConfig, api_key: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            provider_name: name.to_owned(),
            api_key,
            base_url: config.base_url().to_owned(),
            headers: config.headers.clone(),
        }
    }

    pub fn new(api_key: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            provider_name: "anthropic".into(),
            api_key: Some(api_key),
            base_url: "https://api.anthropic.com/v1".into(),
            headers: HashMap::new(),
        }
    }

    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url;
        self
    }
}
```

**Step 2: Update Provider impl**

- `name()` returns `&self.provider_name`
- `stream_chat` uses `self.provider_name` in errors
- Anthropic-specific headers (`x-api-key`, `anthropic-version`) always sent
- Custom headers from config added on top
- Use `stream_lines` from sse.rs
- `x-api-key` header only sent if `api_key` is `Some`

**Step 3: Update lib.rs exports**

```rust
pub use anthropic::{AnthropicCompatProvider, parse_anthropic_sse_line};
pub type AnthropicProvider = AnthropicCompatProvider;
```

**Step 4: Add tests**

```rust
#[test]
fn from_config_uses_provider_name() {
    let config = ProviderConfig {
        adapter: AdapterKind::Anthropic,
        base_url: None,
        api_key_env: None,
        headers: HashMap::new(),
    };
    let provider = AnthropicCompatProvider::from_config("my-anthropic", &config, Some("key".into()));
    assert_eq!(provider.name(), "my-anthropic");
    assert_eq!(provider.base_url, "https://api.anthropic.com/v1");
}

#[test]
fn from_config_with_custom_base_url() {
    let config = ProviderConfig {
        adapter: AdapterKind::Anthropic,
        base_url: Some("https://proxy.example.com/anthropic".into()),
        api_key_env: None,
        headers: HashMap::new(),
    };
    let provider = AnthropicCompatProvider::from_config("proxy", &config, Some("key".into()));
    assert_eq!(provider.base_url, "https://proxy.example.com/anthropic");
}
```

**Step 5: Verify**

```bash
cargo build && cargo test && cargo clippy
```

**Step 6: Commit**

```
feat(providers): rename AnthropicProvider to AnthropicCompatProvider with config support

Adds from_config() constructor, configurable provider name, optional
API key, and custom headers. Old name kept as type alias.
```

---

## Task 5: Rewrite OllamaProvider to native /api/chat

**Files:**
- Modify: `crates/ucode-providers/src/ollama.rs`

This is the biggest change. The current Ollama provider uses the OpenAI-compatible `/v1/chat/completions` endpoint. We rewrite it to use Ollama's native `/api/chat` endpoint which uses NDJSON streaming (not SSE) and supports thinking, stats, and richer sampling.

**Step 1: Define Ollama native request types**

```rust
use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use crate::config::ProviderConfig;
use crate::provider::{Capabilities, ChatRequest, Provider, ProviderFuture, ToolDef};
use ucode_core::{CoreError, Event, EventStream, ToolCall as CoreToolCall};

#[derive(Serialize)]
struct OllamaNativeRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<OllamaOptions>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<OllamaTool>,
    /// Enable thinking/reasoning mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    think: Option<bool>,
}

#[derive(Serialize)]
struct OllamaMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    num_ctx: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_k: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    min_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<i64>,
}

#[derive(Serialize)]
struct OllamaTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: OllamaFunction,
}

#[derive(Serialize)]
struct OllamaFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}
```

**Step 2: Define Ollama native response types**

Ollama native streaming returns NDJSON — each line is a complete JSON object:

```rust
/// A single NDJSON line from Ollama's `/api/chat` streaming response.
#[derive(Debug, Deserialize)]
struct OllamaChatResponse {
    #[serde(default)]
    message: Option<OllamaResponseMessage>,
    #[serde(default)]
    done: bool,
    #[serde(default)]
    done_reason: Option<String>,
    // Performance stats (only present in final response where done=true)
    #[serde(default)]
    total_duration: Option<u64>,
    #[serde(default)]
    load_duration: Option<u64>,
    #[serde(default)]
    prompt_eval_count: Option<u64>,
    #[serde(default)]
    eval_count: Option<u64>,
    #[serde(default)]
    eval_duration: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct OllamaResponseMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    thinking: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OllamaResponseToolCall>>,
}

#[derive(Debug, Deserialize)]
struct OllamaResponseToolCall {
    function: OllamaResponseFunction,
}

#[derive(Debug, Deserialize)]
struct OllamaResponseFunction {
    name: String,
    arguments: serde_json::Value,
}
```

**Step 3: Write NDJSON line parser**

```rust
/// Parse a single NDJSON line from Ollama's native `/api/chat` response.
pub fn parse_ollama_line(line: &str, _acc: &mut ()) -> Vec<Event> {
    let line = line.trim();
    if line.is_empty() {
        return vec![];
    }

    let resp: OllamaChatResponse = match serde_json::from_str(line) {
        Ok(r) => r,
        Err(_) => return vec![],
    };

    let mut events = Vec::new();

    if let Some(ref msg) = resp.message {
        // Thinking content (emitted as Event::Thinking if we add that, or skip for now)
        // For now, thinking content is not mapped to an Event variant.

        if let Some(ref content) = msg.content {
            if !content.is_empty() {
                events.push(Event::Token(content.clone()));
            }
        }

        if let Some(ref tool_calls) = msg.tool_calls {
            for (i, tc) in tool_calls.iter().enumerate() {
                events.push(Event::ToolCall(CoreToolCall::new(
                    format!("ollama_tc_{i}"),
                    tc.function.name.clone(),
                    tc.function.arguments.clone(),
                )));
            }
        }
    }

    if resp.done {
        events.push(Event::Done);
    }

    events
}
```

**Step 4: Rewrite OllamaProvider**

```rust
pub struct OllamaProvider {
    client: reqwest::Client,
    provider_name: String,
    base_url: String,
    headers: HashMap<String, String>,
}

impl OllamaProvider {
    pub fn from_config(name: &str, config: &ProviderConfig, _api_key: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            provider_name: name.to_owned(),
            base_url: config.base_url().to_owned(),
            headers: config.headers.clone(),
        }
    }

    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            provider_name: "ollama".into(),
            base_url: "http://localhost:11434".into(),
            headers: HashMap::new(),
        }
    }

    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url;
        self
    }
}

impl Default for OllamaProvider {
    fn default() -> Self {
        Self::new()
    }
}
```

**Step 5: Implement Provider trait with native /api/chat**

```rust
impl Provider for OllamaProvider {
    fn name(&self) -> &str {
        &self.provider_name
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            tool_calls: true,
            json_mode: false,
            max_context: 128_000,
            max_output: 4_096,
            streaming: true,
            token_counting: false,
        }
    }

    fn stream_chat(&self, req: ChatRequest) -> ProviderFuture<Result<EventStream, CoreError>> {
        let client = self.client.clone();
        let url = format!("{}/api/chat", self.base_url);
        let provider_name = self.provider_name.clone();
        let custom_headers = self.headers.clone();

        let body = OllamaNativeRequest {
            model: req.model,
            messages: to_ollama_messages(&req.messages),
            stream: true,
            options: if req.temperature.is_some() {
                Some(OllamaOptions {
                    temperature: req.temperature,
                    num_ctx: None,
                    top_k: None,
                    top_p: None,
                    min_p: None,
                    seed: None,
                })
            } else {
                None
            },
            tools: to_ollama_tools(&req.tools),
            think: None,
        };

        Box::pin(async move {
            let mut request = client
                .post(&url)
                .header("Content-Type", "application/json");

            for (k, v) in &custom_headers {
                request = request.header(k.as_str(), v.as_str());
            }

            let resp = request
                .json(&body)
                .send()
                .await
                .map_err(|e| CoreError::Provider {
                    provider: provider_name.clone(),
                    message: format!("HTTP request failed: {e}"),
                })?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body_text = resp.text().await.unwrap_or_default();
                if status.as_u16() == 401 || status.as_u16() == 403 {
                    return Err(CoreError::Auth {
                        provider: provider_name,
                        auth_kind: ucode_core::AuthErrorKind::Invalid,
                    });
                }
                return Err(CoreError::Provider {
                    provider: provider_name,
                    message: format!("HTTP {status}: {body_text}"),
                });
            }

            Ok(crate::sse::stream_lines(
                resp.bytes_stream(),
                (),
                parse_ollama_line,
            ))
        })
    }
}
```

**Step 6: Update tests**

Remove old tests that tested OpenAI-compat request serialization. Add new tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ucode_core::{Event, Message};

    // ── provider metadata ─────────────────────────────────────────────────

    #[test]
    fn provider_name_default() {
        assert_eq!(OllamaProvider::new().name(), "ollama");
    }

    #[test]
    fn provider_name_from_config() {
        let config = ProviderConfig {
            adapter: crate::config::AdapterKind::Ollama,
            base_url: None,
            api_key_env: None,
            headers: HashMap::new(),
        };
        let p = OllamaProvider::from_config("my-ollama", &config, None);
        assert_eq!(p.name(), "my-ollama");
    }

    #[test]
    fn default_base_url() {
        assert_eq!(OllamaProvider::new().base_url, "http://localhost:11434");
    }

    #[test]
    fn custom_base_url() {
        let p = OllamaProvider::new().with_base_url("http://192.168.1.10:11434".into());
        assert_eq!(p.base_url, "http://192.168.1.10:11434");
    }

    // ── NDJSON line parser ────────────────────────────────────────────────

    #[test]
    fn parse_text_token() {
        let line = r#"{"message":{"role":"assistant","content":"Hello"},"done":false}"#;
        let events = parse_ollama_line(line, &mut ());
        assert_eq!(events, vec![Event::Token("Hello".into())]);
    }

    #[test]
    fn parse_empty_content_skipped() {
        let line = r#"{"message":{"role":"assistant","content":""},"done":false}"#;
        let events = parse_ollama_line(line, &mut ());
        assert!(events.is_empty());
    }

    #[test]
    fn parse_done_emits_done() {
        let line = r#"{"message":{"role":"assistant","content":""},"done":true,"done_reason":"stop","total_duration":1234567,"eval_count":42}"#;
        let events = parse_ollama_line(line, &mut ());
        assert_eq!(events, vec![Event::Done]);
    }

    #[test]
    fn parse_tool_call() {
        let line = r#"{"message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":"get_weather","arguments":{"city":"London"}}}]},"done":false}"#;
        let events = parse_ollama_line(line, &mut ());
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::ToolCall(tc) => {
                assert_eq!(tc.name, "get_weather");
                assert_eq!(tc.args, serde_json::json!({"city": "London"}));
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    #[test]
    fn parse_empty_line_ignored() {
        assert!(parse_ollama_line("", &mut ()).is_empty());
        assert!(parse_ollama_line("   ", &mut ()).is_empty());
    }

    #[test]
    fn parse_invalid_json_ignored() {
        assert!(parse_ollama_line("not json", &mut ()).is_empty());
    }

    // ── message conversion ────────────────────────────────────────────────

    #[test]
    fn message_conversion() {
        let messages = vec![
            Message::system("Be helpful."),
            Message::user("Hello"),
            Message::assistant("Hi"),
        ];
        let converted = to_ollama_messages(&messages);
        assert_eq!(converted.len(), 3);
        assert_eq!(converted[0].role, "system");
        assert_eq!(converted[1].role, "user");
        assert_eq!(converted[2].role, "assistant");
    }

    // ── request serialization ─────────────────────────────────────────────

    #[test]
    fn native_request_serialization() {
        let body = OllamaNativeRequest {
            model: "llama3.2".into(),
            messages: vec![OllamaMessage {
                role: "user".into(),
                content: "Hello".into(),
            }],
            stream: true,
            options: None,
            tools: vec![],
            think: None,
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["model"], "llama3.2");
        assert_eq!(json["stream"], true);
        assert!(json.get("options").is_none());
        assert!(json.get("tools").is_none());
        assert!(json.get("think").is_none());
    }

    #[test]
    fn native_request_with_options() {
        let body = OllamaNativeRequest {
            model: "llama3.2".into(),
            messages: vec![],
            stream: true,
            options: Some(OllamaOptions {
                temperature: Some(0.7),
                num_ctx: Some(4096),
                top_k: None,
                top_p: None,
                min_p: None,
                seed: None,
            }),
            tools: vec![],
            think: Some(true),
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["options"]["temperature"], 0.7);
        assert_eq!(json["options"]["num_ctx"], 4096);
        assert_eq!(json["think"], true);
    }
}
```

**Step 7: Verify**

```bash
cargo build && cargo test && cargo clippy
```

**Step 8: Commit**

```
feat(providers): rewrite OllamaProvider to native /api/chat with NDJSON streaming

Replaces OpenAI-compat /v1/chat/completions with Ollama's native
/api/chat endpoint. Supports thinking mode, performance stats,
num_ctx, and richer sampling params. Uses NDJSON streaming.
```

---

## Task 6: New GeminiProvider

**Files:**
- Create: `crates/ucode-providers/src/gemini.rs`
- Modify: `crates/ucode-providers/src/lib.rs`

**Step 1: Define Gemini request types**

```rust
use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use crate::config::ProviderConfig;
use crate::provider::{Capabilities, ChatRequest, Provider, ProviderFuture, ToolDef};
use ucode_core::{CoreError, Event, EventStream, ToolCall as CoreToolCall};

// ── Request types ─────────────────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiRequest {
    contents: Vec<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<GenerationConfig>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<GeminiToolConfig>,
}

#[derive(Serialize, Deserialize, Debug)]
struct GeminiContent {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    parts: Vec<GeminiPart>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
enum GeminiPart {
    Text(String),
    #[serde(rename_all = "camelCase")]
    FunctionCall {
        name: String,
        args: serde_json::Value,
    },
    #[serde(rename_all = "camelCase")]
    FunctionResponse {
        name: String,
        response: serde_json::Value,
    },
}

// Custom serialization needed because Gemini uses `{"text": "..."}` not `{"Text": "..."}`
// Actually, let's use untagged or manual serde. Simpler: use a struct with optional fields.

// Revised approach — use a flat struct with optional fields:
#[derive(Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
struct GeminiPart {
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    function_call: Option<GeminiFunctionCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    function_response: Option<GeminiFunctionResponse>,
    /// Thinking/reasoning content from the model.
    #[serde(skip_serializing_if = "Option::is_none")]
    thought: Option<bool>,
}

#[derive(Serialize, Deserialize, Debug)]
struct GeminiFunctionCall {
    name: String,
    args: serde_json::Value,
}

#[derive(Serialize, Deserialize, Debug)]
struct GeminiFunctionResponse {
    name: String,
    response: serde_json::Value,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<usize>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiToolConfig {
    function_declarations: Vec<GeminiFunctionDecl>,
}

#[derive(Serialize)]
struct GeminiFunctionDecl {
    name: String,
    description: String,
    parameters: serde_json::Value,
}
```

**Step 2: Define Gemini SSE response types**

Gemini streaming uses SSE with `data:` lines containing JSON:

```rust
// ── Response types ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiStreamResponse {
    #[serde(default)]
    candidates: Vec<GeminiCandidate>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiCandidate {
    #[serde(default)]
    content: Option<GeminiContent>,
    #[serde(default)]
    finish_reason: Option<String>,
}
```

**Step 3: Write SSE line parser**

```rust
/// Parse a Gemini SSE `data:` line into events.
pub fn parse_gemini_sse_line(line: &str, _acc: &mut ()) -> Vec<Event> {
    let line = line.trim();

    let data = if let Some(d) = line.strip_prefix("data: ") {
        d.trim()
    } else if let Some(d) = line.strip_prefix("data:") {
        d.trim()
    } else {
        return vec![];
    };

    if data == "[DONE]" {
        return vec![Event::Done];
    }

    let resp: GeminiStreamResponse = match serde_json::from_str(data) {
        Ok(r) => r,
        Err(_) => return vec![],
    };

    let mut events = Vec::new();

    for candidate in &resp.candidates {
        if let Some(ref content) = candidate.content {
            for part in &content.parts {
                if let Some(ref text) = part.text {
                    if !text.is_empty() && part.thought != Some(true) {
                        events.push(Event::Token(text.clone()));
                    }
                }
                if let Some(ref fc) = part.function_call {
                    events.push(Event::ToolCall(CoreToolCall::new(
                        format!("gemini_fc_{}", fc.name),
                        fc.name.clone(),
                        fc.args.clone(),
                    )));
                }
            }
        }

        if candidate.finish_reason.as_deref() == Some("STOP")
            || candidate.finish_reason.as_deref() == Some("MAX_TOKENS")
        {
            events.push(Event::Done);
        }
    }

    events
}
```

**Step 4: Write message conversion**

```rust
fn to_gemini_contents(messages: &[ucode_core::Message]) -> (Option<GeminiContent>, Vec<GeminiContent>) {
    let mut system: Option<GeminiContent> = None;
    let mut contents = Vec::new();

    for m in messages {
        let text: String = m
            .parts
            .iter()
            .filter_map(|p| match p {
                ucode_core::Part::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");

        match m.role {
            ucode_core::Role::System => {
                system = Some(GeminiContent {
                    role: None,
                    parts: vec![GeminiPart {
                        text: Some(text),
                        ..Default::default()
                    }],
                });
            }
            ucode_core::Role::User => {
                contents.push(GeminiContent {
                    role: Some("user".into()),
                    parts: vec![GeminiPart {
                        text: Some(text),
                        ..Default::default()
                    }],
                });
            }
            ucode_core::Role::Assistant => {
                contents.push(GeminiContent {
                    role: Some("model".into()),
                    parts: vec![GeminiPart {
                        text: Some(text),
                        ..Default::default()
                    }],
                });
            }
            ucode_core::Role::Tool => {
                // Tool results not yet mapped
            }
        }
    }

    (system, contents)
}

fn to_gemini_tools(tools: &[ToolDef]) -> Vec<GeminiToolConfig> {
    if tools.is_empty() {
        return vec![];
    }
    vec![GeminiToolConfig {
        function_declarations: tools
            .iter()
            .map(|t| GeminiFunctionDecl {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: t.parameters.clone(),
            })
            .collect(),
    }]
}
```

**Step 5: Implement GeminiProvider**

```rust
pub struct GeminiProvider {
    client: reqwest::Client,
    provider_name: String,
    api_key: Option<String>,
    base_url: String,
    headers: HashMap<String, String>,
}

impl GeminiProvider {
    pub fn from_config(name: &str, config: &ProviderConfig, api_key: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            provider_name: name.to_owned(),
            api_key,
            base_url: config.base_url().to_owned(),
            headers: config.headers.clone(),
        }
    }

    pub fn new(api_key: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            provider_name: "gemini".into(),
            api_key: Some(api_key),
            base_url: "https://generativelanguage.googleapis.com".into(),
            headers: HashMap::new(),
        }
    }
}

impl Provider for GeminiProvider {
    fn name(&self) -> &str {
        &self.provider_name
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            tool_calls: true,
            json_mode: true,
            max_context: 1_000_000,
            max_output: 8_192,
            streaming: true,
            token_counting: false,
        }
    }

    fn stream_chat(&self, req: ChatRequest) -> ProviderFuture<Result<EventStream, CoreError>> {
        let client = self.client.clone();
        let api_key = self.api_key.clone();
        let provider_name = self.provider_name.clone();
        let custom_headers = self.headers.clone();

        // Gemini URL: /v1beta/models/{model}:streamGenerateContent?alt=sse
        let mut url = format!(
            "{}/v1beta/models/{}:streamGenerateContent?alt=sse",
            self.base_url, req.model
        );

        // API key can go as query param
        if let Some(ref key) = api_key {
            url.push_str(&format!("&key={key}"));
        }

        let (system_instruction, contents) = to_gemini_contents(&req.messages);
        let body = GeminiRequest {
            contents,
            system_instruction,
            generation_config: if req.temperature.is_some() || req.max_tokens.is_some() {
                Some(GenerationConfig {
                    temperature: req.temperature,
                    max_output_tokens: req.max_tokens,
                })
            } else {
                None
            },
            tools: to_gemini_tools(&req.tools),
        };

        Box::pin(async move {
            let mut request = client
                .post(&url)
                .header("Content-Type", "application/json");

            // Also send API key as header (some deployments prefer this)
            if let Some(ref key) = api_key {
                request = request.header("x-goog-api-key", key);
            }

            for (k, v) in &custom_headers {
                request = request.header(k.as_str(), v.as_str());
            }

            let resp = request
                .json(&body)
                .send()
                .await
                .map_err(|e| CoreError::Provider {
                    provider: provider_name.clone(),
                    message: format!("HTTP request failed: {e}"),
                })?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body_text = resp.text().await.unwrap_or_default();
                if status.as_u16() == 401 || status.as_u16() == 403 {
                    return Err(CoreError::Auth {
                        provider: provider_name,
                        auth_kind: ucode_core::AuthErrorKind::Invalid,
                    });
                }
                return Err(CoreError::Provider {
                    provider: provider_name,
                    message: format!("HTTP {status}: {body_text}"),
                });
            }

            Ok(crate::sse::stream_lines(
                resp.bytes_stream(),
                (),
                parse_gemini_sse_line,
            ))
        })
    }
}
```

**Step 6: Write tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ucode_core::{Event, Message};

    // ── provider metadata ─────────────────────────────────────────────────

    #[test]
    fn provider_name_default() {
        let p = GeminiProvider::new("key".into());
        assert_eq!(p.name(), "gemini");
    }

    #[test]
    fn provider_capabilities() {
        let caps = GeminiProvider::new("key".into()).capabilities();
        assert!(caps.tool_calls);
        assert!(caps.json_mode);
        assert!(caps.streaming);
        assert_eq!(caps.max_context, 1_000_000);
    }

    // ── SSE line parser ───────────────────────────────────────────────────

    #[test]
    fn parse_text_token() {
        let line = r#"data: {"candidates":[{"content":{"parts":[{"text":"Hello"}],"role":"model"}}]}"#;
        let events = parse_gemini_sse_line(line, &mut ());
        assert_eq!(events, vec![Event::Token("Hello".into())]);
    }

    #[test]
    fn parse_empty_text_skipped() {
        let line = r#"data: {"candidates":[{"content":{"parts":[{"text":""}],"role":"model"}}]}"#;
        let events = parse_gemini_sse_line(line, &mut ());
        assert!(events.is_empty());
    }

    #[test]
    fn parse_done_on_stop() {
        let line = r#"data: {"candidates":[{"content":{"parts":[{"text":""}],"role":"model"},"finishReason":"STOP"}]}"#;
        let events = parse_gemini_sse_line(line, &mut ());
        assert_eq!(events, vec![Event::Done]);
    }

    #[test]
    fn parse_function_call() {
        let line = r#"data: {"candidates":[{"content":{"parts":[{"functionCall":{"name":"get_weather","args":{"city":"London"}}}],"role":"model"}}]}"#;
        let events = parse_gemini_sse_line(line, &mut ());
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::ToolCall(tc) => {
                assert_eq!(tc.name, "get_weather");
                assert_eq!(tc.args, serde_json::json!({"city": "London"}));
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    #[test]
    fn parse_non_data_line_ignored() {
        assert!(parse_gemini_sse_line("", &mut ()).is_empty());
        assert!(parse_gemini_sse_line(": keep-alive", &mut ()).is_empty());
        assert!(parse_gemini_sse_line("event: message", &mut ()).is_empty());
    }

    #[test]
    fn parse_done_marker() {
        let events = parse_gemini_sse_line("data: [DONE]", &mut ());
        assert_eq!(events, vec![Event::Done]);
    }

    // ── message conversion ────────────────────────────────────────────────

    #[test]
    fn system_message_extracted() {
        let messages = vec![Message::system("Be helpful."), Message::user("Hello")];
        let (system, contents) = to_gemini_contents(&messages);
        assert!(system.is_some());
        assert_eq!(system.unwrap().parts[0].text.as_deref(), Some("Be helpful."));
        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0].role.as_deref(), Some("user"));
    }

    #[test]
    fn assistant_mapped_to_model_role() {
        let messages = vec![Message::assistant("Hi there")];
        let (_, contents) = to_gemini_contents(&messages);
        assert_eq!(contents[0].role.as_deref(), Some("model"));
    }

    #[test]
    fn no_system_gives_none() {
        let messages = vec![Message::user("Hello")];
        let (system, contents) = to_gemini_contents(&messages);
        assert!(system.is_none());
        assert_eq!(contents.len(), 1);
    }

    // ── tool config ───────────────────────────────────────────────────────

    #[test]
    fn empty_tools_gives_empty_vec() {
        assert!(to_gemini_tools(&[]).is_empty());
    }

    #[test]
    fn tools_wrapped_in_function_declarations() {
        let tools = vec![ToolDef {
            name: "calc".into(),
            description: "Calculator".into(),
            parameters: serde_json::json!({"type": "object"}),
        }];
        let gemini_tools = to_gemini_tools(&tools);
        assert_eq!(gemini_tools.len(), 1);
        assert_eq!(gemini_tools[0].function_declarations.len(), 1);
        assert_eq!(gemini_tools[0].function_declarations[0].name, "calc");
    }

    // ── request serialization ─────────────────────────────────────────────

    #[test]
    fn request_serialization() {
        let body = GeminiRequest {
            contents: vec![GeminiContent {
                role: Some("user".into()),
                parts: vec![GeminiPart {
                    text: Some("Hello".into()),
                    ..Default::default()
                }],
            }],
            system_instruction: None,
            generation_config: Some(GenerationConfig {
                temperature: Some(0.7),
                max_output_tokens: Some(1024),
            }),
            tools: vec![],
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["contents"][0]["role"], "user");
        assert_eq!(json["contents"][0]["parts"][0]["text"], "Hello");
        assert_eq!(json["generationConfig"]["temperature"], 0.7);
        assert_eq!(json["generationConfig"]["maxOutputTokens"], 1024);
        assert!(json.get("systemInstruction").is_none());
        assert!(json.get("tools").is_none());
    }
}
```

**Step 7: Register module in lib.rs**

Add `pub mod gemini;` and `pub use gemini::{GeminiProvider, parse_gemini_sse_line};`

**Step 8: Verify**

```bash
cargo build && cargo test && cargo clippy
```

**Step 9: Commit**

```
feat(providers): add GeminiProvider with SSE streaming and tool calling

Implements Google's generateContent/streamGenerateContent API with
SSE streaming, function declarations for tool calling, system
instruction support, and thinking content filtering.
```

---

## Task 7: Provider factory and lib.rs cleanup

**Files:**
- Create: `crates/ucode-providers/src/factory.rs`
- Modify: `crates/ucode-providers/src/lib.rs`

**Step 1: Write factory function**

```rust
use crate::config::{AdapterKind, ProviderConfig};
use crate::provider::Provider;
use ucode_core::CoreError;

/// Create a provider instance from config.
///
/// Resolves the API key from the environment variable named by `api_key_env`.
/// Returns an error if the adapter requires an API key but none is available.
pub fn create_provider(
    name: &str,
    config: &ProviderConfig,
) -> Result<Box<dyn Provider>, CoreError> {
    let api_key = config.resolve_api_key();

    match config.adapter {
        AdapterKind::Openai => {
            Ok(Box::new(crate::openai::OpenAiCompatProvider::from_config(
                name, config, api_key,
            )))
        }
        AdapterKind::Anthropic => {
            // Anthropic requires an API key
            if api_key.is_none() && config.api_key_env.is_some() {
                return Err(CoreError::Auth {
                    provider: name.to_owned(),
                    auth_kind: ucode_core::AuthErrorKind::Missing,
                });
            }
            Ok(Box::new(
                crate::anthropic::AnthropicCompatProvider::from_config(name, config, api_key),
            ))
        }
        AdapterKind::Ollama => {
            Ok(Box::new(crate::ollama::OllamaProvider::from_config(
                name, config, api_key,
            )))
        }
        AdapterKind::Gemini => {
            // Gemini requires an API key
            if api_key.is_none() && config.api_key_env.is_some() {
                return Err(CoreError::Auth {
                    provider: name.to_owned(),
                    auth_kind: ucode_core::AuthErrorKind::Missing,
                });
            }
            Ok(Box::new(crate::gemini::GeminiProvider::from_config(
                name, config, api_key,
            )))
        }
    }
}

/// Create all providers from a providers table.
pub fn create_all_providers(
    configs: &std::collections::HashMap<String, ProviderConfig>,
) -> Vec<(String, Result<Box<dyn Provider>, CoreError>)> {
    configs
        .iter()
        .map(|(name, config)| (name.clone(), create_provider(name, config)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn create_ollama_no_api_key() {
        let config = ProviderConfig {
            adapter: AdapterKind::Ollama,
            base_url: None,
            api_key_env: None,
            headers: HashMap::new(),
        };
        let provider = create_provider("ollama", &config).unwrap();
        assert_eq!(provider.name(), "ollama");
    }

    #[test]
    fn create_openai_compat() {
        let config = ProviderConfig {
            adapter: AdapterKind::Openai,
            base_url: Some("https://api.groq.com/openai/v1".into()),
            api_key_env: None,
            headers: HashMap::new(),
        };
        let provider = create_provider("groq", &config).unwrap();
        assert_eq!(provider.name(), "groq");
    }

    #[test]
    fn create_anthropic_missing_key_with_env_configured() {
        let config = ProviderConfig {
            adapter: AdapterKind::Anthropic,
            base_url: None,
            api_key_env: Some("UCODE_TEST_NONEXISTENT_ANTHROPIC_KEY".into()),
            headers: HashMap::new(),
        };
        let result = create_provider("anthropic", &config);
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::Auth { provider, auth_kind } => {
                assert_eq!(provider, "anthropic");
                assert_eq!(auth_kind, ucode_core::AuthErrorKind::Missing);
            }
            other => panic!("expected Auth error, got {other:?}"),
        }
    }

    #[test]
    fn create_anthropic_no_env_configured_ok() {
        // No api_key_env set at all — provider created without key
        let config = ProviderConfig {
            adapter: AdapterKind::Anthropic,
            base_url: None,
            api_key_env: None,
            headers: HashMap::new(),
        };
        let provider = create_provider("anthropic-proxy", &config).unwrap();
        assert_eq!(provider.name(), "anthropic-proxy");
    }

    #[test]
    fn create_gemini_missing_key_with_env_configured() {
        let config = ProviderConfig {
            adapter: AdapterKind::Gemini,
            base_url: None,
            api_key_env: Some("UCODE_TEST_NONEXISTENT_GEMINI_KEY".into()),
            headers: HashMap::new(),
        };
        let result = create_provider("gemini", &config);
        assert!(result.is_err());
    }

    #[test]
    fn create_gemini_no_env_configured_ok() {
        let config = ProviderConfig {
            adapter: AdapterKind::Gemini,
            base_url: None,
            api_key_env: None,
            headers: HashMap::new(),
        };
        let provider = create_provider("gemini-proxy", &config).unwrap();
        assert_eq!(provider.name(), "gemini-proxy");
    }

    #[test]
    fn create_all_providers_mixed() {
        let mut configs = HashMap::new();
        configs.insert(
            "ollama".into(),
            ProviderConfig {
                adapter: AdapterKind::Ollama,
                base_url: None,
                api_key_env: None,
                headers: HashMap::new(),
            },
        );
        configs.insert(
            "openai".into(),
            ProviderConfig {
                adapter: AdapterKind::Openai,
                base_url: None,
                api_key_env: None,
                headers: HashMap::new(),
            },
        );
        let results = create_all_providers(&configs);
        assert_eq!(results.len(), 2);
        // Both should succeed (no api_key_env configured)
        for (_, result) in &results {
            assert!(result.is_ok());
        }
    }
}
```

**Step 2: Update lib.rs**

Final `lib.rs`:

```rust
//! ucode-providers: Provider trait, capability model, and adapters
//! (OpenAI-compat, Anthropic-compat, Ollama native, Gemini).

pub mod anthropic;
pub mod config;
pub mod factory;
pub mod gemini;
pub mod mock;
pub mod ollama;
pub mod openai;
pub mod provider;
pub mod sse;

pub use anthropic::{AnthropicCompatProvider, parse_anthropic_sse_line};
pub use config::{AdapterKind, ProviderConfig, ProvidersTable};
pub use factory::{create_all_providers, create_provider};
pub use gemini::{GeminiProvider, parse_gemini_sse_line};
pub use mock::MockProvider;
pub use ollama::{OllamaProvider, parse_ollama_line};
pub use openai::{OpenAiCompatProvider, parse_sse_line};
pub use provider::{Capabilities, ChatRequest, Provider, ProviderFuture, ToolDef};

// Backward-compat type aliases
pub type OpenaiProvider = OpenAiCompatProvider;
pub type AnthropicProvider = AnthropicCompatProvider;
```

**Step 3: Verify full workspace**

```bash
cargo build && cargo test && cargo clippy
```

Fix any compilation errors in other crates that import old names (the type aliases should handle this).

**Step 4: Commit**

```
feat(providers): add provider factory with config-driven instantiation

create_provider() maps AdapterKind to the correct adapter, resolves
API keys from env vars, and returns clear errors for missing keys.
Updated lib.rs with all new exports and backward-compat aliases.
```

---

## Verification Checklist

After all tasks are complete:

1. `cargo build` — zero errors
2. `cargo test` — all tests pass (target: ~50+ new tests in ucode-providers)
3. `cargo clippy` — zero warnings
4. Verify backward compat: `OpenaiProvider` and `AnthropicProvider` type aliases work
5. Verify config parsing: all TOML examples from PLANS.md parse correctly
6. Verify factory: each adapter kind creates the correct provider type
7. Verify acceptance criteria from PLANS.md lines 668-674:
   - OpenAI-compat works with configurable `base_url` ✓ (Task 3)
   - Anthropic-compat works with configurable `base_url` ✓ (Task 4)
   - Ollama native uses `/api/chat` ✓ (Task 5)
   - Gemini streams via `streamGenerateContent?alt=sse` ✓ (Task 6)
   - Custom headers sent correctly ✓ (Tasks 3, 4, 5, 6)
   - Missing `api_key_env` produces clear error ✓ (Task 7)
