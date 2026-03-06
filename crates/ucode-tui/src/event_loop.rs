use std::time::{Duration, Instant};

use crossterm::event::{Event, EventStream, KeyEventKind, MouseEvent};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use futures_util::StreamExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time;

use crate::app::{AppState, FocusTarget, ToolCallStatus, TranscriptEntry};
use crate::components::input::InputBoxState;
use crate::components::sidebar::SidebarData;
use crate::components::status_bar::{StatusBar, StatusBarState};
use crate::components::title_bar::{TitleBar, TitleBarState};
use crate::components::transcript::TranscriptView;
use crate::keybinds::{Action, InputMode};
use crate::layout::compute_layout;

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
    },
    RouterEvent(String),
    SystemMessage(String),
    PatchProposed {
        file_path: String,
        raw_diff: String,
        patch_id: Option<String>,
    },
    ApprovalRequired {
        tool_name: String,
        command: String,
        cwd: String,
        sandbox_label: String,
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
        if self.mouse_enabled {
            let _ = execute!(
                stderr,
                crossterm::event::DisableMouseCapture,
                LeaveAlternateScreen
            );
        } else {
            let _ = execute!(stderr, LeaveAlternateScreen);
        }
    }
}

// ---------------------------------------------------------------------------
// run_event_loop
// ---------------------------------------------------------------------------

const RENDER_INTERVAL_MS: u64 = 16; // ~60 fps

/// Main async event loop. Blocks until the user exits or a `Quit` event arrives.
pub async fn run_event_loop(
    app: &mut AppState,
    input_box: &mut InputBoxState,
    sidebar_data: &mut SidebarData,
    mut event_rx: UnboundedReceiver<TuiEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    // --- Terminal setup ---
    enable_raw_mode()?;
    let mut stderr = std::io::stderr();
    execute!(stderr, EnterAlternateScreen)?;

    // Mouse capture is optional; we enable it but ignore mouse events for now.
    let mouse_enabled = execute!(stderr, crossterm::event::EnableMouseCapture).is_ok();

    // The guard restores the terminal on drop (including panics).
    let _guard = TerminalGuard::new(mouse_enabled);

    let backend = CrosstermBackend::new(std::io::stderr());
    let mut terminal = Terminal::new(backend)?;

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
                        if handle_terminal_event(event, app, input_box, sidebar_data) {
                            break;
                        }
                        // Drain any remaining buffered events from the channel
                        // so we batch multiple keystrokes into one render frame.
                        while let Ok(event) = term_rx.try_recv() {
                            if handle_terminal_event(event, app, input_box, sidebar_data) {
                                break;
                            }
                        }
                    }
                    None => break,
                }
            }

            // Render tick — only draw when dirty, streaming, or hint is active.
            _ = render_tick.tick(), if app.dirty || app.streaming || app.ctrl_c_hint.is_some() => {
                frame_counter = frame_counter.wrapping_add(1);
                // Expire the Ctrl+C hint once the 2-second window closes.
                if let Some(last) = app.last_ctrl_c
                    && last.elapsed().as_millis() >= 2000
                {
                    app.ctrl_c_hint = None;
                    app.last_ctrl_c = None;
                }
                terminal.draw(|f| render_frame(f, app, input_box, sidebar_data, frame_counter))?;
                app.dirty = false;
            }

            // Streaming tokens and system events from external callers.
            maybe_tui_event = event_rx.recv() => {
                match maybe_tui_event {
                    Some(tui_event) => {
                        if handle_tui_event(tui_event, app) {
                            break;
                        }
                    }
                    None => break,
                }
            }
        }
    }

    // Clean up the reader task.
    reader_handle.abort();

    Ok(())
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
) -> bool {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => {
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
                            app.push_system_message(format!("Executed: {}", cmd.name));
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
                        app.focus = FocusTarget::Input;
                        app.mark_dirty();
                    }
                    crossterm::event::KeyCode::Char('a') => {
                        let path = app.diff_modal.file_path.clone();
                        app.diff_modal.approve();
                        app.push_system_message(format!("Patch approved: {path}"));
                        app.focus = FocusTarget::Input;
                        app.mark_dirty();
                    }
                    crossterm::event::KeyCode::Char('r') => {
                        let path = app.diff_modal.file_path.clone();
                        app.diff_modal.reject();
                        app.push_system_message(format!("Patch rejected: {path}"));
                        app.focus = FocusTarget::Input;
                        app.mark_dirty();
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
                        app.approval_modal.close();
                        app.focus = FocusTarget::Input;
                        app.mark_dirty();
                    }
                    crossterm::event::KeyCode::Char('o') => {
                        let tool = app.approval_modal.tool_name.clone();
                        app.approval_modal.approve_once();
                        app.push_system_message(format!("Approved once: {tool}"));
                        app.focus = FocusTarget::Input;
                        app.mark_dirty();
                    }
                    crossterm::event::KeyCode::Char('s') => {
                        let tool = app.approval_modal.tool_name.clone();
                        app.approval_modal.approve_session();
                        app.push_system_message(format!("Approved for session: {tool}"));
                        app.focus = FocusTarget::Input;
                        app.mark_dirty();
                    }
                    crossterm::event::KeyCode::Char('d') => {
                        let tool = app.approval_modal.tool_name.clone();
                        app.approval_modal.deny();
                        app.push_system_message(format!("Denied: {tool}"));
                        app.focus = FocusTarget::Input;
                        app.mark_dirty();
                    }
                    _ => {}
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

                if is_bare_char || is_shift_char {
                    if let crossterm::event::KeyCode::Char(c) = key.code {
                        input_box.insert_char(c);
                        app.mark_dirty();
                    }
                    return false;
                }

                // Bare (no-modifier) editing keys go to input box.
                if key.modifiers == crossterm::event::KeyModifiers::NONE {
                    match key.code {
                        crossterm::event::KeyCode::Backspace => {
                            input_box.delete_char();
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Delete => {
                            input_box.delete_forward();
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Left => {
                            input_box.move_left();
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Right => {
                            input_box.move_right();
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Home => {
                            input_box.move_home();
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::End => {
                            input_box.move_end();
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Up => {
                            input_box.move_up();
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Down => {
                            input_box.move_down();
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
                            input_box.move_word_left();
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Right => {
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
                // Ctrl+C, Ctrl+P, Ctrl+L, Ctrl+O are intentionally excluded
                // so they still reach the keybind resolver.
                if key.modifiers == crossterm::event::KeyModifiers::CONTROL {
                    match key.code {
                        crossterm::event::KeyCode::Char('a') => {
                            input_box.move_home();
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Char('e') => {
                            input_box.move_end();
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Char('b') => {
                            input_box.move_left();
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Char('f') => {
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
                        crossterm::event::KeyCode::Char('y') => {
                            input_box.yank();
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Char('p') => {
                            input_box.move_up();
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Char('n') => {
                            input_box.move_down();
                            app.mark_dirty();
                            return false;
                        }
                        _ => {} // fall through to keybind resolver
                    }
                }

                if key.modifiers == crossterm::event::KeyModifiers::ALT {
                    match key.code {
                        crossterm::event::KeyCode::Char('b') => {
                            input_box.move_word_left();
                            app.mark_dirty();
                            return false;
                        }
                        crossterm::event::KeyCode::Char('f') => {
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

            // Keybind resolution for modified keys and non-input-focus contexts.
            if let Some(action) = app.keybinds.resolve(&key) {
                return dispatch_action(action, app, input_box);
            }
        }

        Event::Resize(w, h) => {
            app.handle_resize(w, h);
        }

        // Mouse events are captured but not yet handled.
        Event::Mouse(MouseEvent { .. }) => {}

        // Paste, focus, and other events are ignored.
        _ => {}
    }

    false
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
                app.push_user_message(content);
                // Callers are responsible for triggering the LLM; we just
                // record the message and start streaming state.
                app.start_streaming();
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
                // Double Ctrl+C within 2 seconds → exit.
                app.ctrl_c_hint = None;
                return true;
            }
            app.last_ctrl_c = Some(now);
            app.ctrl_c_hint = Some("Press Ctrl+C again to exit".into());
            // TODO: actually cancel active generation when we have one.
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
            use crate::theme::UcodeTheme;
            let next = app.theme.preset.next();
            app.theme = UcodeTheme::from_preset(next);
            app.mark_dirty();
        }

        Action::ToggleDensity => {
            app.density = app.density.next();
            app.mark_dirty();
        }

        // Future phases — no-op for now.
        _ => {}
    }

    false
}

// ---------------------------------------------------------------------------
// TuiEvent handler
// ---------------------------------------------------------------------------

/// Handle an event from the external channel. Returns `true` to exit the loop.
fn handle_tui_event(event: TuiEvent, app: &mut AppState) -> bool {
    match event {
        TuiEvent::StreamToken(token) => app.push_token(&token),
        TuiEvent::StreamDone => app.finalize_streaming(),
        TuiEvent::ToolCallStarted { name } => {
            app.push_tool_call(name);
        }
        TuiEvent::ToolCallCompleted {
            index,
            status,
            duration_ms,
            summary,
        } => {
            app.update_tool_call(index, status, duration_ms, summary);
        }
        TuiEvent::RouterEvent(msg) => app.push_router_event(msg),
        TuiEvent::SystemMessage(msg) => app.push_system_message(msg),
        TuiEvent::PatchProposed {
            file_path,
            raw_diff,
            patch_id,
        } => {
            app.propose_patch(file_path, raw_diff, patch_id);
        }
        TuiEvent::ApprovalRequired {
            tool_name,
            command,
            cwd,
            sandbox_label,
        } => {
            app.request_approval(tool_name, command, cwd, sandbox_label);
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
    app: &AppState,
    input_box: &InputBoxState,
    sidebar_data: &SidebarData,
    frame_counter: u64,
) {
    let area = f.area();
    let areas = compute_layout(area, &app.sidebar, &app.input);

    // Cursor blink: toggle every ~500ms (500 / 16 ≈ 31 frames).
    const BLINK_FRAMES: u64 = 31;
    let show_cursor = (frame_counter / BLINK_FRAMES).is_multiple_of(2);

    // Build today's date string for the title bar.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let date = format_date_from_secs(now.as_secs());

    // Title bar.
    let title_state = TitleBarState {
        session_title: app.session_title.clone(),
        session_id: app.session_id.clone(),
        parent_title: app.parent_title.clone(),
        date,
        in_multiplexer: app.multiplexer.clone(),
    };
    f.render_widget(TitleBar::new(&title_state, &app.theme), areas.title_bar);

    // Transcript.
    let transcript_widget = TranscriptView::new(
        &app.transcript,
        app.scroll_offset,
        app.auto_scroll,
        &app.theme,
        show_cursor,
    );
    f.render_widget(transcript_widget, areas.transcript);

    // Sidebar.
    use crate::components::sidebar::Sidebar;
    f.render_widget(
        Sidebar::new(sidebar_data, &app.theme, app.sidebar.mode),
        areas.sidebar,
    );

    // Input box.
    use crate::components::input::InputBox;
    f.render_widget(InputBox::new(input_box, &app.theme), areas.input);

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
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a `StatusBarState` from the current app and sidebar state.
///
/// Also responsible for expiring the Ctrl+C hint: if `last_ctrl_c` is older
/// than 2 seconds the hint is omitted (the caller holds `&AppState` so we
/// cannot mutate it here; expiry mutation happens in `render_frame`).
fn build_status_bar_state(app: &AppState, sidebar_data: &SidebarData) -> StatusBarState {
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

    // Show the hint only while still within the 2-second Ctrl+C window.
    let hint_message = app.ctrl_c_hint.clone().filter(|_| {
        app.last_ctrl_c
            .is_some_and(|t| t.elapsed().as_millis() < 2000)
    });

    StatusBarState {
        streaming: app.streaming,
        stream_tok_per_sec,
        hint_message,
        model_name: sidebar_data.router.model_name.clone(),
        model_group: sidebar_data.router.model_group,
        sandbox_tier: sidebar_data.router.sandbox_tier,
        tokens_used: sidebar_data.context.tokens_used.to_string(),
        tokens_max: format_token_count(sidebar_data.context.tokens_max),
        cost: format!("${:.4}", sidebar_data.context.cost_session),
        ..StatusBarState::default()
    }
}

/// Format a large token count as a human-readable string (e.g. "200k").
fn format_token_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.0}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.0}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// Format a Unix timestamp (seconds) as "YYYY-MM-DD".
///
/// This is a minimal implementation that avoids pulling in `chrono` just for
/// the title bar date. It handles leap years correctly for dates after 1970.
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

fn is_leap(year: u32) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

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
        };
        let _ = TuiEvent::RouterEvent("rerouted".to_owned());
        let _ = TuiEvent::SystemMessage("info".to_owned());
        let _ = TuiEvent::PatchProposed {
            file_path: "src/lib.rs".to_owned(),
            raw_diff: "+added".to_owned(),
            patch_id: None,
        };
        let _ = TuiEvent::ApprovalRequired {
            tool_name: "run_cmd".to_owned(),
            command: "cargo test".to_owned(),
            cwd: "/tmp".to_owned(),
            sandbox_label: "ws".to_owned(),
        };
        let _ = TuiEvent::Quit;
    }

    #[test]
    fn tui_event_approval_required() {
        let mut app = AppState::new();
        let exited = handle_tui_event(
            TuiEvent::ApprovalRequired {
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
        let app = AppState::new();
        let input_box = InputBoxState::new();
        let sidebar_data = SidebarData::new();

        terminal
            .draw(|f| render_frame(f, &app, &input_box, &sidebar_data, 0))
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

        let exited = handle_tui_event(TuiEvent::StreamToken("hello".to_owned()), &mut app);
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

        let exited = handle_tui_event(TuiEvent::StreamDone, &mut app);
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

    #[test]
    fn format_token_count_values() {
        assert_eq!(format_token_count(0), "0");
        assert_eq!(format_token_count(999), "999");
        assert_eq!(format_token_count(1_000), "1k");
        assert_eq!(format_token_count(200_000), "200k");
        assert_eq!(format_token_count(1_000_000), "1M");
        assert_eq!(format_token_count(2_000_000), "2M");
    }

    // -----------------------------------------------------------------------
    // render_frame with populated transcript
    // -----------------------------------------------------------------------

    #[test]
    fn render_frame_with_transcript() {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");

        let mut app = AppState::new();
        app.push_user_message("hello".to_owned());
        app.start_streaming();
        app.push_token("world");

        let input_box = InputBoxState::new();
        let sidebar_data = SidebarData::new();

        terminal
            .draw(|f| render_frame(f, &app, &input_box, &sidebar_data, 0))
            .expect("draw");
    }

    // -----------------------------------------------------------------------
    // dispatch_action scroll
    // -----------------------------------------------------------------------

    #[test]
    fn dispatch_action_scroll_up_disengages_auto_scroll() {
        let mut app = AppState::new();
        app.push_user_message("msg".to_owned());
        assert!(app.auto_scroll);

        let mut input_box = InputBoxState::new();
        dispatch_action(Action::ScrollUp, &mut app, &mut input_box);
        assert!(!app.auto_scroll);
    }

    #[test]
    fn dispatch_action_scroll_to_bottom_reengages() {
        let mut app = AppState::new();
        app.push_user_message("msg".to_owned());
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
        let exited = handle_tui_event(TuiEvent::Quit, &mut app);
        assert!(exited);
    }

    // -----------------------------------------------------------------------
    // handle_tui_event tool call lifecycle
    // -----------------------------------------------------------------------

    #[test]
    fn handle_tui_event_tool_call_lifecycle() {
        let mut app = AppState::new();

        handle_tui_event(
            TuiEvent::ToolCallStarted {
                name: "read_file".to_owned(),
            },
            &mut app,
        );
        assert_eq!(app.transcript.len(), 1);

        handle_tui_event(
            TuiEvent::ToolCallCompleted {
                index: 0,
                status: ToolCallStatus::Success,
                duration_ms: Some(100),
                summary: Some("read 5 lines".to_owned()),
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
        handle_terminal_event(event, &mut app, &mut input_box, &mut sidebar_data);
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
        handle_terminal_event(left, &mut app, &mut input_box, &mut sidebar_data);

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
        handle_terminal_event(home, &mut app, &mut input_box, &mut sidebar_data);
        input_box.insert_char('z');
        assert_eq!(input_box.content, "zabc");

        // End moves cursor to end.
        let end = Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::End,
            crossterm::event::KeyModifiers::NONE,
        ));
        handle_terminal_event(end, &mut app, &mut input_box, &mut sidebar_data);
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
        handle_terminal_event(esc, &mut app, &mut input_box, &mut sidebar_data);
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
            handle_terminal_event(ev, &mut app, &mut input_box, &mut sidebar_data);
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
        handle_terminal_event(enter, &mut app, &mut input_box, &mut sidebar_data);

        assert!(!app.palette.visible);
        let has_system_msg = app.transcript.iter().any(|e| {
            matches!(e, crate::app::TranscriptEntry::SystemMessage(msg) if msg.contains("Executed"))
        });
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
            .draw(|f| render_frame(f, &app, &input_box, &sidebar_data, 0))
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
        handle_terminal_event(ev, &mut app, &mut input_box, &mut sidebar_data);
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
        handle_terminal_event(ev, &mut app, &mut input_box, &mut sidebar_data);
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
        handle_terminal_event(ev, &mut app, &mut input_box, &mut sidebar_data);
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
        handle_terminal_event(ev, &mut app, &mut input_box, &mut sidebar_data);
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
        handle_terminal_event(ev, &mut app, &mut input_box, &mut sidebar_data);
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
        handle_terminal_event(ev, &mut app, &mut input_box, &mut sidebar_data);
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
        handle_terminal_event(ev, &mut app, &mut input_box, &mut sidebar_data);
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
        handle_terminal_event(ev, &mut app, &mut input_box, &mut sidebar_data);
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
        handle_terminal_event(ev, &mut app, &mut input_box, &mut sidebar_data);
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
        let exited = handle_terminal_event(ev, &mut app, &mut input_box, &mut sidebar_data);
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
        let exited = handle_terminal_event(ev, &mut app, &mut input_box, &mut sidebar_data);
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
        let exited = handle_terminal_event(ev, &mut app, &mut input_box, &mut sidebar_data);
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
        let exited = handle_terminal_event(ev, &mut app, &mut input_box, &mut sidebar_data);
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
        handle_terminal_event(ev, &mut app, &mut input_box, &mut sidebar_data);
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
        let first =
            handle_terminal_event(ctrl_c.clone(), &mut app, &mut input_box, &mut sidebar_data);
        assert!(!first, "first Ctrl+C should not exit");

        // Second press immediately after — should exit.
        let second = handle_terminal_event(ctrl_c, &mut app, &mut input_box, &mut sidebar_data);
        assert!(second, "second Ctrl+C within 2 s should exit");
    }

    // -----------------------------------------------------------------------
    // Diff modal integration
    // -----------------------------------------------------------------------

    #[test]
    fn tui_event_patch_proposed() {
        let mut app = AppState::new();
        let exited = handle_tui_event(
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
        handle_terminal_event(esc, &mut app, &mut input_box, &mut sidebar_data);
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
        handle_terminal_event(a_key, &mut app, &mut input_box, &mut sidebar_data);
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
        handle_terminal_event(r_key, &mut app, &mut input_box, &mut sidebar_data);
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
        handle_terminal_event(down, &mut app, &mut input_box, &mut sidebar_data);
        assert_eq!(app.diff_modal.scroll_offset, 1);

        // Up arrow scrolls back
        let up = Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Up,
            crossterm::event::KeyModifiers::NONE,
        ));
        handle_terminal_event(up, &mut app, &mut input_box, &mut sidebar_data);
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
            .draw(|f| render_frame(f, &app, &input_box, &sidebar_data, 0))
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
        handle_terminal_event(esc, &mut app, &mut input_box, &mut sidebar_data);
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
        handle_terminal_event(key_o, &mut app, &mut input_box, &mut sidebar_data);
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
        handle_terminal_event(key_s, &mut app, &mut input_box, &mut sidebar_data);
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
        handle_terminal_event(key_d, &mut app, &mut input_box, &mut sidebar_data);
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
            .draw(|f| render_frame(f, &app, &input_box, &sidebar_data, 0))
            .expect("draw");
    }

    // -----------------------------------------------------------------------
    // dispatch_action ToggleTheme / ToggleDensity
    // -----------------------------------------------------------------------

    #[test]
    fn dispatch_toggle_theme_cycles_preset() {
        use crate::theme::ThemePreset;

        let mut app = AppState::new();
        let mut input_box = InputBoxState::new();
        assert_eq!(app.theme.preset, ThemePreset::Hybrid);

        dispatch_action(Action::ToggleTheme, &mut app, &mut input_box);
        assert_eq!(app.theme.preset, ThemePreset::Dark);

        dispatch_action(Action::ToggleTheme, &mut app, &mut input_box);
        assert_eq!(app.theme.preset, ThemePreset::Light);

        dispatch_action(Action::ToggleTheme, &mut app, &mut input_box);
        assert_eq!(app.theme.preset, ThemePreset::Hybrid);
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
}
