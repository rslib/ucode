# ucode EPIC — Phase 11 and Beyond

> **Previous phases (0–10) are complete.**
> Full history: `docs/backups/2026-03-07/EPIC.md`
> Detailed task history: `docs/backups/2026-03-07/PLANS.md`

## What was completed (Phases 0–10)

All 10 phases of the original plan are done:

- Phase 0: Workspace bootstrap + CI
- Phase 1: Core runtime (messages, events, router, session, subagents, inter-agent comm, directive parser, token budget, session lifecycle/fork, token governance, structured logging)
- Phase 2: Auth (keychain, all auth flows, provider-specific handlers, token refresh, fallback, OAuth wiring)
- Phase 3: Provider adapters (OpenAI, Anthropic, Ollama, Gemini, generic multi-protocol refactor)
- Phase 4: Built-in tools (registry, fs, search, patch, cmd runner, git, AST, sandbox policy, confirmation gates, checkpoints, background jobs, artifacts)
- Phase 5: MCP client (stdio, SSE, HTTP transports, registry, native launchers, per-server policy, resources/prompts)
- Phase 6: Skills (SKILL.md discovery, parsing, execution binding)
- Phase 7: Fullscreen TUI (transcript, input, sidebar, approvals, palette, visual system, slash commands, toasts, plugin UI API, copy mode, search, keybind overlay, markdown rendering, tmux integration, keybinding presets)
- Phase 8: Plugins and hooks (manifest, hooks API v1, plugin API contract, WIT/WASM runtime, isolation model, external plugin infrastructure, context management system)
- Phase 9: Headless/CI mode
- Phase 10: Agent loop (AppConfig, core loop, TUI wiring, CLI wiring, /models modal, slash command dispatch fix)

## Deferred items from previous phases

These were explicitly deferred and are now carried forward:

- **ISSUE 0305 / Task 3.5** — Prompt/context caching integration [P2]: Provider-aware cache hints for repeated prompt prefixes. Deferred because no provider currently supports it end-to-end in our stack.
- **ISSUE 0807 / Task 8.7** — Remote plugin install/update with trust verification [P1]: Install/update plugins from git/url/registry with Ed25519 signature verification and rollback. Deferred because local plugin loading works and remote distribution is a later-stage concern.
- **ISSUE 0903** — Packaging and distribution (Linux/macOS): `cargo install` instructions, optional GitHub Releases binaries. Deferred until the product is stable enough to ship.
- **ISSUE 0904** — Security threat model and audit trail verification: Formal trust boundary documentation and end-to-end audit trail integration test. Deferred until all runtime components are stable.

These are tracked in PLANS.md under Phase 12 (Deferred).

---

## EPIC 11 — TUI functional completion

> **Goal:** Make the TUI fully functional and production-quality — smooth scrolling, real approval round-trips, mouse input, paste/attachment support, generation cancellation, live sidebar, and session persistence. The TUI components exist but several are cosmetic stubs or have broken wiring.

### Discovered gaps (from code audit, 2026-03-07)

**Agent loop gaps (blocking everything else):**

1. `AgentMessage` enum only has `UserMessage` and `SetModel` — missing `Cancel`, `ApprovalDecision`
2. Tool execution is fully automatic with no approval gate — `process_turn` calls `tool_registry.invoke()` immediately without pausing for TUI approval
3. No cancellation token passed to provider streams — Ctrl+C from TUI does nothing
4. `Part::Image` does not exist in `ucode-core` message types — multimodal input not wired

**TUI gaps (architectural):**

5. Scroll is fundamentally broken: `scroll_offset` is an entry index, but `TranscriptView` renders all entries as lines — inconsistent model makes long messages unsmoothable
6. Approval modal is cosmetic: `ApprovalRequired` event is sent to TUI but decision is never sent back to agent loop; agent never waits for it
7. Mouse completely ignored: `Event::Mouse` falls into `_ => {}`
8. Paste ignored: `Event::Paste` falls into `_ => {}`
9. Sidebar is static: `SidebarData` never updated from agent events
10. No session management: `session_id` and `session_title` are empty strings, never loaded/saved
11. Input box uses rendered cursor (`│` character), not terminal cursor — blocks native text selection

**Image/file handling reality (target: tmux + pure terminal):**

- Drag-drop in terminals: not a protocol — terminal emulator converts OS DnD to bracketed paste of file path only. Works in tmux.
- Image clipboard paste: only way is shell out to `xclip -t image/png` or `wl-paste --type image/png`. Fragile but unavoidable.
- Sixel in tmux: supported since 3.4 but requires compile flag + config. Disappears on resize. Halfblocks always work.
- Popup overlay approach: better than inline sixel in scrollable transcript because it avoids resize artifacts.

---

### ISSUE 1101 — AgentMessage expansion + approval gate [P0]

**Goal:** Add `Cancel` and `ApprovalDecision` variants to `AgentMessage`; implement a real approval gate in `process_turn` using oneshot channels.

**Scope:**

- Add `AgentMessage::Cancel` variant
- Add `AgentMessage::ApprovalDecision { tool_call_id: String, approved: bool }` variant
- Add `AgentEvent::ApprovalRequired { tool_call_id, tool_name, command, cwd, sandbox_label }` variant
- In `process_turn`: before invoking each tool call, send `AgentEvent::ApprovalRequired` and await a oneshot channel for the decision
- Store pending approval oneshots in a `HashMap<String, oneshot::Sender<bool>>` on the agent loop state
- On `AgentMessage::ApprovalDecision`: look up the oneshot sender by `tool_call_id` and send the decision
- On `AgentMessage::Cancel`: abort the current stream (drop it) and clear pending approvals

**Acceptance tests:**

- Tool call that requires approval blocks until TUI sends `ApprovalDecision`.
- Denied tool call returns an error `ToolResult` without executing.
- `Cancel` message aborts the current generation and clears pending approvals.

**Owner:** Agent

---

### ISSUE 1102 — Cancellation token for provider streams [P0]

**Goal:** Wire a `CancellationToken` (from `tokio-util`) into `process_turn` and `followup_turn` so that `AgentMessage::Cancel` actually stops the streaming LLM call.

**Scope:**

- Add `tokio-util` dependency with `CancellationToken`
- Pass a `CancellationToken` into `process_turn` and `followup_turn`
- In the stream loop, select between `stream.next()` and `token.cancelled()` — on cancel, break and emit `AgentEvent::StreamDone`
- On `AgentMessage::Cancel` in the main loop: call `token.cancel()`, then create a fresh token for the next turn

**Acceptance tests:**

- Ctrl+C from TUI sends `Cancel`, stream stops within one poll cycle.
- After cancel, the agent loop is ready to accept the next `UserMessage`.

**Owner:** Agent

---

### ISSUE 1103 — Line-level scroll in TranscriptView [P0]

**Goal:** Replace entry-index scroll with line-level scroll so long messages (code blocks, tool outputs) can be scrolled through smoothly.

**Scope:**

- Add `rendered_line_count: usize` tracking per `TranscriptEntry` (computed on render, cached)
- Change `scroll_offset` from entry index to line index
- `TranscriptView::render` skips lines until `scroll_offset` is reached, then renders until area is full
- Page-up/page-down scroll by `area.height` lines; arrow scroll by 1 line
- Auto-scroll to bottom on new content (when already at bottom)
- `entry_height` already exists for markdown — reuse it for line counting

**Acceptance tests:**

- A 200-line code block can be scrolled through line by line.
- Auto-scroll to bottom works when new tokens arrive.
- Page-up/page-down moves by viewport height.

**Owner:** TUI

---

### ISSUE 1104 — Wire approval modal decision back to agent loop [P0]

**Goal:** Connect the existing `ApprovalModalState` to `AgentMessage::ApprovalDecision` so the agent actually waits for and receives the user's decision.

**Scope:**

- On `TuiEvent::ApprovalRequired`: open approval modal and store `tool_call_id`
- On modal `Approve`/`Deny` keypress: send `AgentMessage::ApprovalDecision { tool_call_id, approved }` via the agent message sender
- Approval modal already renders correctly — only the send-back wiring is missing
- Handle the case where the agent sender is `None` (no agent connected) gracefully

**Acceptance tests:**

- `run_cmd` tool call opens approval modal; pressing `a` sends `ApprovalDecision { approved: true }`.
- Pressing `d` sends `ApprovalDecision { approved: false }`; tool result shows error.
- Modal closes after decision is sent.

**Owner:** TUI/Agent

---

### ISSUE 1105 — Mouse event handling [P1]

**Goal:** Handle `Event::Mouse` in the event loop for scroll and click-to-focus.

**Scope:**

- `MouseEventKind::ScrollUp` / `ScrollDown`: scroll transcript by 3 lines
- `MouseEventKind::Down(MouseButton::Left)`: click-to-focus (transcript vs input box)
- Route mouse events through `handle_terminal_event` instead of falling into `_ => {}`
- Respect `app.mouse_enabled` flag — skip if false

**Acceptance tests:**

- Mouse scroll wheel scrolls transcript.
- Clicking input box focuses it; clicking transcript area focuses transcript.

**Owner:** TUI

---

### ISSUE 1106 — Bracketed paste + file/image discrimination [P1]

**Goal:** Handle `Event::Paste` to detect file paths (drag-drop) vs text, and route accordingly.

**Scope:**

- `Event::Paste(text)`: if text looks like a file path (starts with `/` or `~/`, no newlines, file exists on disk), treat as file attachment; otherwise insert as text into input box
- For file attachments: add `[file: path]` tag to input box content and store path in `app.pending_attachments`
- For image files (`.png`, `.jpg`, `.gif`, `.webp`): additionally attempt clipboard image read (see ISSUE 1107)
- For text paste: insert at cursor position in input box

**Acceptance tests:**

- Pasting `/home/user/file.txt` adds `[file: /home/user/file.txt]` to input.
- Pasting plain text inserts it at cursor.
- Pasting a `.png` path triggers image attachment flow.

**Owner:** TUI

---

### ISSUE 1107 — Clipboard image read + image popup overlay [P2]

**Goal:** Read image data from clipboard via `xclip`/`wl-paste` and display it in a popup overlay using `ratatui-image` with halfblocks fallback.

**Scope:**

- Add `ratatui-image` dependency; run protocol picker at startup (sixel → halfblocks)
- On image attachment: shell out to `xclip -t image/png -o` or `wl-paste --type image/png` to read raw PNG bytes
- Store image bytes in `app.pending_attachments` alongside path
- Add `TranscriptEntry::Attachment { path, preview_bytes: Option<Vec<u8>> }` variant
- Image popup overlay: `Ctrl+I` opens popup showing image using `ratatui-image` halfblocks renderer
- Popup closes on `Esc`
- Graceful fallback: if clipboard read fails, show `[image: path]` text only

**Acceptance tests:**

- Pasting an image path and pressing `Ctrl+I` shows the image in a popup.
- Halfblocks render correctly in tmux.
- If `xclip`/`wl-paste` is unavailable, shows text fallback without crashing.

**Owner:** TUI

---

### ISSUE 1108 — Live sidebar updates from agent events [P0]

**Goal:** Update `SidebarData` from `TuiEvent`s so the sidebar reflects real agent state.

**Scope:**

- On `TuiEvent::ToolCallStarted`: add entry to `sidebar_data.tool_calls` with `Running` status
- On `TuiEvent::ToolCallCompleted`: update matching entry status to `Success`/`Failed`
- On `TuiEvent::SystemMessage` (provider/model info): parse and update `sidebar_data.provider` and `sidebar_data.model`
- On `TuiEvent::StreamToken`: update `sidebar_data.token_count` (increment)
- On `TuiEvent::StreamDone`: update `sidebar_data.is_generating = false`
- Pass `sidebar_data` into `handle_tui_event` (currently missing from signature)

**Acceptance tests:**

- Tool call appears in sidebar while running; updates to success/failed on completion.
- Provider/model name updates in sidebar after `/connect`.
- Token count increments during streaming.

**Owner:** TUI

---

### ISSUE 1109 — Session persistence and picker [P1]

**Goal:** Load/save session on startup/exit and provide a session picker overlay.

**Scope:**

- On TUI startup: load most recent session from `SessionStore` (or create new)
- Display `session_id` (truncated) and `session_title` in status bar
- `Ctrl+S` opens session picker overlay: list sessions sorted by `last_active_at`, navigate with Up/Down, Enter to switch
- On session switch: clear transcript, load new session's messages, update sidebar
- On exit: save current session
- Session picker overlay: reuse overlay queue pattern from existing overlays

**Acceptance tests:**

- Restarting TUI restores the previous session's transcript.
- `Ctrl+S` opens session list; selecting a session loads its transcript.
- Session title appears in status bar.

**Owner:** TUI/Core

---

### ISSUE 1110 — Auto-connect from config on startup [P0]

**Goal:** When a provider is configured (env var or keyring), spawn the agent loop automatically without requiring `/connect`.

**Scope:**

- In `ucode-cli/src/main.rs`: call `AppConfig::load_default()`, then `config.default_provider()`
- If a provider is found: build `AgentLoopConfig` and pass it to `run()` as `Some(PendingAgentSetup)`
- `spawn_agent_loop` already handles `PendingAgentSetup` — just needs to be triggered at startup
- Emit a `TuiEvent::SystemMessage` confirming which provider was auto-connected
- If no provider found: show a toast suggesting `/connect`

**Acceptance tests:**

- With `ANTHROPIC_API_KEY` set, TUI starts and agent is ready without `/connect`.
- With no provider configured, TUI starts and shows a toast suggesting `/connect`.

**Owner:** CLI/Agent

---

## EPIC 12 — Deferred items (carried forward)

> These items were explicitly deferred in Phases 0–10. They are tracked here for visibility. See `docs/backups/2026-03-07/EPIC.md` for original scope.

### ISSUE 1201 — Prompt/context caching integration [P2]

Originally ISSUE 0305 / Task 3.5. Provider-aware cache hints for repeated prompt prefixes. Requires provider support (Anthropic cache_control, OpenAI prompt caching). Deferred until provider APIs stabilize.

### ISSUE 1202 — Remote plugin install/update with trust verification [P1]

Originally ISSUE 0807 / Task 8.7. Install/update plugins from git/url/registry with Ed25519 signature verification, trust records, and rollback. Deferred until local plugin ecosystem matures.

### ISSUE 1203 — Packaging and distribution (Linux/macOS) [P1]

Originally ISSUE 0903. `cargo install` instructions, GitHub Releases binaries, optional Homebrew formula. Deferred until product is stable.

### ISSUE 1204 — Security threat model and audit trail verification [P1]

Originally ISSUE 0904. Formal trust boundary documentation (`docs/security-threat-model.md`) and end-to-end audit trail integration test. Deferred until all runtime components are stable.

---

## Done checklist (Phase 11 targets)

1. Agent loop has real approval gate — tool calls block until TUI approves/denies
2. `AgentMessage::Cancel` stops generation within one poll cycle
3. Transcript scrolls line-by-line through long code blocks and tool outputs
4. Approval modal decision is sent back to agent loop (not cosmetic)
5. Mouse scroll and click-to-focus work
6. Bracketed paste inserts text or creates file attachment
7. Sidebar updates live from agent events (tool calls, provider, token count)
8. Session persists across restarts; session picker works
9. Auto-connect from env var/keyring on startup
10. Image popup overlay with halfblocks rendering (P2)
