use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use futures_util::StreamExt as _;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
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
    /// A tool call requires user approval before execution.
    ApprovalRequired {
        tool_call_id: String,
        tool_name: String,
        command: String,
        cwd: String,
        sandbox_label: String,
    },
    /// Informational message from the agent infrastructure.
    SystemMessage(String),
    /// An error occurred (non-fatal; the loop continues).
    Error(String),
}

// ---------------------------------------------------------------------------
// AgentMessage
// ---------------------------------------------------------------------------

/// File content attached via @path/to/file reference.
#[derive(Debug, Clone)]
pub struct FileContext {
    pub path: String,
    pub content: String,
}

/// Messages sent from the TUI to the agent loop.
#[derive(Debug, Clone)]
pub enum AgentMessage {
    /// A user-typed message to send to the LLM.
    UserMessage {
        text: String,
        /// If set, route to this agent instead of the default.
        target_agent: Option<String>,
        /// File contents to prepend as context.
        file_context: Vec<FileContext>,
    },
    /// Switch the model used for subsequent turns.
    SetModel(String),
    /// Cancel the current generation and clear pending approvals.
    Cancel,
    /// Respond to an approval request for a tool call.
    ApprovalDecision {
        tool_call_id: String,
        approved: bool,
    },
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

    // Pending approval oneshots: tool_call_id -> sender.
    let mut pending_approvals: HashMap<String, oneshot::Sender<bool>> = HashMap::new();

    // Cancellation token for the current turn. Replaced on each new turn.
    let mut cancel_token = CancellationToken::new();

    while let Some(agent_msg) = message_rx.recv().await {
        match agent_msg {
            AgentMessage::SetModel(new_model) => {
                model = new_model;
                session.set_active_model(Some(model.clone()));
                let _ = event_tx.send(AgentEvent::SystemMessage(format!(
                    "provider={} model={}",
                    config.provider_name, model
                )));
            }
            AgentMessage::Cancel => {
                cancel_token.cancel();
                // Deny all pending approvals.
                for (_id, sender) in pending_approvals.drain() {
                    let _ = sender.send(false);
                }
                // Create a fresh token for the next turn.
                cancel_token = CancellationToken::new();
                let _ = event_tx.send(AgentEvent::StreamDone);
            }
            AgentMessage::ApprovalDecision {
                tool_call_id,
                approved,
            } => {
                if let Some(sender) = pending_approvals.remove(&tool_call_id) {
                    let _ = sender.send(approved);
                } else {
                    warn!("approval decision for unknown tool_call_id: {tool_call_id}");
                }
            }
            AgentMessage::UserMessage {
                text: user_text,
                target_agent: _,
                file_context,
            } => {
                debug!(
                    "agent loop: received user message ({} chars)",
                    user_text.len()
                );

                // Fresh cancellation token for this turn.
                cancel_token = CancellationToken::new();

                // Prepend any attached file contents before the user text.
                let full_message = if file_context.is_empty() {
                    user_text
                } else {
                    let mut parts = String::new();
                    for fc in &file_context {
                        parts.push_str(&format!(
                            "<file path=\"{}\">\n{}\n</file>\n\n",
                            fc.path, fc.content
                        ));
                    }
                    parts.push_str(&user_text);
                    parts
                };

                session.push_message(Message::user(&full_message));

                let tool_results = process_turn(
                    provider.as_ref(),
                    &model,
                    &event_tx,
                    &mut session,
                    &tool_registry,
                    &cancel_token,
                    &mut message_rx,
                    &mut pending_approvals,
                )
                .await;

                // If cancelled, skip follow-up.
                if cancel_token.is_cancelled() {
                    continue;
                }

                // If there were tool calls, do a follow-up call with the updated transcript.
                if !tool_results.is_empty() {
                    followup_turn(
                        provider.as_ref(),
                        &model,
                        &event_tx,
                        &mut session,
                        &cancel_token,
                    )
                    .await;
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
///
/// Tool calls that require approval (currently: `run_cmd`) will send an
/// `ApprovalRequired` event and block until the TUI responds with an
/// `ApprovalDecision` message.
#[allow(clippy::too_many_arguments)]
async fn process_turn(
    provider: &dyn Provider,
    model: &str,
    event_tx: &mpsc::UnboundedSender<AgentEvent>,
    session: &mut Session,
    tool_registry: &Arc<ToolRegistry>,
    cancel_token: &CancellationToken,
    message_rx: &mut mpsc::UnboundedReceiver<AgentMessage>,
    pending_approvals: &mut HashMap<String, oneshot::Sender<bool>>,
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

    // Stream tokens with cancellation support.
    loop {
        tokio::select! {
            biased;

            _ = cancel_token.cancelled() => {
                debug!("agent loop: stream cancelled");
                break;
            }

            maybe_event = stream.next() => {
                match maybe_event {
                    Some(Event::Token(tok)) => {
                        text_buf.push_str(&tok);
                        let _ = event_tx.send(AgentEvent::Token(tok));
                    }
                    Some(Event::ToolCall(tc)) => {
                        tool_calls.push(tc);
                    }
                    Some(Event::Error(e)) => {
                        let _ = event_tx.send(AgentEvent::Error(format!("stream error: {e}")));
                    }
                    Some(Event::Done) | None => break,
                    // Log, Patch, ToolResult, Compaction -- not acted on here.
                    Some(_) => {}
                }
            }
        }
    }

    let _ = event_tx.send(AgentEvent::StreamDone);

    if cancel_token.is_cancelled() {
        // Still record whatever text we got before cancellation.
        if !text_buf.is_empty() {
            session.push_message(Message::assistant(text_buf));
        }
        return Vec::new();
    }

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

    // Execute tool calls with approval gate.
    let mut results: Vec<ToolResult> = Vec::new();
    for tc in &tool_calls {
        if cancel_token.is_cancelled() {
            break;
        }

        // Approval gate: tools that require confirmation block here.
        let needs_approval = requires_approval(&tc.name);
        if needs_approval {
            let (approval_tx, approval_rx) = oneshot::channel::<bool>();
            pending_approvals.insert(tc.id.clone(), approval_tx);

            let _ = event_tx.send(AgentEvent::ApprovalRequired {
                tool_call_id: tc.id.clone(),
                tool_name: tc.name.clone(),
                command: tool_call_command_preview(tc),
                cwd: std::env::current_dir()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
                sandbox_label: "workspace".into(),
            });

            // Wait for approval decision or cancellation.
            let approved = tokio::select! {
                biased;
                _ = cancel_token.cancelled() => {
                    pending_approvals.remove(&tc.id);
                    break;
                }
                // While waiting for approval, drain other messages (SetModel, Cancel, other ApprovalDecisions).
                result = wait_for_approval(approval_rx, message_rx, pending_approvals, cancel_token) => {
                    result
                }
            };

            if !approved {
                let tool_result = ToolResult::new(
                    &tc.id,
                    &tc.name,
                    serde_json::Value::String("Tool call denied by user".into()),
                    true,
                );
                let _ = event_tx.send(AgentEvent::ToolCallCompleted {
                    name: tc.name.clone(),
                    success: false,
                    duration_ms: 0,
                    output: Some("denied".into()),
                });
                session.push_message(Message::tool_result(
                    &tool_result.id,
                    &tool_result.name,
                    tool_result.result.clone(),
                    tool_result.is_error,
                ));
                results.push(tool_result);
                continue;
            }
        }

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

/// Wait for the approval oneshot to resolve while draining other agent messages
/// (Cancel, ApprovalDecision for other tool calls, SetModel).
async fn wait_for_approval(
    approval_rx: oneshot::Receiver<bool>,
    message_rx: &mut mpsc::UnboundedReceiver<AgentMessage>,
    pending_approvals: &mut HashMap<String, oneshot::Sender<bool>>,
    cancel_token: &CancellationToken,
) -> bool {
    tokio::pin!(approval_rx);

    loop {
        tokio::select! {
            biased;

            _ = cancel_token.cancelled() => return false,

            result = &mut approval_rx => {
                return result.unwrap_or(false);
            }

            maybe_msg = message_rx.recv() => {
                match maybe_msg {
                    Some(AgentMessage::Cancel) => {
                        cancel_token.cancel();
                        return false;
                    }
                    Some(AgentMessage::ApprovalDecision { tool_call_id, approved }) => {
                        // Route to the correct pending approval.
                        if let Some(sender) = pending_approvals.remove(&tool_call_id) {
                            let _ = sender.send(approved);
                        }
                        // The approval_rx for *our* tool call will resolve on the
                        // next loop iteration if this was for us.
                    }
                    Some(AgentMessage::SetModel(_)) => {
                        // Ignore model changes during approval wait.
                    }
                    Some(AgentMessage::UserMessage { .. }) => {
                        // Ignore new user messages during approval wait.
                    }
                    None => return false, // Channel closed.
                }
            }
        }
    }
}

/// Follow-up provider call after tool execution. Collects tokens and sends
/// events but does not recurse into further tool calls.
async fn followup_turn(
    provider: &dyn Provider,
    model: &str,
    event_tx: &mpsc::UnboundedSender<AgentEvent>,
    session: &mut Session,
    cancel_token: &CancellationToken,
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

    loop {
        tokio::select! {
            biased;

            _ = cancel_token.cancelled() => {
                debug!("agent loop: follow-up stream cancelled");
                break;
            }

            maybe_event = stream.next() => {
                match maybe_event {
                    Some(Event::Token(tok)) => {
                        text_buf.push_str(&tok);
                        let _ = event_tx.send(AgentEvent::Token(tok));
                    }
                    Some(Event::Error(e)) => {
                        let _ = event_tx.send(AgentEvent::Error(format!("follow-up stream error: {e}")));
                    }
                    Some(Event::Done) | None => break,
                    Some(_) => {}
                }
            }
        }
    }

    let _ = event_tx.send(AgentEvent::StreamDone);

    if !text_buf.is_empty() {
        session.push_message(Message::assistant(text_buf));
    }
}

/// Check if a tool requires user approval before execution.
fn requires_approval(tool_name: &str) -> bool {
    matches!(tool_name, "run_cmd" | "write_file" | "apply_patch")
}

/// Extract a short command preview from a tool call's args for display.
fn tool_call_command_preview(tc: &ToolCall) -> String {
    if let Some(cmd) = tc.args.get("command").and_then(|v| v.as_str()) {
        return cmd.to_string();
    }
    if let Some(path) = tc.args.get("path").and_then(|v| v.as_str()) {
        return format!("{} {path}", tc.name);
    }
    tc.name.clone()
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
        let _ = AgentEvent::ApprovalRequired {
            tool_call_id: "tc-1".into(),
            tool_name: "run_cmd".into(),
            command: "echo hi".into(),
            cwd: "/tmp".into(),
            sandbox_label: "workspace".into(),
        };
    }

    #[test]
    fn agent_message_variants_constructible() {
        let _ = AgentMessage::UserMessage {
            text: "hello".into(),
            target_agent: None,
            file_context: vec![],
        };
        let _ = AgentMessage::SetModel("gpt-4".into());
        let _ = AgentMessage::Cancel;
        let _ = AgentMessage::ApprovalDecision {
            tool_call_id: "tc-1".into(),
            approved: true,
        };
    }

    #[test]
    fn requires_approval_for_dangerous_tools() {
        assert!(requires_approval("run_cmd"));
        assert!(requires_approval("write_file"));
        assert!(requires_approval("apply_patch"));
        assert!(!requires_approval("read_file"));
        assert!(!requires_approval("search"));
        assert!(!requires_approval("list_dir"));
    }

    #[test]
    fn tool_call_command_preview_extracts_command() {
        let tc = ToolCall {
            id: "tc-1".into(),
            name: "run_cmd".into(),
            args: serde_json::json!({"command": "echo hello"}),
        };
        assert_eq!(tool_call_command_preview(&tc), "echo hello");
    }

    #[test]
    fn tool_call_command_preview_extracts_path() {
        let tc = ToolCall {
            id: "tc-2".into(),
            name: "write_file".into(),
            args: serde_json::json!({"path": "/tmp/foo.txt", "content": "bar"}),
        };
        assert_eq!(tool_call_command_preview(&tc), "write_file /tmp/foo.txt");
    }

    #[test]
    fn tool_call_command_preview_fallback() {
        let tc = ToolCall {
            id: "tc-3".into(),
            name: "search".into(),
            args: serde_json::json!({"query": "foo"}),
        };
        assert_eq!(tool_call_command_preview(&tc), "search");
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
