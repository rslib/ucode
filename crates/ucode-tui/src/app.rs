use std::time::Instant;

use tokio::sync::mpsc::UnboundedSender;

use crate::command_registry::CommandRegistry;
use crate::components::input::AutocompleteEntry;
use crate::components::toast::{ToastLevel, ToastState};
use crate::keybinds::{KeybindPreset, KeybindResolver};
use crate::layout::{InputState, SidebarState, TerminalSize};
use crate::overlays::approval_modal::ApprovalModalState;
use crate::overlays::copy_mode::CopyModeState;
use crate::overlays::diff_modal::DiffModalState;
use crate::overlays::keybind_overlay::KeybindOverlayState;
use crate::overlays::overlay_queue::{
    ActiveOverlay, OverlayAction, OverlayNext, OverlayQueue, OverlayRequest,
};
use crate::overlays::palette::PaletteState;
use crate::overlays::search_overlay::SearchOverlayState;
use crate::theme::{Density, UcodeTheme};

// ---------------------------------------------------------------------------
// ToolCallStatus
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolCallStatus {
    Running,
    Success,
    Failed,
    PendingApproval,
}

// ---------------------------------------------------------------------------
// StreamingMessage
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct StreamingMessage {
    pub content: String,
    pub started_at: Instant,
    pub token_count: usize,
    pub is_complete: bool,
}

impl StreamingMessage {
    pub fn new() -> Self {
        Self {
            content: String::new(),
            started_at: Instant::now(),
            token_count: 0,
            is_complete: false,
        }
    }

    /// Append `token` directly to content and increment the counter.
    pub fn push_token(&mut self, token: &str) {
        self.content.push_str(token);
        self.token_count += 1;
    }

    pub fn elapsed_secs(&self) -> f64 {
        self.started_at.elapsed().as_secs_f64()
    }

    /// Tokens per second since the message started. Returns 0.0 if elapsed is
    /// effectively zero to avoid division by zero.
    pub fn tokens_per_sec(&self) -> f64 {
        let elapsed = self.elapsed_secs();
        if elapsed < f64::EPSILON {
            0.0
        } else {
            self.token_count as f64 / elapsed
        }
    }

    pub fn finalize(&mut self) {
        self.is_complete = true;
    }
}

impl Default for StreamingMessage {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// TranscriptEntry
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum TranscriptEntry {
    UserMessage(String),
    /// Completed assistant message.
    AssistantMessage(String),
    /// In-progress assistant message.
    Streaming(StreamingMessage),
    ToolCall {
        name: String,
        status: ToolCallStatus,
        duration_ms: Option<u64>,
        summary: Option<String>,
        thinking: Option<String>,
        output: Option<String>,
    },
    /// e.g. "rate-limit on anthropic -> openai/gpt-4o"
    RouterEvent(String),
    /// Info/warning from the system.
    SystemMessage(String),
    PatchProposed {
        file_path: String,
        raw_diff: String,
        patch_id: Option<String>,
    },
}

// Manual PartialEq for StreamingMessage so TranscriptEntry can derive it.
// Two StreamingMessages are equal when their content, token_count, and
// is_complete match; we deliberately ignore started_at (an Instant).
impl PartialEq for StreamingMessage {
    fn eq(&self, other: &Self) -> bool {
        self.content == other.content
            && self.token_count == other.token_count
            && self.is_complete == other.is_complete
    }
}

// ---------------------------------------------------------------------------
// FocusTarget
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FocusTarget {
    #[default]
    Input,
    Transcript,
    Sidebar,
    Overlay,
}

// ---------------------------------------------------------------------------
// Multiplexer detection
// ---------------------------------------------------------------------------

/// Detect the name of the running terminal multiplexer from environment
/// variables. Returns `None` when not inside a known multiplexer.
fn detect_multiplexer() -> Option<String> {
    // TMUX is set by tmux itself.
    if std::env::var("TMUX").is_ok() {
        return Some("tmux".to_owned());
    }
    // Zellij sets ZELLIJ.
    if std::env::var("ZELLIJ").is_ok() {
        return Some("zellij".to_owned());
    }
    // GNU Screen sets STY.
    if std::env::var("STY").is_ok() {
        return Some("screen".to_owned());
    }
    None
}

// ---------------------------------------------------------------------------
// AppState
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct AppState {
    pub theme: UcodeTheme,
    pub density: Density,
    pub sidebar: SidebarState,
    pub input: InputState,
    pub keybinds: KeybindResolver,
    pub transcript: Vec<TranscriptEntry>,
    pub scroll_offset: usize,
    /// When true the view is pinned to the bottom of the transcript.
    pub auto_scroll: bool,
    pub focus: FocusTarget,
    /// Set whenever state changes; cleared by the render loop after drawing.
    pub dirty: bool,
    /// True while a streaming response is in progress.
    pub streaming: bool,
    pub terminal_size: TerminalSize,
    pub session_title: String,
    pub session_id: String,
    pub parent_title: Option<String>,
    /// Detected multiplexer name (e.g. "tmux", "zellij", "screen").
    pub multiplexer: Option<String>,
    /// Optional channel to notify external systems when the user sends a message.
    /// The real CLI uses this to forward prompts to the LLM.
    #[allow(clippy::type_complexity)]
    pub message_tx: Option<UnboundedSender<String>>,
    pub command_registry: CommandRegistry,
    pub palette: PaletteState,
    pub diff_modal: DiffModalState,
    pub approval_modal: ApprovalModalState,
    pub keybind_overlay: KeybindOverlayState,
    pub search_overlay: SearchOverlayState,
    pub copy_mode: CopyModeState,
    pub overlay_queue: OverlayQueue,
    /// Timestamp of the last Ctrl+C press, for double-Ctrl+C exit detection.
    pub last_ctrl_c: Option<Instant>,
    /// Transient hint set after the first Ctrl+C; cleared when the 2-second
    /// window expires or when the double-Ctrl+C exit fires.
    pub ctrl_c_hint: Option<String>,
    pub toasts: ToastState,
}

impl AppState {
    pub fn new() -> Self {
        let terminal_size = TerminalSize {
            width: 120,
            height: 40,
        };
        let sidebar = SidebarState::new(terminal_size.sidebar_mode());

        let command_registry = CommandRegistry::with_builtins();
        let palette = PaletteState::from_registry(&command_registry);

        Self {
            theme: UcodeTheme::default(),
            density: Density::default(),
            sidebar,
            input: InputState::default(),
            keybinds: KeybindResolver::new(KeybindPreset::default()),
            transcript: Vec::new(),
            scroll_offset: 0,
            auto_scroll: true,
            focus: FocusTarget::default(),
            dirty: true,
            streaming: false,
            terminal_size,
            session_title: String::new(),
            session_id: String::new(),
            parent_title: None,
            multiplexer: detect_multiplexer(),
            message_tx: None,
            command_registry,
            palette,
            diff_modal: DiffModalState::new(),
            approval_modal: ApprovalModalState::new(),
            keybind_overlay: KeybindOverlayState::new(),
            search_overlay: SearchOverlayState::new(),
            copy_mode: CopyModeState::new(),
            overlay_queue: OverlayQueue::new(),
            last_ctrl_c: None,
            ctrl_c_hint: None,
            toasts: ToastState::new(),
        }
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Query the command registry for commands matching the current input prefix
    /// and return autocomplete entries.
    pub fn slash_completions(&self, input: &str) -> Vec<AutocompleteEntry> {
        let query = input.strip_prefix('/').unwrap_or(input);
        self.command_registry
            .search(query)
            .into_iter()
            .map(|cmd| {
                let entry = AutocompleteEntry::new(&cmd.name, &cmd.description, cmd.source.badge());
                if let Some(hint) = &cmd.args_hint {
                    entry.with_args_hint(hint)
                } else {
                    entry
                }
            })
            .collect()
    }

    // ------------------------------------------------------------------
    // Streaming
    // ------------------------------------------------------------------

    /// Push a new [`TranscriptEntry::Streaming`] and enter streaming mode.
    pub fn start_streaming(&mut self) {
        self.transcript
            .push(TranscriptEntry::Streaming(StreamingMessage::new()));
        self.streaming = true;
        self.mark_dirty();
    }

    /// Append `token` to the active streaming entry.
    ///
    /// If the last entry is not a `Streaming` variant a new one is created
    /// (defensive: callers should always call `start_streaming` first).
    pub fn push_token(&mut self, token: &str) {
        match self.transcript.last_mut() {
            Some(TranscriptEntry::Streaming(msg)) => {
                msg.push_token(token);
            }
            _ => {
                let mut msg = StreamingMessage::new();
                msg.push_token(token);
                self.transcript.push(TranscriptEntry::Streaming(msg));
                self.streaming = true;
            }
        }

        if self.auto_scroll {
            // Keep scroll_offset pointing past the end so the renderer always
            // shows the latest content.  The renderer clamps to the real max.
            self.scroll_offset = self.scroll_offset.saturating_add(1);
        }

        self.mark_dirty();
    }

    /// Convert the last `Streaming` entry into a completed `AssistantMessage`.
    pub fn finalize_streaming(&mut self) {
        if let Some(entry) = self.transcript.last_mut()
            && let TranscriptEntry::Streaming(msg) = entry
        {
            let content = msg.content.clone();
            *entry = TranscriptEntry::AssistantMessage(content);
        }
        self.streaming = false;
        self.mark_dirty();
    }

    // ------------------------------------------------------------------
    // Transcript mutations
    // ------------------------------------------------------------------

    pub fn push_user_message(&mut self, msg: String) {
        if let Some(tx) = &self.message_tx {
            let _ = tx.send(msg.clone());
        }
        self.transcript.push(TranscriptEntry::UserMessage(msg));
        self.mark_dirty();
    }

    /// Push a new `ToolCall` with `Running` status and return its index.
    pub fn push_tool_call(&mut self, name: String) -> usize {
        let index = self.transcript.len();
        self.transcript.push(TranscriptEntry::ToolCall {
            name,
            status: ToolCallStatus::Running,
            duration_ms: None,
            summary: None,
            thinking: None,
            output: None,
        });
        self.mark_dirty();
        index
    }

    /// Update an existing tool call entry by index.
    ///
    /// Silently does nothing if `index` is out of bounds or does not point to
    /// a `ToolCall` entry.
    pub fn update_tool_call(
        &mut self,
        index: usize,
        status: ToolCallStatus,
        duration_ms: Option<u64>,
        summary: Option<String>,
        thinking: Option<String>,
        output: Option<String>,
    ) {
        if let Some(TranscriptEntry::ToolCall {
            status: s,
            duration_ms: d,
            summary: sum,
            thinking: th,
            output: out,
            ..
        }) = self.transcript.get_mut(index)
        {
            *s = status;
            *d = duration_ms;
            *sum = summary;
            *th = thinking;
            *out = output;
            self.dirty = true;
        }
    }

    pub fn push_router_event(&mut self, msg: String) {
        self.transcript.push(TranscriptEntry::RouterEvent(msg));
        self.mark_dirty();
    }

    pub fn push_system_message(&mut self, msg: String) {
        self.transcript.push(TranscriptEntry::SystemMessage(msg));
        self.mark_dirty();
    }

    /// Execute a slash command by name with arguments.
    ///
    /// `name` may include or omit the leading `/`; the registry stores names
    /// with the `/` prefix so we normalise before lookup.
    ///
    /// Multi-word registry names (e.g. `/session rename`) are matched by
    /// greedily consuming leading args: we try `/name arg0`, then `/name arg0
    /// arg1`, etc., before falling back to `/name` alone.
    ///
    /// Returns `true` if the command was found and executed, `false` otherwise.
    pub fn execute_command(&mut self, name: &str, args: &[String]) -> bool {
        let bare_name = name.trim_start_matches('/');

        // Build candidate lookups from most-specific to least-specific.
        // e.g. name="session", args=["rename","foo"] → tries:
        //   "/session rename foo", "/session rename", "/session"
        let mut candidates: Vec<(String, &[String])> = Vec::new();
        for n in (0..=args.len()).rev() {
            let full = if n == 0 {
                format!("/{bare_name}")
            } else {
                format!("/{bare_name} {}", args[..n].join(" "))
            };
            candidates.push((full, &args[n..]));
        }

        // Resolve and clone the needed fields before any mutable borrow.
        let resolved = candidates.iter().find_map(|(lookup, remaining_args)| {
            self.command_registry
                .resolve(lookup)
                .map(|cmd| (cmd.name.clone(), cmd.action.is_some(), *remaining_args))
        });

        if let Some((cmd_name, has_action, remaining_args)) = resolved {
            let bare = cmd_name.trim_start_matches('/');
            let args_str = if remaining_args.is_empty() {
                String::new()
            } else {
                format!(" {}", remaining_args.join(" "))
            };
            self.push_system_message(format!("/{bare}{args_str}"));

            if has_action {
                self.push_system_message(format!("Executed: {bare}"));
            } else {
                self.push_system_message(format!("Command {cmd_name} is not yet implemented"));
            }
            true
        } else {
            let suggestions: Vec<String> = self
                .command_registry
                .suggest(bare_name)
                .into_iter()
                .map(|c| c.name.clone())
                .collect();
            let lookup = format!("/{bare_name}");
            if suggestions.is_empty() {
                self.push_system_message(format!("Unknown command: {lookup}"));
            } else {
                self.push_system_message(format!(
                    "Unknown command: {lookup}. Did you mean: {}?",
                    suggestions.join(", ")
                ));
            }
            false
        }
    }

    /// Push a patch proposal to the transcript and submit through the overlay queue.
    pub fn propose_patch(&mut self, file_path: String, raw_diff: String, patch_id: Option<String>) {
        self.transcript.push(TranscriptEntry::PatchProposed {
            file_path: file_path.clone(),
            raw_diff: raw_diff.clone(),
            patch_id: patch_id.clone(),
        });
        self.submit_overlay(OverlayRequest::Diff {
            file_path,
            raw_diff,
            patch_id,
        });
    }

    /// Push a tool call with `PendingApproval` status and submit through the overlay queue.
    pub fn request_approval(
        &mut self,
        tool_name: String,
        command: String,
        cwd: String,
        sandbox_label: String,
    ) {
        let index = self.push_tool_call(tool_name.clone());
        self.update_tool_call(
            index,
            ToolCallStatus::PendingApproval,
            None,
            None,
            None,
            None,
        );
        self.submit_overlay(OverlayRequest::Approval {
            tool_name,
            command,
            cwd,
            sandbox_label,
            tool_call_index: Some(index),
        });
    }

    /// Open an overlay request on the appropriate modal state struct.
    fn open_overlay(&mut self, request: OverlayRequest) {
        match request {
            OverlayRequest::Approval {
                tool_name,
                command,
                cwd,
                sandbox_label,
                tool_call_index,
            } => {
                self.approval_modal.open_run_cmd(
                    tool_name,
                    command,
                    cwd,
                    sandbox_label,
                    tool_call_index,
                );
            }
            OverlayRequest::Diff {
                file_path,
                raw_diff,
                patch_id,
            } => {
                self.diff_modal.open(file_path, &raw_diff, patch_id);
            }
        }
        self.focus = FocusTarget::Overlay;
    }

    /// Suspend the currently active overlay (hide it without losing state).
    fn suspend_active_overlay(&mut self) {
        if self.diff_modal.visible {
            self.diff_modal.visible = false;
        }
        if self.approval_modal.visible {
            self.approval_modal.visible = false;
        }
    }

    /// Resume a suspended overlay.
    fn resume_overlay(&mut self, overlay: ActiveOverlay) {
        match overlay {
            ActiveOverlay::Diff => {
                self.diff_modal.visible = true;
            }
            ActiveOverlay::Approval => {
                self.approval_modal.visible = true;
            }
        }
        self.focus = FocusTarget::Overlay;
    }

    /// Submit an overlay request through the queue. Handles open/preempt/queue.
    pub fn submit_overlay(&mut self, request: OverlayRequest) {
        let action = self.overlay_queue.submit(request);
        match action {
            OverlayAction::Open(req) => {
                self.open_overlay(req);
                self.mark_dirty();
            }
            OverlayAction::Preempt(req) => {
                self.suspend_active_overlay();
                self.open_overlay(req);
                self.mark_dirty();
            }
            OverlayAction::Queued => {
                self.mark_dirty();
            }
        }
    }

    /// Advance the overlay queue after the current overlay is dismissed.
    /// Call this after closing any system-initiated modal.
    pub fn advance_overlay_queue(&mut self) {
        match self.overlay_queue.dismiss_active() {
            Some(OverlayNext::Open(req)) => {
                self.open_overlay(req);
                self.mark_dirty();
            }
            Some(OverlayNext::Resume(overlay)) => {
                self.resume_overlay(overlay);
                self.mark_dirty();
            }
            None => {
                self.focus = FocusTarget::Input;
                self.mark_dirty();
            }
        }
    }

    // ------------------------------------------------------------------
    // Scroll
    // ------------------------------------------------------------------

    /// Scroll up by `lines`, disengaging auto-scroll.
    pub fn scroll_up(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
        self.auto_scroll = false;
        self.mark_dirty();
    }

    /// Scroll down by `lines`. Re-engages auto-scroll when at the bottom.
    pub fn scroll_down(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_add(lines);
        // We treat "at bottom" as scroll_offset >= transcript length.
        // The renderer is responsible for clamping to the real maximum; here
        // we just check whether we've reached or passed the end.
        if self.scroll_offset >= self.transcript.len() {
            self.auto_scroll = true;
        }
        self.mark_dirty();
    }

    /// Jump to the bottom and re-engage auto-scroll.
    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = self.transcript.len();
        self.auto_scroll = true;
        self.mark_dirty();
    }

    // ------------------------------------------------------------------
    // Toasts
    // ------------------------------------------------------------------

    pub fn toast(&mut self, level: ToastLevel, title: impl Into<String>) {
        self.toasts.push(level, title);
        self.mark_dirty();
    }

    pub fn toast_with_body(
        &mut self,
        level: ToastLevel,
        title: impl Into<String>,
        body: impl Into<String>,
    ) {
        self.toasts.push_with_body(level, title, body);
        self.mark_dirty();
    }

    // ------------------------------------------------------------------
    // Plugin command registration
    // ------------------------------------------------------------------

    /// Register a command from a plugin in the command registry and refresh the palette.
    pub fn register_plugin_command(&mut self, plugin_name: &str, name: &str, description: &str) {
        use crate::command_registry::{CommandCategory, CommandDef, CommandSource};
        self.command_registry.register(CommandDef {
            name: name.to_owned(),
            description: description.to_owned(),
            category: CommandCategory::Plugins,
            source: CommandSource::Plugin(plugin_name.to_owned()),
            args_hint: None,
            action: None,
        });
        self.palette =
            crate::overlays::palette::PaletteState::from_registry(&self.command_registry);
    }

    /// Remove all commands registered by `plugin_name` and refresh the palette.
    pub fn remove_plugin_commands(&mut self, plugin_name: &str) {
        self.command_registry.remove_by_source_name(plugin_name);
        self.palette =
            crate::overlays::palette::PaletteState::from_registry(&self.command_registry);
    }

    // ------------------------------------------------------------------
    // Plugin UI lifecycle
    // ------------------------------------------------------------------

    /// Called when a plugin session starts. Currently a no-op placeholder;
    /// plugins register their UI elements individually.
    pub fn plugin_session_start(&mut self, _plugin_name: &str) {}

    /// Called when a plugin session ends. Cleans up all palette commands for
    /// the plugin. Sidebar sections must be cleaned up separately via
    /// `SidebarData::remove_plugin_sections`.
    pub fn plugin_session_end(&mut self, plugin_name: &str) {
        self.remove_plugin_commands(plugin_name);
    }

    // ------------------------------------------------------------------
    // Resize
    // ------------------------------------------------------------------

    pub fn handle_resize(&mut self, width: u16, height: u16) {
        self.terminal_size = TerminalSize { width, height };
        self.sidebar.mode = self.terminal_size.sidebar_mode();
        self.mark_dirty();
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlays::overlay_queue::ActiveOverlay;

    #[test]
    fn streaming_message_push_token() {
        let mut msg = StreamingMessage::new();
        msg.push_token("Hello");
        msg.push_token(", ");
        msg.push_token("world");
        assert_eq!(msg.content, "Hello, world");
        assert_eq!(msg.token_count, 3);
    }

    #[test]
    fn streaming_message_tokens_per_sec() {
        let mut msg = StreamingMessage::new();
        // Push enough tokens that elapsed > 0 is virtually guaranteed.
        for _ in 0..10 {
            msg.push_token("tok");
        }
        // Spin briefly so elapsed_secs() is non-zero.
        let start = Instant::now();
        while start.elapsed().as_nanos() == 0 {}

        // tok/s should be positive (we can't assert an exact value).
        assert!(msg.tokens_per_sec() >= 0.0);
        assert_eq!(msg.token_count, 10);
    }

    #[test]
    fn streaming_message_finalize() {
        let mut msg = StreamingMessage::new();
        assert!(!msg.is_complete);
        msg.finalize();
        assert!(msg.is_complete);
    }

    #[test]
    fn app_state_new_defaults() {
        let app = AppState::new();
        assert!(app.auto_scroll);
        assert!(app.dirty);
        assert!(!app.streaming);
        assert_eq!(app.focus, FocusTarget::Input);
        assert_eq!(app.scroll_offset, 0);
        assert!(app.transcript.is_empty());
    }

    #[test]
    fn app_state_push_user_message() {
        let mut app = AppState::new();
        app.push_user_message("hello".to_owned());
        assert_eq!(app.transcript.len(), 1);
        assert_eq!(
            app.transcript[0],
            TranscriptEntry::UserMessage("hello".to_owned())
        );
    }

    #[test]
    fn app_state_streaming_lifecycle() {
        let mut app = AppState::new();
        app.start_streaming();
        assert!(app.streaming);
        assert_eq!(app.transcript.len(), 1);
        assert!(matches!(app.transcript[0], TranscriptEntry::Streaming(_)));

        app.push_token("Hello");
        app.push_token(" ");
        app.push_token("world");

        app.finalize_streaming();
        assert!(!app.streaming);
        assert_eq!(app.transcript.len(), 1);
        assert_eq!(
            app.transcript[0],
            TranscriptEntry::AssistantMessage("Hello world".to_owned())
        );
    }

    #[test]
    fn app_state_scroll_disengages_auto_scroll() {
        let mut app = AppState::new();
        app.push_user_message("msg".to_owned());
        assert!(app.auto_scroll);
        app.scroll_up(1);
        assert!(!app.auto_scroll);
    }

    #[test]
    fn app_state_scroll_to_bottom_reengages() {
        let mut app = AppState::new();
        app.push_user_message("msg".to_owned());
        app.scroll_up(1);
        assert!(!app.auto_scroll);
        app.scroll_to_bottom();
        assert!(app.auto_scroll);
    }

    #[test]
    fn app_state_push_tool_call() {
        let mut app = AppState::new();
        let idx = app.push_tool_call("read_file".to_owned());
        assert_eq!(idx, 0);
        match &app.transcript[0] {
            TranscriptEntry::ToolCall { name, status, .. } => {
                assert_eq!(name, "read_file");
                assert_eq!(*status, ToolCallStatus::Running);
            }
            _ => panic!("expected ToolCall"),
        }
    }

    #[test]
    fn app_state_update_tool_call() {
        let mut app = AppState::new();
        let idx = app.push_tool_call("write_file".to_owned());
        app.update_tool_call(
            idx,
            ToolCallStatus::Success,
            Some(42),
            Some("wrote 3 lines".to_owned()),
            None,
            None,
        );
        match &app.transcript[idx] {
            TranscriptEntry::ToolCall {
                status,
                duration_ms,
                summary,
                ..
            } => {
                assert_eq!(*status, ToolCallStatus::Success);
                assert_eq!(*duration_ms, Some(42));
                assert_eq!(summary.as_deref(), Some("wrote 3 lines"));
            }
            _ => panic!("expected ToolCall"),
        }
    }

    #[test]
    fn app_state_propose_patch() {
        let mut app = AppState::new();
        app.propose_patch(
            "src/lib.rs".to_owned(),
            "+added".to_owned(),
            Some("p1".to_owned()),
        );
        assert!(app.diff_modal.visible);
        assert_eq!(app.diff_modal.file_path, "src/lib.rs");
        assert_eq!(app.focus, FocusTarget::Overlay);
        assert_eq!(app.transcript.len(), 1);
        assert!(matches!(
            app.transcript[0],
            TranscriptEntry::PatchProposed { .. }
        ));
    }

    #[test]
    fn app_state_request_approval() {
        let mut app = AppState::new();
        app.request_approval(
            "run_cmd".to_owned(),
            "cargo test --workspace".to_owned(),
            "/home/user/code/ucode".to_owned(),
            "ws workspace".to_owned(),
        );
        assert!(app.approval_modal.visible);
        assert_eq!(app.approval_modal.tool_name, "run_cmd");
        assert_eq!(app.focus, FocusTarget::Overlay);
        assert_eq!(app.transcript.len(), 1);
        match &app.transcript[0] {
            TranscriptEntry::ToolCall { name, status, .. } => {
                assert_eq!(name, "run_cmd");
                assert_eq!(*status, ToolCallStatus::PendingApproval);
            }
            _ => panic!("expected ToolCall"),
        }
    }

    #[test]
    fn update_tool_call_with_thinking_and_output() {
        let mut app = AppState::new();
        let idx = app.push_tool_call("search".to_owned());
        app.update_tool_call(
            idx,
            ToolCallStatus::Success,
            Some(150),
            Some("query=foo".to_owned()),
            Some("Let me search for foo".to_owned()),
            Some("Found 5 matches".to_owned()),
        );
        match &app.transcript[idx] {
            TranscriptEntry::ToolCall {
                thinking, output, ..
            } => {
                assert_eq!(thinking.as_deref(), Some("Let me search for foo"));
                assert_eq!(output.as_deref(), Some("Found 5 matches"));
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    #[test]
    fn propose_patch_uses_overlay_queue() {
        let mut app = AppState::new();
        app.propose_patch("f.rs".to_owned(), "+line".to_owned(), None);
        assert!(app.diff_modal.visible);
        assert_eq!(app.overlay_queue.active(), Some(ActiveOverlay::Diff));
    }

    #[test]
    fn request_approval_preempts_diff() {
        let mut app = AppState::new();
        app.propose_patch("f.rs".to_owned(), "+line".to_owned(), None);
        assert!(app.diff_modal.visible);

        app.request_approval(
            "run_cmd".to_owned(),
            "cargo test".to_owned(),
            "/tmp".to_owned(),
            "workspace".to_owned(),
        );
        // Diff should be suspended (invisible), approval should be active
        assert!(!app.diff_modal.visible);
        assert!(app.approval_modal.visible);
    }

    #[test]
    fn advance_queue_resumes_after_preemption() {
        let mut app = AppState::new();
        app.propose_patch("f.rs".to_owned(), "+line".to_owned(), None);
        app.request_approval(
            "run_cmd".to_owned(),
            "cargo test".to_owned(),
            "/tmp".to_owned(),
            "workspace".to_owned(),
        );

        // Dismiss approval
        app.approval_modal.close();
        app.advance_overlay_queue();

        // Diff should resume
        assert!(app.diff_modal.visible);
        assert!(!app.approval_modal.visible);
        assert_eq!(app.focus, FocusTarget::Overlay);
    }

    #[test]
    fn advance_queue_returns_to_input_when_empty() {
        let mut app = AppState::new();
        app.propose_patch("f.rs".to_owned(), "+line".to_owned(), None);

        app.diff_modal.close();
        app.advance_overlay_queue();

        assert!(!app.diff_modal.visible);
        assert_eq!(app.focus, FocusTarget::Input);
    }

    // ------------------------------------------------------------------
    // execute_command
    // ------------------------------------------------------------------

    #[test]
    fn test_execute_command_known() {
        let mut app = AppState::new();
        let result = app.execute_command("connect", &[]);
        assert!(result, "known command should return true");
        // Should have two system messages: the echo and the "not yet implemented" note.
        let sys_msgs: Vec<_> = app
            .transcript
            .iter()
            .filter_map(|e| {
                if let TranscriptEntry::SystemMessage(m) = e {
                    Some(m.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert!(
            sys_msgs.iter().any(|m| m.contains("connect")),
            "transcript should mention the command name"
        );
    }

    #[test]
    fn test_execute_command_unknown() {
        let mut app = AppState::new();
        let result = app.execute_command("nonexistent", &[]);
        assert!(!result, "unknown command should return false");
        let has_unknown = app.transcript.iter().any(
            |e| matches!(e, TranscriptEntry::SystemMessage(m) if m.contains("Unknown command")),
        );
        assert!(has_unknown, "should emit Unknown command message");
    }

    #[test]
    fn test_execute_command_with_slash_prefix() {
        let mut app = AppState::new();
        // With leading slash — same as without.
        let result = app.execute_command("/connect", &[]);
        assert!(result, "execute_command should accept names with leading /");
    }

    #[test]
    fn test_execute_command_unknown_shows_suggestions() {
        let mut app = AppState::new();
        // "conect" is close enough to "/connect" to get a suggestion.
        let result = app.execute_command("conect", &[]);
        assert!(!result);
        let has_suggestion = app
            .transcript
            .iter()
            .any(|e| matches!(e, TranscriptEntry::SystemMessage(m) if m.contains("Did you mean")));
        assert!(has_suggestion, "should suggest similar commands");
    }

    #[test]
    fn test_execute_command_with_args() {
        let mut app = AppState::new();
        let args = vec!["my-session".to_owned()];
        // "/session rename" is a known command.
        let result = app.execute_command("session rename", &args);
        assert!(result);
        let has_args = app
            .transcript
            .iter()
            .any(|e| matches!(e, TranscriptEntry::SystemMessage(m) if m.contains("my-session")));
        assert!(has_args, "args should appear in the echo message");
    }

    #[test]
    fn app_state_dirty_flag() {
        let mut app = AppState::new();
        // new() sets dirty = true
        assert!(app.dirty);

        // Simulate render loop clearing the flag.
        app.dirty = false;
        assert!(!app.dirty);

        // Any mutation re-sets it.
        app.push_user_message("hi".to_owned());
        assert!(app.dirty);

        app.dirty = false;
        app.start_streaming();
        assert!(app.dirty);

        app.dirty = false;
        app.push_token("tok");
        assert!(app.dirty);

        app.dirty = false;
        app.finalize_streaming();
        assert!(app.dirty);

        app.dirty = false;
        app.scroll_up(1);
        assert!(app.dirty);

        app.dirty = false;
        app.scroll_to_bottom();
        assert!(app.dirty);
    }

    // ------------------------------------------------------------------
    // Plugin command registration
    // ------------------------------------------------------------------

    #[test]
    fn register_plugin_command_adds_to_registry_and_palette() {
        let mut app = AppState::new();
        let initial_count = app.command_registry.list().len();
        app.register_plugin_command("code-analyzer", "/analyze", "Run code analysis");
        assert_eq!(app.command_registry.list().len(), initial_count + 1);
        assert!(app.command_registry.resolve("/analyze").is_some());
        // Palette should be refreshed.
        assert_eq!(app.palette.commands.len(), initial_count + 1);
    }

    #[test]
    fn register_plugin_command_uses_plugin_source() {
        let mut app = AppState::new();
        app.register_plugin_command("my-plugin", "/my-cmd", "My command");
        let cmd = app.command_registry.resolve("/my-cmd").unwrap();
        assert_eq!(
            cmd.source,
            crate::command_registry::CommandSource::Plugin("my-plugin".to_owned())
        );
    }

    #[test]
    fn remove_plugin_commands_removes_from_registry_and_palette() {
        let mut app = AppState::new();
        app.register_plugin_command("code-analyzer", "/analyze", "Run analysis");
        app.register_plugin_command("code-analyzer", "/report", "Generate report");
        app.register_plugin_command("other-plugin", "/other", "Other command");
        let before = app.command_registry.list().len();
        app.remove_plugin_commands("code-analyzer");
        assert_eq!(app.command_registry.list().len(), before - 2);
        assert!(app.command_registry.resolve("/analyze").is_none());
        assert!(app.command_registry.resolve("/report").is_none());
        assert!(app.command_registry.resolve("/other").is_some());
        // Palette should be refreshed.
        assert_eq!(
            app.palette.commands.len(),
            app.command_registry.list().len()
        );
    }

    #[test]
    fn plugin_session_start_is_noop() {
        let mut app = AppState::new();
        let before_count = app.command_registry.list().len();
        app.plugin_session_start("my-plugin");
        // No side effects.
        assert_eq!(app.command_registry.list().len(), before_count);
    }

    #[test]
    fn plugin_session_end_removes_commands() {
        let mut app = AppState::new();
        app.register_plugin_command("my-plugin", "/cmd1", "Command 1");
        app.register_plugin_command("my-plugin", "/cmd2", "Command 2");
        let before = app.command_registry.list().len();
        app.plugin_session_end("my-plugin");
        assert_eq!(app.command_registry.list().len(), before - 2);
        assert!(app.command_registry.resolve("/cmd1").is_none());
        assert!(app.command_registry.resolve("/cmd2").is_none());
    }
}
