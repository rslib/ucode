//! Interactive demo: launch the TUI with a scripted AI coding assistant interaction.
//!
//! Run with: cargo run -p ucode-tui --example demo
//!
//! Type a message and press Enter to trigger a simulated AI interaction that
//! showcases all TUI features: tool calls (with thinking/output), router events,
//! system messages, patch proposals, approval modals, streaming responses,
//! system-triggered toasts, search overlay, keybind overlay, and selection mode.
//!
//! - First message: tool calls + streaming response + checkpoint/agent toasts
//! - Second message: failed tool, approval modal, patch proposal + budget/failure toasts
//! - Third+ messages: simple echo
//!
//! F6 toggles theme, F7 toggles density. Ctrl+P opens command palette.
//! ? opens keybind reference. Ctrl+F opens search. Ctrl+Y enters selection mode.
//!
//! Tab navigation:
//!   Alt+1..5  — jump to Chat / Subagents / Tools / MCP / Logs
//!   Alt+l/h   — next / previous tab
//!
//! Panel navigation (non-Chat tabs):
//!   Tab       — toggle focus between master list and detail buffer
//!   Enter     — open detail view for selected item
//!   Esc       — detail→master, or panel→Chat tab
//!   j/k       — navigate list (vim preset)
//!   Ctrl+n/p  — navigate list (emacs preset)
//!   Up/Down   — navigate list (all presets)
//!   PgUp/PgDn — scroll detail buffer
//!
//! All tabs show fake operational data.

use std::time::Duration;

use tokio::sync::mpsc;
use ucode_tui::app::ToolCallStatus;
use ucode_tui::command_registry::{CommandCategory, CommandDef, CommandSource};
use ucode_tui::components::master_detail::ListItem;
use ucode_tui::components::sidebar::sections::PluginSidebarSection;
use ucode_tui::components::tab_bar::TabId;
use ucode_tui::components::toast::ToastLevel;
use ucode_tui::event_loop::TuiEvent;
use ucode_tui::overlays::palette::PaletteState;

// ---------------------------------------------------------------------------
// Transcript index constants
//
// The TUI's SendMessage action calls push_user_message + start_streaming
// before our fake_llm receives the message. We hardcode the indices based on
// the known sequence of transcript mutations.
//
// Startup:
//   [0] SystemMessage (we send on startup)
//
// First user message (TUI adds [1] UserMessage, [2] Streaming):
//   [3] RouterEvent
//   [4] ToolCall "sequential_thinking"   <- tool_index = 4
//   [5] ToolCall "Read"                  <- tool_index = 5
//   [6] ToolCall "Grep"                  <- tool_index = 6
//   [2] finalized to AssistantMessage by StreamDone
//
// Second user message (TUI adds [7] UserMessage, [8] Streaming):
//   [9]  ToolCall "Write"                <- tool_index = 9
//   [10] SystemMessage
//   [11] PatchProposed
//   [12] ToolCall "run_cmd" (via request_approval's internal push_tool_call)
//   [8]  finalized to AssistantMessage by StreamDone
// ---------------------------------------------------------------------------

const TOOL_IDX_SEQUENTIAL_THINKING: usize = 4;
const TOOL_IDX_READ: usize = 5;
const TOOL_IDX_GREP: usize = 6;
const TOOL_IDX_WRITE: usize = 9;

const PATCH_DIFF: &str = "\
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,5 +1,8 @@
+//! ucode library crate
+
 pub mod config;
 pub mod core;
+pub mod utils;

-pub fn init() {
+pub fn init() -> Result<(), Box<dyn std::error::Error>> {
     // TODO: initialize subsystems
+    Ok(())
 }";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Stream a string word-by-word into the TUI. Returns early if the channel closes or cancelled.
async fn stream_words(
    text: &str,
    delay_ms: u64,
    tui_tx: &mpsc::UnboundedSender<TuiEvent>,
    cancel: &tokio::sync::watch::Receiver<bool>,
) {
    for word in text.split_inclusive(|c: char| c.is_whitespace()) {
        if *cancel.borrow() {
            return;
        }
        if tui_tx.send(TuiEvent::StreamToken(word.to_owned())).is_err() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    }
}

/// Sleep that returns early if cancelled. Returns `true` if cancelled.
async fn cancellable_sleep(ms: u64, cancel: &tokio::sync::watch::Receiver<bool>) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(Duration::from_millis(ms)) => false,
        _ = cancel_signal(cancel) => true,
    }
}

/// Wait until the cancel flag becomes true.
async fn cancel_signal(cancel: &tokio::sync::watch::Receiver<bool>) {
    let mut rx = cancel.clone();
    while !*rx.borrow() {
        if rx.changed().await.is_err() {
            // Sender dropped — treat as cancelled.
            return;
        }
    }
}

// ---------------------------------------------------------------------------
// Scripted sequences
// ---------------------------------------------------------------------------

/// First-message sequence: router event, three tool calls, streaming response.
async fn run_sequence_one(
    tui_tx: &mpsc::UnboundedSender<TuiEvent>,
    cancel: &tokio::sync::watch::Receiver<bool>,
) {
    // Router fallback notification.
    if cancellable_sleep(200, cancel).await {
        return;
    }
    if tui_tx
        .send(TuiEvent::RouterEvent(
            "provider fallback: anthropic/claude-sonnet-4 -> anthropic/claude-sonnet-4-20250514"
                .to_owned(),
        ))
        .is_err()
    {
        return;
    }

    // Tool call 1: sequential_thinking (thinking only, no output).
    if cancellable_sleep(400, cancel).await {
        return;
    }
    if tui_tx
        .send(TuiEvent::ToolCallStarted {
            name: "sequential_thinking".to_owned(),
        })
        .is_err()
    {
        return;
    }
    if cancellable_sleep(380, cancel).await {
        return;
    }
    if tui_tx
        .send(TuiEvent::ToolCallCompleted {
            index: TOOL_IDX_SEQUENTIAL_THINKING,
            status: ToolCallStatus::Success,
            duration_ms: Some(380),
            summary: Some("thought=analyzing the request".to_owned()),
            thinking: Some(
                "Let me break down what the user is asking for. They want to understand \
                 the codebase structure and find the relevant files."
                    .to_owned(),
            ),
            output: None,
        })
        .is_err()
    {
        return;
    }

    // Tool call 2: Read (output only, no thinking).
    if cancellable_sleep(200, cancel).await {
        return;
    }
    if tui_tx
        .send(TuiEvent::ToolCallStarted {
            name: "Read".to_owned(),
        })
        .is_err()
    {
        return;
    }
    if cancellable_sleep(300, cancel).await {
        return;
    }
    if tui_tx
        .send(TuiEvent::ToolCallCompleted {
            index: TOOL_IDX_READ,
            status: ToolCallStatus::Success,
            duration_ms: Some(45),
            summary: Some("file=src/main.rs, offset=1, limit=50".to_owned()),
            thinking: None,
            output: Some("fn main() {\n    println!(\"Hello, world!\");\n}".to_owned()),
        })
        .is_err()
    {
        return;
    }

    // Tool call 3: Grep (both thinking and output).
    if cancellable_sleep(150, cancel).await {
        return;
    }
    if tui_tx
        .send(TuiEvent::ToolCallStarted {
            name: "Grep".to_owned(),
        })
        .is_err()
    {
        return;
    }
    if cancellable_sleep(350, cancel).await {
        return;
    }
    if tui_tx
        .send(TuiEvent::ToolCallCompleted {
            index: TOOL_IDX_GREP,
            status: ToolCallStatus::Success,
            duration_ms: Some(120),
            summary: Some("pattern=TODO, include=*.rs".to_owned()),
            thinking: Some(
                "I should search for any TODO comments to understand pending work".to_owned(),
            ),
            output: Some("Found 3 matches in 2 files".to_owned()),
        })
        .is_err()
    {
        return;
    }

    // Streaming response — showcases markdown rendering.
    if cancellable_sleep(200, cancel).await {
        return;
    }
    stream_words(
        "## Analysis Results\n\n\
         I've analyzed the codebase. Here's what I found:\n\n\
         - The main entry point is in `src/main.rs`\n\
         - There are **3 TODO items** across *2 files*\n\
         - The project structure follows standard Rust conventions\n\n\
         ### TODO Summary\n\n\
         | File | Line | Description |\n\
         |------|------|-------------|\n\
         | src/lib.rs | 42 | Initialize subsystems |\n\
         | src/config.rs | 15 | Load from env |\n\
         | src/config.rs | 28 | Validate schema |\n\n\
         ```rust\n// Example fix for the first TODO:\npub fn init() -> Result<(), Box<dyn std::error::Error>> {\n    config::load()?;\n    Ok(())\n}\n```\n\n\
         Would you like me to address any of the TODO items?",
        35,
        tui_tx,
        cancel,
    )
    .await;
    if *cancel.borrow() {
        return;
    }

    let _ = tui_tx.send(TuiEvent::StreamDone);

    // System-triggered toasts: checkpoint saved, agent completed.
    if cancellable_sleep(200, cancel).await {
        return;
    }
    let _ = tui_tx.send(TuiEvent::CheckpointCreated {
        name: "auto-save-1".to_owned(),
    });
    if cancellable_sleep(200, cancel).await {
        return;
    }
    let _ = tui_tx.send(TuiEvent::AgentCompleted {
        agent_id: "agent-001".to_owned(),
        name: "code-analyzer".to_owned(),
    });
}

/// Second-message sequence: failed tool, system message, approval modal, patch, streaming.
async fn run_sequence_two(
    tui_tx: &mpsc::UnboundedSender<TuiEvent>,
    cancel: &tokio::sync::watch::Receiver<bool>,
) {
    // Tool call: Write — fails.
    if cancellable_sleep(200, cancel).await {
        return;
    }
    if tui_tx
        .send(TuiEvent::ToolCallStarted {
            name: "Write".to_owned(),
        })
        .is_err()
    {
        return;
    }
    if cancellable_sleep(500, cancel).await {
        return;
    }
    if tui_tx
        .send(TuiEvent::ToolCallCompleted {
            index: TOOL_IDX_WRITE,
            status: ToolCallStatus::Failed,
            duration_ms: Some(480),
            summary: Some("file=src/config.rs".to_owned()),
            thinking: Some("I need to create the config module".to_owned()),
            output: None,
        })
        .is_err()
    {
        return;
    }

    // System message about the failure.
    if cancellable_sleep(100, cancel).await {
        return;
    }
    if tui_tx
        .send(TuiEvent::SystemMessage(
            "Tool failed: permission denied (sandbox: workspace)".to_owned(),
        ))
        .is_err()
    {
        return;
    }

    // Patch proposal — opens diff modal.
    if cancellable_sleep(300, cancel).await {
        return;
    }
    if tui_tx
        .send(TuiEvent::PatchProposed {
            file_path: "src/lib.rs".to_owned(),
            raw_diff: PATCH_DIFF.to_owned(),
            patch_id: Some("patch-001".to_owned()),
        })
        .is_err()
    {
        return;
    }

    // Approval required for cargo test — preempts diff modal, opens approval modal.
    if cancellable_sleep(500, cancel).await {
        return;
    }
    if tui_tx
        .send(TuiEvent::ApprovalRequired {
            tool_name: "run_cmd".to_owned(),
            command: "cargo test --workspace".to_owned(),
            cwd: "/home/user/project".to_owned(),
            sandbox_label: "workspace".to_owned(),
            tool_call_id: Some("tc-demo-001".to_owned()),
        })
        .is_err()
    {
        return;
    }

    // Streaming response — showcases markdown with inline styles.
    if cancellable_sleep(300, cancel).await {
        return;
    }
    stream_words(
        "### Permission Issue\n\n\
         I encountered a **permission denied** error with `src/config.rs` \
         (sandbox: *workspace*). However, I've prepared a patch for `src/lib.rs`.\n\n\
         **Next steps:**\n\n\
         1. Handle the ~~pending~~ approval for `cargo test`\n\
         2. Review the diff for `src/lib.rs`\n\
         3. The diff will resume automatically after approval\n\n\
         See [Rust sandbox docs](https://doc.rust-lang.org/cargo/) for details.",
        35,
        tui_tx,
        cancel,
    )
    .await;
    if *cancel.borrow() {
        return;
    }

    let _ = tui_tx.send(TuiEvent::StreamDone);

    // System-triggered toasts: budget warning, agent failure.
    if cancellable_sleep(200, cancel).await {
        return;
    }
    let _ = tui_tx.send(TuiEvent::BudgetWarning {
        used_pct: 75.0,
        message: "Token budget at 75%".to_owned(),
    });
    if cancellable_sleep(200, cancel).await {
        return;
    }
    let _ = tui_tx.send(TuiEvent::AgentFailed {
        agent_id: "agent-002".to_owned(),
        name: "test-runner".to_owned(),
        error: "timeout after 30s".to_owned(),
    });
}

/// Third-and-beyond sequence: echo with markdown formatting.
async fn run_sequence_echo(
    user_msg: &str,
    tui_tx: &mpsc::UnboundedSender<TuiEvent>,
    cancel: &tokio::sync::watch::Receiver<bool>,
) {
    if cancellable_sleep(300, cancel).await {
        return;
    }

    let response = format!(
        "You said: *\"{user_msg}\"*\n\n\
         ### Keyboard Shortcuts\n\n\
         | Key | Action |\n\
         |-----|--------|\n\
         | `?` | Keybind reference |\n\
          | `Ctrl+F` | Search transcript |\n\
          | `Ctrl+Y` | Selection mode |\n\
          | `v/V/^V` | Char/Line/Block select |\n\
          | `Ctrl+P` | Command palette |"
    );

    stream_words(&response, 35, tui_tx, cancel).await;
    if *cancel.borrow() {
        return;
    }
    let _ = tui_tx.send(TuiEvent::StreamDone);
}

// ---------------------------------------------------------------------------
// Fake LLM
// ---------------------------------------------------------------------------

/// Simulated LLM that runs scripted sequences showcasing all TUI features.
async fn fake_llm(
    mut user_rx: mpsc::UnboundedReceiver<ucode_agent::AgentMessage>,
    tui_tx: mpsc::UnboundedSender<TuiEvent>,
) {
    // Startup system message — transcript[0].
    tokio::time::sleep(Duration::from_millis(500)).await;
    if tui_tx
        .send(TuiEvent::SystemMessage(
             "Demo mode -- showcasing all TUI features. Type any message to trigger a simulated \
              AI interaction.\n\nKeys: F6 theme, F7 density, ? keybinds, Ctrl+F search, Ctrl+Y select, Ctrl+P palette"
                .to_owned(),
        ))
        .is_err()
    {
        return;
    }

    let mut message_count: u32 = 0;
    let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);

    while let Some(agent_msg) = user_rx.recv().await {
        let user_msg = match agent_msg {
            ucode_agent::AgentMessage::UserMessage { text, .. } => text,
            ucode_agent::AgentMessage::SetModel(model) => {
                let _ = tui_tx.send(TuiEvent::SystemMessage(format!(
                    "Model switched to: {model}"
                )));
                continue;
            }
            ucode_agent::AgentMessage::Cancel => {
                // Signal the running sequence to stop.
                let _ = cancel_tx.send(true);
                let _ = tui_tx.send(TuiEvent::SystemMessage("Generation cancelled.".to_owned()));
                let _ = tui_tx.send(TuiEvent::StreamDone);
                continue;
            }
            ucode_agent::AgentMessage::ApprovalDecision { .. } => {
                // Demo doesn't implement real approval round-trips.
                continue;
            }
        };
        message_count += 1;

        // Reset cancellation for the new sequence.
        let _ = cancel_tx.send(false);

        match message_count {
            1 => run_sequence_one(&tui_tx, &cancel_rx).await,
            2 => run_sequence_two(&tui_tx, &cancel_rx).await,
            _ => run_sequence_echo(&user_msg, &tui_tx, &cancel_rx).await,
        }

        // Stop if the channel closed mid-sequence.
        if tui_tx.is_closed() {
            return;
        }
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (tui_tx, tui_rx) = ucode_tui::create_event_channel();
    let (user_tx, user_rx) = mpsc::unbounded_channel::<ucode_agent::AgentMessage>();

    // Spawn the scripted LLM responder.
    tokio::spawn(fake_llm(user_rx, tui_tx.clone()));

    // Build app state with a demo plugin command that showcases the plugin badge
    // and args_hint rendering in both the autocomplete dropdown and palette.
    let mut app = ucode_tui::app::AppState::new();
    app.command_registry.register(CommandDef {
        name: "/analyze".to_owned(),
        description: "Run code analysis".to_owned(),
        category: CommandCategory::Plugins,
        source: CommandSource::Plugin("code-analyzer".to_owned()),
        args_hint: Some("<path>".to_owned()),
        action: None,
    });
    app.palette = PaletteState::from_registry(&app.command_registry);
    app.message_tx = Some(user_tx);

    // Startup toast — visible immediately on startup.
    // Checkpoint and budget toasts are demonstrated via system-triggered events in the sequences.
    app.toast(ToastLevel::Info, "Session started");

    // -- Fake data for tabbed panels ------------------------------------------

    // Subagent runs.
    app.subagents_panel.set_items(vec![
        ListItem::new("rust-expert", "✓", "1.2s  890 tok").with_subtitle("C3+D1: @mention routing"),
        ListItem::new("explore", "✓", "0.4s  210 tok").with_subtitle("Find layout test files"),
        ListItem::new("quick-fix", "⟳", "running...").with_subtitle("Fix layout test assertions"),
    ]);
    app.subagents_panel.set_buffer(
        "# Rust-Expert Task\n\n\
         C3+D1: @mention routing (59 tool calls)\n\n\
         ## Summary\n\n\
         - Added `FileContext` struct\n\
         - Changed `AgentMessage::UserMessage` from tuple to struct variant\n\
         - Updated 20+ call sites\n\n\
         ### Verification\n\
         ```\n\
         cargo test --workspace\n\
         1725 passed, 0 failed\n\
         ```"
        .into(),
    );

    // Tool calls.
    app.tools_panel.set_items(vec![
        ListItem::new("Read", "✓", "45ms").with_subtitle("layout.rs"),
        ListItem::new("Edit", "✓", "12ms").with_subtitle("layout.rs"),
        ListItem::new("Bash", "✓", "1.2s").with_subtitle("cargo test"),
        ListItem::new("Grep", "✓", "120ms").with_subtitle("pattern=TODO"),
        ListItem::new("Write", "✗", "480ms").with_subtitle("config.rs — denied"),
    ]);
    app.tools_panel.set_buffer(
        "Read\nStatus: ✓ Success\nDuration: 45ms\n\n\
         Input:\n  file: crates/ucode-tui/src/layout.rs\n  offset: 286\n  limit: 100\n\n\
         Output:\n  176: #[test]\n  177: fn terminal_size_minimum() {\n  178:     assert!(..."
            .into(),
    );

    // MCP servers.
    app.mcp_panel.set_items(vec![
        ListItem::new("context7", "●", "12 tools"),
        ListItem::new("git-docs", "●", "3 tools"),
        ListItem::new("arxiv", "○", "disconnected"),
    ]);
    app.mcp_panel.set_buffer(
        "context7\nStatus: connected\nTools: 12\n\n\
         -- Tool Catalog --\n\
         resolve-library-id\n  Resolve package name to library ID\n\
         query-docs\n  Query documentation and examples\n\n\
         -- Request Log (14 calls) --\n\n\
         14:23:05 query-docs         ✓  45ms\n  lib=/vercel/next.js\n\n\
         14:22:58 resolve-library-id ✓  120ms\n  name=\"next.js\""
            .into(),
    );

    // Log events.
    app.logs_panel.set_items(vec![
        ListItem::new("agent_spawn", "INFO", "14:23:01").with_subtitle("rust-expert"),
        ListItem::new("model_switch", "INFO", "14:23:00").with_subtitle("opus-4-6"),
        ListItem::new("budget_warning", "WARN", "14:22:58").with_subtitle("75% used"),
        ListItem::new("tool_failed", "ERROR", "14:22:45").with_subtitle("Write: denied"),
    ]);
    app.logs_panel.set_buffer(
        "Agent Spawn\n\n\
         Time: 2026-03-07 14:23:01\nLevel: INFO\nType: agent_spawn\n\n\
         Agent: rust-expert\nTask: C3+D1 @mention routing\nModel: claude-opus-4-6\n\n\
         Detail:\n  Spawned subagent rust-expert for implementing\n  AgentMessage struct expansion."
            .into(),
    );

    // Badge counts.
    app.tab_bar.set_badge(TabId::Subagents, 3);
    app.tab_bar.set_badge(TabId::Tools, 5);
    app.tab_bar.set_badge(TabId::Logs, 4);

    let mut input_box = ucode_tui::components::input::InputBoxState::new();
    let mut sidebar_data = ucode_tui::components::sidebar::SidebarData::new();
    sidebar_data.register_plugin_section(PluginSidebarSection {
        plugin_name: "code-analyzer".into(),
        section_id: "code-analyzer-stats".into(),
        title: "CODE ANALYSIS".into(),
        lines: vec!["  complexity: 12".into(), "  coverage: 87%".into()],
        priority: 100,
        collapsed: false,
    });

    // Sidebar footer.
    sidebar_data.footer_dir = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "~/code/ucode".into());
    sidebar_data.footer_version = format!("ucode v{}", env!("CARGO_PKG_VERSION"));

    ucode_tui::event_loop::run_event_loop(
        &mut app,
        &mut input_box,
        &mut sidebar_data,
        tui_tx,
        tui_rx,
        None,
    )
    .await
}
