//! Interactive demo: launch the TUI with a scripted AI coding assistant interaction.
//!
//! Run with: cargo run -p ucode-tui --example demo
//!
//! Type a message and press Enter to trigger a simulated AI interaction that
//! showcases all TUI features: tool calls (with thinking/output), router events,
//! system messages, patch proposals, approval modals, streaming responses,
//! system-triggered toasts, search overlay, keybind overlay, and copy mode.
//!
//! - First message: tool calls + streaming response + checkpoint/agent toasts
//! - Second message: failed tool, approval modal, patch proposal + budget/failure toasts
//! - Third+ messages: simple echo
//!
//! F6 toggles theme, F7 toggles density. Ctrl+P opens command palette.
//! ? opens keybind reference. Ctrl+F opens search. v enters copy mode.

use std::time::Duration;

use tokio::sync::mpsc;
use ucode_tui::app::ToolCallStatus;
use ucode_tui::command_registry::{CommandCategory, CommandDef, CommandSource};
use ucode_tui::components::sidebar::sections::PluginSidebarSection;
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

/// Stream a string word-by-word into the TUI. Returns early if the channel closes.
async fn stream_words(text: &str, delay_ms: u64, tui_tx: &mpsc::UnboundedSender<TuiEvent>) {
    for word in text.split_inclusive(|c: char| c.is_whitespace()) {
        if tui_tx.send(TuiEvent::StreamToken(word.to_owned())).is_err() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    }
}

// ---------------------------------------------------------------------------
// Scripted sequences
// ---------------------------------------------------------------------------

/// First-message sequence: router event, three tool calls, streaming response.
async fn run_sequence_one(tui_tx: &mpsc::UnboundedSender<TuiEvent>) {
    // Router fallback notification.
    tokio::time::sleep(Duration::from_millis(200)).await;
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
    tokio::time::sleep(Duration::from_millis(400)).await;
    if tui_tx
        .send(TuiEvent::ToolCallStarted {
            name: "sequential_thinking".to_owned(),
        })
        .is_err()
    {
        return;
    }
    tokio::time::sleep(Duration::from_millis(380)).await;
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
    tokio::time::sleep(Duration::from_millis(200)).await;
    if tui_tx
        .send(TuiEvent::ToolCallStarted {
            name: "Read".to_owned(),
        })
        .is_err()
    {
        return;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
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
    tokio::time::sleep(Duration::from_millis(150)).await;
    if tui_tx
        .send(TuiEvent::ToolCallStarted {
            name: "Grep".to_owned(),
        })
        .is_err()
    {
        return;
    }
    tokio::time::sleep(Duration::from_millis(350)).await;
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
    tokio::time::sleep(Duration::from_millis(200)).await;
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
         ```rust\n\
         // Example fix for the first TODO:\n\
         pub fn init() -> Result<(), Box<dyn std::error::Error>> {\n\
             config::load()?;\n\
             Ok(())\n\
         }\n\
         ```\n\n\
         Would you like me to address any of the TODO items?",
        35,
        tui_tx,
    )
    .await;

    let _ = tui_tx.send(TuiEvent::StreamDone);

    // System-triggered toasts: checkpoint saved, agent completed.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let _ = tui_tx.send(TuiEvent::CheckpointCreated {
        name: "auto-save-1".to_owned(),
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    let _ = tui_tx.send(TuiEvent::AgentCompleted {
        agent_id: "agent-001".to_owned(),
        name: "code-analyzer".to_owned(),
    });
}

/// Second-message sequence: failed tool, system message, approval modal, patch, streaming.
async fn run_sequence_two(tui_tx: &mpsc::UnboundedSender<TuiEvent>) {
    // Tool call: Write — fails.
    tokio::time::sleep(Duration::from_millis(200)).await;
    if tui_tx
        .send(TuiEvent::ToolCallStarted {
            name: "Write".to_owned(),
        })
        .is_err()
    {
        return;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
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
    tokio::time::sleep(Duration::from_millis(100)).await;
    if tui_tx
        .send(TuiEvent::SystemMessage(
            "Tool failed: permission denied (sandbox: workspace)".to_owned(),
        ))
        .is_err()
    {
        return;
    }

    // Patch proposal — opens diff modal.
    tokio::time::sleep(Duration::from_millis(300)).await;
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
    tokio::time::sleep(Duration::from_millis(500)).await;
    if tui_tx
        .send(TuiEvent::ApprovalRequired {
            tool_name: "run_cmd".to_owned(),
            command: "cargo test --workspace".to_owned(),
            cwd: "/home/user/project".to_owned(),
            sandbox_label: "workspace".to_owned(),
        })
        .is_err()
    {
        return;
    }

    // Streaming response — showcases markdown with inline styles.
    tokio::time::sleep(Duration::from_millis(300)).await;
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
    )
    .await;

    let _ = tui_tx.send(TuiEvent::StreamDone);

    // System-triggered toasts: budget warning, agent failure.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let _ = tui_tx.send(TuiEvent::BudgetWarning {
        used_pct: 75.0,
        message: "Token budget at 75%".to_owned(),
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    let _ = tui_tx.send(TuiEvent::AgentFailed {
        agent_id: "agent-002".to_owned(),
        name: "test-runner".to_owned(),
        error: "timeout after 30s".to_owned(),
    });
}

/// Third-and-beyond sequence: echo with markdown formatting.
async fn run_sequence_echo(user_msg: &str, tui_tx: &mpsc::UnboundedSender<TuiEvent>) {
    tokio::time::sleep(Duration::from_millis(300)).await;

    let response = format!(
        "You said: *\"{user_msg}\"*\n\n\
         ### Keyboard Shortcuts\n\n\
         | Key | Action |\n\
         |-----|--------|\n\
         | `?` | Keybind reference |\n\
         | `Ctrl+F` | Search transcript |\n\
         | `v` | Copy mode |\n\
         | `Ctrl+P` | Command palette |"
    );

    stream_words(&response, 35, tui_tx).await;
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
             AI interaction.\n\nKeys: F6 theme, F7 density, ? keybinds, Ctrl+F search, v copy mode, Ctrl+P palette"
                .to_owned(),
        ))
        .is_err()
    {
        return;
    }

    let mut message_count: u32 = 0;

    while let Some(agent_msg) = user_rx.recv().await {
        let user_msg = match agent_msg {
            ucode_agent::AgentMessage::UserMessage(text) => text,
            ucode_agent::AgentMessage::SetModel(model) => {
                let _ = tui_tx.send(TuiEvent::SystemMessage(format!(
                    "Model switched to: {model}"
                )));
                continue;
            }
        };
        message_count += 1;

        match message_count {
            1 => run_sequence_one(&tui_tx).await,
            2 => run_sequence_two(&tui_tx).await,
            _ => run_sequence_echo(&user_msg, &tui_tx).await,
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
