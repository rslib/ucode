# Agent Orchestration Implementation Plan (ISSUE 0901 + 0902)

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Create the `ucode-agent` crate that connects user messages to LLM providers, executes tool calls, persists sessions, and sends events back to the TUI/CLI -- making the app actually work end-to-end.

**Architecture:** A new `ucode-agent` crate exposes an `AgentLoop` that receives user messages via an `mpsc` channel, builds `ChatRequest`s using the session transcript + tool definitions, calls `provider.stream_chat()`, processes the resulting `EventStream` (forwarding tokens to TUI, executing tool calls via `ToolRegistry`, appending results to the transcript), saves the session, and loops. Config is loaded from `${UCODE_HOME}/ucode.toml` with env-var auto-discovery fallback.

**Tech Stack:** Rust 2024 edition, tokio async, toml crate for config parsing, existing workspace crates (ucode-core, ucode-providers, ucode-tools, ucode-auth, ucode-tui).

---

### Task 1: Create ucode-agent crate skeleton

**Files:**
- Create: `crates/ucode-agent/Cargo.toml`
- Create: `crates/ucode-agent/src/lib.rs`
- Modify: `Cargo.toml` (workspace root, lines 2-15 members list, lines 18-30 default-members)

**Step 1: Create the crate directory**

```bash
mkdir -p crates/ucode-agent/src
```

**Step 2: Write Cargo.toml**

Create `crates/ucode-agent/Cargo.toml`:

```toml
[package]
name = "ucode-agent"
version.workspace = true
edition.workspace = true
authors.workspace = true
license.workspace = true

[dependencies]
tokio = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
toml = "0.8"
thiserror = { workspace = true }
anyhow = { workspace = true }
tracing = { workspace = true }
futures-util = { workspace = true }

ucode-core = { workspace = true }
ucode-providers = { workspace = true }
ucode-tools = { workspace = true }
ucode-auth = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
tokio = { workspace = true, features = ["test-util"] }
```

**Step 3: Write initial lib.rs**

Create `crates/ucode-agent/src/lib.rs`:

```rust
//! ucode-agent: orchestration loop connecting user messages to LLM providers.

pub mod config;

pub use config::AppConfig;
```

**Step 4: Create empty config module**

Create `crates/ucode-agent/src/config.rs`:

```rust
//! Application configuration: TOML loading + env-var auto-discovery.
```

**Step 5: Add to workspace**

In root `Cargo.toml`, add `"crates/ucode-agent"` to both `members` and `default-members` arrays. Also add `ucode-agent = { path = "crates/ucode-agent" }` to `[workspace.dependencies]`.

**Step 6: Verify it compiles**

```bash
cargo check -p ucode-agent
```

Expected: success, no errors.

**Step 7: Commit**

```bash
git add crates/ucode-agent/ Cargo.toml
git commit -m "feat: create ucode-agent crate skeleton"
```

---

### Task 2: Implement AppConfig with TOML loading and env-var fallback

**Files:**
- Create: `crates/ucode-agent/src/config.rs` (replace empty file)
- Modify: `crates/ucode-agent/src/lib.rs`

**Step 1: Write tests for config loading**

Add to `crates/ucode-agent/src/config.rs`:

```rust
//! Application configuration: TOML loading + env-var auto-discovery.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use ucode_providers::config::{AdapterKind, ProviderConfig, ProvidersTable};

/// Top-level application configuration.
#[derive(Debug, Clone)]
pub struct AppConfig {
    /// Provider configurations keyed by name.
    pub providers: HashMap<String, ProviderConfig>,
    /// Path the config was loaded from, if any.
    pub config_path: Option<PathBuf>,
}

/// Well-known env vars that map to provider configs.
const ENV_PROVIDER_MAP: &[(&str, &str, AdapterKind)] = &[
    ("ANTHROPIC_API_KEY", "anthropic", AdapterKind::Anthropic),
    ("OPENAI_API_KEY", "openai", AdapterKind::Openai),
    ("GEMINI_API_KEY", "gemini", AdapterKind::Gemini),
    ("GOOGLE_API_KEY", "gemini", AdapterKind::Gemini),
];

impl AppConfig {
    /// Load config from the given TOML file path.
    /// Returns an error only if the file exists but is malformed.
    pub fn from_file(path: &Path) -> Result<Self, ConfigError> {
        if !path.exists() {
            return Ok(Self {
                providers: HashMap::new(),
                config_path: None,
            });
        }
        let content = std::fs::read_to_string(path)
            .map_err(|e| ConfigError::Io(path.to_owned(), e))?;
        let table: ProvidersTable = toml::from_str(&content)
            .map_err(|e| ConfigError::Parse(path.to_owned(), e))?;
        Ok(Self {
            providers: table.providers,
            config_path: Some(path.to_owned()),
        })
    }

    /// Discover providers from well-known environment variables.
    /// Only adds providers whose env var is actually set.
    pub fn discover_from_env(&mut self) {
        for &(env_var, name, ref adapter) in ENV_PROVIDER_MAP {
            if self.providers.contains_key(name) {
                continue; // TOML config takes precedence
            }
            if std::env::var(env_var).ok().filter(|v| !v.is_empty()).is_some() {
                self.providers.insert(
                    name.to_owned(),
                    ProviderConfig {
                        adapter: adapter.clone(),
                        base_url: None,
                        api_key_env: Some(env_var.to_owned()),
                        headers: HashMap::new(),
                    },
                );
            }
        }
    }

    /// Load config from the default path, then discover env vars.
    /// This is the standard entry point.
    pub fn load_default() -> Result<Self, ConfigError> {
        let config_home = ucode_core::logging::default_config_home();
        let config_path = config_home.join("ucode.toml");
        let mut config = Self::from_file(&config_path)?;
        config.discover_from_env();
        Ok(config)
    }

    /// Returns true if at least one provider is configured.
    pub fn has_providers(&self) -> bool {
        !self.providers.is_empty()
    }

    /// Get the first available provider name, preferring anthropic > openai > others.
    pub fn default_provider(&self) -> Option<&str> {
        for preferred in &["anthropic", "openai", "gemini"] {
            if self.providers.contains_key(*preferred) {
                return Some(preferred);
            }
        }
        self.providers.keys().next().map(|s| s.as_str())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read config file {0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("failed to parse config file {0}: {1}")]
    Parse(PathBuf, toml::de::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_file_missing_returns_empty() {
        let cfg = AppConfig::from_file(Path::new("/nonexistent/ucode.toml")).unwrap();
        assert!(cfg.providers.is_empty());
        assert!(cfg.config_path.is_none());
    }

    #[test]
    fn from_file_valid_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ucode.toml");
        std::fs::write(
            &path,
            r#"
[providers.openai]
type = "openai"
api_key_env = "OPENAI_API_KEY"

[providers.anthropic]
type = "anthropic"
api_key_env = "ANTHROPIC_API_KEY"
"#,
        )
        .unwrap();
        let cfg = AppConfig::from_file(&path).unwrap();
        assert_eq!(cfg.providers.len(), 2);
        assert!(cfg.providers.contains_key("openai"));
        assert!(cfg.providers.contains_key("anthropic"));
        assert_eq!(cfg.config_path, Some(path));
    }

    #[test]
    fn from_file_malformed_toml_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ucode.toml");
        std::fs::write(&path, "this is not valid toml [[[").unwrap();
        assert!(AppConfig::from_file(&path).is_err());
    }

    #[test]
    fn discover_env_adds_missing_providers() {
        let mut cfg = AppConfig {
            providers: HashMap::new(),
            config_path: None,
        };
        // Simulate ANTHROPIC_API_KEY being set
        unsafe { std::env::set_var("ANTHROPIC_API_KEY", "sk-test-123") };
        cfg.discover_from_env();
        unsafe { std::env::remove_var("ANTHROPIC_API_KEY") };

        assert!(cfg.providers.contains_key("anthropic"));
        let p = &cfg.providers["anthropic"];
        assert_eq!(p.api_key_env.as_deref(), Some("ANTHROPIC_API_KEY"));
    }

    #[test]
    fn discover_env_does_not_override_toml() {
        let mut cfg = AppConfig {
            providers: HashMap::new(),
            config_path: None,
        };
        // Pre-populate from "TOML"
        cfg.providers.insert(
            "openai".to_owned(),
            ProviderConfig {
                adapter: AdapterKind::Openai,
                base_url: Some("https://custom.example.com".to_owned()),
                api_key_env: Some("OPENAI_API_KEY".to_owned()),
                headers: HashMap::new(),
            },
        );
        unsafe { std::env::set_var("OPENAI_API_KEY", "sk-test") };
        cfg.discover_from_env();
        unsafe { std::env::remove_var("OPENAI_API_KEY") };

        // Should keep the custom base_url from TOML, not overwrite
        assert_eq!(
            cfg.providers["openai"].base_url.as_deref(),
            Some("https://custom.example.com")
        );
    }

    #[test]
    fn default_provider_prefers_anthropic() {
        let mut cfg = AppConfig {
            providers: HashMap::new(),
            config_path: None,
        };
        cfg.providers.insert(
            "openai".to_owned(),
            ProviderConfig {
                adapter: AdapterKind::Openai,
                base_url: None,
                api_key_env: None,
                headers: HashMap::new(),
            },
        );
        cfg.providers.insert(
            "anthropic".to_owned(),
            ProviderConfig {
                adapter: AdapterKind::Anthropic,
                base_url: None,
                api_key_env: None,
                headers: HashMap::new(),
            },
        );
        assert_eq!(cfg.default_provider(), Some("anthropic"));
    }

    #[test]
    fn has_providers_empty() {
        let cfg = AppConfig {
            providers: HashMap::new(),
            config_path: None,
        };
        assert!(!cfg.has_providers());
    }
}
```

**Step 2: Update lib.rs exports**

```rust
//! ucode-agent: orchestration loop connecting user messages to LLM providers.

pub mod config;

pub use config::{AppConfig, ConfigError};
```

**Step 3: Run tests**

```bash
cargo test -p ucode-agent
```

Expected: all 6 tests pass.

**Step 4: Commit**

```bash
git add crates/ucode-agent/
git commit -m "feat(agent): add AppConfig with TOML loading and env-var discovery"
```

---

### Task 3: Implement the agent loop core

**Files:**
- Create: `crates/ucode-agent/src/loop.rs`
- Modify: `crates/ucode-agent/src/lib.rs`

**Step 1: Write the agent loop module**

Create `crates/ucode-agent/src/loop.rs`:

```rust
//! Core agent loop: receives user messages, calls provider, executes tools, sends events.

use std::sync::Arc;
use std::time::Instant;

use futures_util::StreamExt;
use tokio::sync::mpsc;

use ucode_auth::CredentialStore;
use ucode_core::message::{Message, Part, Role, ToolCall, ToolResult};
use ucode_core::{Event, Session, SessionStore};
use ucode_providers::config::ProviderConfig;
use ucode_providers::factory::create_provider;
use ucode_providers::provider::{ChatRequest, Provider};
use ucode_tools::registry::ToolRegistry;

/// Events sent from the agent loop back to the UI.
/// These map 1:1 to TuiEvent variants but are UI-agnostic.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// A text token from the model.
    Token(String),
    /// The model finished generating.
    StreamDone,
    /// A tool call is starting.
    ToolCallStarted { name: String },
    /// A tool call completed.
    ToolCallCompleted {
        name: String,
        success: bool,
        duration_ms: u64,
        output: Option<String>,
    },
    /// A system-level message (e.g., "Using provider: anthropic").
    SystemMessage(String),
    /// An error occurred.
    Error(String),
}

/// Configuration for the agent loop.
pub struct AgentLoopConfig {
    /// Provider name (key in AppConfig.providers).
    pub provider_name: String,
    /// Provider configuration.
    pub provider_config: ProviderConfig,
    /// Model to use (e.g., "claude-sonnet-4-20250514").
    pub model: String,
    /// Credential store for auth.
    pub credential_store: Option<Arc<dyn CredentialStore>>,
}

/// Run the agent loop.
///
/// - `message_rx`: receives user messages (strings) from the TUI/CLI.
/// - `event_tx`: sends `AgentEvent`s back to the TUI/CLI.
/// - `config`: provider + model configuration.
/// - `session_store`: for persisting sessions.
/// - `session`: the active session (mutated in place).
/// - `tool_registry`: available tools.
///
/// The loop runs until `message_rx` is closed (sender dropped) or an
/// unrecoverable error occurs.
pub async fn run_agent_loop(
    mut message_rx: mpsc::UnboundedReceiver<String>,
    event_tx: mpsc::UnboundedSender<AgentEvent>,
    config: AgentLoopConfig,
    session_store: Arc<SessionStore>,
    mut session: Session,
    tool_registry: Arc<ToolRegistry>,
) {
    // Create the provider
    let provider = match create_provider(
        &config.provider_name,
        &config.provider_config,
        config.credential_store,
    ) {
        Ok(p) => p,
        Err(e) => {
            let _ = event_tx.send(AgentEvent::Error(format!(
                "Failed to create provider '{}': {e}",
                config.provider_name
            )));
            return;
        }
    };

    let _ = event_tx.send(AgentEvent::SystemMessage(format!(
        "Using provider: {} (model: {})",
        config.provider_name, config.model
    )));

    session.set_active_model(Some(config.model.clone()));

    // Main loop: wait for user messages
    while let Some(user_msg) = message_rx.recv().await {
        // Add user message to transcript
        session.push_message(Message {
            role: Role::User,
            parts: vec![Part::Text(user_msg)],
        });

        // Build chat request
        let tool_defs = tool_registry.tool_defs();
        let req = ChatRequest {
            model: config.model.clone(),
            messages: session.transcript.clone(),
            temperature: None,
            max_tokens: None,
            tools: tool_defs,
            json_mode: false,
        };

        // Call provider
        let stream = match provider.stream_chat(req).await {
            Ok(s) => s,
            Err(e) => {
                let _ = event_tx.send(AgentEvent::Error(format!("Provider error: {e}")));
                continue;
            }
        };

        // Process the event stream
        let mut assistant_text = String::new();
        let mut pending_tool_calls: Vec<ToolCall> = Vec::new();

        tokio::pin!(stream);
        while let Some(event) = stream.next().await {
            match event {
                Event::Token(tok) => {
                    assistant_text.push_str(&tok);
                    let _ = event_tx.send(AgentEvent::Token(tok));
                }
                Event::ToolCall(tc) => {
                    pending_tool_calls.push(tc);
                }
                Event::Done => {
                    let _ = event_tx.send(AgentEvent::StreamDone);
                }
                Event::Error(e) => {
                    let _ = event_tx.send(AgentEvent::Error(format!("Stream error: {e}")));
                }
                Event::Log(msg) => {
                    tracing::info!("provider log: {msg}");
                }
                _ => {}
            }
        }

        // Build assistant message parts
        let mut parts = Vec::new();
        if !assistant_text.is_empty() {
            parts.push(Part::Text(assistant_text));
        }
        for tc in &pending_tool_calls {
            parts.push(Part::ToolCall(tc.clone()));
        }
        if !parts.is_empty() {
            session.push_message(Message {
                role: Role::Assistant,
                parts,
            });
        }

        // Execute tool calls
        for tc in &pending_tool_calls {
            let _ = event_tx.send(AgentEvent::ToolCallStarted {
                name: tc.name.clone(),
            });

            let start = Instant::now();
            let result = tool_registry.invoke(&tc.id, &tc.name, tc.args.clone()).await;
            let duration_ms = start.elapsed().as_millis() as u64;

            match result {
                Ok(tool_result) => {
                    let output_str = serde_json::to_string(&tool_result.result).ok();
                    session.record_tool_use(tc.name.clone(), true, duration_ms);
                    session.push_message(Message {
                        role: Role::User,
                        parts: vec![Part::ToolResult(tool_result)],
                    });
                    let _ = event_tx.send(AgentEvent::ToolCallCompleted {
                        name: tc.name.clone(),
                        success: true,
                        duration_ms,
                        output: output_str,
                    });
                }
                Err(e) => {
                    session.record_tool_use(tc.name.clone(), false, duration_ms);
                    let error_result = ToolResult {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                        result: serde_json::json!({ "error": e.to_string() }),
                        is_error: true,
                    };
                    session.push_message(Message {
                        role: Role::User,
                        parts: vec![Part::ToolResult(error_result)],
                    });
                    let _ = event_tx.send(AgentEvent::ToolCallCompleted {
                        name: tc.name.clone(),
                        success: false,
                        duration_ms,
                        output: Some(e.to_string()),
                    });
                }
            }
        }

        // If there were tool calls, send the tool results back to the model
        // for a follow-up response (tool-use loop).
        if !pending_tool_calls.is_empty() {
            let tool_defs = tool_registry.tool_defs();
            let followup_req = ChatRequest {
                model: config.model.clone(),
                messages: session.transcript.clone(),
                temperature: None,
                max_tokens: None,
                tools: tool_defs,
                json_mode: false,
            };

            match provider.stream_chat(followup_req).await {
                Ok(followup_stream) => {
                    let mut followup_text = String::new();
                    tokio::pin!(followup_stream);
                    while let Some(event) = followup_stream.next().await {
                        match event {
                            Event::Token(tok) => {
                                followup_text.push_str(&tok);
                                let _ = event_tx.send(AgentEvent::Token(tok));
                            }
                            Event::Done => {
                                let _ = event_tx.send(AgentEvent::StreamDone);
                            }
                            Event::Error(e) => {
                                let _ =
                                    event_tx.send(AgentEvent::Error(format!("Followup error: {e}")));
                            }
                            _ => {}
                        }
                    }
                    if !followup_text.is_empty() {
                        session.push_message(Message {
                            role: Role::Assistant,
                            parts: vec![Part::Text(followup_text)],
                        });
                    }
                }
                Err(e) => {
                    let _ = event_tx.send(AgentEvent::Error(format!("Followup error: {e}")));
                }
            }
        }

        // Save session after each turn
        if let Err(e) = session_store.save(&session) {
            tracing::error!("failed to save session: {e}");
        }
    }

    tracing::info!("agent loop exiting: message channel closed");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_event_debug() {
        // Smoke test that AgentEvent variants are constructible
        let ev = AgentEvent::Token("hello".into());
        assert!(format!("{ev:?}").contains("Token"));

        let ev = AgentEvent::ToolCallStarted {
            name: "read_file".into(),
        };
        assert!(format!("{ev:?}").contains("read_file"));
    }
}
```

**Step 2: Update lib.rs**

```rust
//! ucode-agent: orchestration loop connecting user messages to LLM providers.

pub mod config;
// `loop` is a keyword, so we use `agent_loop` as the module name.
#[path = "loop.rs"]
pub mod agent_loop;

pub use agent_loop::{AgentEvent, AgentLoopConfig, run_agent_loop};
pub use config::{AppConfig, ConfigError};
```

**Step 3: Verify it compiles**

```bash
cargo check -p ucode-agent
```

Expected: success.

**Step 4: Run tests**

```bash
cargo test -p ucode-agent
```

Expected: all tests pass (config tests + agent_event_debug).

**Step 5: Commit**

```bash
git add crates/ucode-agent/
git commit -m "feat(agent): implement core agent loop with tool execution"
```

---

### Task 4: Wire agent loop into TUI

**Files:**
- Modify: `crates/ucode-tui/Cargo.toml` (add ucode-agent dependency)
- Modify: `crates/ucode-tui/src/lib.rs` (spawn agent task)
- Modify: `crates/ucode-tui/src/event_loop.rs` (handle AgentEvent -> TuiEvent mapping)

**Step 1: Add ucode-agent dependency to ucode-tui**

In `crates/ucode-tui/Cargo.toml`, add under `[dependencies]`:

```toml
ucode-agent = { workspace = true }
```

Also add to workspace root `Cargo.toml` `[workspace.dependencies]` if not already there (done in Task 1).

**Step 2: Create agent-to-TUI event bridge in event_loop.rs**

Add a function that maps `AgentEvent` to `TuiEvent` and sends it. Add this near the top of `event_loop.rs`:

```rust
use ucode_agent::AgentEvent;

/// Convert an AgentEvent to a TuiEvent and send it.
pub fn bridge_agent_event(event: AgentEvent, tx: &crate::TuiEventSender) {
    let tui_event = match event {
        AgentEvent::Token(tok) => TuiEvent::StreamToken(tok),
        AgentEvent::StreamDone => TuiEvent::StreamDone,
        AgentEvent::ToolCallStarted { name } => TuiEvent::ToolCallStarted { name },
        AgentEvent::ToolCallCompleted {
            name,
            success,
            duration_ms,
            output,
        } => {
            // Find the tool call index from the transcript -- use 0 as fallback
            let status = if success {
                ToolCallStatus::Success
            } else {
                ToolCallStatus::Error
            };
            TuiEvent::ToolCallCompleted {
                index: 0,
                status,
                duration_ms: Some(duration_ms),
                summary: Some(name),
                thinking: None,
                output,
            }
        }
        AgentEvent::SystemMessage(msg) => TuiEvent::SystemMessage(msg),
        AgentEvent::Error(msg) => TuiEvent::Toast {
            level: ToastLevel::Error,
            title: "Agent Error".into(),
            body: Some(msg),
        },
    };
    let _ = tx.send(tui_event);
}
```

**Step 3: Update `run()` in lib.rs to spawn agent loop**

Modify `crates/ucode-tui/src/lib.rs`:

```rust
//! ucode-tui: ratatui-based fullscreen terminal UI

pub mod app;
pub mod clipboard;
pub mod command_registry;
pub mod components;
pub mod event_loop;
pub mod keybinds;
pub mod layout;
pub mod overlays;
pub mod terminal;
pub mod theme;

/// Channel sender for external systems to send events to the TUI.
pub type TuiEventSender = tokio::sync::mpsc::UnboundedSender<event_loop::TuiEvent>;

/// Create a TUI event channel pair.
pub fn create_event_channel() -> (
    TuiEventSender,
    tokio::sync::mpsc::UnboundedReceiver<event_loop::TuiEvent>,
) {
    tokio::sync::mpsc::unbounded_channel()
}

/// Configuration for spawning the agent loop alongside the TUI.
pub struct AgentConfig {
    pub loop_config: ucode_agent::AgentLoopConfig,
    pub session_store: std::sync::Arc<ucode_core::SessionStore>,
    pub session: ucode_core::Session,
    pub tool_registry: std::sync::Arc<ucode_tools::registry::ToolRegistry>,
}

/// Run the fullscreen TUI. This is the main entry point.
///
/// Takes both ends of the TUI event channel so that auth tasks spawned inside
/// the loop can send results back via the sender. Also accepts an optional
/// agent config to spawn the agent loop.
/// Blocks until the user exits or the sender is dropped.
pub async fn run(
    event_tx: TuiEventSender,
    event_rx: tokio::sync::mpsc::UnboundedReceiver<event_loop::TuiEvent>,
    agent_config: Option<AgentConfig>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut app = app::AppState::new();

    // If agent config is provided, set up the message channel and spawn the loop
    let _agent_handle = if let Some(ac) = agent_config {
        let (msg_tx, msg_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        app.message_tx = Some(msg_tx);

        let (agent_event_tx, mut agent_event_rx) =
            tokio::sync::mpsc::unbounded_channel::<ucode_agent::AgentEvent>();

        // Spawn bridge task: forward AgentEvents to TuiEvents
        let bridge_tx = event_tx.clone();
        let bridge_handle = tokio::spawn(async move {
            while let Some(ev) = agent_event_rx.recv().await {
                event_loop::bridge_agent_event(ev, &bridge_tx);
            }
        });

        // Spawn the agent loop
        let agent_handle = tokio::spawn(ucode_agent::run_agent_loop(
            msg_rx,
            agent_event_tx,
            ac.loop_config,
            ac.session_store,
            ac.session,
            ac.tool_registry,
        ));

        Some((agent_handle, bridge_handle))
    } else {
        app.message_tx = None;
        None
    };

    let mut input_box = components::input::InputBoxState::new();
    let mut sidebar_data = components::sidebar::SidebarData::new();
    event_loop::run_event_loop(
        &mut app,
        &mut input_box,
        &mut sidebar_data,
        event_tx,
        event_rx,
    )
    .await
}
```

**Step 4: Update demo.rs to match new `run()` signature**

The demo.rs file calls `run()` -- update it to pass `None` for agent_config instead of `None` for message_tx. Find the call site and change accordingly.

**Step 5: Verify it compiles**

```bash
cargo check -p ucode-tui
```

Expected: success.

**Step 6: Run tests**

```bash
cargo test -p ucode-tui
```

Expected: all existing TUI tests pass.

**Step 7: Commit**

```bash
git add crates/ucode-tui/ crates/ucode-agent/
git commit -m "feat(tui): wire agent loop into TUI with event bridge"
```

---

### Task 5: Wire agent loop into CLI (Run command + default TUI launch)

**Files:**
- Modify: `crates/ucode-cli/Cargo.toml` (add ucode-agent, ucode-tools deps)
- Modify: `crates/ucode-cli/src/main.rs` (wire Run command and default TUI launch)

**Step 1: Add dependencies**

In `crates/ucode-cli/Cargo.toml`, add:

```toml
ucode-agent = { workspace = true }
ucode-tools = { workspace = true }
ucode-tui = { workspace = true }
```

**Step 2: Wire the default (no subcommand) case to launch TUI with agent**

In `main.rs`, update the `None` arm of the command match to:

```rust
None => {
    // Load config
    let app_config = ucode_agent::AppConfig::load_default()
        .map_err(|e| anyhow::anyhow!("config error: {e}"))?;

    if !app_config.has_providers() {
        eprintln!(
            "No providers configured. Set ANTHROPIC_API_KEY, OPENAI_API_KEY, \
             or create ~/.config/ucode/ucode.toml"
        );
        eprintln!("Run `ucode auth status` to check credentials.");
        // Still launch TUI but without agent
        let (event_tx, event_rx) = ucode_tui::create_event_channel();
        ucode_tui::run(event_tx, event_rx, None).await?;
    } else {
        let provider_name = app_config
            .default_provider()
            .expect("has_providers was true")
            .to_owned();
        let provider_config = app_config.providers[&provider_name].clone();

        // Default model per adapter
        let model = default_model_for(&provider_config.adapter);

        let cred_store: std::sync::Arc<dyn ucode_auth::CredentialStore> =
            std::sync::Arc::new(store);

        let session = session_store.create(std::env::current_dir().unwrap_or_default())?;
        let session_store = std::sync::Arc::new(session_store);

        let tool_registry = std::sync::Arc::new(ucode_tools::registry::ToolRegistry::new());

        let agent_config = ucode_tui::AgentConfig {
            loop_config: ucode_agent::AgentLoopConfig {
                provider_name,
                provider_config,
                model,
                credential_store: Some(cred_store),
            },
            session_store,
            session,
            tool_registry,
        };

        let (event_tx, event_rx) = ucode_tui::create_event_channel();
        ucode_tui::run(event_tx, event_rx, Some(agent_config)).await?;
    }
}
```

Add a helper function:

```rust
fn default_model_for(adapter: &ucode_providers::config::AdapterKind) -> String {
    match adapter {
        ucode_providers::config::AdapterKind::Anthropic => "claude-sonnet-4-20250514".to_owned(),
        ucode_providers::config::AdapterKind::Openai => "gpt-4o".to_owned(),
        ucode_providers::config::AdapterKind::Gemini => "gemini-2.0-flash".to_owned(),
        ucode_providers::config::AdapterKind::Ollama => "llama3.2".to_owned(),
        ucode_providers::config::AdapterKind::Copilot => "gpt-4o".to_owned(),
    }
}
```

**Step 3: Wire the `Run` command for headless mode**

Update the `Some(Command::Run { .. })` arm to actually run the agent loop:

```rust
Some(Command::Run {
    prompt,
    resume_session,
    timeout,
}) => {
    let app_config = ucode_agent::AppConfig::load_default()
        .map_err(|e| anyhow::anyhow!("config error: {e}"))?;

    let provider_name = app_config
        .default_provider()
        .ok_or_else(|| anyhow::anyhow!("No providers configured"))?
        .to_owned();
    let provider_config = app_config.providers[&provider_name].clone();
    let model = default_model_for(&provider_config.adapter);

    let cred_store: std::sync::Arc<dyn ucode_auth::CredentialStore> =
        std::sync::Arc::new(store);

    let session = if let Some(id) = resume_session {
        session_store.load(&id)?
    } else {
        session_store.create(std::env::current_dir().unwrap_or_default())?
    };
    let session_store = std::sync::Arc::new(session_store);

    let tool_registry = std::sync::Arc::new(ucode_tools::registry::ToolRegistry::new());

    let loop_config = ucode_agent::AgentLoopConfig {
        provider_name,
        provider_config,
        model,
        credential_store: Some(cred_store),
    };

    // Create channels
    let (msg_tx, msg_rx) = tokio::sync::mpsc::unbounded_channel();
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();

    // Send the prompt
    let _ = msg_tx.send(prompt);
    drop(msg_tx); // Close channel so agent loop exits after processing

    // Spawn agent loop
    let agent_handle = tokio::spawn(ucode_agent::run_agent_loop(
        msg_rx,
        event_tx,
        loop_config,
        session_store,
        session,
        tool_registry,
    ));

    // Collect events
    let mut runner = headless::HeadlessRunner::new(cli.json_output);
    let mut events = Vec::new();

    let timeout_dur = std::time::Duration::from_secs(timeout);
    let deadline = tokio::time::Instant::now() + timeout_dur;

    loop {
        tokio::select! {
            ev = event_rx.recv() => {
                match ev {
                    Some(agent_ev) => {
                        let he = runner.record_event(&agent_event_to_core_event(&agent_ev));
                        events.push(he);
                        if !cli.json_output {
                            print_agent_event(&agent_ev);
                        }
                    }
                    None => break, // channel closed
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                eprintln!("timeout after {timeout}s");
                break;
            }
        }
    }

    agent_handle.await.ok();

    if cli.json_output {
        let exit_code = headless::HeadlessRunner::determine_exit_code(&events);
        let out = runner.build_output(events, headless::HeadlessUsage::default(), exit_code);
        match runner.format_output(&out) {
            Ok(json) => println!("{json}"),
            Err(e) => tracing::error!("failed to serialize: {e}"),
        }
    }
}
```

Add helper functions for headless output:

```rust
fn agent_event_to_core_event(ev: &ucode_agent::AgentEvent) -> ucode_core::Event {
    match ev {
        ucode_agent::AgentEvent::Token(t) => ucode_core::Event::Token(t.clone()),
        ucode_agent::AgentEvent::StreamDone => ucode_core::Event::Done,
        ucode_agent::AgentEvent::Error(e) => {
            ucode_core::Event::Error(ucode_core::CoreError::Internal(e.clone()))
        }
        ucode_agent::AgentEvent::SystemMessage(m) => ucode_core::Event::Log(m.clone()),
        ucode_agent::AgentEvent::ToolCallStarted { name } => {
            ucode_core::Event::Log(format!("tool call: {name}"))
        }
        ucode_agent::AgentEvent::ToolCallCompleted {
            name,
            success,
            duration_ms,
            output,
        } => ucode_core::Event::Log(format!(
            "tool {name} {} in {duration_ms}ms",
            if *success { "succeeded" } else { "failed" }
        )),
    }
}

fn print_agent_event(ev: &ucode_agent::AgentEvent) {
    match ev {
        ucode_agent::AgentEvent::Token(t) => print!("{t}"),
        ucode_agent::AgentEvent::StreamDone => println!(),
        ucode_agent::AgentEvent::SystemMessage(m) => eprintln!("[system] {m}"),
        ucode_agent::AgentEvent::Error(e) => eprintln!("[error] {e}"),
        ucode_agent::AgentEvent::ToolCallStarted { name } => {
            eprintln!("[tool] starting: {name}")
        }
        ucode_agent::AgentEvent::ToolCallCompleted {
            name,
            success,
            duration_ms,
            ..
        } => {
            let status = if *success { "ok" } else { "failed" };
            eprintln!("[tool] {name}: {status} ({duration_ms}ms)");
        }
    }
}
```

**Step 4: Verify it compiles**

```bash
cargo check -p ucode-cli
```

Expected: success.

**Step 5: Run tests**

```bash
cargo test -p ucode-cli
```

Expected: all existing CLI tests pass.

**Step 6: Commit**

```bash
git add crates/ucode-cli/ crates/ucode-tui/
git commit -m "feat(cli): wire agent loop into Run command and default TUI launch"
```

---

### Task 6: Register built-in tools in the tool registry

**Files:**
- Modify: `crates/ucode-cli/src/main.rs` (register tools before passing registry)

**Step 1: Check what built-in tools exist**

Look at `crates/ucode-tools/src/` for existing tool implementations. Register them in the `ToolRegistry` before passing it to the agent loop.

The exact registration depends on what tools are implemented. At minimum, create the registry and register any available tools:

```rust
let mut tool_registry = ucode_tools::registry::ToolRegistry::new();
// Register built-in tools
ucode_tools::register_builtins(&mut tool_registry);
let tool_registry = std::sync::Arc::new(tool_registry);
```

If `register_builtins` doesn't exist, create it in `ucode-tools/src/lib.rs`:

```rust
/// Register all built-in tools with the given registry.
pub fn register_builtins(registry: &mut registry::ToolRegistry) {
    // Tools will be registered here as they are implemented
    let _ = registry; // suppress unused warning for now
}
```

**Step 2: Verify**

```bash
cargo check --workspace
```

**Step 3: Commit**

```bash
git add crates/ucode-tools/ crates/ucode-cli/
git commit -m "feat(tools): add register_builtins entry point for tool registration"
```

---

### Task 7: Write docs/config.md (ISSUE 0902 acceptance)

**Files:**
- Create: `docs/config.md`

**Step 1: Write the config documentation**

Create `docs/config.md` with:
- Config file location and `UCODE_HOME` override
- Full example `ucode.toml` with all providers
- Precedence rules (defaults < global config < env vars for discovery)
- Per-provider configuration options
- Environment variable auto-discovery table

**Step 2: Commit**

```bash
git add docs/config.md
git commit -m "docs: add config.md with TOML format and env-var discovery (ISSUE 0902)"
```

---

### Task 8: Write docs/e2e.md (ISSUE 0901 acceptance)

**Files:**
- Create: `docs/e2e.md`

**Step 1: Write the e2e scenario documentation**

Create `docs/e2e.md` with:
- Happy path scenario (connect, send prompt, receive response, tool call, save session)
- How to run the scenario manually
- Expected behavior at each step
- Error scenarios (no provider, auth expired, tool failure)

**Step 2: Commit**

```bash
git add docs/e2e.md
git commit -m "docs: add e2e.md with happy path scenario test (ISSUE 0901)"
```

---

### Task 9: Full workspace verification

**Step 1: Run all tests**

```bash
cargo test --workspace
```

Expected: all tests pass.

**Step 2: Run clippy**

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: no warnings.

**Step 3: Check formatting**

```bash
cargo fmt --all --check
```

Expected: no formatting issues.

**Step 4: Update EPIC.md**

Mark ISSUE 0901 and ISSUE 0902 as DONE.

**Step 5: Commit**

```bash
git add EPIC.md
git commit -m "docs: mark ISSUE 0901 and ISSUE 0902 as DONE in EPIC.md"
```
