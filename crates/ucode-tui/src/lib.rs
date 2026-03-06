//! ucode-tui: ratatui-based fullscreen terminal UI

pub mod app;
pub mod clipboard;
pub mod components;
pub mod event_loop;
pub mod keybinds;
pub mod layout;
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

/// Run the fullscreen TUI. This is the main entry point.
///
/// Takes an event receiver for external systems to push events.
/// Blocks until the user exits or the sender is dropped.
pub async fn run(
    event_rx: tokio::sync::mpsc::UnboundedReceiver<event_loop::TuiEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut app = app::AppState::new();
    let mut input_box = components::input::InputBoxState::new();
    let mut sidebar_data = components::sidebar::SidebarData::new();
    event_loop::run_event_loop(&mut app, &mut input_box, &mut sidebar_data, event_rx).await
}
