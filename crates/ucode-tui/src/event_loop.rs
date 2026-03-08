use std::time::{Duration, Instant};

use crossterm::event::{
    DisableBracketedPaste, EnableBracketedPaste, Event, EventStream, KeyEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use futures_util::StreamExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use tokio::sync::mpsc;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;
use tokio::time;

use ucode_core::directive::{Directive, parse_input};

use crate::app::{AppState, FocusTarget, ToolCallStatus, TranscriptEntry};
use crate::components::input::InputBoxState;
use crate::components::master_detail::MasterDetail;
use crate::components::sidebar::SidebarData;
use crate::components::status_bar::{StatusBar, StatusBarState};
use crate::components::tab_bar::{TabBar, TabId};
use crate::components::toast::ToastLevel;
use crate::components::transcript::TranscriptView;
use crate::keybinds::{Action, InputMode};
use crate::layout::compute_layout;
use crate::overlays::connect_modal::ConnectPhase;

// ---------------------------------------------------------------------------
// TuiEvent
// ---------------------------------------------------------------------------

/// Events that external systems can push into the TUI.
#[derive(Debug)]
pub enum TuiEvent {
    StreamToken(String),
    StreamDone,
    ToolCallStarted {
        name: String,
    },
    ToolCallCompleted {
        index: usize,
        status: ToolCallStatus,
        duration_ms: Option<u64>,
        summary: Option<String>,
        thinking: Option<String>,
        output: Option<String>,
    },
    RouterEvent(String),
    SystemMessage(String),
    PatchProposed {
        file_path: String,
        raw_diff: String,
        patch_id: Option<String>,
    },
    ApprovalRequired {
        /// Set when the approval comes from the agent loop (for sending back the decision).
        tool_call_id: Option<String>,
        tool_name: String,
        command: String,
        cwd: String,
        sandbox_label: String,
    },
    Toast {
        level: ToastLevel,
        title: String,
        body: Option<String>,
    },
    CheckpointCreated {
        name: String,
    },
    BudgetWarning {
        used_pct: f64,
        message: String,
    },
    AgentCompleted {
        agent_id: String,
        name: String,
    },
    AgentFailed {
        agent_id: String,
        name: String,
        error: String,
    },
    McpServerCrashed {
        server_name: String,
        error: String,
    },
    AuthExpired {
        provider: String,
    },
    AuthCompleted {
        provider: String,
    },
    AuthFailed {
        provider: String,
        error: String,
    },
    /// Image data loaded from clipboard or file, ready for display.
    ImageDataReady {
        label: String,
        data: Vec<u8>,
    },
    VerifyResult {
        provider: String,
        success: bool,
        message: Option<String>,
    },
    DeviceCodeReady {
        provider: String,
        user_code: String,
        verification_uri: String,
    },
    ModelsListed {
        provider: String,
        models: Vec<ucode_providers::ModelInfo>,
    },
    ModelsListFailed {
        error: String,
    },
    Quit,
}

// ---------------------------------------------------------------------------
// Terminal cleanup guard
// ---------------------------------------------------------------------------

/// Restores the terminal to its original state when dropped.
///
/// This ensures cleanup happens even if the event loop returns via `?` or
/// if a panic unwinds the stack.
struct TerminalGuard {
    mouse_enabled: bool,
}

impl TerminalGuard {
    fn new(mouse_enabled: bool) -> Self {
        Self { mouse_enabled }
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        // Best-effort cleanup — ignore errors during drop.
        let _ = disable_raw_mode();
        let mut stderr = std::io::stderr();
        let _ = crate::terminal::restore_terminal_title(&mut stderr);
        if self.mouse_enabled {
            let _ = execute!(
                stderr,
                crossterm::event::DisableMouseCapture,
                DisableBracketedPaste,
                LeaveAlternateScreen
            );
        } else {
            let _ = execute!(stderr, DisableBracketedPaste, LeaveAlternateScreen);
        }
    }
}

// ---------------------------------------------------------------------------
// run_event_loop
// ---------------------------------------------------------------------------

const RENDER_INTERVAL_MS: u64 = 16; // ~60 fps

/// Main async event loop. Blocks until the user exits or a `Quit` event arrives.
///
/// `event_tx` / `event_rx` are the two ends of the same channel; the sender is
/// cloned into spawned auth tasks so they can push results back into the loop.
pub async fn run_event_loop(
    app: &mut AppState,
    input_box: &mut InputBoxState,
    sidebar_data: &mut SidebarData,
    event_tx: UnboundedSender<TuiEvent>,
    mut event_rx: UnboundedReceiver<TuiEvent>,
    mut pending_setup: Option<crate::PendingAgentSetup>,
) -> Result<(), Box<dyn std::error::Error>> {
    // --- Terminal setup ---
    enable_raw_mode()?;
    let mut stderr = std::io::stderr();
    execute!(stderr, EnterAlternateScreen)?;
    execute!(stderr, EnableBracketedPaste)?;

    // Set terminal title.
    let _ = crate::terminal::set_terminal_title("ucode", &mut stderr);

    // Mouse capture is optional and can be disabled for tmux mouse-mode coexistence.
    let mouse_enabled = if app.mouse_enabled {
        execute!(stderr, crossterm::event::EnableMouseCapture).is_ok()
    } else {
        false
    };

    // The guard restores the terminal on drop (including panics).
    let _guard = TerminalGuard::new(mouse_enabled);

    let backend = CrosstermBackend::new(std::io::stderr());
    let mut terminal = Terminal::new(backend)?;

    // Detect image rendering protocol (sixel, kitty, halfblocks).
    // from_query_stdio() queries the terminal for font size and capabilities.
    // Falls back to halfblocks (always works) if detection fails.
    app.image_picker = match ratatui_image::picker::Picker::from_query_stdio() {
        Ok(picker) => Some(picker),
        Err(_) => {
            // Halfblocks always work — no terminal capability query needed.
            Some(ratatui_image::picker::Picker::halfblocks())
        }
    };

    // Spawn a dedicated task to read terminal events and forward them over
    // a channel. This decouples event reading from the render loop so that
    // fast typing naturally batches — the reader pushes events as fast as
    // they arrive, and the main loop drains the channel on each iteration.
    let (term_tx, mut term_rx) = mpsc::unbounded_channel::<Event>();
    let reader_handle = tokio::spawn(async move {
        let mut reader = EventStream::new();
        while let Some(Ok(event)) = reader.next().await {
            if term_tx.send(event).is_err() {
                break;
            }
        }
    });

    let mut render_tick = time::interval(Duration::from_millis(RENDER_INTERVAL_MS));
    // Don't burst-render on startup.
    render_tick.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

    let mut frame_counter: u64 = 0;
    // Tracks a running browser-OAuth or device-code task. Aborted on Esc.
    let mut auth_task: Option<JoinHandle<()>> = None;

    // Initial render.
    terminal.draw(|f| render_frame(f, app, input_box, sidebar_data, frame_counter))?;
    app.dirty = false;

    loop {
        tokio::select! {
            // Biased: drain all pending input before rendering.
            biased;

            // Terminal input events (highest priority).
            maybe_event = term_rx.recv() => {
                match maybe_event {
                    Some(event) => {
                        if handle_terminal_event(event, app, input_box, sidebar_data, &event_tx, &mut auth_task) {
                            break;
                        }
                        // Drain any remaining buffered events from the channel
                        // so we batch multiple keystrokes into one render frame.
                        while let Ok(event) = term_rx.try_recv() {
                            if handle_terminal_event(event, app, input_box, sidebar_data, &event_tx, &mut auth_task) {
                                break;
                            }
                        }
                        // Always sync input layout after processing events so
                        // the input box height updates immediately (even when
                        // dispatch_action returns early).
                        sync_input_layout(app, input_box);
                    }
                    None => break,
                }
            }

            // Render tick — only draw when dirty, streaming, or hint is active.
            _ = render_tick.tick(), if app.dirty || app.streaming || app.ctrl_c_hint.is_some() || !app.toasts.is_empty() => {
                frame_counter = frame_counter.wrapping_add(1);
                // Expire the Ctrl+C hint once the 2-second window closes.
                if let Some(last) = app.last_ctrl_c
                    && last.elapsed().as_millis() >= 2000
                {
                    app.ctrl_c_hint = None;
                    app.last_ctrl_c = None;
                }
                // Tick expired toasts.
                if app.toasts.tick() {
                    app.mark_dirty();
                }
                terminal.draw(|f| render_frame(f, app, input_box, sidebar_data, frame_counter))?;
                app.dirty = false;
            }

            // Streaming tokens and system events from external callers.
            maybe_tui_event = event_rx.recv() => {
                match maybe_tui_event {
                    Some(tui_event) => {
                        // Check if this is a successful VerifyResult and we can
                        // spawn the agent loop now (mid-session connect).
                        if let TuiEvent::VerifyResult { ref provider, success: true, .. } = tui_event
                            && app.message_tx.is_none()
                            && let Some(setup) = pending_setup.take()
                        {
                            let provider_id = provider.clone();
                            try_spawn_agent_after_connect(
                                &provider_id, setup, &event_tx, app,
                            );
                        }
                        if handle_tui_event(tui_event, app, sidebar_data, &event_tx) {
                            break;
                        }
                    }
                    None => break,
                }
            }
        }

        // Drain pending async actions that need `event_tx`.
        if app.models_fetch_pending {
            app.models_fetch_pending = false;
            spawn_models_fetch(app, &event_tx);
        }
    }

    // Save the current session before exiting.
    if let Some(ref store) = app.session_store
        && let Some(ref session_id) = app.current_session_id
        && let Ok(mut session) = store.load(session_id)
    {
        session.transcript = app.transcript_to_messages();
        session.meta.title = if app.session_title.is_empty() {
            None
        } else {
            Some(app.session_title.clone())
        };
        let _ = store.save(&session);
    }

    // Clean up the reader task.
    reader_handle.abort();

    Ok(())
}

// ---------------------------------------------------------------------------
// Auth task helpers
// ---------------------------------------------------------------------------

/// Return the `BrowserOAuthConfig` for the given provider / display-name pair.
fn get_oauth_config(
    provider_id: &str,
    display_name: &str,
) -> Option<ucode_auth::BrowserOAuthConfig> {
    match provider_id {
        "openai" => Some(ucode_auth::openai_subscription_oauth_config()),
        "anthropic" => {
            if display_name.contains("Max") {
                Some(ucode_auth::anthropic_max_oauth_config())
            } else {
                Some(ucode_auth::anthropic_console_oauth_config())
            }
        }
        _ => None,
    }
}

/// If the connect modal just transitioned to `BrowserOAuth` or `DeviceCode`,
/// abort any existing auth task and spawn the appropriate async flow.
fn maybe_spawn_auth_task(
    app: &mut AppState,
    event_tx: &UnboundedSender<TuiEvent>,
    auth_task: &mut Option<JoinHandle<()>>,
) {
    // Abort any previously running task before starting a new one.
    if let Some(old) = auth_task.take() {
        old.abort();
    }

    let cred_store = app.credential_store.clone();

    match &app.connect_modal.phase {
        ConnectPhase::BrowserOAuth {
            provider_id,
            display_name,
            ..
        } => {
            let provider_id = provider_id.clone();
            let display_name = display_name.clone();
            let tx = event_tx.clone();

            let Some(config) = get_oauth_config(&provider_id, &display_name) else {
                return;
            };

            // Update the phase with the auth URL so the UI can display it.
            app.connect_modal.phase = ConnectPhase::BrowserOAuth {
                provider_id: provider_id.clone(),
                display_name: display_name.clone(),
                url: Some(config.auth_url.clone()),
            };

            // Best-effort: open the auth URL in the user's browser.
            try_open_url(&config.auth_url);

            // `browser_oauth_authorize` uses `rand::thread_rng()` internally,
            // which is `!Send`. Run it on a dedicated blocking thread with its
            // own single-threaded tokio runtime so it never crosses a Send
            // boundary in the main executor.
            let handle = tokio::task::spawn_blocking(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("auth rt");
                rt.block_on(async move {
                    match ucode_auth::browser_oauth_authorize(&config).await {
                        Ok(material) => {
                            if let Some(store) = cred_store {
                                let _ = store.store(&provider_id, &material);
                            }
                            let _ = tx.send(TuiEvent::AuthCompleted {
                                provider: provider_id,
                            });
                        }
                        Err(e) => {
                            let _ = tx.send(TuiEvent::AuthFailed {
                                provider: provider_id,
                                error: e.to_string(),
                            });
                        }
                    }
                });
            });
            // Wrap in a plain JoinHandle<()> so auth_task stays homogeneous.
            *auth_task = Some(tokio::spawn(async move {
                let _ = handle.await;
            }));
        }

        ConnectPhase::DeviceCode { provider_id, .. } => {
            let provider_id = provider_id.clone();
            let tx = event_tx.clone();

            let config = match provider_id.as_str() {
                "github-copilot" => Some(ucode_auth::github_copilot_device_config(None)),
                _ => None,
            };
            let Some(config) = config else {
                return;
            };

            let handle = tokio::spawn(async move {
                let client = reqwest::Client::new();
                match ucode_auth::request_device_code(&client, &config).await {
                    Ok(pending) => {
                        let _ = tx.send(TuiEvent::DeviceCodeReady {
                            provider: provider_id.clone(),
                            user_code: pending.user_code.clone(),
                            verification_uri: pending.verification_uri.clone(),
                        });
                        match ucode_auth::poll_for_token(&client, &config, &pending).await {
                            Ok(material) => {
                                if let Some(store) = cred_store {
                                    let _ = store.store(&provider_id, &material);
                                }
                                let _ = tx.send(TuiEvent::AuthCompleted {
                                    provider: provider_id,
                                });
                            }
                            Err(e) => {
                                let _ = tx.send(TuiEvent::AuthFailed {
                                    provider: provider_id,
                                    error: e.to_string(),
                                });
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(TuiEvent::AuthFailed {
                            provider: provider_id,
                            error: e.to_string(),
                        });
                    }
                }
            });
            *auth_task = Some(handle);
        }

        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Input layout sync
// ---------------------------------------------------------------------------

/// Update the input box layout height and ensure the cursor is visible.
fn sync_input_layout(app: &mut AppState, input_box: &mut InputBoxState) {
    use crate::layout::INPUT_MAX_LINES;
    app.input.line_count = (input_box.line_count() as u16).clamp(1, INPUT_MAX_LINES);
    input_box.ensure_cursor_visible(INPUT_MAX_LINES as usize);
}

fn update_autocomplete(app: &AppState, input_box: &mut InputBoxState) {
    if input_box.has_slash_prefix() {
        let entries = app.slash_completions(&input_box.content);
        input_box.autocomplete.show(entries);
    } else if input_box.has_mention_prefix() {
        let entries = app.mention_completions(&input_box.content);
        input_box.autocomplete.show(entries);
    } else {
        input_box.autocomplete.hide();
    }
}

// ---------------------------------------------------------------------------
// Terminal event handler
// ---------------------------------------------------------------------------

/// Handle a single crossterm `Event`. Returns `true` when the loop should exit.
fn handle_terminal_event(
    event: Event,
    app: &mut AppState,
    input_box: &mut InputBoxState,
    _sidebar_data: &mut SidebarData,
    event_tx: &UnboundedSender<TuiEvent>,
    auth_task: &mut Option<JoinHandle<()>>,
) -> bool {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => {
            // Dismiss the image popup on Esc before any other overlay handling.
            if app.image_popup.visible && key.code == crossterm::event::KeyCode::Esc {
                app.image_popup.close();
                app.focus = FocusTarget::Input;
                app.mark_dirty();
                return false;
            }

            // When the search overlay is open, route keys to it.
            if app.search_overlay.visible {
                let preset = app.keybinds.preset;
                match key.code {
                    crossterm::event::KeyCode::Esc => {
                        app.search_overlay.close();
                        app.focus = FocusTarget::Input;
                        app.mark_dirty();
                    }
                    crossterm::event::KeyCode::Enter => match preset {
                        crate::keybinds::KeybindPreset::Vscode => {
                            app.search_overlay.next_match();
                            if let Some(m) = app.search_overlay.current_match_info() {
                                app.scroll_offset = m.transcript_index;
                            }
                            app.mark_dirty();
                        }
                        crate::keybinds::KeybindPreset::Vim
                        | crate::keybinds::KeybindPreset::Emacs => {
                            app.search_overlay.close();
                            app.focus = FocusTarget::Input;
                            app.mark_dirty();
                        }
                    },
                    crossterm::event::KeyCode::Down => {
                        app.search_overlay.next_match();
                        if let Some(m) = app.search_overlay.current_match_info() {
                            app.scroll_offset = m.transcript_index;
                        }
                        app.mark_dirty();
                    }
                    crossterm::event::KeyCode::Up => {
                        app.search_overlay.prev_match();
                        if let Some(m) = app.search_overlay.current_match_info() {
                            app.scroll_offset = m.transcript_index;
                        }
                        app.mark_dirty();
                    }
                    crossterm::event::KeyCode::Backspace => {
                        app.search_overlay.delete_char();
                        let transcript = app.transcript.clone();
                        app.search_overlay.search(&transcript);
                        app.mark_dirty();
                    }
                    crossterm::event::KeyCode::Char(c) => {
                        if preset == crate::keybinds::KeybindPreset::Emacs
                            && key.modifiers == crossterm::event::KeyModifiers::CONTROL
                        {
                            match c {
                                's' => {
                                    app.search_overlay.next_match();
                                    if let Some(m) = app.search_overlay.current_match_info() {
                                        app.scroll_offset = m.transcript_index;
                                    }
                                    app.mark_dirty();
                                }
                                'r' => {
                                    app.search_overlay.prev_match();
                                    if let Some(m) = app.search_overlay.current_match_info() {
                                        app.scroll_offset = m.transcript_index;
                                    }
                                    app.mark_dirty();
                                }
                                'g' => {
                                    app.search_overlay.close();
                                    app.focus = FocusTarget::Input;
                                    app.mark_dirty();
                                }
                                _ => {}
                            }
                        } else if key.modifiers.is_empty()
                            || key.modifiers == crossterm::event::KeyModifiers::SHIFT
                        {
                            app.search_overlay.insert_char(c);
                            let transcript = app.transcript.clone();
                            app.search_overlay.search(&transcript);
                            app.mark_dirty();
                        }
                    }
                    _ => {}
                }
                return false;
            }

            // When the keybind overlay is open, route keys to it.
            if app.keybind_overlay.visible {
                match key.code {
                    crossterm::event::KeyCode::Esc => {
                        app.keybind_overlay.close();
                        app.focus = FocusTarget::Input;
                        app.mark_dirty();
                    }
                    crossterm::event::KeyCode::Up | crossterm::event::KeyCode::Char('k') => {
                        app.keybind_overlay.scroll_up(1);
                        app.mark_dirty();
                    }
                    crossterm::event::KeyCode::Down | crossterm::event::KeyCode::Char('j') => {
                        app.keybind_overlay.scroll_down(1);
                        app.mark_dirty();
                    }
                    crossterm::event::KeyCode::PageUp => {
                        app.keybind_overlay.scroll_up(10);
                        app.mark_dirty();
                    }
                    crossterm::event::KeyCode::PageDown => {
                        app.keybind_overlay.scroll_down(10);
                        app.mark_dirty();
                    }
                    _ => {}
                }
                return false;
            }

            // When copy mode is active, route keys to it.
            if app.copy_mode.active {
                handle_copy_mode_key(&key, app);
                return false;
            }

            // When the models modal is open, route keys to it.
            if app.models_modal.visible {
                match key.code {
                    crossterm::event::KeyCode::Esc => {
                        app.models_modal.close();
                        app.focus = FocusTarget::Input;
                        app.mark_dirty();
                    }
                    crossterm::event::KeyCode::Enter => {
                        if let Some(entry) = app.models_modal.selected_entry().cloned() {
                            // Send SetModel to agent loop.
                            if let Some(tx) = &app.message_tx {
                                let _ = tx.send(ucode_agent::AgentMessage::SetModel(
                                    entry.model_id.clone(),
                                ));
                            }
                            app.active_model = Some(entry.model_id.clone());
                            app.models_modal.close();
                            app.focus = FocusTarget::Input;
                            app.push_system_message(format!(
                                "Switched to model: {}",
                                entry.label()
                            ));
                        }
                        app.mark_dirty();
                    }
                    crossterm::event::KeyCode::Up => {
                        app.models_modal.move_up();
                        app.mark_dirty();
                    }
                    crossterm::event::KeyCode::Down => {
                        app.models_modal.move_down();
                        app.mark_dirty();
                    }
                    crossterm::event::KeyCode::Backspace => {
                        app.models_modal.delete_char();
                        app.mark_dirty();
                    }
                    crossterm::event::KeyCode::Char(c) => {
                        app.models_modal.insert_char(c);
                        app.mark_dirty();
                    }
                    _ => {}
                }
                return false;
            }

            // When the session picker is open, route keys to it.
            if app.session_picker.visible {
                match key.code {
                    crossterm::event::KeyCode::Esc => {
                        app.session_picker.close();
                        app.focus = FocusTarget::Input;
                        app.mark_dirty();
                    }
                    crossterm::event::KeyCode::Enter => {
                        if let Some(session_id) =
                            app.session_picker.selected_session_id().map(str::to_owned)
                            && let Some(ref store) = app.session_store
                        {
                            match store.load(&session_id) {
                                Ok(session) => {
                                    let title = session.meta.title.clone();
                                    app.load_session_transcript(&session.transcript);
                                    app.current_session_id = Some(session_id.clone());
                                    app.session_id = session_id;
                                    app.session_title = title.clone().unwrap_or_default();
                                    app.session_picker.close();
                                    app.focus = FocusTarget::Input;
                                    let label = title.unwrap_or_else(|| "Untitled".to_owned());
                                    app.toast(
                                        crate::components::toast::ToastLevel::Info,
                                        format!("Loaded session: {label}"),
                                    );
                                }
                                Err(e) => {
                                    app.toast(
                                        crate::components::toast::ToastLevel::Error,
                                        format!("Failed to load session: {e}"),
                                    );
                                }
                            }
                        }
                        app.mark_dirty();
                    }
                    crossterm::event::KeyCode::Up => {
                        app.session_picker.move_up();
                        app.mark_dirty();
                    }
                    crossterm::event::KeyCode::Down => {
                        app.session_picker.move_down();
                        app.mark_dirty();
                    }
                    crossterm::event::KeyCode::Backspace => {
                        app.session_picker.delete_char();
                        app.mark_dirty();
                    }
                    crossterm::event::KeyCode::Char(c) => {
                        app.session_picker.insert_char(c);
                        app.mark_dirty();
                    }
                    _ => {}
                }
                return false;
            }

            // When the connect modal is open, route keys to it.
            if app.connect_modal.visible {
                match &app.connect_modal.phase {
                    ConnectPhase::ProviderList => match key.code {
                        crossterm::event::KeyCode::Esc => {
                            app.connect_modal.close();
                            app.focus = FocusTarget::Input;
                            app.mark_dirty();
                        }
                        crossterm::event::KeyCode::Enter => {
                            if let Some(provider) = app.connect_modal.selected_provider().cloned() {
                                app.connect_modal.select_provider(&provider);
                                maybe_spawn_auth_task(app, event_tx, auth_task);
                                app.mark_dirty();
                            }
                        }
                        crossterm::event::KeyCode::Up => {
                            app.connect_modal.move_up();
                            app.mark_dirty();
                        }
                        crossterm::event::KeyCode::Down => {
                            app.connect_modal.move_down();
                            app.mark_dirty();
                        }
                        crossterm::event::KeyCode::Backspace => {
                            app.connect_modal.delete_char();
                            app.mark_dirty();
                        }
                        crossterm::event::KeyCode::Char(c) => {
                            app.connect_modal.insert_char(c);
                            app.mark_dirty();
                        }
                        _ => {}
                    },
                    ConnectPhase::MethodPicker { .. } => match key.code {
                        crossterm::event::KeyCode::Esc => {
                            app.connect_modal.phase = ConnectPhase::ProviderList;
                            app.mark_dirty();
                        }
                        crossterm::event::KeyCode::Enter => {
                            app.connect_modal.select_method();
                            maybe_spawn_auth_task(app, event_tx, auth_task);
                            app.mark_dirty();
                        }
                        crossterm::event::KeyCode::Up => {
                            app.connect_modal.method_up();
                            app.mark_dirty();
                        }
                        crossterm::event::KeyCode::Down => {
                            app.connect_modal.method_down();
                            app.mark_dirty();
                        }
                        _ => {}
                    },
                    ConnectPhase::ApiKeyEntry { .. } => match key.code {
                        crossterm::event::KeyCode::Esc => {
                            app.connect_modal.phase = ConnectPhase::ProviderList;
                            app.mark_dirty();
                        }
                        crossterm::event::KeyCode::Enter => {
                            if let Some((provider_id, api_key)) = app.connect_modal.take_api_key() {
                                let material = ucode_auth::AuthMaterial::ApiKey { key: api_key };
                                let store_result = app
                                    .credential_store
                                    .as_ref()
                                    .map(|s| s.store(&provider_id, &material))
                                    .unwrap_or(Ok(()));
                                match store_result {
                                    Ok(()) => {
                                        let display = app
                                            .connect_modal
                                            .phase_display_name()
                                            .unwrap_or(provider_id.clone());
                                        app.connect_modal.phase = ConnectPhase::Verifying {
                                            provider_id: provider_id.clone(),
                                            display_name: display,
                                        };
                                        app.mark_dirty();

                                        let tx = event_tx.clone();
                                        let cred = app.credential_store.clone();
                                        tokio::spawn(async move {
                                            let result = verify_provider(&provider_id, cred).await;
                                            let _ = tx.send(TuiEvent::VerifyResult {
                                                provider: provider_id,
                                                success: result.is_ok(),
                                                message: result.err(),
                                            });
                                        });
                                    }
                                    Err(e) => {
                                        app.connect_modal.close();
                                        app.focus = FocusTarget::Input;
                                        app.toast(
                                            crate::components::toast::ToastLevel::Error,
                                            format!("Failed to store key: {e}"),
                                        );
                                        app.mark_dirty();
                                    }
                                }
                            }
                        }
                        crossterm::event::KeyCode::Backspace => {
                            app.connect_modal.api_key_delete_char();
                            app.mark_dirty();
                        }
                        crossterm::event::KeyCode::Char(c) => {
                            app.connect_modal.api_key_insert_char(c);
                            app.mark_dirty();
                        }
                        _ => {}
                    },
                    ConnectPhase::BrowserOAuth { .. } | ConnectPhase::DeviceCode { .. } => {
                        if key.code == crossterm::event::KeyCode::Esc {
                            if let Some(handle) = auth_task.take() {
                                handle.abort();
                            }
                            app.connect_modal.phase = ConnectPhase::ProviderList;
                            app.mark_dirty();
                        }
                    }
                    ConnectPhase::Verifying { .. } => {}
                }
                return false;
            }

            // When the palette is open, route keys to it.
            if app.palette.visible {
                match key.code {
                    crossterm::event::KeyCode::Esc => {
                        app.palette.close();
                        app.focus = FocusTarget::Input;
                        app.mark_dirty();
                    }
                    crossterm::event::KeyCode::Enter => {
                        if let Some(cmd) = app.palette.execute_selected() {
                            if let Some(action) = app.execute_command(&cmd.name, &[]) {
                                app.focus = FocusTarget::Input;
                                return dispatch_action(action, app, input_box);
                            }
                            app.focus = FocusTarget::Input;
                        }
                    }
                    crossterm::event::KeyCode::Up => {
                        app.palette.move_up();
                        app.mark_dirty();
                    }
                    crossterm::event::KeyCode::Down => {
                        app.palette.move_down();
                        app.mark_dirty();
                    }
                    crossterm::event::KeyCode::Backspace => {
                        app.palette.delete_char();
                        app.mark_dirty();
                    }
                    crossterm::event::KeyCode::Char(c) => {
                        app.palette.insert_char(c);
                        app.mark_dirty();
                    }
                    _ => {}
                }
                return false;
            }

            // When the diff modal is open, route keys to it.
            if app.diff_modal.visible {
                match key.code {
                    crossterm::event::KeyCode::Esc => {
                        app.diff_modal.close();
                        app.advance_overlay_queue();
                    }
                    crossterm::event::KeyCode::Char('a') => {
                        let path = app.diff_modal.file_path.clone();
                        app.diff_modal.approve();
                        app.push_system_message(format!("Patch approved: {path}"));
                        app.advance_overlay_queue();
                    }
                    crossterm::event::KeyCode::Char('r') => {
                        let path = app.diff_modal.file_path.clone();
                        app.diff_modal.reject();
                        app.push_system_message(format!("Patch rejected: {path}"));
                        app.advance_overlay_queue();
                    }
                    crossterm::event::KeyCode::Up | crossterm::event::KeyCode::Char('k') => {
                        app.diff_modal.scroll_up(1);
                        app.mark_dirty();
                    }
                    crossterm::event::KeyCode::Down | crossterm::event::KeyCode::Char('j') => {
                        app.diff_modal.scroll_down(1);
                        app.mark_dirty();
                    }
                    crossterm::event::KeyCode::PageUp => {
                        app.diff_modal.scroll_up(10);
                        app.mark_dirty();
                    }
                    crossterm::event::KeyCode::PageDown => {
                        app.diff_modal.scroll_down(10);
                        app.mark_dirty();
                    }
                    _ => {}
                }
                return false;
            }

            // When the approval modal is open, route keys to it.
            if app.approval_modal.visible {
                match key.code {
                    crossterm::event::KeyCode::Esc => {
                        // Esc = deny (same as 'd').
                        send_approval_decision(app, false);
                        app.approval_modal.close();
                        app.advance_overlay_queue();
                    }
                    crossterm::event::KeyCode::Char('o') => {
                        let tool = app.approval_modal.tool_name.clone();
                        send_approval_decision(app, true);
                        app.approval_modal.approve_once();
                        app.push_system_message(format!("Approved once: {tool}"));
                        app.advance_overlay_queue();
                    }
                    crossterm::event::KeyCode::Char('s') => {
                        let tool = app.approval_modal.tool_name.clone();
                        send_approval_decision(app, true);
                        app.approval_modal.approve_session();
                        app.push_system_message(format!("Approved for session: {tool}"));
                        app.advance_overlay_queue();
                    }
                    crossterm::event::KeyCode::Char('d') => {
                        let tool = app.approval_modal.tool_name.clone();
                        send_approval_decision(app, false);
                        app.approval_modal.deny();
                        app.push_system_message(format!("Denied: {tool}"));
                        app.advance_overlay_queue();
                    }
                    _ => {}
                }
                return false;
            }

            // When focus is on a panel tab, route keys to panel navigation.
            if app.focus == FocusTarget::Panel {
                let preset = app.keybinds.preset;
                let tab = app.tab_bar.active;
                let panel = match tab {
                    TabId::Subagents => Some(&mut app.subagents_panel),
                    TabId::Tools => Some(&mut app.tools_panel),
                    TabId::Mcp => Some(&mut app.mcp_panel),
                    TabId::Logs => Some(&mut app.logs_panel),
                    TabId::Chat => None,
                };
                if let Some(panel) = panel {
                    match key.code {
                        crossterm::event::KeyCode::Tab => {
                            panel.toggle_focus();
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Esc => {
                            if panel.focus == crate::components::master_detail::PanelFocus::Detail {
                                panel.focus_master();
                            } else {
                                app.tab_bar.active = TabId::Chat;
                                app.focus = FocusTarget::Input;
                            }
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Enter => {
                            if panel.focus == crate::components::master_detail::PanelFocus::Master {
                                panel.focus_detail();
                                app.mark_dirty();
                            }
                            return false;
                        }
                        _ => {}
                    }

                    use crate::components::master_detail::PanelFocus;
                    match panel.focus {
                        PanelFocus::Master => {
                            let handled = match preset {
                                crate::keybinds::KeybindPreset::Vim => match key.code {
                                    crossterm::event::KeyCode::Char('j')
                                        if key.modifiers.is_empty() =>
                                    {
                                        panel.select_next();
                                        true
                                    }
                                    crossterm::event::KeyCode::Char('k')
                                        if key.modifiers.is_empty() =>
                                    {
                                        panel.select_prev();
                                        true
                                    }
                                    crossterm::event::KeyCode::Up => {
                                        panel.select_prev();
                                        true
                                    }
                                    crossterm::event::KeyCode::Down => {
                                        panel.select_next();
                                        true
                                    }
                                    _ => false,
                                },
                                crate::keybinds::KeybindPreset::Emacs => {
                                    match (key.code, key.modifiers) {
                                        (
                                            crossterm::event::KeyCode::Char('n'),
                                            crossterm::event::KeyModifiers::CONTROL,
                                        ) => {
                                            panel.select_next();
                                            true
                                        }
                                        (
                                            crossterm::event::KeyCode::Char('p'),
                                            crossterm::event::KeyModifiers::CONTROL,
                                        ) => {
                                            panel.select_prev();
                                            true
                                        }
                                        (crossterm::event::KeyCode::Up, _) => {
                                            panel.select_prev();
                                            true
                                        }
                                        (crossterm::event::KeyCode::Down, _) => {
                                            panel.select_next();
                                            true
                                        }
                                        _ => false,
                                    }
                                }
                                crate::keybinds::KeybindPreset::Vscode => match key.code {
                                    crossterm::event::KeyCode::Up => {
                                        panel.select_prev();
                                        true
                                    }
                                    crossterm::event::KeyCode::Down => {
                                        panel.select_next();
                                        true
                                    }
                                    crossterm::event::KeyCode::Char('n')
                                        if key.modifiers
                                            == crossterm::event::KeyModifiers::CONTROL =>
                                    {
                                        panel.select_next();
                                        true
                                    }
                                    crossterm::event::KeyCode::Char('p')
                                        if key.modifiers
                                            == crossterm::event::KeyModifiers::CONTROL =>
                                    {
                                        panel.select_prev();
                                        true
                                    }
                                    _ => false,
                                },
                            };
                            if handled {
                                app.mark_dirty();
                                return false;
                            }
                        }
                        PanelFocus::Detail => {
                            let handled = match preset {
                                crate::keybinds::KeybindPreset::Vim => match key.code {
                                    crossterm::event::KeyCode::Char('j')
                                        if key.modifiers.is_empty() =>
                                    {
                                        panel.scroll_buffer_down(1);
                                        true
                                    }
                                    crossterm::event::KeyCode::Char('k')
                                        if key.modifiers.is_empty() =>
                                    {
                                        panel.scroll_buffer_up(1);
                                        true
                                    }
                                    crossterm::event::KeyCode::Up => {
                                        panel.scroll_buffer_up(1);
                                        true
                                    }
                                    crossterm::event::KeyCode::Down => {
                                        panel.scroll_buffer_down(1);
                                        true
                                    }
                                    crossterm::event::KeyCode::PageUp => {
                                        panel.scroll_buffer_up(10);
                                        true
                                    }
                                    crossterm::event::KeyCode::PageDown => {
                                        panel.scroll_buffer_down(10);
                                        true
                                    }
                                    _ => false,
                                },
                                crate::keybinds::KeybindPreset::Emacs => {
                                    match (key.code, key.modifiers) {
                                        (
                                            crossterm::event::KeyCode::Char('n'),
                                            crossterm::event::KeyModifiers::CONTROL,
                                        ) => {
                                            panel.scroll_buffer_down(1);
                                            true
                                        }
                                        (
                                            crossterm::event::KeyCode::Char('p'),
                                            crossterm::event::KeyModifiers::CONTROL,
                                        ) => {
                                            panel.scroll_buffer_up(1);
                                            true
                                        }
                                        (crossterm::event::KeyCode::Up, _) => {
                                            panel.scroll_buffer_up(1);
                                            true
                                        }
                                        (crossterm::event::KeyCode::Down, _) => {
                                            panel.scroll_buffer_down(1);
                                            true
                                        }
                                        (crossterm::event::KeyCode::PageUp, _) => {
                                            panel.scroll_buffer_up(10);
                                            true
                                        }
                                        (crossterm::event::KeyCode::PageDown, _) => {
                                            panel.scroll_buffer_down(10);
                                            true
                                        }
                                        _ => false,
                                    }
                                }
                                crate::keybinds::KeybindPreset::Vscode => match key.code {
                                    crossterm::event::KeyCode::Up => {
                                        panel.scroll_buffer_up(1);
                                        true
                                    }
                                    crossterm::event::KeyCode::Down => {
                                        panel.scroll_buffer_down(1);
                                        true
                                    }
                                    crossterm::event::KeyCode::PageUp => {
                                        panel.scroll_buffer_up(10);
                                        true
                                    }
                                    crossterm::event::KeyCode::PageDown => {
                                        panel.scroll_buffer_down(10);
                                        true
                                    }
                                    _ => false,
                                },
                            };
                            if handled {
                                // Clamp scroll so we can't go past the last line.
                                // The detail pane height equals transcript_area height
                                // minus the 1-row pane header.
                                let viewport_h =
                                    app.transcript_area.height.saturating_sub(1) as usize;
                                panel.clamp_buffer_scroll(viewport_h);
                                app.mark_dirty();
                                return false;
                            }
                        }
                    }
                }

                // Fall through to keybind resolver for Alt+1..5, Ctrl+C, etc.
                if let Some(action) = app.keybinds.resolve(&key) {
                    return dispatch_action(action, app, input_box);
                }
                return false;
            }

            // When focus is on the input box, send characters and editing
            // keys directly to it. Only Ctrl/Alt modified keys and
            // non-character keys (Enter, Esc, arrows, etc.) go through
            // keybind resolution. This prevents bare-letter bindings
            // (e.g. 'a' -> ApproveAction) from eating typed text.
            if app.focus == FocusTarget::Input {
                let is_bare_char = matches!(key.code, crossterm::event::KeyCode::Char(_))
                    && key.modifiers == crossterm::event::KeyModifiers::NONE;
                let is_shift_char = matches!(key.code, crossterm::event::KeyCode::Char(_))
                    && key.modifiers == crossterm::event::KeyModifiers::SHIFT;

                // Ctrl+C with active selection: copy to clipboard instead of cancel.
                if key.modifiers == crossterm::event::KeyModifiers::CONTROL
                    && key.code == crossterm::event::KeyCode::Char('c')
                    && input_box.selecting
                {
                    if let Some(text) = input_box.selected_text() {
                        do_clipboard_copy(text, app);
                    }
                    input_box.cancel_selecting();
                    app.mark_dirty();
                    return false;
                }

                // Ctrl+X with active selection: cut to clipboard.
                if key.modifiers == crossterm::event::KeyModifiers::CONTROL
                    && key.code == crossterm::event::KeyCode::Char('x')
                    && input_box.selecting
                {
                    if let Some(text) = input_box.selected_text() {
                        do_clipboard_copy(text, app);
                    }
                    input_box.delete_selected();
                    app.mark_dirty();
                    return false;
                }

                if is_bare_char || is_shift_char {
                    // When the input box is empty, check if the key matches
                    // a keybind first (e.g. `v` → EnterCopyMode, `?` →
                    // ShowKeybindOverlay). If it does, dispatch the action
                    // instead of inserting the character.
                    if input_box.content.is_empty()
                        && let Some(action) = app.keybinds.resolve(&key)
                    {
                        return dispatch_action(action, app, input_box);
                    }
                    if let crossterm::event::KeyCode::Char(c) = key.code {
                        if input_box.selecting {
                            input_box.delete_selected();
                        }
                        input_box.insert_char(c);
                        update_autocomplete(app, input_box);
                        app.mark_dirty();
                    }
                    return false;
                }

                // Shift+key: extend selection while moving.
                let is_shift = key.modifiers == crossterm::event::KeyModifiers::SHIFT;
                let is_shift_ctrl = key.modifiers
                    == (crossterm::event::KeyModifiers::SHIFT
                        | crossterm::event::KeyModifiers::CONTROL);

                if is_shift {
                    match key.code {
                        crossterm::event::KeyCode::Left => {
                            input_box.ensure_selecting();
                            input_box.move_left();
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Right => {
                            input_box.ensure_selecting();
                            input_box.move_right();
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Up => {
                            input_box.ensure_selecting();
                            input_box.move_up();
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Down => {
                            input_box.ensure_selecting();
                            input_box.move_down();
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Home => {
                            input_box.ensure_selecting();
                            input_box.move_home();
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::End => {
                            input_box.ensure_selecting();
                            input_box.move_end();
                            app.mark_dirty();
                            return false;
                        }
                        _ => {} // fall through
                    }
                }

                if is_shift_ctrl {
                    match key.code {
                        crossterm::event::KeyCode::Left => {
                            input_box.ensure_selecting();
                            input_box.move_word_left();
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Right => {
                            input_box.ensure_selecting();
                            input_box.move_word_right();
                            app.mark_dirty();
                            return false;
                        }
                        _ => {} // fall through
                    }
                }

                // Bare (no-modifier) editing keys go to input box.
                if key.modifiers == crossterm::event::KeyModifiers::NONE {
                    match key.code {
                        crossterm::event::KeyCode::Backspace => {
                            if input_box.selecting {
                                input_box.delete_selected();
                            } else {
                                input_box.delete_char();
                            }
                            update_autocomplete(app, input_box);
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Delete => {
                            if input_box.selecting {
                                input_box.delete_selected();
                            } else {
                                input_box.delete_forward();
                            }
                            update_autocomplete(app, input_box);
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Left => {
                            input_box.cancel_selecting();
                            input_box.move_left();
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Right => {
                            input_box.cancel_selecting();
                            input_box.move_right();
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Home => {
                            input_box.cancel_selecting();
                            input_box.move_home();
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::End => {
                            input_box.cancel_selecting();
                            input_box.move_end();
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Up => {
                            input_box.cancel_selecting();
                            if input_box.autocomplete.visible {
                                input_box.autocomplete.prev();
                            } else if input_box.cursor_on_first_line() {
                                input_box.history_prev();
                            } else {
                                input_box.move_up();
                            }
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Down => {
                            input_box.cancel_selecting();
                            if input_box.autocomplete.visible {
                                input_box.autocomplete.next();
                            } else if input_box.cursor_on_last_line() {
                                input_box.history_next();
                            } else {
                                input_box.move_down();
                            }
                            app.mark_dirty();
                            return false;
                        }
                        _ => {} // fall through to readline / keybind resolution
                    }
                }

                // Modified arrow/backspace keys for word-level navigation and deletion.
                let is_ctrl = key.modifiers == crossterm::event::KeyModifiers::CONTROL;
                let is_alt = key.modifiers == crossterm::event::KeyModifiers::ALT;
                if is_ctrl || is_alt {
                    match key.code {
                        crossterm::event::KeyCode::Left => {
                            input_box.cancel_selecting();
                            input_box.move_word_left();
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Right => {
                            input_box.cancel_selecting();
                            input_box.move_word_right();
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Backspace => {
                            input_box.delete_word_back();
                            app.mark_dirty();
                            return false;
                        }
                        _ => {} // fall through
                    }
                }

                // Readline-style Ctrl/Alt bindings — intercepted before the
                // app-level keybind resolver when focus is on the input box.
                // Ctrl+C, Ctrl+P, Ctrl+L, Ctrl+O, Ctrl+Y are intentionally excluded
                // so they still reach the keybind resolver.
                if key.modifiers == crossterm::event::KeyModifiers::CONTROL {
                    match key.code {
                        crossterm::event::KeyCode::Char('a') => {
                            input_box.cancel_selecting();
                            input_box.move_home();
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Char('e') => {
                            input_box.cancel_selecting();
                            input_box.move_end();
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Char('b') => {
                            input_box.cancel_selecting();
                            input_box.move_left();
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Char('f') => {
                            input_box.cancel_selecting();
                            input_box.move_right();
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Char('d') => {
                            // Only consume if input is non-empty; otherwise fall
                            // through so Ctrl+D can still mean Exit.
                            if !input_box.is_empty() {
                                input_box.delete_forward();
                                app.mark_dirty();
                                return false;
                            }
                        }
                        crossterm::event::KeyCode::Char('h') => {
                            input_box.delete_char();
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Char('k') => {
                            input_box.kill_to_end();
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Char('u') => {
                            input_box.kill_to_start();
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Char('w') => {
                            input_box.delete_word_back();
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Char('t') => {
                            input_box.transpose_chars();
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Char('p') => {
                            if input_box.autocomplete.visible {
                                input_box.autocomplete.prev();
                            } else {
                                input_box.move_up();
                            }
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Char('n') => {
                            if input_box.autocomplete.visible {
                                input_box.autocomplete.next();
                            } else {
                                input_box.move_down();
                            }
                            app.mark_dirty();
                            return false;
                        }
                        _ => {} // fall through to keybind resolver
                    }
                }

                if key.modifiers == crossterm::event::KeyModifiers::ALT {
                    // Let app-level keybinds (e.g. Alt+1 → SelectTab1) take
                    // priority over readline shortcuts.
                    if app.keybinds.resolve(&key).is_none() {
                        match key.code {
                            crossterm::event::KeyCode::Char('b') => {
                                input_box.cancel_selecting();
                                input_box.move_word_left();
                                app.mark_dirty();
                                return false;
                            }
                            crossterm::event::KeyCode::Char('f') => {
                                input_box.cancel_selecting();
                                input_box.move_word_right();
                                app.mark_dirty();
                                return false;
                            }
                            crossterm::event::KeyCode::Char('d') => {
                                input_box.delete_word_forward();
                                app.mark_dirty();
                                return false;
                            }
                            crossterm::event::KeyCode::Char('t') => {
                                input_box.transpose_words();
                                app.mark_dirty();
                                return false;
                            }
                            crossterm::event::KeyCode::Char('u') => {
                                input_box.upcase_word();
                                app.mark_dirty();
                                return false;
                            }
                            crossterm::event::KeyCode::Char('l') => {
                                input_box.downcase_word();
                                app.mark_dirty();
                                return false;
                            }
                            crossterm::event::KeyCode::Char('c') => {
                                input_box.capitalize_word();
                                app.mark_dirty();
                                return false;
                            }
                            _ => {} // fall through to keybind resolver
                        }
                    }
                }
            }

            // Keybind resolution for modified keys and non-input-focus contexts.
            if let Some(action) = app.keybinds.resolve(&key) {
                return dispatch_action(action, app, input_box);
            }
        }

        Event::Resize(w, h) => {
            app.handle_resize(w, h);
        }

        // Mouse events: scroll and click-to-focus.
        Event::Mouse(me) if app.mouse_enabled => {
            use crossterm::event::{MouseButton, MouseEventKind};
            match me.kind {
                MouseEventKind::ScrollUp => {
                    if app.tab_bar.active == TabId::Chat {
                        app.scroll_up(3);
                    } else if let Some(panel) = app.active_panel_mut() {
                        match panel.focus {
                            crate::components::master_detail::PanelFocus::Master => {
                                panel.select_prev()
                            }
                            crate::components::master_detail::PanelFocus::Detail => {
                                panel.scroll_buffer_up(3)
                            }
                        }
                        app.mark_dirty();
                    }
                }
                MouseEventKind::ScrollDown => {
                    if app.tab_bar.active == TabId::Chat {
                        app.scroll_down(3);
                    } else {
                        let viewport_h = app.transcript_area.height.saturating_sub(1) as usize;
                        if let Some(panel) = app.active_panel_mut() {
                            match panel.focus {
                                crate::components::master_detail::PanelFocus::Master => {
                                    panel.select_next()
                                }
                                crate::components::master_detail::PanelFocus::Detail => {
                                    panel.scroll_buffer_down(3);
                                    panel.clamp_buffer_scroll(viewport_h);
                                }
                            }
                            app.mark_dirty();
                        }
                    }
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    // Check if click is in tab bar area.
                    let in_tab_bar = me.row >= app.tab_bar_area.y
                        && me.row < app.tab_bar_area.y + app.tab_bar_area.height
                        && me.column >= app.tab_bar_area.x
                        && me.column < app.tab_bar_area.x + app.tab_bar_area.width;

                    if in_tab_bar {
                        if let Some(tab) = app.tab_bar.hit_test(me.column, app.tab_bar_area.x) {
                            app.tab_bar.active = tab;
                            app.focus = if tab == TabId::Chat {
                                FocusTarget::Input
                            } else {
                                FocusTarget::Panel
                            };
                            app.mark_dirty();
                        }
                    } else if app.tab_bar.active != TabId::Chat {
                        // Click in panel content area — determine master vs detail.
                        let content_area = app.transcript_area;
                        if me.row >= content_area.y
                            && me.row < content_area.y + content_area.height
                            && me.column >= content_area.x
                            && me.column < content_area.x + content_area.width
                        {
                            app.focus = FocusTarget::Panel;
                            let list_width = (content_area.width * 30 / 100)
                                .max(15)
                                .min(content_area.width.saturating_sub(5));
                            let click_x = me.column.saturating_sub(content_area.x);
                            // The header row occupies row 0 of the content area;
                            // list items start at row 1.
                            let click_y = me.row.saturating_sub(content_area.y) as usize;
                            if let Some(panel) = app.active_panel_mut() {
                                if click_x <= list_width {
                                    panel.focus_master();
                                    // Each item takes 3 rows: label, detail, blank.
                                    // Row 0 is the pane header, so subtract 1 for content offset.
                                    if click_y > 0 {
                                        let item_row = click_y - 1; // relative to list content
                                        let idx = item_row / 3;
                                        panel.select_index(idx);
                                    }
                                } else {
                                    panel.focus_detail();
                                }
                            }
                            app.mark_dirty();
                        }
                    } else {
                        // Chat tab: check if click is in transcript area.
                        let in_transcript = me.row >= app.transcript_area.y
                            && me.row < app.transcript_area.y + app.transcript_area.height
                            && me.column >= app.transcript_area.x
                            && me.column < app.transcript_area.x + app.transcript_area.width;

                        if in_transcript && app.copy_mode.total_lines > 0 {
                            // Compute visual line from click position, using the
                            // same clamped offset the renderer uses.
                            let viewport_h = app.transcript_area.height as usize;
                            let max_off = app.copy_mode.total_lines.saturating_sub(viewport_h);
                            let effective_offset = app.scroll_offset.min(max_off);
                            let relative_row = (me.row - app.transcript_area.y) as usize;
                            let visual_line = (effective_offset + relative_row)
                                .min(app.copy_mode.total_lines.saturating_sub(1));
                            let col = me.column.saturating_sub(app.transcript_area.x) as usize;

                            // Enter selection mode phase 1 at clicked line and column.
                            app.copy_mode.enter(visual_line, col);
                            app.focus = FocusTarget::Transcript;
                        } else {
                            // Click outside transcript — focus input, exit copy mode.
                            if app.copy_mode.active {
                                app.copy_mode.exit();
                            }
                            let height = app.terminal_size.height;
                            if me.row >= height.saturating_sub(3) {
                                app.focus = FocusTarget::Input;
                            }
                        }
                        app.mark_dirty();
                    }
                }
                MouseEventKind::Drag(MouseButton::Left) => {
                    let in_transcript = me.row >= app.transcript_area.y
                        && me.row < app.transcript_area.y + app.transcript_area.height
                        && me.column >= app.transcript_area.x
                        && me.column < app.transcript_area.x + app.transcript_area.width;

                    if in_transcript && app.copy_mode.active && app.copy_mode.total_lines > 0 {
                        let viewport_h = app.transcript_area.height as usize;
                        let max_off = app.copy_mode.total_lines.saturating_sub(viewport_h);
                        let effective_offset = app.scroll_offset.min(max_off);
                        let relative_row = (me.row - app.transcript_area.y) as usize;
                        let visual_line = (effective_offset + relative_row)
                            .min(app.copy_mode.total_lines.saturating_sub(1));
                        let col = me.column.saturating_sub(app.transcript_area.x) as usize;

                        // Start Char visual selection if not already selecting.
                        if !app.copy_mode.selecting {
                            app.copy_mode.start_selecting_with_mode(
                                crate::overlays::copy_mode::VisualMode::Char,
                            );
                        }
                        // Move cursor to drag position.
                        app.copy_mode.cursor = crate::overlays::copy_mode::Position {
                            line: visual_line,
                            col,
                        };
                        app.mark_dirty();
                    }
                }
                MouseEventKind::Up(MouseButton::Left) => {
                    // On mouse release: if we were selecting via drag, auto-copy and exit.
                    if app.copy_mode.active && app.copy_mode.selecting {
                        let all_lines = crate::components::transcript::compute_all_lines(
                            &app.transcript,
                            &app.theme,
                            app.transcript_area.width,
                            false,
                        );
                        let text = crate::overlays::copy_mode::collect_selected_text(
                            &all_lines,
                            &app.copy_mode,
                        );
                        if !text.trim().is_empty() {
                            do_clipboard_copy(&text, app);
                        }
                        app.copy_mode.exit();
                        app.focus = FocusTarget::Input;
                        app.mark_dirty();
                    }
                }
                _ => {}
            }
        }

        // Paste events: detect file paths, images, or plain text.
        Event::Paste(text) => {
            if text.trim().is_empty() {
                // Empty paste text may indicate an image in the clipboard.
                // Try reading image data (blocking, but fast for local clipboard).
                if let Some(data) = crate::clipboard::read_clipboard_image() {
                    let _ = event_tx.send(TuiEvent::ImageDataReady {
                        label: "clipboard".to_owned(),
                        data,
                    });
                }
            } else {
                let trimmed = text.trim();
                let is_path = !trimmed.contains('\n')
                    && (trimmed.starts_with('/') || trimmed.starts_with("~/"))
                    && std::path::Path::new(trimmed).exists();

                if is_path {
                    let tag = format!("[file: {trimmed}]");
                    input_box.insert_str(&tag);
                } else {
                    input_box.insert_str(&text);
                }
            }
            app.mark_dirty();
        }

        // Other events (FocusGained, FocusLost, disabled mouse) are ignored.
        _ => {}
    }

    false
}

// ---------------------------------------------------------------------------
// Clipboard copy helper
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Copy-mode key handler (preset-aware)
// ---------------------------------------------------------------------------

/// Get the text and display width of a specific visual line in the transcript.
fn copy_mode_line_info(app: &AppState, line_idx: usize) -> (String, usize) {
    let lines = crate::components::transcript::compute_all_lines(
        &app.transcript,
        &app.theme,
        app.transcript_area.width,
        false,
    );
    if line_idx < lines.len() {
        let text = crate::overlays::copy_mode::line_to_text(&lines[line_idx]);
        let width: usize = lines[line_idx]
            .spans
            .iter()
            .map(|s| unicode_width::UnicodeWidthStr::width(s.content.as_ref()))
            .sum();
        (text, width)
    } else {
        (String::new(), 0)
    }
}

/// Adjust scroll_offset after cursor moved up.
fn copy_mode_scroll_up(app: &mut AppState) {
    if app.copy_mode.cursor.line < app.scroll_offset {
        app.scroll_offset = app.copy_mode.cursor.line;
    }
}

/// Adjust scroll_offset after cursor moved down.
fn copy_mode_scroll_down(app: &mut AppState) {
    let viewport_height = app.transcript_area.height as usize;
    if viewport_height > 0 && app.copy_mode.cursor.line >= app.scroll_offset + viewport_height {
        app.scroll_offset = app
            .copy_mode
            .cursor
            .line
            .saturating_sub(viewport_height - 1);
    }
}

/// Yank (copy) the current selection to clipboard and exit copy mode.
fn copy_mode_yank(app: &mut AppState) {
    if app.copy_mode.selecting {
        let all_lines = crate::components::transcript::compute_all_lines(
            &app.transcript,
            &app.theme,
            app.transcript_area.width,
            false,
        );
        let text = crate::overlays::copy_mode::collect_selected_text(&all_lines, &app.copy_mode);
        do_clipboard_copy(&text, app);
        app.copy_mode.exit();
        app.focus = FocusTarget::Input;
    }
    app.mark_dirty();
}

/// Ensure copy mode is in char-selection mode. If not selecting, starts it.
/// If already selecting in a different mode, switches to Char.
fn copy_mode_ensure_char_selecting(app: &mut AppState) {
    use crate::overlays::copy_mode::VisualMode;
    if !app.copy_mode.selecting {
        app.copy_mode.start_selecting_with_mode(VisualMode::Char);
    } else if app.copy_mode.mode != VisualMode::Char {
        app.copy_mode.mode = VisualMode::Char;
    }
}

/// Toggle a visual selection mode: start it, switch to it, or exit if already active.
fn copy_mode_toggle_visual(app: &mut AppState, mode: crate::overlays::copy_mode::VisualMode) {
    if !app.copy_mode.selecting {
        app.copy_mode.start_selecting_with_mode(mode);
    } else if app.copy_mode.mode == mode {
        app.copy_mode.exit_selecting();
    } else {
        app.copy_mode.mode = mode;
    }
    app.mark_dirty();
}

/// Handle a key event while copy mode is active. Dispatches to preset-specific
/// handlers after processing shared keys.
fn handle_copy_mode_key(key: &crossterm::event::KeyEvent, app: &mut AppState) {
    use crate::keybinds::KeybindPreset;
    use crate::overlays::copy_mode::VisualMode;
    use crossterm::event::{KeyCode, KeyModifiers};

    let preset = app.keybinds.preset;
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    // =======================================================================
    // Shared keys (all presets)
    // =======================================================================
    match key.code {
        // Esc: exit selecting (phase 2→1) or exit copy mode entirely.
        KeyCode::Esc => {
            if app.copy_mode.selecting {
                app.copy_mode.exit_selecting();
            } else {
                app.copy_mode.exit();
                app.focus = FocusTarget::Input;
            }
            app.mark_dirty();
            return;
        }
        // Arrow keys: universal navigation.
        KeyCode::Up if key.modifiers == KeyModifiers::NONE => {
            app.copy_mode.move_up();
            copy_mode_scroll_up(app);
            app.mark_dirty();
            return;
        }
        KeyCode::Down if key.modifiers == KeyModifiers::NONE => {
            app.copy_mode.move_down();
            copy_mode_scroll_down(app);
            app.mark_dirty();
            return;
        }
        KeyCode::Left if key.modifiers == KeyModifiers::NONE => {
            app.copy_mode.move_left();
            app.mark_dirty();
            return;
        }
        KeyCode::Right if key.modifiers == KeyModifiers::NONE => {
            app.copy_mode.move_right();
            app.mark_dirty();
            return;
        }
        // Shift+Arrow: start/extend char selection while moving (GUI-style).
        KeyCode::Up if key.modifiers == KeyModifiers::SHIFT => {
            copy_mode_ensure_char_selecting(app);
            app.copy_mode.move_up();
            copy_mode_scroll_up(app);
            app.mark_dirty();
            return;
        }
        KeyCode::Down if key.modifiers == KeyModifiers::SHIFT => {
            copy_mode_ensure_char_selecting(app);
            app.copy_mode.move_down();
            copy_mode_scroll_down(app);
            app.mark_dirty();
            return;
        }
        KeyCode::Left if key.modifiers == KeyModifiers::SHIFT => {
            copy_mode_ensure_char_selecting(app);
            app.copy_mode.move_left();
            app.mark_dirty();
            return;
        }
        KeyCode::Right if key.modifiers == KeyModifiers::SHIFT => {
            copy_mode_ensure_char_selecting(app);
            app.copy_mode.move_right();
            app.mark_dirty();
            return;
        }
        // Shift+Home/End: select to line start/end (all presets).
        KeyCode::Home if key.modifiers == KeyModifiers::SHIFT => {
            copy_mode_ensure_char_selecting(app);
            app.copy_mode.move_to_line_start();
            app.mark_dirty();
            return;
        }
        KeyCode::End if key.modifiers == KeyModifiers::SHIFT => {
            copy_mode_ensure_char_selecting(app);
            let (_, width) = copy_mode_line_info(app, app.copy_mode.cursor.line);
            app.copy_mode.move_to_line_end(width);
            app.mark_dirty();
            return;
        }
        // v/V/Ctrl+V: visual selection (works in all terminals, all presets).
        KeyCode::Char('v') if !ctrl => {
            copy_mode_toggle_visual(app, VisualMode::Char);
            return;
        }
        KeyCode::Char('V') => {
            copy_mode_toggle_visual(app, VisualMode::Line);
            return;
        }
        KeyCode::Char('v') if ctrl => {
            copy_mode_toggle_visual(app, VisualMode::Block);
            return;
        }
        // y: yank selection (all presets).
        KeyCode::Char('y') if !ctrl => {
            copy_mode_yank(app);
            return;
        }
        // Enter: yank selection (all presets).
        KeyCode::Enter => {
            copy_mode_yank(app);
            return;
        }
        // o: swap anchor/cursor (all presets).
        KeyCode::Char('o') if !ctrl => {
            if app.copy_mode.selecting {
                app.copy_mode.swap_anchor();
            }
            app.mark_dirty();
            return;
        }
        _ => {}
    }

    // =======================================================================
    // Preset-specific keys
    // =======================================================================
    match preset {
        KeybindPreset::Vim => handle_copy_mode_vim(key, app),
        KeybindPreset::Emacs => handle_copy_mode_emacs(key, app),
        KeybindPreset::Vscode => handle_copy_mode_vscode(key, app),
    }
}

/// Vim copy-mode keys: hjkl, w/b/e, 0/$, g/G.
fn handle_copy_mode_vim(key: &crossterm::event::KeyEvent, app: &mut AppState) {
    use crate::overlays::copy_mode::{self};
    use crossterm::event::KeyCode;

    match key.code {
        // --- Navigation ---
        KeyCode::Char('k') => {
            app.copy_mode.move_up();
            copy_mode_scroll_up(app);
            app.mark_dirty();
        }
        KeyCode::Char('j') => {
            app.copy_mode.move_down();
            copy_mode_scroll_down(app);
            app.mark_dirty();
        }
        KeyCode::Char('h') => {
            app.copy_mode.move_left();
            app.mark_dirty();
        }
        KeyCode::Char('l') => {
            app.copy_mode.move_right();
            app.mark_dirty();
        }
        // w: next word start
        KeyCode::Char('w') => {
            let (text, _) = copy_mode_line_info(app, app.copy_mode.cursor.line);
            let col = copy_mode::next_word_start(&text, app.copy_mode.cursor.col);
            app.copy_mode.move_to_col(col);
            app.mark_dirty();
        }
        // b: previous word start
        KeyCode::Char('b') => {
            let (text, _) = copy_mode_line_info(app, app.copy_mode.cursor.line);
            let col = copy_mode::prev_word_start(&text, app.copy_mode.cursor.col);
            app.copy_mode.move_to_col(col);
            app.mark_dirty();
        }
        // e: next word end
        KeyCode::Char('e') => {
            let (text, _) = copy_mode_line_info(app, app.copy_mode.cursor.line);
            let col = copy_mode::next_word_end(&text, app.copy_mode.cursor.col);
            app.copy_mode.move_to_col(col);
            app.mark_dirty();
        }
        // 0: line start
        KeyCode::Char('0') => {
            app.copy_mode.move_to_line_start();
            app.mark_dirty();
        }
        // $: line end
        KeyCode::Char('$') => {
            let (_, width) = copy_mode_line_info(app, app.copy_mode.cursor.line);
            app.copy_mode.move_to_line_end(width);
            app.mark_dirty();
        }
        // g: first line (simplified from gg)
        KeyCode::Char('g') => {
            app.copy_mode.move_to_first_line();
            copy_mode_scroll_up(app);
            app.mark_dirty();
        }
        // G: last line
        KeyCode::Char('G') => {
            app.copy_mode.move_to_last_line();
            copy_mode_scroll_down(app);
            app.mark_dirty();
        }

        _ => {}
    }
}

/// Emacs copy-mode keys: Ctrl+B/F/N/P, Alt+B/F, Ctrl+A/E, Ctrl+Space, Alt+W, Ctrl+G.
fn handle_copy_mode_emacs(key: &crossterm::event::KeyEvent, app: &mut AppState) {
    use crate::overlays::copy_mode::{self, VisualMode};
    use crossterm::event::{KeyCode, KeyModifiers};

    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);

    match key.code {
        // --- Navigation ---
        KeyCode::Char('p') if ctrl => {
            app.copy_mode.move_up();
            copy_mode_scroll_up(app);
            app.mark_dirty();
        }
        KeyCode::Char('n') if ctrl => {
            app.copy_mode.move_down();
            copy_mode_scroll_down(app);
            app.mark_dirty();
        }
        KeyCode::Char('b') if ctrl => {
            app.copy_mode.move_left();
            app.mark_dirty();
        }
        KeyCode::Char('f') if ctrl => {
            app.copy_mode.move_right();
            app.mark_dirty();
        }
        // Alt+B: word left
        KeyCode::Char('b') if alt => {
            let (text, _) = copy_mode_line_info(app, app.copy_mode.cursor.line);
            let col = copy_mode::prev_word_start(&text, app.copy_mode.cursor.col);
            app.copy_mode.move_to_col(col);
            app.mark_dirty();
        }
        // Alt+F: word right
        KeyCode::Char('f') if alt => {
            let (text, _) = copy_mode_line_info(app, app.copy_mode.cursor.line);
            let col = copy_mode::next_word_start(&text, app.copy_mode.cursor.col);
            app.copy_mode.move_to_col(col);
            app.mark_dirty();
        }
        // Ctrl+A: line start
        KeyCode::Char('a') if ctrl => {
            app.copy_mode.move_to_line_start();
            app.mark_dirty();
        }
        // Ctrl+E: line end
        KeyCode::Char('e') if ctrl => {
            let (_, width) = copy_mode_line_info(app, app.copy_mode.cursor.line);
            app.copy_mode.move_to_line_end(width);
            app.mark_dirty();
        }
        // Alt+<: first line
        KeyCode::Char('<') if alt => {
            app.copy_mode.move_to_first_line();
            copy_mode_scroll_up(app);
            app.mark_dirty();
        }
        // Alt+>: last line
        KeyCode::Char('>') if alt => {
            app.copy_mode.move_to_last_line();
            copy_mode_scroll_down(app);
            app.mark_dirty();
        }

        // --- Emacs-specific selection ---
        // Ctrl+Space: toggle char selection (set/unset mark) — also available via v
        KeyCode::Char(' ') if ctrl => {
            copy_mode_toggle_visual(app, VisualMode::Char);
        }
        // Ctrl+G: cancel (exit selecting or exit copy mode) — also available via Esc
        KeyCode::Char('g') if ctrl => {
            if app.copy_mode.selecting {
                app.copy_mode.exit_selecting();
            } else {
                app.copy_mode.exit();
                app.focus = FocusTarget::Input;
            }
            app.mark_dirty();
        }
        // Alt+W: copy selection (emacs kill-ring-save) — also available via y
        KeyCode::Char('w') if alt => {
            copy_mode_yank(app);
        }
        _ => {}
    }
}

/// VSCode copy-mode keys: Home/End, Ctrl+Left/Right, Ctrl+Home/End,
/// plus Shift variants for selection.
fn handle_copy_mode_vscode(key: &crossterm::event::KeyEvent, app: &mut AppState) {
    use crate::overlays::copy_mode::{self};
    use crossterm::event::{KeyCode, KeyModifiers};

    let mods = key.modifiers;
    let ctrl = mods.contains(KeyModifiers::CONTROL);
    let shift = mods.contains(KeyModifiers::SHIFT);

    match key.code {
        // --- Navigation (no shift) ---
        KeyCode::Home if !ctrl && !shift => {
            app.copy_mode.move_to_line_start();
            app.mark_dirty();
        }
        KeyCode::End if !ctrl && !shift => {
            let (_, width) = copy_mode_line_info(app, app.copy_mode.cursor.line);
            app.copy_mode.move_to_line_end(width);
            app.mark_dirty();
        }
        KeyCode::Home if ctrl && !shift => {
            app.copy_mode.move_to_first_line();
            copy_mode_scroll_up(app);
            app.mark_dirty();
        }
        KeyCode::End if ctrl && !shift => {
            app.copy_mode.move_to_last_line();
            copy_mode_scroll_down(app);
            app.mark_dirty();
        }
        KeyCode::Left if ctrl && !shift => {
            let (text, _) = copy_mode_line_info(app, app.copy_mode.cursor.line);
            let col = copy_mode::prev_word_start(&text, app.copy_mode.cursor.col);
            app.copy_mode.move_to_col(col);
            app.mark_dirty();
        }
        KeyCode::Right if ctrl && !shift => {
            let (text, _) = copy_mode_line_info(app, app.copy_mode.cursor.line);
            let col = copy_mode::next_word_start(&text, app.copy_mode.cursor.col);
            app.copy_mode.move_to_col(col);
            app.mark_dirty();
        }

        // --- Shift selection variants (vscode-specific) ---
        // Shift+Ctrl+Home: select to first line
        KeyCode::Home if ctrl && shift => {
            copy_mode_ensure_char_selecting(app);
            app.copy_mode.move_to_first_line();
            copy_mode_scroll_up(app);
            app.mark_dirty();
        }
        // Shift+Ctrl+End: select to last line
        KeyCode::End if ctrl && shift => {
            copy_mode_ensure_char_selecting(app);
            app.copy_mode.move_to_last_line();
            copy_mode_scroll_down(app);
            app.mark_dirty();
        }
        // Shift+Ctrl+Left: select word left
        KeyCode::Left if ctrl && shift => {
            copy_mode_ensure_char_selecting(app);
            let (text, _) = copy_mode_line_info(app, app.copy_mode.cursor.line);
            let col = copy_mode::prev_word_start(&text, app.copy_mode.cursor.col);
            app.copy_mode.move_to_col(col);
            app.mark_dirty();
        }
        // Shift+Ctrl+Right: select word right
        KeyCode::Right if ctrl && shift => {
            copy_mode_ensure_char_selecting(app);
            let (text, _) = copy_mode_line_info(app, app.copy_mode.cursor.line);
            let col = copy_mode::next_word_start(&text, app.copy_mode.cursor.col);
            app.copy_mode.move_to_col(col);
            app.mark_dirty();
        }

        _ => {}
    }
}

fn do_clipboard_copy(text: &str, app: &mut AppState) {
    let mut writer = std::io::stderr();
    match crate::clipboard::write_clipboard(
        text,
        crate::clipboard::ClipboardMethod::default(),
        &mut writer,
    ) {
        Ok(()) => {
            let line_count = text.lines().count();
            let label = if line_count == 1 { "line" } else { "lines" };
            app.toasts.push(
                crate::components::toast::ToastLevel::Success,
                format!("Copied {line_count} {label}"),
            );
        }
        Err(e) => {
            app.toasts.push(
                crate::components::toast::ToastLevel::Error,
                format!("Copy failed: {e}"),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Action dispatcher
// ---------------------------------------------------------------------------

/// Dispatch a resolved `Action`. Returns `true` when the loop should exit.
fn dispatch_action(action: Action, app: &mut AppState, input_box: &mut InputBoxState) -> bool {
    // Approximate viewport height for page-scroll calculations.
    let viewport_height = app.terminal_size.height.saturating_sub(4) as usize;
    let half_page = (viewport_height / 2).max(1);

    match action {
        Action::Exit => return true,

        Action::ScrollUp => app.scroll_up(1),
        Action::ScrollDown => app.scroll_down(1),
        Action::PageUp | Action::HalfPageUp => app.scroll_up(half_page),
        Action::PageDown | Action::HalfPageDown => app.scroll_down(half_page),

        Action::ScrollToTop => {
            app.scroll_offset = 0;
            app.auto_scroll = false;
            app.mark_dirty();
        }
        Action::ScrollToBottom => app.scroll_to_bottom(),

        Action::ToggleSidebar => {
            use crate::layout::{SIDEBAR_DEFAULT_WIDTH, SidebarMode};
            app.sidebar.mode = match app.sidebar.mode {
                SidebarMode::Full => SidebarMode::Hidden,
                SidebarMode::IconStrip => SidebarMode::Full,
                SidebarMode::Hidden => {
                    // Restore to the mode appropriate for the current terminal width.
                    app.terminal_size.sidebar_mode()
                }
            };
            // Ensure width is sane when restoring.
            if app.sidebar.width == 0 {
                app.sidebar.width = SIDEBAR_DEFAULT_WIDTH;
            }
            app.mark_dirty();
        }

        Action::GrowSidebar => {
            app.sidebar.grow();
            app.mark_dirty();
        }
        Action::ShrinkSidebar => {
            app.sidebar.shrink();
            app.mark_dirty();
        }

        Action::SendMessage => {
            let content = input_box.take_content();
            if !content.is_empty() {
                input_box.push_history(content.clone());
                if app.streaming {
                    // Queue the message — it will be sent when the current
                    // stream finishes (see TuiEvent::StreamDone handler).
                    app.message_queue.push(content);
                    app.mark_dirty();
                } else if content.starts_with('/') {
                    let parsed = parse_input(&content, &[]);
                    let slash_cmd = parsed.directives.iter().find_map(|d| {
                        if let Directive::SlashCommand { name, args, .. } = d {
                            Some((name.clone(), args.clone()))
                        } else {
                            None
                        }
                    });
                    if let Some((name, args)) = slash_cmd {
                        if let Some(action) = app.execute_command(&name, &args) {
                            return dispatch_action(action, app, input_box);
                        }
                    } else {
                        app.push_user_message(content, None, vec![]);
                        app.start_streaming();
                    }
                } else {
                    // Parse @mentions and file references.
                    let agent_names = app.agent_registry.enabled_names();
                    let agent_refs: Vec<&str> = agent_names.iter().map(|s| s.as_str()).collect();
                    let parsed = parse_input(&content, &agent_refs);

                    let mut target_agent: Option<String> = None;
                    let mut file_context: Vec<ucode_agent::FileContext> = Vec::new();
                    let mut display_parts: Vec<String> = Vec::new();

                    for directive in &parsed.directives {
                        match directive {
                            Directive::Mention { name, .. } => {
                                target_agent = Some(name.clone());
                            }
                            Directive::FileRef { path, .. } => {
                                if let Ok(file_content) = std::fs::read_to_string(path) {
                                    file_context.push(ucode_agent::FileContext {
                                        path: path.clone(),
                                        content: file_content,
                                    });
                                }
                            }
                            Directive::Text { content: text, .. } => {
                                display_parts.push(text.clone());
                            }
                            _ => {}
                        }
                    }

                    let display_text = display_parts.join("").trim().to_string();
                    let user_text = if display_text.is_empty() {
                        content
                    } else {
                        display_text
                    };

                    app.push_user_message(user_text, target_agent, file_context);
                    app.start_streaming();
                }
            }
        }

        Action::NewlineInInput => {
            input_box.insert_newline();
            app.mark_dirty();
        }

        Action::Dismiss => {
            if input_box.autocomplete.visible {
                input_box.autocomplete.hide();
            } else {
                input_box.clear();
            }
            app.mark_dirty();
        }

        Action::CancelGeneration => {
            let now = Instant::now();
            if let Some(last) = app.last_ctrl_c
                && now.duration_since(last).as_millis() < 2000
            {
                // Double Ctrl+C within 2 seconds -> exit.
                app.ctrl_c_hint = None;
                return true;
            }
            app.last_ctrl_c = Some(now);

            // If generating, cancel the active generation and finalize.
            if app.streaming {
                if let Some(tx) = &app.message_tx {
                    let _ = tx.send(ucode_agent::AgentMessage::Cancel);
                }
                app.finalize_streaming();
                // Discard all queued messages — the user explicitly cancelled.
                app.message_queue.clear();
                app.focus = FocusTarget::Input;
                app.ctrl_c_hint = None;
            } else {
                app.ctrl_c_hint = Some("Press Ctrl+C again to exit".into());
            }
            app.mark_dirty();
        }

        Action::EnterInsertMode => {
            app.keybinds.set_mode(InputMode::Insert);
            app.focus = FocusTarget::Input;
            app.mark_dirty();
        }

        Action::EnterNormalMode => {
            app.keybinds.set_mode(InputMode::Normal);
            app.mark_dirty();
        }

        Action::OpenPalette => {
            app.palette.open();
            app.focus = FocusTarget::Overlay;
            app.mark_dirty();
        }

        Action::ShowKeybindOverlay => {
            let resolver = app.keybinds.clone();
            app.keybind_overlay.open(&resolver);
            app.focus = FocusTarget::Overlay;
            app.mark_dirty();
        }

        Action::ShowDiff => {
            // Re-open the diff modal for the last PatchProposed in the transcript.
            if let Some(TranscriptEntry::PatchProposed {
                file_path,
                raw_diff,
                patch_id,
            }) = app
                .transcript
                .iter()
                .rev()
                .find(|e| matches!(e, TranscriptEntry::PatchProposed { .. }))
            {
                let fp = file_path.clone();
                let rd = raw_diff.clone();
                let pid = patch_id.clone();
                app.diff_modal.open(fp, &rd, pid);
                app.focus = FocusTarget::Overlay;
                app.mark_dirty();
            }
        }

        Action::ToggleTheme => {
            app.theme = app.theme.next_theme();
            app.mark_dirty();
        }

        Action::ToggleDensity => {
            app.density = app.density.next();
            app.mark_dirty();
        }

        Action::OpenConnect => {
            app.connect_modal.open(&std::collections::HashMap::new());
            app.focus = FocusTarget::Overlay;
            app.mark_dirty();
        }

        Action::OpenModels => {
            if app.providers.is_empty() {
                app.push_system_message("No providers connected. Use /connect first.".to_owned());
            } else {
                app.models_modal
                    .open(app.active_model.clone(), app.providers.len());
                app.focus = FocusTarget::Overlay;
                app.models_fetch_pending = true;
            }
            app.mark_dirty();
        }

        Action::OpenImagePopup => {
            if app.image_popup.visible {
                app.image_popup.close();
                app.focus = FocusTarget::Input;
            } else if app.image_popup.protocol.is_some() {
                // Re-show the last loaded image.
                app.image_popup.visible = true;
                app.focus = FocusTarget::Overlay;
            }
            app.mark_dirty();
        }

        Action::AcceptAutocomplete => {
            if let Some(entry) = input_box.autocomplete.selected_entry().cloned() {
                let is_mention = input_box.has_mention_prefix();
                input_box.content.clear();
                input_box.cursor_pos = 0;
                input_box.cursor_col = 0;
                if is_mention {
                    input_box.insert_char('@');
                }
                for c in entry.name.chars() {
                    input_box.insert_char(c);
                }
                // Trailing space so the user can immediately type arguments/message.
                input_box.insert_char(' ');
                input_box.autocomplete.hide();
            } else {
                // Tab with no autocomplete active: cycle through Primary agents.
                let cyclable = app.agent_registry.cyclable_agents();
                if cyclable.len() > 1 {
                    let current_idx = cyclable
                        .iter()
                        .position(|a| a.name == app.active_agent)
                        .unwrap_or(0);
                    let next_idx = (current_idx + 1) % cyclable.len();
                    app.active_agent = cyclable[next_idx].name.clone();
                }
            }
            app.mark_dirty();
        }

        Action::SearchTranscript => {
            app.search_overlay.open(app.keybinds.preset);
            app.focus = FocusTarget::Overlay;
            app.mark_dirty();
        }

        Action::NextSearchMatch => {
            app.search_overlay.next_match();
            if let Some(m) = app.search_overlay.current_match_info() {
                app.scroll_offset = m.transcript_index;
            }
            app.mark_dirty();
        }

        Action::PrevSearchMatch => {
            app.search_overlay.prev_match();
            if let Some(m) = app.search_overlay.current_match_info() {
                app.scroll_offset = m.transcript_index;
            }
            app.mark_dirty();
        }

        Action::EnterCopyMode | Action::EnterSelectionMode => {
            if app.copy_mode.total_lines > 0 {
                let cursor_line = app
                    .scroll_offset
                    .min(app.copy_mode.total_lines.saturating_sub(1));
                app.copy_mode.enter(cursor_line, 0);
                app.focus = FocusTarget::Transcript;
                app.mark_dirty();
            }
        }

        Action::SetMark => {
            if app.copy_mode.total_lines > 0 {
                let cursor_line = app
                    .scroll_offset
                    .min(app.copy_mode.total_lines.saturating_sub(1));
                app.copy_mode.enter(cursor_line, 0);
                app.focus = FocusTarget::Transcript;
                app.mark_dirty();
            }
        }

        Action::YankSelection | Action::CopySelection => {
            if app.copy_mode.active && app.copy_mode.selecting {
                let all_lines = crate::components::transcript::compute_all_lines(
                    &app.transcript,
                    &app.theme,
                    app.transcript_area.width,
                    false,
                );
                let text =
                    crate::overlays::copy_mode::collect_selected_text(&all_lines, &app.copy_mode);
                do_clipboard_copy(&text, app);
                app.copy_mode.exit();
                app.focus = FocusTarget::Input;
                app.mark_dirty();
            }
        }

        Action::OpenSessionSwitcher => {
            if let Some(ref store) = app.session_store
                && let Ok(sessions) = store.list(false)
            {
                app.session_picker
                    .open(sessions, app.current_session_id.as_deref());
                app.focus = FocusTarget::Overlay;
                app.mark_dirty();
            }
        }

        Action::SelectTab1 => {
            app.tab_bar.active = TabId::Chat;
            app.focus = FocusTarget::Input;
            app.mark_dirty();
        }
        Action::SelectTab2 => {
            app.tab_bar.active = TabId::Subagents;
            app.focus = FocusTarget::Panel;
            app.mark_dirty();
        }
        Action::SelectTab3 => {
            app.tab_bar.active = TabId::Tools;
            app.focus = FocusTarget::Panel;
            app.mark_dirty();
        }
        Action::SelectTab4 => {
            app.tab_bar.active = TabId::Mcp;
            app.focus = FocusTarget::Panel;
            app.mark_dirty();
        }
        Action::SelectTab5 => {
            app.tab_bar.active = TabId::Logs;
            app.focus = FocusTarget::Panel;
            app.mark_dirty();
        }
        Action::NextTab => {
            app.tab_bar.next();
            app.focus = if app.tab_bar.active == TabId::Chat {
                FocusTarget::Input
            } else {
                FocusTarget::Panel
            };
            app.mark_dirty();
        }
        Action::PrevTab => {
            app.tab_bar.prev();
            app.focus = if app.tab_bar.active == TabId::Chat {
                FocusTarget::Input
            } else {
                FocusTarget::Panel
            };
            app.mark_dirty();
        }

        // Future phases — no-op for now.
        _ => {}
    }

    false
}

// ---------------------------------------------------------------------------
// Verification ping
// ---------------------------------------------------------------------------

/// Verify that a stored credential actually works by making a lightweight API
/// call. Returns `Ok(())` on success or `Err(message)` on failure.
async fn verify_provider(
    provider: &str,
    cred_store: Option<std::sync::Arc<dyn ucode_auth::CredentialStore>>,
) -> Result<(), String> {
    // Use the provided store, or fall back to a bare KeyringStore if none is available
    // (e.g., during early startup before the store is wired into AppState).
    let fallback;
    let store: &dyn ucode_auth::CredentialStore = match cred_store.as_deref() {
        Some(s) => s,
        None => {
            fallback = ucode_auth::KeyringStore::new();
            &fallback
        }
    };
    let material = ucode_auth::CredentialStore::load(store, provider).map_err(|e| e.to_string())?;

    let client = reqwest::Client::new();

    match (provider, &material) {
        ("openai", ucode_auth::AuthMaterial::ApiKey { key }) => {
            let resp = client
                .get("https://api.openai.com/v1/models")
                .header("Authorization", format!("Bearer {key}"))
                .timeout(std::time::Duration::from_secs(10))
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if resp.status().is_success() {
                Ok(())
            } else {
                Err(format!("HTTP {}", resp.status()))
            }
        }
        ("openai", ucode_auth::AuthMaterial::OAuth { access_token, .. }) => {
            let resp = client
                .get("https://api.openai.com/v1/models")
                .header("Authorization", format!("Bearer {access_token}"))
                .timeout(std::time::Duration::from_secs(10))
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if resp.status().is_success() {
                Ok(())
            } else {
                Err(format!("HTTP {}", resp.status()))
            }
        }
        ("anthropic", ucode_auth::AuthMaterial::ApiKey { key }) => {
            let resp = client
                .get("https://api.anthropic.com/v1/models")
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01")
                .timeout(std::time::Duration::from_secs(10))
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if resp.status().is_success() {
                Ok(())
            } else {
                Err(format!("HTTP {}", resp.status()))
            }
        }
        ("anthropic", ucode_auth::AuthMaterial::OAuth { access_token, .. }) => {
            let resp = client
                .get("https://api.anthropic.com/v1/models")
                .header("Authorization", format!("Bearer {access_token}"))
                .header("anthropic-version", "2023-06-01")
                .timeout(std::time::Duration::from_secs(10))
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if resp.status().is_success() {
                Ok(())
            } else {
                Err(format!("HTTP {}", resp.status()))
            }
        }
        _ => {
            // Unknown provider — skip verification and report success.
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// TuiEvent handler
// ---------------------------------------------------------------------------

/// Handle an event from the external channel. Returns `true` to exit the loop.
fn handle_tui_event(
    event: TuiEvent,
    app: &mut AppState,
    sidebar_data: &mut SidebarData,
    event_tx: &UnboundedSender<TuiEvent>,
) -> bool {
    match event {
        TuiEvent::StreamToken(ref token) => {
            app.push_token(token);
            sidebar_data.context.tokens_used += 1;
        }
        TuiEvent::StreamDone => {
            app.finalize_streaming();
            // Restore focus to the input box so the user can immediately
            // type the next message — matches OpenCode behaviour.
            if app.focus != FocusTarget::Overlay {
                app.focus = FocusTarget::Input;
            }
            // Auto-send the next queued message (FIFO).
            if !app.message_queue.is_empty() {
                let next = app.message_queue.remove(0);
                app.push_user_message(next, None, vec![]);
                app.start_streaming();
            }
        }
        TuiEvent::ToolCallStarted { ref name } => {
            app.push_tool_call(name.clone());
            sidebar_data
                .tools
                .entries
                .push(crate::components::sidebar::sections::ToolEntry {
                    name: name.clone(),
                    status: ToolCallStatus::Running,
                    duration: None,
                });
        }
        TuiEvent::ToolCallCompleted {
            index,
            status,
            duration_ms,
            summary,
            thinking,
            output,
        } => {
            app.update_tool_call(
                index,
                status.clone(),
                duration_ms,
                summary,
                thinking,
                output,
            );
            // Update sidebar tool call status.
            if let Some(entry) = sidebar_data.tools.entries.get_mut(index) {
                entry.status = status;
                if let Some(ms) = duration_ms {
                    entry.duration = Some(format!("{ms}ms"));
                }
            }
        }
        TuiEvent::RouterEvent(msg) => app.push_router_event(msg),
        TuiEvent::SystemMessage(ref msg) => {
            // Parse provider/model info from system messages.
            if let Some(rest) = msg.strip_prefix("provider=") {
                let parts: Vec<&str> = rest.splitn(2, " model=").collect();
                if parts.len() == 2 {
                    if let Some(chain) = sidebar_data.router.fallback_chain.first_mut() {
                        *chain = parts[0].to_string();
                    } else {
                        sidebar_data
                            .router
                            .fallback_chain
                            .push(parts[0].to_string());
                    }
                    sidebar_data.router.model_name = parts[1].to_string();
                }
            }
            app.push_system_message(msg.clone());
        }
        TuiEvent::PatchProposed {
            file_path,
            raw_diff,
            patch_id,
        } => {
            app.propose_patch(file_path, raw_diff, patch_id);
        }
        TuiEvent::ApprovalRequired {
            tool_call_id,
            tool_name,
            command,
            cwd,
            sandbox_label,
        } => {
            app.request_approval(tool_name, command, cwd, sandbox_label);
            // Store the tool_call_id so the approval modal can send back the decision.
            app.pending_tool_call_id = tool_call_id;
        }
        TuiEvent::Toast { level, title, body } => {
            if let Some(body) = body {
                app.toast_with_body(level, title, body);
            } else {
                app.toast(level, title);
            }
        }
        TuiEvent::CheckpointCreated { name } => {
            app.toast(ToastLevel::Info, format!("Checkpoint created: {name}"));
        }
        TuiEvent::BudgetWarning {
            used_pct: _,
            message,
        } => {
            app.toast_with_body(ToastLevel::Warning, "Budget warning", message);
        }
        TuiEvent::AgentCompleted { agent_id: _, name } => {
            app.toast(ToastLevel::Success, format!("Agent completed: {name}"));
        }
        TuiEvent::AgentFailed {
            agent_id: _,
            name,
            error,
        } => {
            app.toast_with_body(ToastLevel::Error, format!("Agent failed: {name}"), error);
        }
        TuiEvent::McpServerCrashed { server_name, error } => {
            app.toast_with_body(
                ToastLevel::Error,
                format!("MCP server crashed: {server_name}"),
                error,
            );
        }
        TuiEvent::AuthExpired { provider } => {
            app.toast(ToastLevel::Warning, format!("Auth expired: {provider}"));
        }
        TuiEvent::AuthCompleted { provider } => {
            let display = provider.clone();
            app.connect_modal.phase = ConnectPhase::Verifying {
                provider_id: provider.clone(),
                display_name: display,
            };
            app.mark_dirty();

            let provider_clone = provider.clone();
            let tx = event_tx.clone();
            let cred = app.credential_store.clone();
            tokio::spawn(async move {
                let result = verify_provider(&provider_clone, cred).await;
                let _ = tx.send(TuiEvent::VerifyResult {
                    provider: provider_clone,
                    success: result.is_ok(),
                    message: result.err(),
                });
            });
        }
        TuiEvent::AuthFailed { provider, error } => {
            app.connect_modal.close();
            app.focus = FocusTarget::Input;
            app.toast(
                crate::components::toast::ToastLevel::Error,
                format!("{provider} auth failed: {error}"),
            );
        }
        TuiEvent::VerifyResult {
            provider,
            success,
            message,
        } => {
            app.connect_modal.close();
            app.focus = FocusTarget::Input;
            if success {
                app.toast(
                    crate::components::toast::ToastLevel::Info,
                    format!("{provider} connected"),
                );
            } else {
                let msg = message.unwrap_or_default();
                app.toast_with_body(
                    crate::components::toast::ToastLevel::Warning,
                    format!("{provider} connected"),
                    format!("Verification failed: {msg}"),
                );
            }
        }
        TuiEvent::DeviceCodeReady {
            provider,
            user_code,
            verification_uri,
        } => {
            // Best-effort: open the verification URL in the user's browser.
            try_open_url(&verification_uri);

            if let ConnectPhase::DeviceCode {
                provider_id,
                display_name,
                ..
            } = &app.connect_modal.phase
                && *provider_id == provider
            {
                app.connect_modal.phase = ConnectPhase::DeviceCode {
                    provider_id: provider_id.clone(),
                    display_name: display_name.clone(),
                    user_code,
                    verification_uri,
                };
                app.mark_dirty();
            }
        }
        TuiEvent::ModelsListed { provider, models } => {
            app.models_modal.add_models(&provider, &models);
            app.mark_dirty();
        }
        TuiEvent::ModelsListFailed { error } => {
            app.models_modal.add_error(&error);
            app.push_system_message(format!("Failed to list models: {error}"));
            app.mark_dirty();
        }
        TuiEvent::ImageDataReady { label, data } => {
            if let Some(ref mut picker) = app.image_picker {
                match app.image_popup.open(picker, label, &data) {
                    Ok(()) => {
                        app.focus = FocusTarget::Overlay;
                    }
                    Err(e) => {
                        app.toasts.push_with_body(
                            crate::components::toast::ToastLevel::Error,
                            "Image Error",
                            e,
                        );
                    }
                }
            } else {
                app.toasts.push(
                    crate::components::toast::ToastLevel::Warning,
                    "No image protocol available",
                );
            }
            app.mark_dirty();
        }
        TuiEvent::Quit => return true,
    }
    false
}

// ---------------------------------------------------------------------------
// render_frame
// ---------------------------------------------------------------------------

/// Render the full TUI into the current frame.
///
/// `frame_counter` drives cursor blink: the streaming cursor is visible when
/// `(frame_counter / BLINK_FRAMES) % 2 == 0`.
pub fn render_frame(
    f: &mut ratatui::Frame,
    app: &mut AppState,
    input_box: &InputBoxState,
    sidebar_data: &SidebarData,
    frame_counter: u64,
) {
    let area = f.area();
    let show_input = app.tab_bar.active == crate::components::tab_bar::TabId::Chat;
    let areas = compute_layout(area, &app.sidebar, &app.input, show_input);

    app.transcript_area = areas.content;
    app.tab_bar_area = areas.tab_bar;

    // Update total_lines for copy mode navigation bounds.
    use crate::components::transcript::entry_height;
    app.copy_mode.total_lines = app
        .transcript
        .iter()
        .map(|e| entry_height(e, areas.content.width))
        .sum::<usize>();

    // Normalize scroll_offset so event handlers (mouse click, Ctrl+Y, etc.)
    // always see a value consistent with the renderer's clamped offset.
    // Without this, the raw scroll_offset (accumulated token count or
    // usize::MAX/2 sentinel from auto-scroll) causes mouse clicks and
    // copy-mode entry to always map to the last visual line.
    let viewport_h = areas.content.height as usize;
    let max_offset = app.copy_mode.total_lines.saturating_sub(viewport_h);
    if app.scroll_offset > max_offset {
        app.scroll_offset = max_offset;
    }

    // Cursor blink: toggle every ~500ms (500 / 16 ≈ 31 frames).
    const BLINK_FRAMES: u64 = 31;
    let show_cursor = (frame_counter / BLINK_FRAMES).is_multiple_of(2);

    // Tab bar.
    f.render_widget(TabBar::new(&app.tab_bar, &app.theme), areas.tab_bar);

    // Content area — depends on active tab.
    match app.tab_bar.active {
        TabId::Chat => {
            let transcript_widget = TranscriptView::new(
                &app.transcript,
                app.scroll_offset,
                app.auto_scroll,
                &app.theme,
                show_cursor,
                &app.copy_mode,
            );
            f.render_widget(transcript_widget, areas.content);
        }
        TabId::Subagents => {
            let widget = MasterDetail::new(&app.subagents_panel, &app.theme)
                .empty_message("No subagent runs in this session");
            f.render_widget(widget, areas.content);
        }
        TabId::Tools => {
            let widget = MasterDetail::new(&app.tools_panel, &app.theme)
                .empty_message("No tool calls in this session")
                .show_filter(true);
            f.render_widget(widget, areas.content);
        }
        TabId::Mcp => {
            let widget = MasterDetail::new(&app.mcp_panel, &app.theme)
                .empty_message("No MCP servers connected");
            f.render_widget(widget, areas.content);
        }
        TabId::Logs => {
            let widget = MasterDetail::new(&app.logs_panel, &app.theme)
                .empty_message("No events logged")
                .show_filter(true);
            f.render_widget(widget, areas.content);
        }
    }

    // Transcript scrollbar — only show on Chat tab when content overflows viewport.
    if app.tab_bar.active == TabId::Chat {
        let total = app.copy_mode.total_lines;
        let viewport = areas.content.height as usize;
        if total > viewport {
            use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState};
            let mut scrollbar_state = ScrollbarState::default()
                .content_length(total)
                .viewport_content_length(viewport)
                .position(app.scroll_offset);
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .thumb_style(app.theme.accent_style())
                .track_style(app.theme.dim_style());
            f.render_stateful_widget(scrollbar, areas.content, &mut scrollbar_state);
        }
    }

    // Sidebar — only on Chat tab.
    if show_input {
        use crate::components::sidebar::Sidebar;
        f.render_widget(
            Sidebar::new(sidebar_data, &app.theme, app.sidebar.mode),
            areas.sidebar,
        );
    }

    // Queued-message indicators (stacked above the input box).
    // Each queued message takes 2 rows: badge line + message line.
    // Layout matches OpenCode: QUEUED badge on its own line above the message.
    // These overlay the bottom of the transcript area.
    if show_input && !app.message_queue.is_empty() {
        use ratatui::style::{Modifier, Style};
        use ratatui::text::{Line, Span};

        let rows_per_msg: u16 = 2;
        let total_rows = (app.message_queue.len() as u16) * rows_per_msg;
        // Overlay the bottom of the content area; cap at half the content height.
        let max_rows = areas.content.height / 2;
        let rows_to_render = total_rows.min(max_rows);
        let msgs_to_render = (rows_to_render / rows_per_msg) as usize;

        // Render from top (oldest) to bottom (newest), stacking upward from input.
        let start_idx = app.message_queue.len().saturating_sub(msgs_to_render);
        for (i, msg) in app.message_queue[start_idx..].iter().enumerate() {
            let slot_y = areas.input.y.saturating_sub(rows_to_render) + (i as u16) * rows_per_msg;

            // Row 1: badge
            let badge_area = ratatui::layout::Rect {
                x: areas.input.x + 1,
                y: slot_y,
                width: areas.input.width.saturating_sub(2),
                height: 1,
            };
            if badge_area.y < areas.input.y && badge_area.height > 0 {
                let badge_line = Line::from(vec![Span::styled(
                    " QUEUED ",
                    Style::default()
                        .fg(app.theme.background)
                        .bg(app.theme.accent)
                        .add_modifier(Modifier::BOLD),
                )]);
                f.render_widget(ratatui::widgets::Clear, badge_area);
                f.render_widget(badge_line, badge_area);
            }

            // Row 2: message text
            let msg_area = ratatui::layout::Rect {
                x: areas.input.x + 1,
                y: slot_y + 1,
                width: areas.input.width.saturating_sub(2),
                height: 1,
            };
            if msg_area.y < areas.input.y && msg_area.height > 0 {
                let max_width = msg_area.width as usize;
                let display_msg = if msg.len() > max_width {
                    format!("{}...", &msg[..max_width.saturating_sub(3)])
                } else {
                    msg.clone()
                };
                let msg_line = Line::from(vec![Span::styled(
                    display_msg,
                    Style::default().fg(app.theme.text_dim),
                )]);
                f.render_widget(ratatui::widgets::Clear, msg_area);
                f.render_widget(msg_line, msg_area);
            }
        }
    }

    // Input box — only on Chat tab.
    if show_input {
        use crate::components::input::InputBox;
        f.render_widget(InputBox::new(input_box, &app.theme, true), areas.input);
    }

    // Input box scrollbar — only show when content exceeds visible lines.
    if show_input {
        let total_lines = input_box.line_count();
        let visible_lines = app.input.line_count as usize;
        if total_lines > visible_lines {
            use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState};
            let mut scrollbar_state = ScrollbarState::default()
                .content_length(total_lines)
                .viewport_content_length(visible_lines)
                .position(input_box.scroll_offset);
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .thumb_style(app.theme.accent_style())
                .track_style(app.theme.dim_style());
            f.render_stateful_widget(scrollbar, areas.input, &mut scrollbar_state);
        }
    }

    // Autocomplete dropdown (rendered above the input box).
    if show_input && input_box.autocomplete.visible && !input_box.autocomplete.entries.is_empty() {
        use crate::components::input::AutocompleteDropdown;
        use ratatui::widgets::Clear;

        let max_rows = 8u16;
        let entry_count = input_box.autocomplete.entries.len() as u16;
        let dropdown_height = entry_count.min(max_rows);

        let dropdown_area = ratatui::layout::Rect {
            x: areas.input.x + 1,
            y: areas.input.y.saturating_sub(dropdown_height),
            width: areas.input.width.saturating_sub(2),
            height: dropdown_height,
        };

        // Only render if there's space above the input and the area is valid.
        if dropdown_area.y < areas.input.y && dropdown_area.height > 0 {
            f.render_widget(Clear, dropdown_area);
            f.render_widget(
                AutocompleteDropdown::new(&input_box.autocomplete, &app.theme),
                dropdown_area,
            );
        }
    }

    // Status bar.
    let status_state = build_status_bar_state(app, sidebar_data);
    f.render_widget(StatusBar::new(&status_state, &app.theme), areas.status_bar);

    // Palette overlay (rendered last so it's on top).
    if app.palette.visible {
        use crate::overlays::palette::PaletteOverlay;
        f.render_widget(PaletteOverlay::new(&app.palette, &app.theme), area);
    }

    // Diff modal overlay (rendered last so it's on top of everything).
    if app.diff_modal.visible {
        use crate::overlays::diff_modal::DiffModal;
        f.render_widget(DiffModal::new(&app.diff_modal, &app.theme), area);
    }

    // Approval modal overlay.
    if app.approval_modal.visible {
        use crate::overlays::approval_modal::ApprovalModal;
        f.render_widget(ApprovalModal::new(&app.approval_modal, &app.theme), area);
    }

    // Connect modal overlay.
    if app.connect_modal.visible {
        use crate::overlays::connect_modal::ConnectModal;
        f.render_widget(ConnectModal::new(&app.connect_modal, &app.theme), area);
    }

    // Models modal overlay.
    if app.models_modal.visible {
        use crate::overlays::models_modal::ModelsModal;
        f.render_widget(ModelsModal::new(&app.models_modal, &app.theme), area);
    }

    // Session picker overlay.
    if app.session_picker.visible {
        use crate::overlays::session_picker::SessionPicker;
        f.render_widget(SessionPicker::new(&app.session_picker, &app.theme), area);
    }

    // Image popup overlay (rendered with mutable state for StatefulProtocol).
    if app.image_popup.visible {
        use crate::overlays::image_popup::ImagePopup;
        ImagePopup::new(&mut app.image_popup, &app.theme).render(area, f.buffer_mut());
    }

    // Keybind reference overlay.
    if app.keybind_overlay.visible {
        use crate::overlays::keybind_overlay::KeybindOverlay;
        f.render_widget(KeybindOverlay::new(&app.keybind_overlay, &app.theme), area);
    }

    // Search overlay bar (1 row at the top of the content area).
    if app.search_overlay.visible {
        use crate::overlays::search_overlay::SearchOverlay;
        let search_area = Rect {
            height: 1,
            ..areas.content
        };
        f.render_widget(
            SearchOverlay::new(&app.search_overlay, &app.theme),
            search_area,
        );
    }

    // Toast notifications (rendered last, on top of everything).
    if !app.toasts.is_empty() {
        use crate::components::toast::ToastStack;
        f.render_widget(
            ToastStack {
                state: &app.toasts,
                theme: &app.theme,
            },
            area,
        );
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Spawn async tasks to list models from all connected providers.
/// Each provider sends its results back via `TuiEvent::ModelsListed` or
/// `TuiEvent::ModelsListFailed`.
fn spawn_models_fetch(app: &AppState, event_tx: &UnboundedSender<TuiEvent>) {
    let providers: Vec<(String, ucode_providers::ProviderConfig)> = app
        .providers
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let cred = app.credential_store.clone();
    let tx = event_tx.clone();
    tokio::spawn(async move {
        for (name, config) in providers {
            match ucode_providers::create_provider(&name, &config, cred.clone()) {
                Ok(provider) => match provider.list_models().await {
                    Ok(models) => {
                        let _ = tx.send(TuiEvent::ModelsListed {
                            provider: name,
                            models,
                        });
                    }
                    Err(e) => {
                        let _ = tx.send(TuiEvent::ModelsListFailed {
                            error: format!("{name}: {e}"),
                        });
                    }
                },
                Err(e) => {
                    let _ = tx.send(TuiEvent::ModelsListFailed {
                        error: format!("{name}: failed to create provider: {e}"),
                    });
                }
            }
        }
    });
}

/// Attempt to spawn the agent loop after a successful `/connect`.
/// Maps the provider_id to an `AdapterKind` and `ProviderConfig`, then spawns.
fn try_spawn_agent_after_connect(
    provider_id: &str,
    setup: crate::PendingAgentSetup,
    event_tx: &UnboundedSender<TuiEvent>,
    app: &mut AppState,
) {
    use ucode_providers::config::{AdapterKind, ProviderConfig};

    let (adapter, default_model) = match provider_id {
        "github-copilot" => (AdapterKind::Copilot, "gpt-4o"),
        "anthropic" => (AdapterKind::Anthropic, "claude-sonnet-4-20250514"),
        "openai" => (AdapterKind::Openai, "gpt-4o"),
        _ => {
            tracing::warn!("unknown provider for agent spawn: {provider_id}");
            return;
        }
    };

    let provider_config = ProviderConfig {
        adapter,
        base_url: None,
        api_key_env: None,
        headers: std::collections::HashMap::new(),
    };

    // Store provider info for /models.
    app.providers
        .insert(provider_id.to_owned(), provider_config.clone());
    app.credential_store = Some(setup.credential_store.clone());

    let loop_config = ucode_agent::AgentLoopConfig {
        provider_name: provider_id.to_owned(),
        provider_config,
        model: default_model.to_owned(),
        credential_store: Some(setup.credential_store),
    };

    let ac = crate::AgentConfig {
        loop_config,
        session_store: setup.session_store,
        session: setup.session,
        tool_registry: setup.tool_registry,
        // Mid-session connect: the providers map already has entries from
        // startup; the new provider was inserted above.
        all_providers: std::collections::HashMap::new(),
    };

    let (msg_tx, _agent_handle, _bridge_handle) = crate::spawn_agent_loop(ac, event_tx);
    app.message_tx = Some(msg_tx);
    app.active_model = Some(default_model.to_owned());

    tracing::info!("agent loop spawned after /connect for {provider_id}");
}

/// Best-effort attempt to open a URL in the user's default browser.
/// Silently ignores any failure (missing opener, headless environment, etc.).
fn try_open_url(url: &str) {
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(not(target_os = "macos"))]
    let cmd = "xdg-open";

    let _ = std::process::Command::new(cmd)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// Build a `StatusBarState` from the current app and sidebar state.
///
/// Also responsible for expiring the Ctrl+C hint: if `last_ctrl_c` is older
/// than 2 seconds the hint is omitted (the caller holds `&AppState` so we
/// cannot mutate it here; expiry mutation happens in `render_frame`).
fn build_status_bar_state(app: &AppState, _sidebar_data: &SidebarData) -> StatusBarState {
    let stream_tok_per_sec = if app.streaming {
        app.transcript.last().and_then(|e| {
            if let crate::app::TranscriptEntry::Streaming(msg) = e {
                let tps = msg.tokens_per_sec();
                if tps > 0.0 { Some(tps) } else { None }
            } else {
                None
            }
        })
    } else {
        None
    };

    let hint_message = app.ctrl_c_hint.clone().filter(|_| {
        app.last_ctrl_c
            .is_some_and(|t| t.elapsed().as_millis() < 2000)
    });

    StatusBarState {
        streaming: app.streaming,
        stream_tok_per_sec,
        hint_message,
        copy_mode_label: app.copy_mode.status_label().map(String::from),
        ..StatusBarState::default()
    }
}

/// Format a Unix timestamp (seconds) as "YYYY-MM-DD".
///
/// This is a minimal implementation that avoids pulling in `chrono` just for
/// date formatting. It handles leap years correctly for dates after 1970.
#[allow(dead_code)]
fn format_date_from_secs(secs: u64) -> String {
    // Days since Unix epoch.
    let days = secs / 86_400;

    // Gregorian calendar computation.
    let mut year = 1970u32;
    let mut remaining = days;

    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        year += 1;
    }

    let month_days: [u64; 12] = [
        31,
        if is_leap(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];

    let mut month = 1u32;
    for &md in &month_days {
        if remaining < md {
            break;
        }
        remaining -= md;
        month += 1;
    }

    let day = remaining + 1;
    format!("{year:04}-{month:02}-{day:02}")
}

#[allow(dead_code)]
fn is_leap(year: u32) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

// ---------------------------------------------------------------------------
// Approval decision helper
// ---------------------------------------------------------------------------

/// Send an approval decision back to the agent loop, if a tool_call_id is pending.
fn send_approval_decision(app: &mut AppState, approved: bool) {
    if let (Some(tool_call_id), Some(tx)) =
        (app.pending_tool_call_id.take(), app.message_tx.as_ref())
    {
        let _ = tx.send(ucode_agent::AgentMessage::ApprovalDecision {
            tool_call_id,
            approved,
        });
    }
}

// ---------------------------------------------------------------------------
// Agent event bridge
// ---------------------------------------------------------------------------

/// Convert an [`ucode_agent::AgentEvent`] to a [`TuiEvent`] and send it
/// through the TUI channel.
pub fn bridge_agent_event(
    event: ucode_agent::AgentEvent,
    tx: &tokio::sync::mpsc::UnboundedSender<TuiEvent>,
) {
    use ucode_agent::AgentEvent;
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
            let status = if success {
                crate::app::ToolCallStatus::Success
            } else {
                crate::app::ToolCallStatus::Failed
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
        AgentEvent::ApprovalRequired {
            tool_call_id,
            tool_name,
            command,
            cwd,
            sandbox_label,
        } => TuiEvent::ApprovalRequired {
            tool_call_id: Some(tool_call_id),
            tool_name,
            command,
            cwd,
            sandbox_label,
        },
        AgentEvent::SystemMessage(msg) => TuiEvent::SystemMessage(msg),
        AgentEvent::Error(msg) => TuiEvent::Toast {
            level: crate::components::toast::ToastLevel::Error,
            title: "Agent Error".into(),
            body: Some(msg),
        },
    };
    let _ = tx.send(tui_event);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    /// Thin wrapper used by tests: creates a throwaway channel and auth_task
    /// so tests don't need to thread those through every call site.
    fn term_event(
        event: Event,
        app: &mut AppState,
        input_box: &mut InputBoxState,
        sidebar_data: &mut SidebarData,
    ) -> bool {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel::<TuiEvent>();
        let mut auth_task: Option<JoinHandle<()>> = None;
        handle_terminal_event(event, app, input_box, sidebar_data, &tx, &mut auth_task)
    }

    /// Thin wrapper used by tests: creates a throwaway channel and sidebar_data
    /// so tests don't need to thread them through every `handle_tui_event` call site.
    fn tui_event(event: TuiEvent, app: &mut AppState) -> bool {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel::<TuiEvent>();
        let mut sidebar_data = SidebarData::new();
        handle_tui_event(event, app, &mut sidebar_data, &tx)
    }

    // -----------------------------------------------------------------------
    // TuiEvent variants
    // -----------------------------------------------------------------------

    #[test]
    fn tui_event_variants() {
        // Verify all variants can be constructed without panicking.
        let _ = TuiEvent::StreamToken("hello".to_owned());
        let _ = TuiEvent::StreamDone;
        let _ = TuiEvent::ToolCallStarted {
            name: "read_file".to_owned(),
        };
        let _ = TuiEvent::ToolCallCompleted {
            index: 0,
            status: ToolCallStatus::Success,
            duration_ms: Some(42),
            summary: Some("done".to_owned()),
            thinking: None,
            output: None,
        };
        let _ = TuiEvent::RouterEvent("rerouted".to_owned());
        let _ = TuiEvent::SystemMessage("info".to_owned());
        let _ = TuiEvent::PatchProposed {
            file_path: "src/lib.rs".to_owned(),
            raw_diff: "+added".to_owned(),
            patch_id: None,
        };
        let _ = TuiEvent::ApprovalRequired {
            tool_call_id: None,
            tool_name: "run_cmd".to_owned(),
            command: "cargo test".to_owned(),
            cwd: "/tmp".to_owned(),
            sandbox_label: "ws".to_owned(),
        };
        let _ = TuiEvent::CheckpointCreated {
            name: "v1.0".to_owned(),
        };
        let _ = TuiEvent::BudgetWarning {
            used_pct: 75.0,
            message: "75% used".to_owned(),
        };
        let _ = TuiEvent::AgentCompleted {
            agent_id: "a1".to_owned(),
            name: "reviewer".to_owned(),
        };
        let _ = TuiEvent::AgentFailed {
            agent_id: "a1".to_owned(),
            name: "reviewer".to_owned(),
            error: "timeout".to_owned(),
        };
        let _ = TuiEvent::McpServerCrashed {
            server_name: "filesystem".to_owned(),
            error: "segfault".to_owned(),
        };
        let _ = TuiEvent::AuthExpired {
            provider: "github".to_owned(),
        };
        let _ = TuiEvent::Quit;
    }

    #[test]
    fn tui_event_approval_required() {
        let mut app = AppState::new();
        let exited = tui_event(
            TuiEvent::ApprovalRequired {
                tool_call_id: Some("tc-1".to_owned()),
                tool_name: "run_cmd".to_owned(),
                command: "cargo test".to_owned(),
                cwd: "/tmp".to_owned(),
                sandbox_label: "ws".to_owned(),
            },
            &mut app,
        );
        assert!(!exited);
        assert!(app.approval_modal.visible);
        assert_eq!(app.focus, FocusTarget::Overlay);
    }

    // -----------------------------------------------------------------------
    // render_frame does not panic
    // -----------------------------------------------------------------------

    #[test]
    fn render_frame_does_not_panic() {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = AppState::new();
        let input_box = InputBoxState::new();
        let sidebar_data = SidebarData::new();

        terminal
            .draw(|f| render_frame(f, &mut app, &input_box, &sidebar_data, 0))
            .expect("draw");
    }

    // -----------------------------------------------------------------------
    // handle_key_exit
    // -----------------------------------------------------------------------

    #[test]
    fn handle_key_exit() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        // Simulate the Exit action directly via dispatch_action.
        let exited = dispatch_action(Action::Exit, &mut app, &mut input_box);
        assert!(exited, "Exit action should signal loop termination");

        // Non-exit action should not terminate.
        let _ = &mut sidebar_data; // suppress unused warning
        let not_exited = dispatch_action(Action::ScrollUp, &mut app, &mut input_box);
        assert!(!not_exited, "ScrollUp should not terminate the loop");
    }

    // -----------------------------------------------------------------------
    // handle_resize
    // -----------------------------------------------------------------------

    #[test]
    fn handle_resize() {
        let mut app = AppState::new();
        app.handle_resize(160, 50);
        assert_eq!(app.terminal_size.width, 160);
        assert_eq!(app.terminal_size.height, 50);
        assert!(app.dirty);
    }

    // -----------------------------------------------------------------------
    // handle_stream_token
    // -----------------------------------------------------------------------

    #[test]
    fn handle_stream_token() {
        let mut app = AppState::new();
        app.start_streaming();

        let exited = tui_event(TuiEvent::StreamToken("hello".to_owned()), &mut app);
        assert!(!exited);

        match app.transcript.last() {
            Some(crate::app::TranscriptEntry::Streaming(msg)) => {
                assert!(msg.content.contains("hello"));
            }
            other => panic!("expected Streaming entry, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // handle_stream_done
    // -----------------------------------------------------------------------

    #[test]
    fn handle_stream_done() {
        let mut app = AppState::new();
        app.start_streaming();
        app.push_token("world");

        let exited = tui_event(TuiEvent::StreamDone, &mut app);
        assert!(!exited);
        assert!(!app.streaming);

        match app.transcript.last() {
            Some(crate::app::TranscriptEntry::AssistantMessage(text)) => {
                assert_eq!(text, "world");
            }
            other => panic!("expected AssistantMessage, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // create_event_channel (tested via crate public API)
    // -----------------------------------------------------------------------

    #[test]
    fn create_event_channel() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<TuiEvent>();
        tx.send(TuiEvent::Quit).expect("send");
        let received = rx.try_recv().expect("recv");
        assert!(matches!(received, TuiEvent::Quit));
    }

    // -----------------------------------------------------------------------
    // format_date_from_secs
    // -----------------------------------------------------------------------

    #[test]
    fn format_date_epoch() {
        // Unix epoch = 1970-01-01
        assert_eq!(format_date_from_secs(0), "1970-01-01");
    }

    #[test]
    fn format_date_known_date() {
        // 2026-03-05 00:00:00 UTC
        // Days from epoch: computed manually.
        // 2026-03-05: let's verify with a known offset.
        // 2000-01-01 = 10957 days from epoch.
        // 2026 - 2000 = 26 years.
        // Leap years in [2000..2026): 2000,2004,2008,2012,2016,2020,2024 = 7 leap years
        // Non-leap: 26 - 7 = 19 years
        // Days: 7*366 + 19*365 = 2562 + 6935 = 9497 days from 2000-01-01
        // 2026-01-01 = 10957 + 9497 = 20454 days from epoch
        // Jan: 31, Feb: 28 (2026 not leap), Mar 1-5: 4 more days
        // 2026-03-05 = 20454 + 31 + 28 + 4 = 20517 days from epoch
        let secs = 20517u64 * 86_400;
        assert_eq!(format_date_from_secs(secs), "2026-03-05");
    }

    // -----------------------------------------------------------------------
    // render_frame with populated transcript
    // -----------------------------------------------------------------------

    #[test]
    fn render_frame_with_transcript() {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");

        let mut app = AppState::new();
        app.push_user_message("hello".to_owned(), None, vec![]);
        app.start_streaming();
        app.push_token("world");

        let input_box = InputBoxState::new();
        let sidebar_data = SidebarData::new();

        terminal
            .draw(|f| render_frame(f, &mut app, &input_box, &sidebar_data, 0))
            .expect("draw");
    }

    // -----------------------------------------------------------------------
    // dispatch_action scroll
    // -----------------------------------------------------------------------

    #[test]
    fn dispatch_action_scroll_up_disengages_auto_scroll() {
        let mut app = AppState::new();
        app.push_user_message("msg".to_owned(), None, vec![]);
        assert!(app.auto_scroll);

        let mut input_box = InputBoxState::new();
        dispatch_action(Action::ScrollUp, &mut app, &mut input_box);
        assert!(!app.auto_scroll);
    }

    #[test]
    fn dispatch_action_scroll_to_bottom_reengages() {
        let mut app = AppState::new();
        app.push_user_message("msg".to_owned(), None, vec![]);
        app.scroll_up(1);
        assert!(!app.auto_scroll);

        let mut input_box = InputBoxState::new();
        dispatch_action(Action::ScrollToBottom, &mut app, &mut input_box);
        assert!(app.auto_scroll);
    }

    // -----------------------------------------------------------------------
    // handle_tui_event Quit
    // -----------------------------------------------------------------------

    #[test]
    fn handle_tui_event_quit() {
        let mut app = AppState::new();
        let exited = tui_event(TuiEvent::Quit, &mut app);
        assert!(exited);
    }

    // -----------------------------------------------------------------------
    // handle_tui_event tool call lifecycle
    // -----------------------------------------------------------------------

    #[test]
    fn handle_tui_event_tool_call_lifecycle() {
        let mut app = AppState::new();

        tui_event(
            TuiEvent::ToolCallStarted {
                name: "read_file".to_owned(),
            },
            &mut app,
        );
        assert_eq!(app.transcript.len(), 1);

        tui_event(
            TuiEvent::ToolCallCompleted {
                index: 0,
                status: ToolCallStatus::Success,
                duration_ms: Some(100),
                summary: Some("read 5 lines".to_owned()),
                thinking: None,
                output: None,
            },
            &mut app,
        );

        match &app.transcript[0] {
            crate::app::TranscriptEntry::ToolCall {
                status,
                duration_ms,
                summary,
                ..
            } => {
                assert_eq!(*status, ToolCallStatus::Success);
                assert_eq!(*duration_ms, Some(100));
                assert_eq!(summary.as_deref(), Some("read 5 lines"));
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Input editing key forwarding
    // -----------------------------------------------------------------------

    #[test]
    fn handle_input_editing_backspace() {
        let mut app = AppState::new();
        app.focus = FocusTarget::Input;
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        // Type "abc" then backspace.
        input_box.insert_char('a');
        input_box.insert_char('b');
        input_box.insert_char('c');

        let event = Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Backspace,
            crossterm::event::KeyModifiers::NONE,
        ));
        term_event(event, &mut app, &mut input_box, &mut sidebar_data);
        assert_eq!(input_box.content, "ab");
    }

    #[test]
    fn handle_input_editing_arrow_keys() {
        let mut app = AppState::new();
        app.focus = FocusTarget::Input;
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        input_box.insert_char('a');
        input_box.insert_char('b');

        // Move left, then type 'x' — should insert between a and b.
        let left = Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Left,
            crossterm::event::KeyModifiers::NONE,
        ));
        term_event(left, &mut app, &mut input_box, &mut sidebar_data);

        input_box.insert_char('x');
        assert_eq!(input_box.content, "axb");
    }

    #[test]
    fn handle_input_editing_home_end() {
        let mut app = AppState::new();
        app.focus = FocusTarget::Input;
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        input_box.insert_char('a');
        input_box.insert_char('b');
        input_box.insert_char('c');

        // Home moves cursor to start.
        let home = Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Home,
            crossterm::event::KeyModifiers::NONE,
        ));
        term_event(home, &mut app, &mut input_box, &mut sidebar_data);
        input_box.insert_char('z');
        assert_eq!(input_box.content, "zabc");

        // End moves cursor to end.
        let end = Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::End,
            crossterm::event::KeyModifiers::NONE,
        ));
        term_event(end, &mut app, &mut input_box, &mut sidebar_data);
        input_box.insert_char('!');
        assert_eq!(input_box.content, "zabc!");
    }

    // -----------------------------------------------------------------------
    // Palette integration
    // -----------------------------------------------------------------------

    #[test]
    fn dispatch_open_palette() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        dispatch_action(Action::OpenPalette, &mut app, &mut input_box);
        assert!(app.palette.visible);
        assert_eq!(app.focus, FocusTarget::Overlay);
    }

    #[test]
    fn palette_esc_closes() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        dispatch_action(Action::OpenPalette, &mut app, &mut input_box);
        assert!(app.palette.visible);

        let esc = Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        ));
        term_event(esc, &mut app, &mut input_box, &mut sidebar_data);
        assert!(!app.palette.visible);
        assert_eq!(app.focus, FocusTarget::Input);
    }

    #[test]
    fn palette_typing_filters() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        dispatch_action(Action::OpenPalette, &mut app, &mut input_box);

        for c in "session".chars() {
            let ev = Event::Key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(c),
                crossterm::event::KeyModifiers::NONE,
            ));
            term_event(ev, &mut app, &mut input_box, &mut sidebar_data);
        }

        assert_eq!(app.palette.filtered_indices.len(), 3);
    }

    #[test]
    fn palette_enter_executes_and_closes() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        dispatch_action(Action::OpenPalette, &mut app, &mut input_box);

        let enter = Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        ));
        term_event(enter, &mut app, &mut input_box, &mut sidebar_data);

        assert!(!app.palette.visible);
        // execute_command always emits at least one system message (the echo line).
        let has_system_msg = app
            .transcript
            .iter()
            .any(|e| matches!(e, crate::app::TranscriptEntry::SystemMessage(_)));
        assert!(has_system_msg);
    }

    #[test]
    fn render_frame_with_palette_open() {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");

        let mut app = AppState::new();
        app.palette.open();
        let input_box = InputBoxState::new();
        let sidebar_data = SidebarData::new();

        terminal
            .draw(|f| render_frame(f, &mut app, &input_box, &sidebar_data, 0))
            .expect("draw");
    }

    // -----------------------------------------------------------------------
    // Readline keybinding tests (Part 2)
    // -----------------------------------------------------------------------

    fn make_key_event(
        code: crossterm::event::KeyCode,
        modifiers: crossterm::event::KeyModifiers,
    ) -> Event {
        Event::Key(crossterm::event::KeyEvent::new(code, modifiers))
    }

    #[test]
    fn handle_readline_ctrl_a_moves_home() {
        let mut app = AppState::new();
        app.focus = FocusTarget::Input;
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        for c in "hello".chars() {
            input_box.insert_char(c);
        }
        // cursor is at end; Ctrl+A should move to start
        let ev = make_key_event(
            crossterm::event::KeyCode::Char('a'),
            crossterm::event::KeyModifiers::CONTROL,
        );
        term_event(ev, &mut app, &mut input_box, &mut sidebar_data);
        // insert 'z' at cursor (now at start)
        input_box.insert_char('z');
        assert_eq!(input_box.content, "zhello");
    }

    #[test]
    fn handle_readline_ctrl_e_moves_end() {
        let mut app = AppState::new();
        app.focus = FocusTarget::Input;
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        for c in "hello".chars() {
            input_box.insert_char(c);
        }
        // move to start first
        input_box.move_home();
        // Ctrl+E should move to end
        let ev = make_key_event(
            crossterm::event::KeyCode::Char('e'),
            crossterm::event::KeyModifiers::CONTROL,
        );
        term_event(ev, &mut app, &mut input_box, &mut sidebar_data);
        input_box.insert_char('!');
        assert_eq!(input_box.content, "hello!");
    }

    #[test]
    fn handle_readline_ctrl_k_kills_to_end() {
        let mut app = AppState::new();
        app.focus = FocusTarget::Input;
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        for c in "hello world".chars() {
            input_box.insert_char(c);
        }
        // move cursor to position 5 (after "hello")
        input_box.move_home();
        for _ in 0..5 {
            input_box.move_right();
        }
        let ev = make_key_event(
            crossterm::event::KeyCode::Char('k'),
            crossterm::event::KeyModifiers::CONTROL,
        );
        term_event(ev, &mut app, &mut input_box, &mut sidebar_data);
        assert_eq!(input_box.content, "hello");
    }

    #[test]
    fn handle_readline_ctrl_w_deletes_word() {
        let mut app = AppState::new();
        app.focus = FocusTarget::Input;
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        for c in "hello world".chars() {
            input_box.insert_char(c);
        }
        let ev = make_key_event(
            crossterm::event::KeyCode::Char('w'),
            crossterm::event::KeyModifiers::CONTROL,
        );
        term_event(ev, &mut app, &mut input_box, &mut sidebar_data);
        assert_eq!(input_box.content, "hello ");
    }

    #[test]
    fn handle_readline_alt_left_moves_word() {
        let mut app = AppState::new();
        app.focus = FocusTarget::Input;
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        for c in "hello world".chars() {
            input_box.insert_char(c);
        }
        // cursor at end; Alt+Left should move to start of "world" (pos 6)
        let ev = make_key_event(
            crossterm::event::KeyCode::Left,
            crossterm::event::KeyModifiers::ALT,
        );
        term_event(ev, &mut app, &mut input_box, &mut sidebar_data);
        input_box.insert_char('X');
        assert_eq!(input_box.content, "hello Xworld");
    }

    #[test]
    fn handle_readline_ctrl_right_moves_word() {
        let mut app = AppState::new();
        app.focus = FocusTarget::Input;
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        for c in "hello world".chars() {
            input_box.insert_char(c);
        }
        input_box.move_home();
        // Ctrl+Right from start should move to end of "hello" (pos 5)
        let ev = make_key_event(
            crossterm::event::KeyCode::Right,
            crossterm::event::KeyModifiers::CONTROL,
        );
        term_event(ev, &mut app, &mut input_box, &mut sidebar_data);
        input_box.insert_char('X');
        assert_eq!(input_box.content, "helloX world");
    }

    #[test]
    fn handle_readline_alt_backspace_deletes_word() {
        let mut app = AppState::new();
        app.focus = FocusTarget::Input;
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        for c in "hello world".chars() {
            input_box.insert_char(c);
        }
        // cursor at end; Alt+Backspace should delete "world"
        let ev = make_key_event(
            crossterm::event::KeyCode::Backspace,
            crossterm::event::KeyModifiers::ALT,
        );
        term_event(ev, &mut app, &mut input_box, &mut sidebar_data);
        assert_eq!(input_box.content, "hello ");
    }

    #[test]
    fn handle_readline_ctrl_p_moves_up() {
        let mut app = AppState::new();
        app.focus = FocusTarget::Input;
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        for c in "abc\ndef".chars() {
            if c == '\n' {
                input_box.insert_newline();
            } else {
                input_box.insert_char(c);
            }
        }
        // cursor is at end of line 1 (after 'f')
        let ev = make_key_event(
            crossterm::event::KeyCode::Char('p'),
            crossterm::event::KeyModifiers::CONTROL,
        );
        term_event(ev, &mut app, &mut input_box, &mut sidebar_data);
        // cursor should now be on line 0 at col 3 (after 'c')
        input_box.insert_char('X');
        assert_eq!(input_box.content, "abcX\ndef");
    }

    #[test]
    fn handle_readline_up_arrow_moves_up_in_input() {
        let mut app = AppState::new();
        app.focus = FocusTarget::Input;
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        for c in "abc\ndef".chars() {
            if c == '\n' {
                input_box.insert_newline();
            } else {
                input_box.insert_char(c);
            }
        }
        // cursor is at end of line 1 (after 'f')
        let ev = make_key_event(
            crossterm::event::KeyCode::Up,
            crossterm::event::KeyModifiers::NONE,
        );
        term_event(ev, &mut app, &mut input_box, &mut sidebar_data);
        // cursor should now be on line 0 at col 3 (after 'c')
        input_box.insert_char('X');
        assert_eq!(input_box.content, "abcX\ndef");
    }

    #[test]
    fn handle_readline_ctrl_d_empty_falls_through() {
        let mut app = AppState::new();
        app.focus = FocusTarget::Input;
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        // Ctrl+D is no longer bound to Exit in the vscode preset (it was removed to
        // avoid confusion with readline delete-forward). On empty input it falls through
        // to the keybind resolver, but finds no binding, so it does NOT exit.
        let ev = make_key_event(
            crossterm::event::KeyCode::Char('d'),
            crossterm::event::KeyModifiers::CONTROL,
        );
        let exited = term_event(ev, &mut app, &mut input_box, &mut sidebar_data);
        assert!(
            !exited,
            "Ctrl+D no longer exits; use Ctrl+Q or double Ctrl+C"
        );
    }

    // -----------------------------------------------------------------------
    // Ctrl+Q universal exit
    // -----------------------------------------------------------------------

    #[test]
    fn handle_ctrl_q_exits() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        let ev = make_key_event(
            crossterm::event::KeyCode::Char('q'),
            crossterm::event::KeyModifiers::CONTROL,
        );
        let exited = term_event(ev, &mut app, &mut input_box, &mut sidebar_data);
        assert!(exited, "Ctrl+Q should exit unconditionally");
    }

    #[test]
    fn handle_ctrl_q_exits_with_nonempty_input() {
        let mut app = AppState::new();
        app.focus = FocusTarget::Input;
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        // Even with text in the input box, Ctrl+Q must exit.
        for c in "some text".chars() {
            input_box.insert_char(c);
        }
        let ev = make_key_event(
            crossterm::event::KeyCode::Char('q'),
            crossterm::event::KeyModifiers::CONTROL,
        );
        let exited = term_event(ev, &mut app, &mut input_box, &mut sidebar_data);
        assert!(exited, "Ctrl+Q should exit even when input is non-empty");
    }

    // -----------------------------------------------------------------------
    // Double Ctrl+C exit
    // -----------------------------------------------------------------------

    #[test]
    fn handle_single_ctrl_c_does_not_exit() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        let ev = make_key_event(
            crossterm::event::KeyCode::Char('c'),
            crossterm::event::KeyModifiers::CONTROL,
        );
        let exited = term_event(ev, &mut app, &mut input_box, &mut sidebar_data);
        assert!(!exited, "single Ctrl+C should not exit");
        assert!(app.last_ctrl_c.is_some(), "last_ctrl_c should be recorded");
    }

    #[test]
    fn handle_ctrl_c_shows_hint() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        let ev = make_key_event(
            crossterm::event::KeyCode::Char('c'),
            crossterm::event::KeyModifiers::CONTROL,
        );
        term_event(ev, &mut app, &mut input_box, &mut sidebar_data);
        assert_eq!(
            app.ctrl_c_hint.as_deref(),
            Some("Press Ctrl+C again to exit"),
            "hint should be set after first Ctrl+C"
        );
    }

    #[test]
    fn handle_double_ctrl_c_exits() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        let ctrl_c = make_key_event(
            crossterm::event::KeyCode::Char('c'),
            crossterm::event::KeyModifiers::CONTROL,
        );

        // First press — should not exit.
        let first = term_event(ctrl_c.clone(), &mut app, &mut input_box, &mut sidebar_data);
        assert!(!first, "first Ctrl+C should not exit");

        // Second press immediately after — should exit.
        let second = term_event(ctrl_c, &mut app, &mut input_box, &mut sidebar_data);
        assert!(second, "second Ctrl+C within 2 s should exit");
    }

    // -----------------------------------------------------------------------
    // Diff modal integration
    // -----------------------------------------------------------------------

    #[test]
    fn tui_event_patch_proposed() {
        let mut app = AppState::new();
        let exited = tui_event(
            TuiEvent::PatchProposed {
                file_path: "src/lib.rs".to_owned(),
                raw_diff: "+added line\n-removed line".to_owned(),
                patch_id: Some("p1".to_owned()),
            },
            &mut app,
        );
        assert!(!exited);
        assert!(app.diff_modal.visible);
        assert_eq!(app.diff_modal.file_path, "src/lib.rs");
        assert_eq!(app.focus, FocusTarget::Overlay);
    }

    #[test]
    fn diff_modal_esc_closes() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        app.propose_patch("f.rs".to_owned(), "+line".to_owned(), None);
        assert!(app.diff_modal.visible);

        let esc = Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        ));
        term_event(esc, &mut app, &mut input_box, &mut sidebar_data);
        assert!(!app.diff_modal.visible);
        assert_eq!(app.focus, FocusTarget::Input);
    }

    #[test]
    fn diff_modal_approve_closes_and_logs() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        app.propose_patch("f.rs".to_owned(), "+line".to_owned(), None);

        let a_key = Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('a'),
            crossterm::event::KeyModifiers::NONE,
        ));
        term_event(a_key, &mut app, &mut input_box, &mut sidebar_data);
        assert!(!app.diff_modal.visible);
        let has_approved = app
            .transcript
            .iter()
            .any(|e| matches!(e, TranscriptEntry::SystemMessage(msg) if msg.contains("approved")));
        assert!(has_approved);
    }

    #[test]
    fn diff_modal_reject_closes_and_logs() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        app.propose_patch("f.rs".to_owned(), "-line".to_owned(), None);

        let r_key = Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('r'),
            crossterm::event::KeyModifiers::NONE,
        ));
        term_event(r_key, &mut app, &mut input_box, &mut sidebar_data);
        assert!(!app.diff_modal.visible);
        let has_rejected = app
            .transcript
            .iter()
            .any(|e| matches!(e, TranscriptEntry::SystemMessage(msg) if msg.contains("rejected")));
        assert!(has_rejected);
    }

    #[test]
    fn diff_modal_scroll_keys() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        let raw = (0..30)
            .map(|i| format!("+line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        app.propose_patch("f.rs".to_owned(), raw, None);
        assert_eq!(app.diff_modal.scroll_offset, 0);

        // Down arrow scrolls
        let down = Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Down,
            crossterm::event::KeyModifiers::NONE,
        ));
        term_event(down, &mut app, &mut input_box, &mut sidebar_data);
        assert_eq!(app.diff_modal.scroll_offset, 1);

        // Up arrow scrolls back
        let up = Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Up,
            crossterm::event::KeyModifiers::NONE,
        ));
        term_event(up, &mut app, &mut input_box, &mut sidebar_data);
        assert_eq!(app.diff_modal.scroll_offset, 0);
    }

    #[test]
    fn dispatch_show_diff_reopens_modal() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();

        // Propose a patch, then close it.
        app.propose_patch("f.rs".to_owned(), "+line".to_owned(), None);
        app.diff_modal.close();
        assert!(!app.diff_modal.visible);

        // ShowDiff should re-open it.
        dispatch_action(Action::ShowDiff, &mut app, &mut input_box);
        assert!(app.diff_modal.visible);
        assert_eq!(app.focus, FocusTarget::Overlay);
    }

    #[test]
    fn render_frame_with_diff_modal() {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");

        let mut app = AppState::new();
        app.propose_patch(
            "src/lib.rs".to_owned(),
            "+added\n-removed\n context".to_owned(),
            None,
        );
        let input_box = InputBoxState::new();
        let sidebar_data = SidebarData::new();

        terminal
            .draw(|f| render_frame(f, &mut app, &input_box, &sidebar_data, 0))
            .expect("draw");
    }

    // -----------------------------------------------------------------------
    // Approval modal key handling
    // -----------------------------------------------------------------------

    #[test]
    fn approval_modal_esc_closes() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        app.request_approval(
            "run_cmd".to_owned(),
            "cargo test".to_owned(),
            "/tmp".to_owned(),
            "ws".to_owned(),
        );
        assert!(app.approval_modal.visible);

        let esc = Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        ));
        term_event(esc, &mut app, &mut input_box, &mut sidebar_data);
        assert!(!app.approval_modal.visible);
        assert_eq!(app.focus, FocusTarget::Input);
    }

    #[test]
    fn approval_modal_approve_once() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        app.request_approval(
            "run_cmd".to_owned(),
            "cargo test".to_owned(),
            "/tmp".to_owned(),
            "ws".to_owned(),
        );

        let key_o = Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('o'),
            crossterm::event::KeyModifiers::NONE,
        ));
        term_event(key_o, &mut app, &mut input_box, &mut sidebar_data);
        assert!(!app.approval_modal.visible);
        assert_eq!(app.focus, FocusTarget::Input);
        // Should have a system message about approval.
        let has_approved_msg = app.transcript.iter().any(
            |e| matches!(e, TranscriptEntry::SystemMessage(msg) if msg.contains("Approved once")),
        );
        assert!(has_approved_msg);
    }

    #[test]
    fn approval_modal_approve_session() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        app.request_approval(
            "run_cmd".to_owned(),
            "cargo test".to_owned(),
            "/tmp".to_owned(),
            "ws".to_owned(),
        );

        let key_s = Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('s'),
            crossterm::event::KeyModifiers::NONE,
        ));
        term_event(key_s, &mut app, &mut input_box, &mut sidebar_data);
        assert!(!app.approval_modal.visible);
        let has_session_msg = app.transcript.iter().any(|e| {
            matches!(e, TranscriptEntry::SystemMessage(msg) if msg.contains("Approved for session"))
        });
        assert!(has_session_msg);
    }

    #[test]
    fn approval_modal_deny() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        app.request_approval(
            "run_cmd".to_owned(),
            "cargo test".to_owned(),
            "/tmp".to_owned(),
            "ws".to_owned(),
        );

        let key_d = Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('d'),
            crossterm::event::KeyModifiers::NONE,
        ));
        term_event(key_d, &mut app, &mut input_box, &mut sidebar_data);
        assert!(!app.approval_modal.visible);
        let has_denied_msg = app
            .transcript
            .iter()
            .any(|e| matches!(e, TranscriptEntry::SystemMessage(msg) if msg.contains("Denied")));
        assert!(has_denied_msg);
    }

    #[test]
    fn render_frame_with_approval_modal() {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");

        let mut app = AppState::new();
        app.request_approval(
            "run_cmd".to_owned(),
            "cargo test --workspace".to_owned(),
            "/home/user/code/ucode".to_owned(),
            "ws workspace".to_owned(),
        );

        let input_box = InputBoxState::new();
        let sidebar_data = SidebarData::new();

        terminal
            .draw(|f| render_frame(f, &mut app, &input_box, &sidebar_data, 0))
            .expect("draw");
    }

    // -----------------------------------------------------------------------
    // dispatch_action ToggleTheme / ToggleDensity
    // -----------------------------------------------------------------------

    #[test]
    fn dispatch_toggle_theme_cycles_preset() {
        use ucode_themes::theme_names;

        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        assert_eq!(app.theme.name, "ucode");

        dispatch_action(Action::ToggleTheme, &mut app, &mut input_box);
        assert_eq!(app.theme.name, "tokyonight");

        // Cycle through all built-in themes and verify we wrap back to "ucode".
        let names = theme_names();
        let mut t = app.theme.clone();
        for _ in 1..names.len() {
            dispatch_action(Action::ToggleTheme, &mut app, &mut input_box);
            t = app.theme.clone();
        }
        assert_eq!(t.name, "ucode");
    }

    #[test]
    fn dispatch_toggle_density_cycles() {
        use crate::theme::Density;

        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        // Default density is Comfortable.
        assert_eq!(app.density, Density::Comfortable);

        dispatch_action(Action::ToggleDensity, &mut app, &mut input_box);
        assert_eq!(app.density, Density::Compact);

        dispatch_action(Action::ToggleDensity, &mut app, &mut input_box);
        assert_eq!(app.density, Density::Comfortable);
    }

    // -----------------------------------------------------------------------
    // Slash command autocomplete
    // -----------------------------------------------------------------------

    #[test]
    fn test_slash_completions_method() {
        let app = AppState::new();
        // Empty query after "/" returns all commands.
        let all = app.slash_completions("/");
        assert!(!all.is_empty());

        // Prefix "session" narrows results.
        let session = app.slash_completions("/session");
        assert!(!session.is_empty());
        assert!(session.iter().all(|e| e.name.contains("session")));

        // Non-matching query returns empty.
        let none = app.slash_completions("/zzznomatch");
        assert!(none.is_empty());
    }

    #[test]
    fn test_slash_completions_include_args_hint() {
        let app = AppState::new();
        let results = app.slash_completions("/session rename");
        let rename = results
            .iter()
            .find(|e| e.name == "/session rename")
            .expect("/session rename must appear in completions");
        assert_eq!(
            rename.args_hint.as_deref(),
            Some("<name>"),
            "/session rename should carry the <name> args_hint"
        );
    }

    #[test]
    fn test_slash_prefix_triggers_autocomplete() {
        let mut app = AppState::new();
        app.focus = FocusTarget::Input;
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        // Type "/con" — should trigger autocomplete.
        for c in "/con".chars() {
            let ev = make_key_event(
                crossterm::event::KeyCode::Char(c),
                crossterm::event::KeyModifiers::NONE,
            );
            term_event(ev, &mut app, &mut input_box, &mut sidebar_data);
        }

        assert!(
            input_box.autocomplete.visible,
            "autocomplete should be visible after /con"
        );
        assert!(
            !input_box.autocomplete.entries.is_empty(),
            "should have matching entries for /con"
        );
        // "/connect" should be among the results.
        assert!(
            input_box
                .autocomplete
                .entries
                .iter()
                .any(|e| e.name == "/connect"),
            "/connect should appear in completions for /con"
        );
    }

    #[test]
    fn test_non_slash_input_no_autocomplete() {
        let mut app = AppState::new();
        app.focus = FocusTarget::Input;
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        for c in "hello".chars() {
            let ev = make_key_event(
                crossterm::event::KeyCode::Char(c),
                crossterm::event::KeyModifiers::NONE,
            );
            term_event(ev, &mut app, &mut input_box, &mut sidebar_data);
        }

        assert!(
            !input_box.autocomplete.visible,
            "autocomplete must not show for non-slash input"
        );
    }

    #[test]
    fn test_backspace_updates_autocomplete() {
        let mut app = AppState::new();
        app.focus = FocusTarget::Input;
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        // Type "/con".
        for c in "/con".chars() {
            let ev = make_key_event(
                crossterm::event::KeyCode::Char(c),
                crossterm::event::KeyModifiers::NONE,
            );
            term_event(ev, &mut app, &mut input_box, &mut sidebar_data);
        }
        assert!(input_box.autocomplete.visible);
        let entries_before = input_box.autocomplete.entries.clone();

        // Backspace → "/co".
        let bs = make_key_event(
            crossterm::event::KeyCode::Backspace,
            crossterm::event::KeyModifiers::NONE,
        );
        term_event(bs, &mut app, &mut input_box, &mut sidebar_data);

        assert_eq!(input_box.content, "/co");
        assert!(
            input_box.autocomplete.visible,
            "autocomplete should still be visible after backspace"
        );
        // The entry set may differ (broader match for "/co").
        let entries_after = &input_box.autocomplete.entries;
        // "/co" matches at least as many commands as "/con".
        assert!(entries_after.len() >= entries_before.len());
    }

    #[test]
    fn test_backspace_removes_slash_hides_autocomplete() {
        let mut app = AppState::new();
        app.focus = FocusTarget::Input;
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        // Type "/" only.
        let slash = make_key_event(
            crossterm::event::KeyCode::Char('/'),
            crossterm::event::KeyModifiers::NONE,
        );
        term_event(slash, &mut app, &mut input_box, &mut sidebar_data);
        assert!(input_box.autocomplete.visible);

        // Backspace removes the "/" → empty input, no slash prefix.
        let bs = make_key_event(
            crossterm::event::KeyCode::Backspace,
            crossterm::event::KeyModifiers::NONE,
        );
        term_event(bs, &mut app, &mut input_box, &mut sidebar_data);

        assert!(input_box.content.is_empty());
        assert!(
            !input_box.autocomplete.visible,
            "autocomplete should hide when slash is removed"
        );
    }

    #[test]
    fn test_accept_autocomplete_replaces_input() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();

        // Manually show autocomplete with a known entry.
        use crate::components::input::AutocompleteEntry;
        input_box.autocomplete.show(vec![
            AutocompleteEntry::new("/connect", "Connect provider", "[builtin]"),
            AutocompleteEntry::new("/skills", "Browse skills", "[builtin]"),
        ]);
        assert!(input_box.autocomplete.visible);
        assert_eq!(input_box.autocomplete.selected, 0);

        dispatch_action(Action::AcceptAutocomplete, &mut app, &mut input_box);

        // Input should be the selected entry name + trailing space.
        assert_eq!(input_box.content, "/connect ");
        assert!(
            !input_box.autocomplete.visible,
            "autocomplete should hide after accept"
        );
    }

    #[test]
    fn test_accept_autocomplete_no_op_when_hidden() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();

        for c in "hello".chars() {
            input_box.insert_char(c);
        }
        // Autocomplete is not visible.
        assert!(!input_box.autocomplete.visible);

        dispatch_action(Action::AcceptAutocomplete, &mut app, &mut input_box);

        // Input should be unchanged.
        assert_eq!(input_box.content, "hello");
    }

    #[test]
    fn test_up_down_navigate_autocomplete() {
        let mut app = AppState::new();
        app.focus = FocusTarget::Input;
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        // Type "/" to open autocomplete with all commands.
        let slash = make_key_event(
            crossterm::event::KeyCode::Char('/'),
            crossterm::event::KeyModifiers::NONE,
        );
        term_event(slash, &mut app, &mut input_box, &mut sidebar_data);
        assert!(input_box.autocomplete.visible);
        assert_eq!(input_box.autocomplete.selected, 0);

        // Down arrow should advance selection.
        let down = make_key_event(
            crossterm::event::KeyCode::Down,
            crossterm::event::KeyModifiers::NONE,
        );
        term_event(down, &mut app, &mut input_box, &mut sidebar_data);
        assert_eq!(input_box.autocomplete.selected, 1);

        // Up arrow should go back.
        let up = make_key_event(
            crossterm::event::KeyCode::Up,
            crossterm::event::KeyModifiers::NONE,
        );
        term_event(up, &mut app, &mut input_box, &mut sidebar_data);
        assert_eq!(input_box.autocomplete.selected, 0);

        // Input content should be unchanged (arrows navigated autocomplete, not cursor).
        assert_eq!(input_box.content, "/");
    }

    #[test]
    fn test_esc_hides_autocomplete_via_dismiss() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();

        use crate::components::input::AutocompleteEntry;
        input_box.autocomplete.show(vec![AutocompleteEntry::new(
            "/connect",
            "Connect provider",
            "[builtin]",
        )]);
        assert!(input_box.autocomplete.visible);

        // Dismiss action hides autocomplete without clearing input.
        for c in "/con".chars() {
            input_box.insert_char(c);
        }
        dispatch_action(Action::Dismiss, &mut app, &mut input_box);

        assert!(
            !input_box.autocomplete.visible,
            "Dismiss should hide autocomplete"
        );
        // Content should be preserved (not cleared) when autocomplete was visible.
        assert_eq!(input_box.content, "/con");
    }

    // -----------------------------------------------------------------------
    // Slash command execution on Enter
    // -----------------------------------------------------------------------

    #[test]
    fn test_slash_command_executed_on_enter() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();

        for c in "/connect".chars() {
            input_box.insert_char(c);
        }
        dispatch_action(Action::SendMessage, &mut app, &mut input_box);

        // Should have system messages, not a user message + streaming.
        assert!(
            !app.streaming,
            "streaming should not start for a slash command"
        );
        let has_user_msg = app
            .transcript
            .iter()
            .any(|e| matches!(e, TranscriptEntry::UserMessage(_)));
        assert!(
            !has_user_msg,
            "slash command must not produce a user message"
        );
        let has_system_msg = app
            .transcript
            .iter()
            .any(|e| matches!(e, TranscriptEntry::SystemMessage(_)));
        assert!(
            has_system_msg,
            "slash command should produce system messages"
        );
    }

    #[test]
    fn test_unknown_slash_command_shows_suggestions() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();

        for c in "/conect".chars() {
            input_box.insert_char(c);
        }
        dispatch_action(Action::SendMessage, &mut app, &mut input_box);

        let has_suggestion = app
            .transcript
            .iter()
            .any(|e| matches!(e, TranscriptEntry::SystemMessage(m) if m.contains("Did you mean")));
        assert!(
            has_suggestion,
            "typo should trigger Did you mean suggestion"
        );
        assert!(!app.streaming);
    }

    #[test]
    fn test_unknown_slash_command_no_suggestions() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();

        for c in "/zzzzz".chars() {
            input_box.insert_char(c);
        }
        dispatch_action(Action::SendMessage, &mut app, &mut input_box);

        let has_unknown = app.transcript.iter().any(|e| {
            matches!(e, TranscriptEntry::SystemMessage(m)
                if m.contains("Unknown command") && !m.contains("Did you mean"))
        });
        assert!(
            has_unknown,
            "completely unknown command should say Unknown command without suggestions"
        );
        assert!(!app.streaming);
    }

    #[test]
    fn test_regular_message_still_works() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();

        for c in "hello world".chars() {
            input_box.insert_char(c);
        }
        dispatch_action(Action::SendMessage, &mut app, &mut input_box);

        let has_user_msg = app
            .transcript
            .iter()
            .any(|e| matches!(e, TranscriptEntry::UserMessage(m) if m == "hello world"));
        assert!(has_user_msg, "regular message should become a UserMessage");
        assert!(app.streaming, "regular message should start streaming");
    }

    #[test]
    fn test_slash_command_with_args() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();

        for c in "/session rename my-session".chars() {
            input_box.insert_char(c);
        }
        dispatch_action(Action::SendMessage, &mut app, &mut input_box);

        assert!(!app.streaming);
        let has_args_echo = app
            .transcript
            .iter()
            .any(|e| matches!(e, TranscriptEntry::SystemMessage(m) if m.contains("my-session")));
        assert!(has_args_echo, "args should appear in the command echo");
    }

    #[test]
    fn test_palette_execute_uses_execute_command() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        dispatch_action(Action::OpenPalette, &mut app, &mut input_box);
        assert!(app.palette.visible);

        let enter = Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        ));
        term_event(enter, &mut app, &mut input_box, &mut sidebar_data);

        assert!(!app.palette.visible);
        // execute_command produces system messages (echo + status), not the old "Executed: name".
        let has_system_msg = app
            .transcript
            .iter()
            .any(|e| matches!(e, TranscriptEntry::SystemMessage(_)));
        assert!(
            has_system_msg,
            "palette Enter should produce system messages via execute_command"
        );
    }

    // -----------------------------------------------------------------------
    // Toast rendering
    // -----------------------------------------------------------------------

    #[test]
    fn test_render_frame_with_toasts() {
        use crate::components::toast::ToastLevel;

        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");

        let mut app = AppState::new();
        app.toast(ToastLevel::Info, "Session started");
        app.toast(ToastLevel::Success, "Checkpoint created");
        app.toast_with_body(
            ToastLevel::Warning,
            "Budget warning",
            "75% of token budget used",
        );

        let input_box = InputBoxState::new();
        let sidebar_data = SidebarData::new();

        terminal
            .draw(|f| render_frame(f, &mut app, &input_box, &sidebar_data, 0))
            .expect("render with toasts must not panic");

        assert_eq!(app.toasts.len(), 3);
    }

    #[test]
    fn tui_event_toast_variant() {
        let mut app = AppState::new();
        let exited = tui_event(
            TuiEvent::Toast {
                level: ToastLevel::Error,
                title: "Something failed".to_owned(),
                body: Some("check logs".to_owned()),
            },
            &mut app,
        );
        assert!(!exited);
        assert_eq!(app.toasts.len(), 1);
        assert_eq!(app.toasts.visible()[0].title, "Something failed");
        assert_eq!(app.toasts.visible()[0].body.as_deref(), Some("check logs"));
    }

    // -----------------------------------------------------------------------
    // System-triggered semantic toast events (ISSUE 0708b)
    // -----------------------------------------------------------------------

    #[test]
    fn tui_event_checkpoint_created() {
        let mut app = AppState::new();
        let exited = tui_event(
            TuiEvent::CheckpointCreated {
                name: "v1.0".to_owned(),
            },
            &mut app,
        );
        assert!(!exited);
        assert_eq!(app.toasts.len(), 1);
        let toast = &app.toasts.visible()[0];
        assert_eq!(toast.level, ToastLevel::Info);
        assert_eq!(toast.title, "Checkpoint created: v1.0");
        assert!(toast.body.is_none());
    }

    #[test]
    fn tui_event_budget_warning() {
        let mut app = AppState::new();
        let exited = tui_event(
            TuiEvent::BudgetWarning {
                used_pct: 75.0,
                message: "75% of token budget used".to_owned(),
            },
            &mut app,
        );
        assert!(!exited);
        assert_eq!(app.toasts.len(), 1);
        let toast = &app.toasts.visible()[0];
        assert_eq!(toast.level, ToastLevel::Warning);
        assert_eq!(toast.title, "Budget warning");
        assert_eq!(toast.body.as_deref(), Some("75% of token budget used"));
    }

    #[test]
    fn tui_event_agent_completed() {
        let mut app = AppState::new();
        let exited = tui_event(
            TuiEvent::AgentCompleted {
                agent_id: "agent-42".to_owned(),
                name: "code-reviewer".to_owned(),
            },
            &mut app,
        );
        assert!(!exited);
        assert_eq!(app.toasts.len(), 1);
        let toast = &app.toasts.visible()[0];
        assert_eq!(toast.level, ToastLevel::Success);
        assert_eq!(toast.title, "Agent completed: code-reviewer");
        assert!(toast.body.is_none());
    }

    #[test]
    fn tui_event_agent_failed() {
        let mut app = AppState::new();
        let exited = tui_event(
            TuiEvent::AgentFailed {
                agent_id: "agent-42".to_owned(),
                name: "code-reviewer".to_owned(),
                error: "timeout after 30s".to_owned(),
            },
            &mut app,
        );
        assert!(!exited);
        assert_eq!(app.toasts.len(), 1);
        let toast = &app.toasts.visible()[0];
        assert_eq!(toast.level, ToastLevel::Error);
        assert_eq!(toast.title, "Agent failed: code-reviewer");
        assert_eq!(toast.body.as_deref(), Some("timeout after 30s"));
    }

    #[test]
    fn tui_event_mcp_server_crashed() {
        let mut app = AppState::new();
        let exited = tui_event(
            TuiEvent::McpServerCrashed {
                server_name: "filesystem".to_owned(),
                error: "segfault".to_owned(),
            },
            &mut app,
        );
        assert!(!exited);
        assert_eq!(app.toasts.len(), 1);
        let toast = &app.toasts.visible()[0];
        assert_eq!(toast.level, ToastLevel::Error);
        assert_eq!(toast.title, "MCP server crashed: filesystem");
        assert_eq!(toast.body.as_deref(), Some("segfault"));
    }

    #[test]
    fn tui_event_auth_expired() {
        let mut app = AppState::new();
        let exited = tui_event(
            TuiEvent::AuthExpired {
                provider: "github".to_owned(),
            },
            &mut app,
        );
        assert!(!exited);
        assert_eq!(app.toasts.len(), 1);
        let toast = &app.toasts.visible()[0];
        assert_eq!(toast.level, ToastLevel::Warning);
        assert_eq!(toast.title, "Auth expired: github");
        assert!(toast.body.is_none());
    }

    // -----------------------------------------------------------------------
    // Keybind overlay integration (ISSUE 0709a)
    // -----------------------------------------------------------------------

    #[test]
    fn dispatch_show_keybind_overlay_opens_overlay() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        assert!(!app.keybind_overlay.visible);

        dispatch_action(Action::ShowKeybindOverlay, &mut app, &mut input_box);

        assert!(app.keybind_overlay.visible);
        assert_eq!(app.focus, FocusTarget::Overlay);
        assert!(!app.keybind_overlay.entries.is_empty());
    }

    #[test]
    fn dispatch_show_keybind_overlay_does_not_exit() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        let exited = dispatch_action(Action::ShowKeybindOverlay, &mut app, &mut input_box);
        assert!(!exited);
    }

    #[test]
    fn keybind_overlay_esc_closes() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        dispatch_action(Action::ShowKeybindOverlay, &mut app, &mut input_box);
        assert!(app.keybind_overlay.visible);

        let esc = Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        ));
        let exited = term_event(esc, &mut app, &mut input_box, &mut sidebar_data);
        assert!(!exited);
        assert!(!app.keybind_overlay.visible);
        assert_eq!(app.focus, FocusTarget::Input);
    }

    #[test]
    fn keybind_overlay_up_scrolls() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        dispatch_action(Action::ShowKeybindOverlay, &mut app, &mut input_box);
        // Scroll down first so we can scroll up.
        app.keybind_overlay.scroll_down(5);
        let before = app.keybind_overlay.scroll_offset;

        let up = Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Up,
            crossterm::event::KeyModifiers::NONE,
        ));
        term_event(up, &mut app, &mut input_box, &mut sidebar_data);
        assert!(app.keybind_overlay.scroll_offset < before);
    }

    #[test]
    fn keybind_overlay_down_scrolls() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        dispatch_action(Action::ShowKeybindOverlay, &mut app, &mut input_box);
        let before = app.keybind_overlay.scroll_offset;

        let down = Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Down,
            crossterm::event::KeyModifiers::NONE,
        ));
        term_event(down, &mut app, &mut input_box, &mut sidebar_data);
        assert!(app.keybind_overlay.scroll_offset > before);
    }

    #[test]
    fn keybind_overlay_k_scrolls_up() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        dispatch_action(Action::ShowKeybindOverlay, &mut app, &mut input_box);
        app.keybind_overlay.scroll_down(5);
        let before = app.keybind_overlay.scroll_offset;

        let k = Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('k'),
            crossterm::event::KeyModifiers::NONE,
        ));
        term_event(k, &mut app, &mut input_box, &mut sidebar_data);
        assert!(app.keybind_overlay.scroll_offset < before);
    }

    #[test]
    fn keybind_overlay_j_scrolls_down() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        dispatch_action(Action::ShowKeybindOverlay, &mut app, &mut input_box);
        let before = app.keybind_overlay.scroll_offset;

        let j = Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('j'),
            crossterm::event::KeyModifiers::NONE,
        ));
        term_event(j, &mut app, &mut input_box, &mut sidebar_data);
        assert!(app.keybind_overlay.scroll_offset > before);
    }

    #[test]
    fn keybind_overlay_page_up_scrolls() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        dispatch_action(Action::ShowKeybindOverlay, &mut app, &mut input_box);
        app.keybind_overlay.scroll_down(15);
        let before = app.keybind_overlay.scroll_offset;

        let pgup = Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::PageUp,
            crossterm::event::KeyModifiers::NONE,
        ));
        term_event(pgup, &mut app, &mut input_box, &mut sidebar_data);
        assert!(app.keybind_overlay.scroll_offset < before);
    }

    #[test]
    fn keybind_overlay_page_down_scrolls() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        dispatch_action(Action::ShowKeybindOverlay, &mut app, &mut input_box);
        let before = app.keybind_overlay.scroll_offset;

        let pgdn = Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::PageDown,
            crossterm::event::KeyModifiers::NONE,
        ));
        term_event(pgdn, &mut app, &mut input_box, &mut sidebar_data);
        assert!(app.keybind_overlay.scroll_offset > before);
    }

    #[test]
    fn keybind_overlay_other_keys_ignored() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        dispatch_action(Action::ShowKeybindOverlay, &mut app, &mut input_box);
        assert!(app.keybind_overlay.visible);

        // 'x' should be ignored (not close the overlay, not exit).
        let x = Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('x'),
            crossterm::event::KeyModifiers::NONE,
        ));
        let exited = term_event(x, &mut app, &mut input_box, &mut sidebar_data);
        assert!(!exited);
        assert!(
            app.keybind_overlay.visible,
            "overlay should still be visible"
        );
    }

    #[test]
    fn render_frame_with_keybind_overlay() {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");

        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        dispatch_action(Action::ShowKeybindOverlay, &mut app, &mut input_box);

        let sidebar_data = SidebarData::new();
        terminal
            .draw(|f| render_frame(f, &mut app, &input_box, &sidebar_data, 0))
            .expect("render with keybind overlay must not panic");
    }

    // -----------------------------------------------------------------------
    // Search overlay integration (ISSUE 0709b)
    // -----------------------------------------------------------------------

    #[test]
    fn dispatch_search_transcript_opens_overlay() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        assert!(!app.search_overlay.visible);

        dispatch_action(Action::SearchTranscript, &mut app, &mut input_box);

        assert!(app.search_overlay.visible);
        assert_eq!(app.focus, FocusTarget::Overlay);
        assert!(app.dirty);
    }

    #[test]
    fn dispatch_search_transcript_does_not_exit() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        let exited = dispatch_action(Action::SearchTranscript, &mut app, &mut input_box);
        assert!(!exited);
    }

    #[test]
    fn dispatch_next_search_match_advances_match() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();

        app.push_user_message("hello world".to_owned(), None, vec![]);
        app.push_user_message("hello again".to_owned(), None, vec![]);

        // Open and search
        app.search_overlay
            .open(crate::keybinds::KeybindPreset::default());
        for c in "hello".chars() {
            app.search_overlay.insert_char(c);
        }
        app.search_overlay.search(&app.transcript);
        assert_eq!(app.search_overlay.match_count(), 2);
        assert_eq!(app.search_overlay.current_match, 0);

        dispatch_action(Action::NextSearchMatch, &mut app, &mut input_box);
        assert_eq!(app.search_overlay.current_match, 1);
        assert!(app.dirty);
    }

    #[test]
    fn dispatch_prev_search_match_goes_back() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();

        app.push_user_message("hello world".to_owned(), None, vec![]);
        app.push_user_message("hello again".to_owned(), None, vec![]);

        app.search_overlay
            .open(crate::keybinds::KeybindPreset::default());
        for c in "hello".chars() {
            app.search_overlay.insert_char(c);
        }
        app.search_overlay.search(&app.transcript);
        app.search_overlay.next_match(); // now at 1

        dispatch_action(Action::PrevSearchMatch, &mut app, &mut input_box);
        assert_eq!(app.search_overlay.current_match, 0);
        assert!(app.dirty);
    }

    #[test]
    fn dispatch_next_search_match_noop_when_no_matches() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        // No search performed, no matches
        let exited = dispatch_action(Action::NextSearchMatch, &mut app, &mut input_box);
        assert!(!exited);
        assert_eq!(app.search_overlay.current_match, 0);
    }

    #[test]
    fn search_overlay_esc_closes() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        dispatch_action(Action::SearchTranscript, &mut app, &mut input_box);
        assert!(app.search_overlay.visible);

        let esc = Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        ));
        let exited = term_event(esc, &mut app, &mut input_box, &mut sidebar_data);
        assert!(!exited);
        assert!(!app.search_overlay.visible);
        assert_eq!(app.focus, FocusTarget::Input);
    }

    #[test]
    fn search_overlay_char_inserts_and_searches() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        app.push_user_message("hello world".to_owned(), None, vec![]);
        dispatch_action(Action::SearchTranscript, &mut app, &mut input_box);
        assert!(app.search_overlay.visible);

        let h = Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('h'),
            crossterm::event::KeyModifiers::NONE,
        ));
        term_event(h, &mut app, &mut input_box, &mut sidebar_data);

        assert_eq!(app.search_overlay.query, "h");
        assert_eq!(app.search_overlay.match_count(), 1);
        assert!(app.dirty);
    }

    #[test]
    fn search_overlay_backspace_removes_char_and_searches() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        app.push_user_message("hello world".to_owned(), None, vec![]);
        dispatch_action(Action::SearchTranscript, &mut app, &mut input_box);

        // Type "he"
        for c in "he".chars() {
            let ev = Event::Key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(c),
                crossterm::event::KeyModifiers::NONE,
            ));
            term_event(ev, &mut app, &mut input_box, &mut sidebar_data);
        }
        assert_eq!(app.search_overlay.query, "he");

        // Backspace → "h"
        let bs = Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Backspace,
            crossterm::event::KeyModifiers::NONE,
        ));
        term_event(bs, &mut app, &mut input_box, &mut sidebar_data);
        assert_eq!(app.search_overlay.query, "h");
        assert!(app.dirty);
    }

    #[test]
    fn search_overlay_enter_advances_match() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        app.push_user_message("hello world".to_owned(), None, vec![]);
        app.push_user_message("hello again".to_owned(), None, vec![]);
        dispatch_action(Action::SearchTranscript, &mut app, &mut input_box);

        for c in "hello".chars() {
            let ev = Event::Key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(c),
                crossterm::event::KeyModifiers::NONE,
            ));
            term_event(ev, &mut app, &mut input_box, &mut sidebar_data);
        }
        assert_eq!(app.search_overlay.match_count(), 2);
        assert_eq!(app.search_overlay.current_match, 0);

        let enter = Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        ));
        term_event(enter, &mut app, &mut input_box, &mut sidebar_data);
        assert_eq!(app.search_overlay.current_match, 1);
    }

    #[test]
    fn search_overlay_down_advances_match() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        app.push_user_message("hello world".to_owned(), None, vec![]);
        app.push_user_message("hello again".to_owned(), None, vec![]);
        dispatch_action(Action::SearchTranscript, &mut app, &mut input_box);

        for c in "hello".chars() {
            let ev = Event::Key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(c),
                crossterm::event::KeyModifiers::NONE,
            ));
            term_event(ev, &mut app, &mut input_box, &mut sidebar_data);
        }

        let down = Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Down,
            crossterm::event::KeyModifiers::NONE,
        ));
        term_event(down, &mut app, &mut input_box, &mut sidebar_data);
        assert_eq!(app.search_overlay.current_match, 1);
    }

    #[test]
    fn search_overlay_up_goes_prev_match() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        app.push_user_message("hello world".to_owned(), None, vec![]);
        app.push_user_message("hello again".to_owned(), None, vec![]);
        dispatch_action(Action::SearchTranscript, &mut app, &mut input_box);

        for c in "hello".chars() {
            let ev = Event::Key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(c),
                crossterm::event::KeyModifiers::NONE,
            ));
            term_event(ev, &mut app, &mut input_box, &mut sidebar_data);
        }
        // Advance to match 1
        app.search_overlay.next_match();

        let up = Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Up,
            crossterm::event::KeyModifiers::NONE,
        ));
        term_event(up, &mut app, &mut input_box, &mut sidebar_data);
        assert_eq!(app.search_overlay.current_match, 0);
    }

    #[test]
    fn search_overlay_other_keys_ignored() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        let mut sidebar_data = SidebarData::new();

        dispatch_action(Action::SearchTranscript, &mut app, &mut input_box);
        assert!(app.search_overlay.visible);

        // F1 should be ignored (not close overlay, not exit)
        let f1 = Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::F(1),
            crossterm::event::KeyModifiers::NONE,
        ));
        let exited = term_event(f1, &mut app, &mut input_box, &mut sidebar_data);
        assert!(!exited);
        assert!(
            app.search_overlay.visible,
            "overlay should still be visible"
        );
    }

    #[test]
    fn render_frame_with_search_overlay_does_not_panic() {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");

        let mut app = AppState::new();
        app.push_user_message("hello world".to_owned(), None, vec![]);
        let mut input_box = InputBoxState::new();
        dispatch_action(Action::SearchTranscript, &mut app, &mut input_box);
        app.search_overlay.insert_char('h');
        app.search_overlay.search(&app.transcript);

        let sidebar_data = SidebarData::new();
        terminal
            .draw(|f| render_frame(f, &mut app, &input_box, &sidebar_data, 0))
            .expect("render with search overlay must not panic");
    }

    #[test]
    fn next_search_match_scrolls_transcript() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();

        // Add enough entries that scrolling matters
        for i in 0..10 {
            app.push_user_message(format!("hello message {i}"), None, vec![]);
        }

        app.search_overlay
            .open(crate::keybinds::KeybindPreset::default());
        for c in "hello".chars() {
            app.search_overlay.insert_char(c);
        }
        app.search_overlay.search(&app.transcript);
        assert!(app.search_overlay.match_count() > 1);

        // Advance to match at index 5
        for _ in 0..5 {
            dispatch_action(Action::NextSearchMatch, &mut app, &mut input_box);
        }

        // scroll_offset should reflect the transcript_index of the current match
        let current = app.search_overlay.current_match_info().unwrap();
        assert_eq!(app.scroll_offset, current.transcript_index);
    }

    // -----------------------------------------------------------------------
    // Tab cycles active agent (ISSUE tab-cycle-agents)
    // -----------------------------------------------------------------------

    #[test]
    fn tab_cycles_active_agent() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();

        // Default agent is "coder".
        assert_eq!(app.active_agent, "coder");

        // cyclable_agents() returns sorted Primary agents: coder, orchestrator, planner
        // (explore is Subagent and excluded).
        dispatch_action(Action::AcceptAutocomplete, &mut app, &mut input_box);
        assert_eq!(app.active_agent, "orchestrator");

        dispatch_action(Action::AcceptAutocomplete, &mut app, &mut input_box);
        assert_eq!(app.active_agent, "planner");

        dispatch_action(Action::AcceptAutocomplete, &mut app, &mut input_box);
        assert_eq!(app.active_agent, "coder"); // wraps around
    }

    #[test]
    fn tab_does_not_cycle_when_autocomplete_visible() {
        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();

        // Show autocomplete with a known entry.
        use crate::components::input::AutocompleteEntry;
        input_box.autocomplete.show(vec![AutocompleteEntry::new(
            "/connect",
            "Connect provider",
            "[builtin]",
        )]);
        assert!(input_box.autocomplete.visible);

        let before = app.active_agent.clone();
        dispatch_action(Action::AcceptAutocomplete, &mut app, &mut input_box);

        // Autocomplete was accepted, not agent cycling.
        assert_eq!(
            app.active_agent, before,
            "agent must not change when autocomplete is visible"
        );
        assert_eq!(input_box.content, "/connect ");
    }
}
