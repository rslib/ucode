use std::time::Duration;

use crossterm::event::{Event, EventStream, KeyEventKind, MouseEvent};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use futures_util::StreamExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time;

use crate::app::{AppState, FocusTarget, ToolCallStatus};
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

    let mut event_stream = EventStream::new();
    let mut render_tick = time::interval(Duration::from_millis(RENDER_INTERVAL_MS));
    // Don't burst-render on startup.
    render_tick.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

    let mut frame_counter: u64 = 0;

    // Initial render.
    terminal.draw(|f| render_frame(f, app, input_box, sidebar_data, frame_counter))?;
    app.dirty = false;

    loop {
        tokio::select! {
            // Terminal input events.
            maybe_event = event_stream.next() => {
                match maybe_event {
                    Some(Ok(event)) => {
                        if handle_terminal_event(event, app, input_box, sidebar_data) {
                            break;
                        }
                    }
                    Some(Err(_)) => {
                        // I/O error on the terminal — exit cleanly.
                        break;
                    }
                    None => break,
                }
            }

            // Render tick — only draw when dirty or streaming.
            _ = render_tick.tick(), if app.dirty || app.streaming => {
                frame_counter = frame_counter.wrapping_add(1);
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
                    None => {
                        // Sender dropped — treat as quit.
                        break;
                    }
                }
            }
        }
    }

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
            if let Some(action) = app.keybinds.resolve(&key) {
                return dispatch_action(action, app, input_box);
            }

            // No bound action — forward printable chars to the input box when
            // focus is on the input area.
            if app.focus == FocusTarget::Input
                && let crossterm::event::KeyCode::Char(c) = key.code
            {
                input_box.insert_char(c);
                app.mark_dirty();
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
            // Signal is sent externally; here we just mark dirty so the UI
            // can reflect the cancellation once the stream ends.
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
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a `StatusBarState` from the current app and sidebar state.
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

    StatusBarState {
        streaming: app.streaming,
        stream_tok_per_sec,
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
        let _ = TuiEvent::Quit;
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
}
