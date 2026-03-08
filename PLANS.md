# ucode PLANS — Phase 11 and Beyond

> **Previous phases (0–10) are complete.**
> Full task history: `docs/backups/2026-03-07/PLANS.md`
> Epic overview: `EPIC.md`

---

## Background

Phases 0–10 delivered a complete foundation: core runtime, auth, provider adapters, built-in tools, MCP client, skills, fullscreen TUI, plugins/WASM runtime, context management, headless CI mode, and the agent loop with live model switching.

The TUI components exist and render correctly, but several critical wiring gaps were discovered in a code audit on 2026-03-07:

- The approval modal is cosmetic — decisions are never sent back to the agent loop
- The agent loop has no approval gate — all tool calls execute immediately
- Scroll is entry-level, not line-level — long messages cannot be scrolled through
- Mouse and paste events are ignored
- The sidebar never updates from agent events
- Session state is never loaded or saved from the TUI
- There is no cancellation path for in-progress generation

Phase 11 fixes all of these. Phase 12 tracks the explicitly deferred items from earlier phases.

---

## Environment constraints

- **Target:** tmux + pure terminal (Alacritty, xterm, etc.)
- **No:** drag-drop file protocol, Kitty graphics protocol, iTerm2 inline images
- **Yes:** sixel (tmux 3.4+ with `allow-passthrough`), halfblocks (always work)
- **Image display strategy:** popup overlay (not inline) — sixel disappears on resize in tmux
- **Image paste:** shell out to `xclip -t image/png` or `wl-paste --type image/png`

---

# Phase 11 — TUI functional completion

## Milestone ordering

Execute in this order — each milestone is a hard prerequisite for the next:

```
A (agent loop) → B (TUI core) → C (image/attachments) → D (session)
```

Within each milestone, tasks can be done in order listed.

---

## Milestone A — Agent loop foundation (prerequisite for everything)

### Task A1 — Expand AgentMessage + AgentEvent enums

**File:** `crates/ucode-agent/src/agent_loop.rs`

Add to `AgentMessage`:
```rust
/// Abort the current in-progress generation.
Cancel,
/// User decision on a pending tool approval.
ApprovalDecision {
    tool_call_id: String,
    approved: bool,
},
```

Add to `AgentEvent`:
```rust
/// Agent is waiting for user approval before executing a tool.
ApprovalRequired {
    tool_call_id: String,
    tool_name: String,
    command: String,
    cwd: String,
    sandbox_label: String,
},
```

Update `match agent_msg` in `run_agent_loop` to handle the new variants (stubs are fine for now — A2 wires the logic).

**Verification:** `cargo check -p ucode-agent` passes.

---

### Task A2 — Approval gate in process_turn

**File:** `crates/ucode-agent/src/agent_loop.rs`

Add to `run_agent_loop` state:
```rust
let mut pending_approvals: HashMap<String, tokio::sync::oneshot::Sender<bool>> = HashMap::new();
```

In `process_turn`, before `tool_registry.invoke()`:
1. Send `AgentEvent::ApprovalRequired { tool_call_id: tc.id.clone(), tool_name: tc.name.clone(), ... }`
2. Create a `oneshot::channel::<bool>()`
3. Store the sender in `pending_approvals` keyed by `tc.id`
4. Await the receiver — this blocks the agent loop until TUI sends a decision
5. If `approved == false`: push an error `ToolResult` and skip `invoke()`

In `run_agent_loop`, handle `AgentMessage::ApprovalDecision`:
```rust
AgentMessage::ApprovalDecision { tool_call_id, approved } => {
    if let Some(tx) = pending_approvals.remove(&tool_call_id) {
        let _ = tx.send(approved);
    }
}
```

**Note:** `process_turn` currently runs as a sequential `await` inside the `UserMessage` arm. To handle `ApprovalDecision` messages while `process_turn` is awaiting, the main loop needs to be restructured. Use `tokio::select!` between `message_rx.recv()` and the approval oneshot receiver, or refactor `process_turn` to yield back to the main loop between tool calls.

**Recommended approach:** Pass `&mut message_rx` into `process_turn` so it can drain approval decisions inline while awaiting tool execution. This avoids restructuring the outer loop.

**Verification:** `cargo test -p ucode-agent` passes. New test: approval gate blocks until decision arrives.

---

### Task A3 — Cancellation token for streams

**File:** `crates/ucode-agent/src/agent_loop.rs`
**Dependency:** Add `tokio-util` with `rt` feature to `ucode-agent/Cargo.toml`

```toml
tokio-util = { version = "0.7", features = ["rt"] }
```

Add `CancellationToken` to agent loop state:
```rust
use tokio_util::sync::CancellationToken;
let mut cancel_token = CancellationToken::new();
```

Pass `cancel_token.clone()` into `process_turn` and `followup_turn`.

In the stream loop, replace:
```rust
while let Some(event) = stream.next().await {
```
with:
```rust
loop {
    tokio::select! {
        _ = cancel_token.cancelled() => break,
        event = stream.next() => match event {
            Some(e) => { /* handle */ }
            None => break,
        }
    }
}
```

On `AgentMessage::Cancel`:
```rust
AgentMessage::Cancel => {
    cancel_token.cancel();
    cancel_token = CancellationToken::new(); // fresh token for next turn
    pending_approvals.clear();
}
```

**Verification:** `cargo test -p ucode-agent` passes. New test: cancel stops stream.

---

### Task A4 — Part::Image in ucode-core (multimodal prep)

**File:** `crates/ucode-core/src/message.rs`

Add to `Part` enum:
```rust
/// An image attachment (base64-encoded bytes + MIME type).
Image {
    mime_type: String,
    data: String, // base64
},
```

Add serde round-trip test.

**Note:** This is prep work for ISSUE 1107. The agent loop and providers do not need to use it yet — just ensure the type exists and serializes correctly.

**Verification:** `cargo test -p ucode-core` passes.

---

## Milestone B — TUI core fixes

### Task B1 — Line-level scroll

**File:** `crates/ucode-tui/src/components/transcript.rs`

Current state: `scroll_offset` is an entry index in `AppState`. `TranscriptView` renders all entries top-to-bottom.

Changes:
1. Change `scroll_offset: usize` in `AppState` to represent a **line** offset (not entry index)
2. In `TranscriptView::render`: iterate entries, compute line count for each entry (reuse `entry_height`), skip lines until `scroll_offset` is consumed, then render until area is full
3. Add `total_rendered_lines: usize` to `AppState` (updated after each render) for scroll bounds
4. Auto-scroll: if `scroll_offset >= total_rendered_lines.saturating_sub(area.height as usize)`, snap to bottom on new content
5. Scroll actions: Up/Down = +/-1 line, PageUp/PageDown = +/-`area.height` lines

**Verification:** `cargo test -p ucode-tui` passes. Manual: 200-line code block scrolls smoothly.

---

### Task B2 — Mouse event handling

**File:** `crates/ucode-tui/src/event_loop.rs`

In `handle_terminal_event`, add a `Event::Mouse(mouse_event)` arm:
```rust
Event::Mouse(me) => handle_mouse_event(app, input_box, me),
```

Implement `handle_mouse_event`:
- `MouseEventKind::ScrollUp`: `app.scroll_offset = app.scroll_offset.saturating_sub(3);`
- `MouseEventKind::ScrollDown`: `app.scroll_offset = (app.scroll_offset + 3).min(app.total_rendered_lines.saturating_sub(1));`
- `MouseEventKind::Down(MouseButton::Left)`: click-to-focus using `app.last_layout` to determine which area was clicked

Store `last_layout: Option<ComputedLayout>` in `AppState` (set after each `compute_layout` call).

**Verification:** `cargo test -p ucode-tui` passes.

---

### Task B3 — Bracketed paste handling

**File:** `crates/ucode-tui/src/event_loop.rs`

In `handle_terminal_event`, add `Event::Paste(text)` arm.

Implement `handle_paste_event`:
- Detect file path: no newlines, starts with `/` or `~/`, file exists on disk
- For file paths: add `[file: path]` tag to input box, store path in `app.pending_attachments`
- For text: insert at cursor position in input box

Add `pending_attachments: Vec<Attachment>` to `AppState`.
Add `enum Attachment { File(String), Image { path: String, data: Option<Vec<u8>> } }`.

**Verification:** `cargo test -p ucode-tui` passes. New test: paste of `/etc/hosts` creates file attachment.

---

### Task B4 — Wire approval modal decision to agent loop

**File:** `crates/ucode-tui/src/event_loop.rs`, `crates/ucode-tui/src/overlays/approval_modal.rs`

Current state: `ApprovalModalState` renders correctly. On approve/deny keypress, it sets `app.overlay_queue` to close. The decision is never sent anywhere.

Changes:
1. Store `pending_approval_tool_call_id: Option<String>` in `AppState`
2. On `TuiEvent::ApprovalRequired { tool_call_id, ... }`: set `app.pending_approval_tool_call_id = Some(tool_call_id)`, open approval modal
3. In approval modal keypress handler, after setting modal result: send `AgentMessage::ApprovalDecision` via `app.agent_tx`
4. Wire `AgentMessage::Cancel` to a new `Action::CancelGeneration` triggered by `Ctrl+C` when not in input mode

**Verification:** Manual test: `run_cmd` tool call opens modal; approve/deny sends correct message.

---

### Task B5 — Live sidebar updates

**File:** `crates/ucode-tui/src/event_loop.rs`

`handle_tui_event` currently does not take `sidebar_data` as a parameter. Add it:
```rust
fn handle_tui_event(
    app: &mut AppState,
    input_box: &mut InputBoxState,
    sidebar_data: &mut SidebarData,  // ADD THIS
    event: TuiEvent,
    event_tx: &UnboundedSender<TuiEvent>,
) -> bool
```

Update all call sites. Then add sidebar updates in the match arms:

- `TuiEvent::StreamToken(_)`: `sidebar_data.token_count += 1; sidebar_data.is_generating = true;`
- `TuiEvent::StreamDone`: `sidebar_data.is_generating = false;`
- `TuiEvent::ToolCallStarted { name }`: push `ToolCallEntry { name, status: Running }` to `sidebar_data.tool_calls`
- `TuiEvent::ToolCallCompleted { index, status, .. }`: update `sidebar_data.tool_calls[index].status`
- `TuiEvent::SystemMessage(msg)`: if msg matches `"provider=X model=Y"` pattern, parse and update `sidebar_data.provider` / `sidebar_data.model`

**Verification:** `cargo test -p ucode-tui` passes. Manual: sidebar shows tool calls and token count during generation.

---

### Task B6 — Cancel generation keybind

**File:** `crates/ucode-tui/src/keybinds.rs`, `crates/ucode-tui/src/event_loop.rs`

Add `Action::CancelGeneration` to the `Action` enum.

In `KeybindResolver`, map `Ctrl+C` (when not in input mode, or when generating) to `Action::CancelGeneration`.

In `dispatch_action`:
```rust
Action::CancelGeneration => {
    if let Some(tx) = &app.agent_tx {
        let _ = tx.send(AgentMessage::Cancel);
    }
    app.dirty = true;
}
```

Update status bar to show `[generating... Ctrl+C to cancel]` when `sidebar_data.is_generating`.

**Verification:** `cargo test -p ucode-tui` passes. Manual: Ctrl+C during generation stops the stream.

---

## Milestone C — Image and file attachments

### Task C1 — ratatui-image dependency + protocol detection

**File:** `crates/ucode-tui/Cargo.toml`

```toml
ratatui-image = { version = "2", features = ["crossterm"] }
```

At TUI startup (in `run_event_loop`), run protocol detection and store in `AppState`.

**Verification:** `cargo check -p ucode-tui` passes.

---

### Task C2 — Clipboard image read

**File:** `crates/ucode-tui/src/clipboard.rs` (new function)

```rust
/// Try to read image bytes from the system clipboard.
/// Tries xclip first (X11), then wl-paste (Wayland).
/// Returns None if neither tool is available or clipboard has no image.
pub async fn read_clipboard_image() -> Option<Vec<u8>>
```

In `handle_paste_event` (Task B3), when the pasted path is an image file, spawn a task to call `read_clipboard_image()` and send the result back via `TuiEvent`.

Add `TuiEvent::ImageDataReady { path: String, data: Vec<u8> }` to the event enum.

**Verification:** `cargo check -p ucode-tui` passes.

---

### Task C3 — Image popup overlay

**File:** `crates/ucode-tui/src/overlays/image_popup.rs` (new file)

- `ImagePopupState { path: String, data: Vec<u8>, scroll: usize }`
- Renders as a centered popup (80% width, 80% height)
- Uses `ratatui_image::StatefulImage` with halfblocks protocol (always works in tmux)
- `Ctrl+I` opens popup for the most recent image attachment
- `Esc` closes popup

Register in `overlays/mod.rs` and `overlay_queue.rs`.

**Verification:** `cargo check -p ucode-tui` passes. Manual: image renders in popup.

---

## Milestone D — Session management

### Task D1 — Auto-connect from config on startup

**File:** `crates/ucode-cli/src/main.rs`

In the TUI startup path:
1. Call `AppConfig::load_default()`
2. Call `config.default_provider()` to find a configured provider
3. If found: build `PendingAgentSetup` and pass into `run()`
4. If not found: show a toast suggesting `/connect`

**Verification:** With `ANTHROPIC_API_KEY` set, `cargo run -- tui` starts with agent ready.

---

### Task D2 — Session load/save in TUI

**File:** `crates/ucode-tui/src/lib.rs`, `crates/ucode-tui/src/event_loop.rs`

On TUI startup:
1. Load most recent non-archived session from `SessionStore` (or create new)
2. Populate `app.transcript` from `session.transcript` (convert `Message` to `TranscriptEntry`)
3. Set `app.session_id` and `app.session_title` from session metadata

On TUI exit (before `TerminalGuard` drops):
1. Save current session via `session_store.save(&session)`

Display in status bar: `[session: {title_truncated}]`

**Verification:** Restart TUI and previous messages appear in transcript.

---

### Task D3 — Session picker overlay

**File:** `crates/ucode-tui/src/overlays/session_picker.rs` (new file)

- `SessionPickerState { sessions: Vec<SessionMeta>, selected: usize, filter: String }`
- Lists sessions sorted by `last_active_at` descending
- Filter by title with Up/Down navigation
- Enter to switch: clear transcript, load new session, update sidebar
- `Ctrl+S` keybind to open

Register in `overlays/mod.rs` and `overlay_queue.rs`. Add `Action::OpenSessionPicker` to keybinds.

**Verification:** `cargo test -p ucode-tui` passes. Manual: session list shows and switching works.

---

# Phase 12 — Deferred items

> These were explicitly deferred in Phases 0–10. Implement when the product is stable enough.
> See `docs/backups/2026-03-07/PLANS.md` for original detailed scope.

### Task 12.1 — Prompt/context caching integration [P2]

Originally Task 3.5 / ISSUE 0305. Add provider-aware cache hints for repeated prompt prefixes. Requires Anthropic `cache_control` beta header support and OpenAI prompt caching. Implement when provider APIs stabilize.

**Key files:** `crates/ucode-providers/src/anthropic.rs`, `crates/ucode-providers/src/openai.rs`

### Task 12.2 — Remote plugin install/update with trust verification [P1]

Originally Task 8.7 / ISSUE 0807. CLI commands: `ucode plugin install <url>`, `ucode plugin update <id>`. Ed25519 signature verification, trust records, rollback on failed update.

**Key files:** `crates/ucode-plugins/src/loader.rs`, `crates/ucode-cli/src/main.rs`

### Task 12.3 — Packaging and distribution [P1]

Originally ISSUE 0903. `cargo install ucode` instructions, GitHub Actions release workflow producing Linux (x86_64-musl) and macOS (aarch64, x86_64) binaries. Optional Homebrew formula.

**Key files:** `.github/workflows/release.yml` (new)

### Task 12.4 — Security threat model and audit trail verification [P1]

Originally ISSUE 0904. Write `docs/security-threat-model.md` covering trust boundaries: model output, tool runtime, MCP servers, plugins, subagents, user approvals. Add integration test that exercises denial/approval/sandbox fallback path end-to-end.

**Key files:** `docs/security-threat-model.md` (new), `tests/` (new integration test)

---

## Done checklist (Phase 11)

- [x] A1: `AgentMessage::Cancel` and `AgentMessage::ApprovalDecision` compile
- [x] A2: Approval gate blocks tool execution until TUI decision arrives
- [x] A3: `Ctrl+C` cancels in-progress generation within one poll cycle
- [x] A4: `Part::Image` exists in `ucode-core` with serde round-trip test
- [x] B1: Line-level scroll — 200-line code block scrolls smoothly
- [x] B2: Mouse scroll and click-to-focus work
- [x] B3: Paste of file path creates attachment; paste of text inserts at cursor
- [x] B4: Approval modal sends decision back to agent loop
- [x] B5: Sidebar updates live (tool calls, provider, token count)
- [x] B6: `Ctrl+C` keybind sends `AgentMessage::Cancel`
- [x] C1: `ratatui-image` dependency added; protocol detected at startup
- [x] C2: `read_clipboard_image()` shells to xclip/wl-paste
- [x] C3: Image popup overlay renders with halfblocks
- [x] D1: Auto-connect from env var/keyring on startup
- [x] D2: Session loads on startup; saves on exit; title in status bar
- [x] D3: `Ctrl+O` opens session picker; switching loads transcript
