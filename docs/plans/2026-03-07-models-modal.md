# `/models` Modal Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the transcript-dump `/models` with an interactive modal that lists models from all connected providers, supports filtering, and lets the user switch the active model mid-session.

**Architecture:** Three-layer change: (1) introduce `AgentMessage` enum in `ucode-agent` so the TUI can send control messages (not just user text) to the running agent loop, (2) create a `ModelsModal` overlay following the established `ConnectModal` pattern, (3) wire the modal's selection to `AgentMessage::SetModel` so the agent loop picks up the new model on the next turn.

**Tech Stack:** Rust, ratatui, tokio channels, ucode-providers `list_models()` API.

---

## Task 1: Introduce `AgentMessage` enum in `ucode-agent`

**Files:**
- Modify: `crates/ucode-agent/src/agent_loop.rs`
- Modify: `crates/ucode-agent/src/lib.rs`

**Why:** The agent loop currently receives `UnboundedReceiver<String>` — only user text. We need a way to send "change model" without restarting the loop. An enum is the minimal, extensible solution.

**Step 1: Add `AgentMessage` enum and update `run_agent_loop` signature**

In `crates/ucode-agent/src/agent_loop.rs`, add above `AgentLoopConfig`:

```rust
/// Messages sent from the TUI to the agent loop.
#[derive(Debug, Clone)]
pub enum AgentMessage {
    /// A user-typed message to send to the LLM.
    UserMessage(String),
    /// Switch the model used for subsequent turns.
    SetModel(String),
}
```

Change `run_agent_loop` parameter from:
```rust
mut message_rx: mpsc::UnboundedReceiver<String>,
```
to:
```rust
mut message_rx: mpsc::UnboundedReceiver<AgentMessage>,
```

Update the recv loop to handle both variants:
```rust
while let Some(agent_msg) = message_rx.recv().await {
    match agent_msg {
        AgentMessage::SetModel(new_model) => {
            model = new_model;
            session.set_active_model(Some(model.clone()));
            let _ = event_tx.send(AgentEvent::SystemMessage(
                format!("provider={} model={}", config.provider_name, model),
            ));
            continue;
        }
        AgentMessage::UserMessage(user_text) => {
            // ... existing message handling logic (unchanged)
        }
    }
}
```

This requires making `model` a mutable local variable instead of reading from `config.model`:
- Before the loop: `let mut model = config.model.clone();`
- Replace `&config.model` with `&model` in `process_turn` and `followup_turn` calls.

**Step 2: Export `AgentMessage` from `lib.rs`**

```rust
pub use agent_loop::{AgentEvent, AgentLoopConfig, AgentMessage, run_agent_loop};
```

**Step 3: Verify**

Run: `cargo check -p ucode-agent`

---

## Task 2: Update TUI to use `AgentMessage`

**Files:**
- Modify: `crates/ucode-tui/src/app.rs` — change `message_tx` type
- Modify: `crates/ucode-tui/src/lib.rs` — update channel creation in `spawn_agent_loop`
- Modify: `crates/ucode-tui/src/event_loop.rs` — update `try_spawn_agent_after_connect`

**Step 1: Change `message_tx` type in `AppState`**

In `app.rs`, change:
```rust
pub message_tx: Option<UnboundedSender<String>>,
```
to:
```rust
pub message_tx: Option<UnboundedSender<ucode_agent::AgentMessage>>,
```

**Step 2: Update `push_user_message` to wrap in `AgentMessage::UserMessage`**

```rust
pub fn push_user_message(&mut self, msg: String) {
    if let Some(tx) = &self.message_tx {
        let _ = tx.send(ucode_agent::AgentMessage::UserMessage(msg.clone()));
    }
    self.transcript.push(TranscriptEntry::UserMessage(msg));
    self.mark_dirty();
}
```

**Step 3: Update `spawn_agent_loop` in `lib.rs`**

Change the channel type:
```rust
let (msg_tx, msg_rx) = tokio::sync::mpsc::unbounded_channel::<ucode_agent::AgentMessage>();
```

**Step 4: Verify**

Run: `cargo check -p ucode-tui`

---

## Task 3: Create `ModelsModalState` in `overlays/models_modal.rs`

**Files:**
- Create: `crates/ucode-tui/src/overlays/models_modal.rs`
- Modify: `crates/ucode-tui/src/overlays/mod.rs` — add `pub mod models_modal;`

**Step 1: Create the state struct and logic**

```rust
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

/// A single model entry in the modal list.
#[derive(Debug, Clone)]
pub struct ModelEntry {
    pub provider: String,
    pub model_id: String,
    pub display_name: Option<String>,
}

impl ModelEntry {
    /// Label shown in the list: "model_id (display_name)" or just "model_id".
    pub fn label(&self) -> String {
        match &self.display_name {
            Some(name) if name != &self.model_id => format!("{} ({})", self.model_id, name),
            _ => self.model_id.clone(),
        }
    }
}

/// State for the models modal overlay.
#[derive(Debug, Clone)]
pub struct ModelsModalState {
    pub visible: bool,
    pub loading: bool,
    /// Number of providers we're still waiting for.
    pub pending_providers: usize,
    pub entries: Vec<ModelEntry>,
    pub filter: String,
    pub filter_cursor: usize,
    pub filtered_indices: Vec<usize>,
    pub selected: usize,
    /// The currently active model id (highlighted in the list).
    pub current_model: Option<String>,
}
```

Implement the same navigation/filter pattern as `ConnectModalState`:
- `open(current_model, provider_count)` — sets visible, loading, clears entries
- `close()`
- `insert_char(c)`, `delete_char()` — filter editing
- `move_up()`, `move_down()` — navigation
- `selected_entry() -> Option<&ModelEntry>` — get selected
- `add_models(provider, models)` — append entries, decrement pending, clear loading when 0
- `add_error(error)` — decrement pending, clear loading when 0
- `update_filter()` — rebuild filtered_indices

**Step 2: Register module**

In `overlays/mod.rs`, add:
```rust
pub mod models_modal;
```

**Step 3: Verify**

Run: `cargo check -p ucode-tui`

---

## Task 4: Create `ModelsModal` widget (rendering)

**Files:**
- Modify: `crates/ucode-tui/src/overlays/models_modal.rs` — add widget

**Step 1: Add the widget struct and `impl Widget`**

Follow the `ConnectModal` rendering pattern:
- Centered popup (60% x 70%)
- Title: ` Models `
- Filter line at top
- Separator
- Scrollable list body with entries grouped by provider (section headers)
- Current model marked with `*` prefix
- Selected row highlighted
- Footer with keybind hints: `↑↓ navigate  type to filter  Enter select  Esc close`
- Loading state: show "Fetching models..." when `loading` is true and entries are empty

```rust
pub struct ModelsModal<'a> {
    state: &'a ModelsModalState,
    theme: &'a crate::theme::UcodeTheme,
}
```

The list rendering should group entries by provider. Build a flat list of "display rows" where some are section headers (provider name) and some are model entries. Only model entries are selectable. Use the same scroll logic as `ConnectModal`.

**Step 2: Verify**

Run: `cargo check -p ucode-tui`

---

## Task 5: Wire `ModelsModal` into `AppState` and rendering

**Files:**
- Modify: `crates/ucode-tui/src/app.rs` — add `models_modal` field
- Modify: `crates/ucode-tui/src/event_loop.rs` — render the modal in `render_frame`

**Step 1: Add field to `AppState`**

```rust
pub models_modal: crate::overlays::models_modal::ModelsModalState,
```

Initialize in `AppState::new()`:
```rust
models_modal: crate::overlays::models_modal::ModelsModalState::new(),
```

**Step 2: Render in `render_frame`**

After the connect modal render block, add:
```rust
if app.models_modal.visible {
    use crate::overlays::models_modal::ModelsModal;
    f.render_widget(ModelsModal::new(&app.models_modal, &app.theme), area);
}
```

**Step 3: Verify**

Run: `cargo check -p ucode-tui`

---

## Task 6: Wire keyboard handling for the models modal

**Files:**
- Modify: `crates/ucode-tui/src/event_loop.rs`

**Step 1: Add keyboard routing in `handle_terminal_event`**

Before the connect modal keyboard block (or after — order matters for overlay priority), add:

```rust
if app.models_modal.visible {
    match key.code {
        KeyCode::Esc => {
            app.models_modal.close();
            app.focus = FocusTarget::Input;
            app.mark_dirty();
        }
        KeyCode::Enter => {
            if let Some(entry) = app.models_modal.selected_entry().cloned() {
                // Send SetModel to agent loop.
                if let Some(tx) = &app.message_tx {
                    let _ = tx.send(ucode_agent::AgentMessage::SetModel(
                        entry.model_id.clone(),
                    ));
                }
                // Update sidebar model name.
                sidebar_data.router.model_name = entry.model_id.clone();
                app.models_modal.close();
                app.focus = FocusTarget::Input;
                app.push_system_message(format!(
                    "Switched to model: {}",
                    entry.label()
                ));
            }
            app.mark_dirty();
        }
        KeyCode::Up => {
            app.models_modal.move_up();
            app.mark_dirty();
        }
        KeyCode::Down => {
            app.models_modal.move_down();
            app.mark_dirty();
        }
        KeyCode::Backspace => {
            app.models_modal.delete_char();
            app.mark_dirty();
        }
        KeyCode::Char(c) => {
            app.models_modal.insert_char(c);
            app.mark_dirty();
        }
        _ => {}
    }
    return false;
}
```

**Step 2: Verify**

Run: `cargo check -p ucode-tui`

---

## Task 7: Wire `Action::OpenModels` and `TuiEvent::ModelsListed`

**Files:**
- Modify: `crates/ucode-tui/src/event_loop.rs`

**Step 1: Update `Action::OpenModels` handler in `dispatch_action`**

Replace the current transcript-dump implementation with:
```rust
Action::OpenModels => {
    if app.providers.is_empty() {
        app.push_system_message(
            "No providers connected. Use /connect first.".to_owned(),
        );
    } else {
        // Open modal in loading state.
        let current = sidebar_data.router.model_name.clone(); // need sidebar_data param
        app.models_modal.open(
            if current.is_empty() { None } else { Some(current) },
            app.providers.len(),
        );
        app.focus = FocusTarget::Overlay;
        // Signal main loop to spawn fetch tasks.
        app.models_fetch_pending = true;
    }
    app.mark_dirty();
}
```

Note: `dispatch_action` doesn't have `sidebar_data`. Two options:
(a) Store `current_model` in `AppState` (cleaner — AppState should know the active model).
(b) Pass `sidebar_data` to `dispatch_action`.

Go with (a): add `pub active_model: Option<String>` to `AppState`. Set it when the agent spawns (from `AgentLoopConfig.model`) and when `SetModel` is sent. Then `dispatch_action` reads `app.active_model`.

**Step 2: Update `TuiEvent::ModelsListed` handler in `handle_tui_event`**

Replace the transcript-dump with:
```rust
TuiEvent::ModelsListed { provider, models } => {
    app.models_modal.add_models(&provider, &models);
    app.mark_dirty();
}
TuiEvent::ModelsListFailed { error } => {
    app.models_modal.add_error(&error);
    app.push_system_message(format!("Failed to list models: {error}"));
    app.mark_dirty();
}
```

**Step 3: Verify**

Run: `cargo check --workspace`

---

## Task 8: Track `active_model` in `AppState`

**Files:**
- Modify: `crates/ucode-tui/src/app.rs`
- Modify: `crates/ucode-tui/src/lib.rs`
- Modify: `crates/ucode-tui/src/event_loop.rs`

**Step 1: Add `active_model` field**

```rust
/// The model currently used by the agent loop.
pub active_model: Option<String>,
```

Initialize as `None` in `AppState::new()`.

**Step 2: Set on agent spawn**

In `lib.rs` `run()`, after spawning:
```rust
app.active_model = Some(ac.loop_config.model.clone());
```

In `try_spawn_agent_after_connect`:
```rust
app.active_model = Some(default_model.to_owned());
```

**Step 3: Update on model switch**

In the models modal Enter handler (Task 6), after sending `SetModel`:
```rust
app.active_model = Some(entry.model_id.clone());
```

**Step 4: Use in `dispatch_action`**

The `OpenModels` handler reads `app.active_model.clone()` instead of needing `sidebar_data`.

**Step 5: Verify**

Run: `cargo check --workspace && cargo test --workspace`

---

## Task 9: Clean up old transcript-dump code

**Files:**
- Modify: `crates/ucode-tui/src/event_loop.rs`

**Step 1: Remove `spawn_models_fetch` standalone helper**

The `spawn_models_fetch` function stays but is now called from the main loop's `models_fetch_pending` drain (unchanged). The difference is the results now go to the modal instead of the transcript.

**Step 2: Verify full flow**

Run: `cargo check --workspace && cargo test --workspace`

---

## Summary of all files touched

| File | Action |
|------|--------|
| `crates/ucode-agent/src/agent_loop.rs` | Add `AgentMessage` enum, update `run_agent_loop` to handle `SetModel` |
| `crates/ucode-agent/src/lib.rs` | Export `AgentMessage` |
| `crates/ucode-tui/src/overlays/models_modal.rs` | **NEW** — `ModelsModalState` + `ModelsModal` widget |
| `crates/ucode-tui/src/overlays/mod.rs` | Add `pub mod models_modal` |
| `crates/ucode-tui/src/app.rs` | Change `message_tx` type, add `models_modal` + `active_model` fields |
| `crates/ucode-tui/src/lib.rs` | Update channel type, set `active_model` on spawn |
| `crates/ucode-tui/src/event_loop.rs` | Modal keyboard handling, updated action/event handlers, render call |
| `crates/ucode-tui/src/keybinds.rs` | Already done (`OpenModels` exists) |
| `crates/ucode-tui/src/command_registry.rs` | Already done (`/models` wired) |
