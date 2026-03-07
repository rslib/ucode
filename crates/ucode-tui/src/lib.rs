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
}

/// Run the fullscreen TUI. This is the main entry point.
///
/// Takes both ends of the TUI event channel so that auth tasks spawned inside
/// the loop can send results back via the sender. If `agent_config` is provided,
/// the agent loop is spawned and wired to the TUI via the event bridge.
/// Blocks until the user exits or the sender is dropped.
pub async fn run(
    event_tx: TuiEventSender,
    event_rx: tokio::sync::mpsc::UnboundedReceiver<event_loop::TuiEvent>,
    agent_config: Option<AgentConfig>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut app = app::AppState::new();

    // If agent config is provided, set up the message channel and spawn the loop.
    let _agent_handle = if let Some(ac) = agent_config {
        let (msg_tx, msg_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        app.message_tx = Some(msg_tx);

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
