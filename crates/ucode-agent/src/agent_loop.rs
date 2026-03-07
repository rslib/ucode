use std::sync::Arc;
use std::time::Instant;

use futures_util::StreamExt as _;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use ucode_auth::CredentialStore;
use ucode_core::event::Event;
use ucode_core::message::{Message, Part, Role, ToolCall, ToolResult};
use ucode_core::session::{Session, SessionStore};
use ucode_providers::config::ProviderConfig;
use ucode_providers::provider::ChatRequest;
use ucode_providers::{Provider, create_provider};
use ucode_tools::ToolRegistry;

// ---------------------------------------------------------------------------
// AgentEvent
// ---------------------------------------------------------------------------

/// UI-agnostic events emitted by the agent loop to the caller.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// A streaming text token from the model.
    Token(String),
    /// The model finished generating for this turn.
    StreamDone,
    /// A tool is about to be executed.
    ToolCallStarted { name: String },
    /// A tool finished executing.
    ToolCallCompleted {
        name: String,
        success: bool,
        duration_ms: u64,
        /// Truncated string representation of the result, if any.
        output: Option<String>,
    },
    /// Informational message from the agent infrastructure.
    SystemMessage(String),
    /// An error occurred (non-fatal; the loop continues).
    Error(String),
}

// ---------------------------------------------------------------------------
// AgentMessage
// ---------------------------------------------------------------------------

/// Messages sent from the TUI to the agent loop.
#[derive(Debug, Clone)]
pub enum AgentMessage {
    /// A user-typed message to send to the LLM.
    UserMessage(String),
    /// Switch the model used for subsequent turns.
    SetModel(String),
}

// ---------------------------------------------------------------------------
// AgentLoopConfig
// ---------------------------------------------------------------------------

/// Configuration for a single agent loop instance.
pub struct AgentLoopConfig {
    pub provider_name: String,
    pub provider_config: ProviderConfig,
    pub model: String,
    pub credential_store: Option<Arc<dyn CredentialStore>>,
}

// ---------------------------------------------------------------------------
// run_agent_loop
// ---------------------------------------------------------------------------

/// Drive the agent: receive user messages, call the provider, execute tools,
/// and emit `AgentEvent`s until `message_rx` is closed.
pub async fn run_agent_loop(
    mut message_rx: mpsc::UnboundedReceiver<AgentMessage>,
    event_tx: mpsc::UnboundedSender<AgentEvent>,
    config: AgentLoopConfig,
    session_store: Arc<SessionStore>,
    mut session: Session,
    tool_registry: Arc<ToolRegistry>,
) {
    let provider = match create_provider(
        &config.provider_name,
        &config.provider_config,
        config.credential_store,
    ) {
        Ok(p) => p,
        Err(e) => {
            let _ = event_tx.send(AgentEvent::Error(format!("provider init failed: {e}")));
            return;
        }
    };

    let _ = event_tx.send(AgentEvent::SystemMessage(format!(
        "provider={} model={}",
        config.provider_name, config.model
    )));

    session.set_active_model(Some(config.model.clone()));

    let mut model = config.model.clone();

    while let Some(agent_msg) = message_rx.recv().await {
        match agent_msg {
            AgentMessage::SetModel(new_model) => {
                model = new_model;
                session.set_active_model(Some(model.clone()));
                let _ = event_tx.send(AgentEvent::SystemMessage(format!(
                    "provider={} model={}",
                    config.provider_name, model
                )));
                continue;
            }
            AgentMessage::UserMessage(user_text) => {
                debug!(
                    "agent loop: received user message ({} chars)",
                    user_text.len()
                );

                session.push_message(Message::user(&user_text));

                let tool_results = process_turn(
                    provider.as_ref(),
                    &model,
                    &event_tx,
                    &mut session,
                    &tool_registry,
                )
                .await;

                // If there were tool calls, do a follow-up call with the updated transcript.
                if !tool_results.is_empty() {
                    followup_turn(provider.as_ref(), &model, &event_tx, &mut session).await;
                }

                session.auto_title_if_needed();

                if let Err(e) = session_store.save(&session) {
                    warn!("failed to save session: {e}");
                    let _ = event_tx.send(AgentEvent::Error(format!("session save failed: {e}")));
                }
            }
        }
    }

    info!("agent loop: message channel closed, exiting");
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Run one provider call, stream tokens, collect tool calls, execute them.
/// Returns the list of `ToolResult`s appended to the transcript (may be empty).
async fn process_turn(
    provider: &dyn Provider,
    model: &str,
    event_tx: &mpsc::UnboundedSender<AgentEvent>,
    session: &mut Session,
    tool_registry: &Arc<ToolRegistry>,
) -> Vec<ToolResult> {
    let mut req = ChatRequest::new(model, session.transcript.clone());
    req.tools = tool_registry.tool_defs();

    let stream = match provider.stream_chat(req).await {
        Ok(s) => s,
        Err(e) => {
            let _ = event_tx.send(AgentEvent::Error(format!("provider error: {e}")));
            return Vec::new();
        }
    };

    tokio::pin!(stream);

    let mut text_buf = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();

    while let Some(event) = stream.next().await {
        match event {
            Event::Token(tok) => {
                text_buf.push_str(&tok);
                let _ = event_tx.send(AgentEvent::Token(tok));
            }
            Event::ToolCall(tc) => {
                tool_calls.push(tc);
            }
            Event::Error(e) => {
                let _ = event_tx.send(AgentEvent::Error(format!("stream error: {e}")));
            }
            Event::Done => break,
            // Log, Patch, ToolResult, Compaction — not acted on here.
            _ => {}
        }
    }

    let _ = event_tx.send(AgentEvent::StreamDone);

    // Build the assistant message: text + any tool calls.
    let mut assistant_parts: Vec<Part> = Vec::new();
    if !text_buf.is_empty() {
        assistant_parts.push(Part::Text(text_buf));
    }
    for tc in &tool_calls {
        assistant_parts.push(Part::ToolCall(tc.clone()));
    }
    if !assistant_parts.is_empty() {
        session.push_message(Message::new(Role::Assistant, assistant_parts));
    }

    // Execute tool calls and collect results.
    let mut results: Vec<ToolResult> = Vec::new();
    for tc in &tool_calls {
        let _ = event_tx.send(AgentEvent::ToolCallStarted {
            name: tc.name.clone(),
        });

        let start = Instant::now();
        let result = tool_registry
            .invoke(&tc.id, &tc.name, tc.args.clone())
            .await;
        let duration_ms = start.elapsed().as_millis() as u64;

        let tool_result = match result {
            Ok(tr) => tr,
            Err(e) => ToolResult::new(
                &tc.id,
                &tc.name,
                serde_json::Value::String(e.to_string()),
                true,
            ),
        };

        let success = !tool_result.is_error;
        let output = result_preview(&tool_result.result);

        session.record_tool_use(tc.name.clone(), true, duration_ms);

        let _ = event_tx.send(AgentEvent::ToolCallCompleted {
            name: tc.name.clone(),
            success,
            duration_ms,
            output,
        });

        session.push_message(Message::tool_result(
            &tool_result.id,
            &tool_result.name,
            tool_result.result.clone(),
            tool_result.is_error,
        ));

        results.push(tool_result);
    }

    results
}

/// Follow-up provider call after tool execution. Collects tokens and sends
/// events but does not recurse into further tool calls.
async fn followup_turn(
    provider: &dyn Provider,
    model: &str,
    event_tx: &mpsc::UnboundedSender<AgentEvent>,
    session: &mut Session,
) {
    let req = ChatRequest::new(model, session.transcript.clone());

    let stream = match provider.stream_chat(req).await {
        Ok(s) => s,
        Err(e) => {
            let _ = event_tx.send(AgentEvent::Error(format!("follow-up provider error: {e}")));
            return;
        }
    };

    tokio::pin!(stream);

    let mut text_buf = String::new();

    while let Some(event) = stream.next().await {
        match event {
            Event::Token(tok) => {
                text_buf.push_str(&tok);
                let _ = event_tx.send(AgentEvent::Token(tok));
            }
            Event::Error(e) => {
                let _ = event_tx.send(AgentEvent::Error(format!("follow-up stream error: {e}")));
            }
            Event::Done => break,
            _ => {}
        }
    }

    let _ = event_tx.send(AgentEvent::StreamDone);

    if !text_buf.is_empty() {
        session.push_message(Message::assistant(text_buf));
    }
}

/// Return a short string preview of a JSON value for display, capped at 120 chars.
fn result_preview(value: &serde_json::Value) -> Option<String> {
    let s = match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => return None,
        other => other.to_string(),
    };
    if s.is_empty() {
        return None;
    }
    if s.len() > 120 {
        Some(format!("{}...", &s[..120]))
    } else {
        Some(s)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_event_debug() {
        let ev = AgentEvent::Token("hello".into());
        assert!(format!("{ev:?}").contains("Token"));
        let ev = AgentEvent::ToolCallStarted {
            name: "read_file".into(),
        };
        assert!(format!("{ev:?}").contains("read_file"));
    }

    #[test]
    fn agent_event_variants_constructible() {
        let _ = AgentEvent::StreamDone;
        let _ = AgentEvent::SystemMessage("info".into());
        let _ = AgentEvent::Error("oops".into());
        let _ = AgentEvent::ToolCallCompleted {
            name: "write_file".into(),
            success: true,
            duration_ms: 42,
            output: Some("ok".into()),
        };
    }

    #[test]
    fn result_preview_truncates() {
        let long = "x".repeat(200);
        let val = serde_json::Value::String(long);
        let preview = result_preview(&val).unwrap();
        assert_eq!(preview.len(), 123); // 120 + "..."
        assert!(preview.ends_with("..."));
    }

    #[test]
    fn result_preview_null_is_none() {
        assert!(result_preview(&serde_json::Value::Null).is_none());
    }

    #[test]
    fn result_preview_short_string() {
        let val = serde_json::Value::String("hello".into());
        assert_eq!(result_preview(&val).as_deref(), Some("hello"));
    }
}
