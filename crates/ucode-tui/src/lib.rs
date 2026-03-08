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
    pub tool_registry: std::sync::Arc<ucode_tools::ToolRegistry>,
    /// All known provider configs (for `/models` to query all providers).
    pub all_providers: std::collections::HashMap<String, ucode_providers::ProviderConfig>,
}

/// Ingredients for spawning an agent loop later (after `/connect`).
/// Contains everything except the provider config, which is determined
/// after authentication completes.
pub struct PendingAgentSetup {
    pub credential_store: std::sync::Arc<dyn ucode_auth::CredentialStore>,
    pub session_store: std::sync::Arc<ucode_core::SessionStore>,
    pub session: ucode_core::Session,
    pub tool_registry: std::sync::Arc<ucode_tools::ToolRegistry>,
}

/// Spawn the agent loop and wire it to the TUI via the event bridge.
/// Returns the message sender for the app and the join handles.
fn spawn_agent_loop(
    ac: AgentConfig,
    event_tx: &TuiEventSender,
) -> (
    tokio::sync::mpsc::UnboundedSender<ucode_agent::AgentMessage>,
    tokio::task::JoinHandle<()>,
    tokio::task::JoinHandle<()>,
) {
    let (msg_tx, msg_rx) = tokio::sync::mpsc::unbounded_channel::<ucode_agent::AgentMessage>();

    let (agent_event_tx, mut agent_event_rx) =
        tokio::sync::mpsc::unbounded_channel::<ucode_agent::AgentEvent>();

    // Spawn bridge task: forward AgentEvents to TuiEvents.
    let bridge_tx = event_tx.clone();
    let bridge_handle = tokio::spawn(async move {
        while let Some(ev) = agent_event_rx.recv().await {
            event_loop::bridge_agent_event(ev, &bridge_tx);
        }
    });

    // Spawn the agent loop.
    let agent_handle = tokio::spawn(ucode_agent::run_agent_loop(
        msg_rx,
        agent_event_tx,
        ac.loop_config,
        ac.session_store,
        ac.session,
        ac.tool_registry,
    ));

    (msg_tx, agent_handle, bridge_handle)
}

/// Run the fullscreen TUI. This is the main entry point.
///
/// Takes both ends of the TUI event channel so that auth tasks spawned inside
/// the loop can send results back via the sender. If `agent_config` is provided,
/// the agent loop is spawned immediately. Otherwise, `pending_setup` can be
/// provided to allow spawning the agent loop after `/connect` succeeds.
/// Blocks until the user exits or the sender is dropped.
pub async fn run(
    event_tx: TuiEventSender,
    event_rx: tokio::sync::mpsc::UnboundedReceiver<event_loop::TuiEvent>,
    agent_config: Option<AgentConfig>,
    pending_setup: Option<PendingAgentSetup>,
    agent_registry: ucode_core::agent_registry::AgentRegistry,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut app = app::AppState::new();
    app.agent_registry = agent_registry;

    // If agent config is provided, spawn the agent loop immediately.
    let mut _agent_handles: Option<(tokio::task::JoinHandle<()>, tokio::task::JoinHandle<()>)> =
        None;

    if let Some(mut ac) = agent_config {
        // Store all provider configs so /models can query all of them.
        app.providers = std::mem::take(&mut ac.all_providers);
        app.credential_store = ac.loop_config.credential_store.clone();

        // Populate session info in the TUI.
        app.session_id = ac.session.meta.id.clone();
        app.session_title = ac
            .session
            .meta
            .title
            .clone()
            .unwrap_or_else(|| truncate_id(&ac.session.meta.id));
        app.session_store = Some(ac.session_store.clone());
        app.current_session_id = Some(ac.session.meta.id.clone());

        let ac_model = ac.loop_config.model.clone();
        let provider_name = ac.loop_config.provider_name.clone();
        let (msg_tx, ah, bh) = spawn_agent_loop(ac, &event_tx);
        app.message_tx = Some(msg_tx);
        app.active_model = Some(ac_model);
        _agent_handles = Some((ah, bh));

        // Emit a system message confirming auto-connect.
        let _ = event_tx.send(event_loop::TuiEvent::Toast {
            level: components::toast::ToastLevel::Info,
            title: format!("Connected to {provider_name}"),
            body: None,
        });
    } else if let Some(ref setup) = pending_setup {
        // Populate session info even without an agent.
        app.session_id = setup.session.meta.id.clone();
        app.session_title = setup
            .session
            .meta
            .title
            .clone()
            .unwrap_or_else(|| truncate_id(&setup.session.meta.id));
        app.session_store = Some(setup.session_store.clone());
        app.current_session_id = Some(setup.session.meta.id.clone());

        // No provider configured — suggest /connect.
        let _ = event_tx.send(event_loop::TuiEvent::Toast {
            level: components::toast::ToastLevel::Info,
            title: "No provider configured".into(),
            body: Some("Use /connect to set up a provider".into()),
        });
    }

    let mut input_box = components::input::InputBoxState::new();
    let mut sidebar_data = components::sidebar::SidebarData::new();
    event_loop::run_event_loop(
        &mut app,
        &mut input_box,
        &mut sidebar_data,
        event_tx,
        event_rx,
        pending_setup,
    )
    .await
}

/// Truncate a session ID to 8 characters for display.
fn truncate_id(id: &str) -> String {
    if id.len() > 8 {
        format!("{}…", &id[..8])
    } else {
        id.to_string()
    }
}
