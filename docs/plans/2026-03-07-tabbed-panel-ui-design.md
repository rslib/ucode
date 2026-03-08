# Tabbed Panel UI Design

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Restructure the TUI into a tabbed panel system with master-detail views, a simplified status bar, an enriched input box, a full-height collapsible sidebar, and persistent session data for subagent runs, tool calls, MCP logs, and structured events.

---

## Layout

```
┌─ Chat │ Subagents │ Tools │ MCP │ Logs ──────────────────────────────────┐
│                                                                    │ TODO     │
│  [Tab content area]                                                │ Sessions │
│  Each non-Chat tab: list panel (left) + buffer panel (right)       │ Git      │
│  Chat tab: full-width transcript                                   │ ...      │
│                                                                    │ (scroll) │
│                                                                    ├──────────│
│                                                                    │ ~/ucode  │
│                                                                    │ v0.1.0   │
├────────────────────────────────────────────────────────────────────┴──────────│
│ Ask anything...                                                               │
│                                                                               │
│ @coder   claude-opus-4-6   Anthropic                                          │
├───────────────────────────────────────────────────────────────────────────────┤
│ tab agents   ctrl+p commands   ctrl+t models                                  │
└───────────────────────────────────────────────────────────────────────────────┘
```

### Vertical stack (top to bottom)

1. **Tab bar** — 1 row. Neovim-style tabs: `Chat | Subagents | Tools | MCP | Logs`. Active tab highlighted. Navigate with keybind (e.g. `Ctrl+1..5` or `gt`/`gT`).
2. **Main content area** — fills remaining space. Content depends on active tab. Shares horizontal space with sidebar.
3. **Input box** — multiline text input. Bottom line inside the box shows: `@agent_name   model-name   Provider` (always visible, even without provider connected — shows `[not connected]`). Agent name uses the agent's theme color.
4. **Status bar** — 1 row. Keybind hints only + optional progress indicator (streaming speed, compaction). Everything else (cost, tokens, git, session title) moves to sidebar.

### Sidebar (right, full height)

- Spans the full terminal height (title bar to status bar), like crush/opencode.
- Collapsible via keybind (e.g. `Ctrl+B`).
- **Scrollable body**: toggleable sections — TODO items, Sessions, Git info, Cost/Tokens, Diff stats.
- **Fixed footer** (always visible at bottom of sidebar): working directory path + `ucode vX.Y.Z`.
- Each section header is clickable/toggleable to collapse/expand.

---

## UI Wireframes

### Full layout — Chat tab active, sidebar open

```
┌─ Chat │ Subagents │ Tools │ MCP │ Logs ──────────────────────┬───────────────┐
│                                                               │ TODO          │
│  You: Fix the broken layout tests                             │ ▸ fix tests   │
│                                                               │   add docs    │
│  Assistant:                                                   │               │
│  I'll fix the 3 failing layout tests. The issue is that       │ SESSIONS      │
│  `compute_layout` now applies horizontal padding...           │ ▸ fix-tests   │
│                                                               │   refactor    │
│  ✓ Read crates/ucode-tui/src/layout.rs           45ms        │   initial     │
│  ✓ Edit crates/ucode-tui/src/layout.rs           12ms        │               │
│  ✓ Bash cargo test -p ucode-tui -- layout       1.2s         │ GIT           │
│                                                               │   main        │
│  All 10 layout tests now pass.                                │   +12 -3      │
│                                                               │               │
│                                                               │ COST          │
│                                                               │   $0.0342     │
│                                                               │   4.2k tokens │
│                                                               │               │
│                                                               ├───────────────┤
│                                                               │ ~/code/ucode  │
│                                                               │ ucode v0.1.0  │
├───────────────────────────────────────────────────────────────┴───────────────┤
│ Ask anything... "Fix broken tests"                                            │
│                                                                               │
│ @coder   claude-opus-4-6   Anthropic                                          │
├───────────────────────────────────────────────────────────────────────────────┤
│ tab agents   ctrl+p commands   ctrl+t models                                  │
└───────────────────────────────────────────────────────────────────────────────┘
```

### Full layout — Chat tab active, sidebar collapsed

```
┌─ Chat │ Subagents │ Tools │ MCP │ Logs ──────────────────────────────────────┐
│                                                                               │
│  You: Fix the broken layout tests                                             │
│                                                                               │
│  Assistant:                                                                   │
│  I'll fix the 3 failing layout tests. The issue is that                       │
│  `compute_layout` now applies horizontal padding...                           │
│                                                                               │
│  ✓ Read crates/ucode-tui/src/layout.rs           45ms                        │
│  ✓ Edit crates/ucode-tui/src/layout.rs           12ms                        │
│  ✓ Bash cargo test -p ucode-tui -- layout       1.2s                         │
│                                                                               │
│  All 10 layout tests now pass.                                                │
│                                                                               │
├───────────────────────────────────────────────────────────────────────────────┤
│ Ask anything... "Fix broken tests"                                            │
│                                                                               │
│ @coder   claude-opus-4-6   Anthropic                                          │
├───────────────────────────────────────────────────────────────────────────────┤
│ tab agents   ctrl+p commands   ctrl+t models                                  │
└───────────────────────────────────────────────────────────────────────────────┘
```

### Chat tab — no provider connected

```
┌─ Chat │ Subagents │ Tools │ MCP │ Logs ──────────────────────┬───────────────┐
│                                                               │ TODO          │
│                                                               │   (empty)     │
│                                                               │               │
│                                                               │ SESSIONS      │
│              Ask anything... "Fix broken tests"               │   (none)      │
│                                                               │               │
│                                                               │               │
│                                                               │               │
│                                                               ├───────────────┤
│                                                               │ ~/code/ucode  │
│                                                               │ ucode v0.1.0  │
├───────────────────────────────────────────────────────────────┴───────────────┤
│ Ask anything...                                                               │
│                                                                               │
│ @coder   [not connected]                                                      │
├───────────────────────────────────────────────────────────────────────────────┤
│ tab agents   ctrl+p commands   /connect to start                              │
└───────────────────────────────────────────────────────────────────────────────┘
```

### Subagents tab — with entries

```
┌─ Chat │ Subagents │ Tools │ MCP │ Logs ──────────────────────┬───────────────┐
│ ┌─ List ──────────────┬─ Buffer ─────────────────────────────┐│ TODO          │
│ │                     │                                      ││ ...           │
│ │ ▸ rust-expert    ✓  │ # Rust-Expert Task                   ││               │
│ │   1.2s  890 tok     │                                      ││ SESSIONS      │
│ │                     │ C3+D1: @mention routing               ││ ...           │
│ │   explore        ✓  │ (59 tool calls)                      ││               │
│ │   0.4s  210 tok     │                                      ││ GIT           │
│ │                     │ ## Summary                            ││ ...           │
│ │   quick-fix      ⟳  │                                      ││               │
│ │   running...        │ - Added `FileContext` struct          ││               │
│ │                     │ - Changed `AgentMessage::UserMessage` ││               │
│ │                     │   from tuple to struct variant        ││               │
│ │                     │ - Updated 20+ call sites              ││               │
│ │                     │                                      ││               │
│ │                     │ ### Verification                      ││               │
│ │                     │ ```                                   ││               │
│ │                     │ cargo test --workspace                ││               │
│ │                     │ 1725 passed, 0 failed                 ││               │
│ │                     │ ```                                   ││               │
│ └─────────────────────┴──────────────────────────────────────┘│               │
│                                                               ├───────────────┤
│                                                               │ ~/code/ucode  │
│                                                               │ ucode v0.1.0  │
├───────────────────────────────────────────────────────────────┴───────────────┤
│ Ask anything...                                                               │
│                                                                               │
│ @coder   claude-opus-4-6   Anthropic                                          │
├───────────────────────────────────────────────────────────────────────────────┤
│ tab agents   ctrl+p commands   ctrl+t models                                  │
└───────────────────────────────────────────────────────────────────────────────┘
```

### Subagents tab — empty state

```
┌─ Chat │ Subagents │ Tools │ MCP │ Logs ──────────────────────┬───────────────┐
│                                                               │               │
│                                                               │               │
│                                                               │               │
│                No subagent runs in this session                │               │
│                                                               │               │
│                                                               │               │
│                                                               ├───────────────┤
│                                                               │ ~/code/ucode  │
│                                                               │ ucode v0.1.0  │
├───────────────────────────────────────────────────────────────┴───────────────┤
│ Ask anything...                                                               │
│                                                                               │
│ @coder   claude-opus-4-6   Anthropic                                          │
├───────────────────────────────────────────────────────────────────────────────┤
│ tab agents   ctrl+p commands   ctrl+t models                                  │
└───────────────────────────────────────────────────────────────────────────────┘
```

### Tools tab — with entries

```
┌─ Chat │ Subagents │ Tools │ MCP │ Logs ──────────────────────┬───────────────┐
│ ┌─ List ──────────────┬─ Buffer ─────────────────────────────┐│               │
│ │ Filter: ________    │                                      ││               │
│ │                     │ Read                                  ││               │
│ │ ▸ Read           ✓  │ Status: ✓ Success                    ││               │
│ │   layout.rs  45ms   │ Duration: 45ms                       ││               │
│ │                     │                                      ││               │
│ │   Edit           ✓  │ Input:                                ││               │
│ │   layout.rs  12ms   │   file: crates/ucode-tui/src/        ││               │
│ │                     │         layout.rs                     ││               │
│ │   Bash           ✓  │   offset: 286                        ││               │
│ │   cargo test 1.2s   │   limit: 100                         ││               │
│ │                     │                                      ││               │
│ │   Grep           ✓  │ Output:                               ││               │
│ │   "TODO"     120ms  │   176: #[test]                        ││               │
│ │                     │   177: fn terminal_size_minimum() {   ││               │
│ │   Write          ✗  │   178:     assert!(TerminalSize {    ││               │
│ │   config.rs  480ms  │   179:         width: 80,            ││               │
│ │                     │   180:         height: 24            ││               │
│ │                     │   ...                                 ││               │
│ └─────────────────────┴──────────────────────────────────────┘│               │
│                                                               ├───────────────┤
│                                                               │ ~/code/ucode  │
│                                                               │ ucode v0.1.0  │
├───────────────────────────────────────────────────────────────┴───────────────┤
│ Ask anything...                                                               │
│                                                                               │
│ @coder   claude-opus-4-6   Anthropic                                          │
├───────────────────────────────────────────────────────────────────────────────┤
│ tab agents   ctrl+p commands   ctrl+t models                                  │
└───────────────────────────────────────────────────────────────────────────────┘
```

### MCP tab — with servers

```
┌─ Chat │ Subagents │ Tools │ MCP │ Logs ──────────────────────┬───────────────┐
│ ┌─ Servers ───────────┬─ Buffer ─────────────────────────────┐│               │
│ │                     │                                      ││               │
│ │ ▸ context7       ●  │ context7                              ││               │
│ │   12 tools          │ Status: connected                    ││               │
│ │                     │ Tools: 12                             ││               │
│ │   git-docs       ●  │                                      ││               │
│ │   3 tools           │ ── Tool Catalog ──────────────────── ││               │
│ │                     │ resolve-library-id                    ││               │
│ │   arxiv          ○  │   Resolve package name to library ID ││               │
│ │   disconnected      │ query-docs                            ││               │
│ │                     │   Query documentation and examples   ││               │
│ │                     │                                      ││               │
│ │                     │ ── Request Log (14 calls) ────────── ││               │
│ │                     │                                      ││               │
│ │                     │ 14:23:05 query-docs         ✓  45ms  ││               │
│ │                     │   lib=/vercel/next.js query="app ro" ││               │
│ │                     │                                      ││               │
│ │                     │ 14:22:58 resolve-library-id ✓  120ms ││               │
│ │                     │   name="next.js"                     ││               │
│ │                     │                                      ││               │
│ └─────────────────────┴──────────────────────────────────────┘│               │
│                                                               ├───────────────┤
│                                                               │ ~/code/ucode  │
│                                                               │ ucode v0.1.0  │
├───────────────────────────────────────────────────────────────┴───────────────┤
│ Ask anything...                                                               │
│                                                                               │
│ @coder   claude-opus-4-6   Anthropic                                          │
├───────────────────────────────────────────────────────────────────────────────┤
│ tab agents   ctrl+p commands   ctrl+t models                                  │
└───────────────────────────────────────────────────────────────────────────────┘
```

### Logs tab — with events

```
┌─ Chat │ Subagents │ Tools │ MCP │ Logs ──────────────────────┬───────────────┐
│ ┌─ Events ────────────┬─ Detail ─────────────────────────────┐│               │
│ │ Filter: [all  ▾]    │                                      ││               │
│ │                     │ Agent Spawn                           ││               │
│ │ 14:23:01 INFO       │                                      ││               │
│ │ ▸ agent_spawn       │ Time: 2026-03-07 14:23:01            ││               │
│ │   rust-expert       │ Level: INFO                          ││               │
│ │                     │ Type: agent_spawn                    ││               │
│ │ 14:23:00 INFO       │                                      ││               │
│ │   model_switch      │ Agent: rust-expert                   ││               │
│ │   opus-4-6          │ Task: C3+D1 @mention routing         ││               │
│ │                     │ Model: claude-opus-4-6               ││               │
│ │ 14:22:58 WARN       │                                      ││               │
│ │   budget_warning    │ Detail:                               ││               │
│ │   75% used          │   Spawned subagent rust-expert for   ││               │
│ │                     │   implementing AgentMessage struct   ││               │
│ │ 14:22:45 ERROR      │   expansion and directive parsing.   ││               │
│ │   tool_failed       │   Estimated token budget: 8000.      ││               │
│ │   Write: denied     │                                      ││               │
│ │                     │                                      ││               │
│ └─────────────────────┴──────────────────────────────────────┘│               │
│                                                               ├───────────────┤
│                                                               │ ~/code/ucode  │
│                                                               │ ucode v0.1.0  │
├───────────────────────────────────────────────────────────────┴───────────────┤
│ Ask anything...                                                               │
│                                                                               │
│ @coder   claude-opus-4-6   Anthropic                                          │
├───────────────────────────────────────────────────────────────────────────────┤
│ tab agents   ctrl+p commands   ctrl+t models                                  │
└───────────────────────────────────────────────────────────────────────────────┘
```

### Logs tab — filtered to errors only

```
┌─ Chat │ Subagents │ Tools │ MCP │ Logs ──────────────────────┬───────────────┐
│ ┌─ Events ────────────┬─ Detail ─────────────────────────────┐│               │
│ │ Filter: [error ▾]   │                                      ││               │
│ │                     │ Tool Failed                           ││               │
│ │ 14:22:45 ERROR      │                                      ││               │
│ │ ▸ tool_failed       │ Time: 2026-03-07 14:22:45            ││               │
│ │   Write: denied     │ Level: ERROR                         ││               │
│ │                     │ Type: tool_failed                    ││               │
│ │ 14:20:12 ERROR      │                                      ││               │
│ │   provider_error    │ Tool: Write                           ││               │
│ │   timeout           │ Error: permission denied              ││               │
│ │                     │   (sandbox: workspace)                ││               │
│ │                     │                                      ││               │
│ │                     │ Detail:                               ││               │
│ │                     │   Attempted to write src/config.rs   ││               │
│ │                     │   but sandbox policy denied the      ││               │
│ │                     │   operation. File is outside the     ││               │
│ │                     │   allowed workspace boundary.        ││               │
│ │                     │                                      ││               │
│ └─────────────────────┴──────────────────────────────────────┘│               │
│                                                               ├───────────────┤
│                                                               │ ~/code/ucode  │
│                                                               │ ucode v0.1.0  │
├───────────────────────────────────────────────────────────────┴───────────────┤
│ Ask anything...                                                               │
│                                                                               │
│ @coder   claude-opus-4-6   Anthropic                                          │
├───────────────────────────────────────────────────────────────────────────────┤
│ tab agents   ctrl+p commands   ctrl+t models                                  │
└───────────────────────────────────────────────────────────────────────────────┘
```

### Session picker popup (/resume)

```
┌─ Chat │ Subagents │ Tools │ MCP │ Logs ──────────────────────┬───────────────┐
│                                                               │               │
│         ┌─ Resume Session ──────────────────────────┐         │               │
│         │ Filter: fix____                           │         │               │
│         │                                           │         │               │
│         │ ▸ fix-layout-tests                        │         │               │
│         │   Mar 7, 14:20  12 msgs  opus-4-6         │         │               │
│         │                                           │         │               │
│         │   fix-auth-flow                           │         │               │
│         │   Mar 7, 10:15   8 msgs  sonnet-4         │         │               │
│         │                                           │         │               │
│         │   fix-clipboard-paste                     │         │               │
│         │   Mar 6, 22:30   5 msgs  sonnet-4         │         │               │
│         │                                           │         │               │
│         │ ↑↓ navigate  Enter select  Esc cancel     │         │               │
│         └───────────────────────────────────────────┘         │               │
│                                                               ├───────────────┤
│                                                               │ ~/code/ucode  │
│                                                               │ ucode v0.1.0  │
├───────────────────────────────────────────────────────────────┴───────────────┤
│ Ask anything...                                                               │
│                                                                               │
│ @coder   claude-opus-4-6   Anthropic                                          │
├───────────────────────────────────────────────────────────────────────────────┤
│ tab agents   ctrl+p commands   ctrl+t models                                  │
└───────────────────────────────────────────────────────────────────────────────┘
```

### Sidebar sections — expanded

```
┌───────────────┐
│ TODO          │
│ ▾ (3 items)   │
│   ☐ fix tests │
│   ☐ add docs  │
│   ☑ refactor  │
│               │
│ SESSIONS      │
│ ▾ (3 items)   │
│ ▸ fix-tests   │  ← current session (highlighted)
│   refactor    │
│   initial     │
│               │
│ GIT           │
│ ▾             │
│   main        │
│   +12 -3      │
│               │
│ COST          │
│ ▾             │
│   $0.0342     │
│   4.2k / 200k │
│               │
├───────────────┤
│ ~/code/ucode  │  ← fixed footer
│ ucode v0.1.0  │
└───────────────┘
```

### Sidebar sections — some collapsed

```
┌───────────────┐
│ TODO        ▸ │  ← collapsed (▸ indicator)
│ SESSIONS    ▸ │
│               │
│ GIT           │
│ ▾             │
│   main        │
│   +12 -3      │
│               │
│ COST          │
│ ▾             │
│   $0.0342     │
│   4.2k / 200k │
│               │
│               │
│               │
│               │
├───────────────┤
│ ~/code/ucode  │
│ ucode v0.1.0  │
└───────────────┘
```

---

## Tab contents (detailed)

### Chat tab

Full-width transcript — the current main view. No list/buffer split. This is the default active tab.

### Subagents tab

| Panel | Content |
|-------|---------|
| **List** (left, ~30% width) | All subagent invocations in this session. Each entry: agent name, status icon (`✓` success, `✗` failed, `⟳` running), duration, token count. Sorted chronologically. Scrollable. Selected entry highlighted. |
| **Buffer** (right, ~70% width) | Full output of the selected subagent. Rendered as markdown. Scrollable. Includes: task description, all tool calls made by the subagent, final output, verification results. |

### Tools tab

| Panel | Content |
|-------|---------|
| **List** (left, ~30% width) | All tool calls in the session. Each entry: tool name, key args (file path, command), status icon, duration. Sorted chronologically. Scrollable. Filter input at top to narrow by tool name. |
| **Buffer** (right, ~70% width) | Selected tool call detail: full input parameters (formatted), full output, thinking (if any), duration, status. Scrollable. |

### MCP tab

| Panel | Content |
|-------|---------|
| **List** (left, ~30% width) | All connected MCP servers. Each entry: server name, connection status (`●` connected, `○` disconnected, `✗` error), tool count. |
| **Buffer** (right, ~70% width) | Two sections for the selected server: (1) **Tool catalog** — list of tools with name and description. (2) **Request log** — all request/response entries for that server across the entire session, chronologically. Each entry: timestamp, method, summary, status, duration. Expandable to show full request/response JSON. |

### Logs tab

| Panel | Content |
|-------|---------|
| **List** (left, ~30% width) | Structured event log. Each entry: timestamp, level badge (`INFO`/`WARN`/`ERROR`), event type, brief summary. Dropdown filter at top to narrow by level (all/info/warn/error). Scrollable. |
| **Buffer** (right, ~70% width) | Selected event detail: all fields expanded. For complex events (agent spawns, tool failures), shows full context. Scrollable. |

---

## Data model

### New types (in ucode-core)

```rust
/// A single subagent invocation and its output.
pub struct SubagentRun {
    pub id: String,
    pub agent_name: String,
    pub task_description: String,
    pub status: RunStatus,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub duration_ms: Option<u64>,
    pub token_count: Option<u64>,
    pub output: String,           // markdown
    pub tool_calls: Vec<String>,  // tool_call IDs cross-referencing ToolRun
}

/// A single tool invocation.
pub struct ToolRun {
    pub id: String,
    pub tool_name: String,
    pub args_summary: String,     // e.g. "file=src/main.rs, offset=1"
    pub status: RunStatus,
    pub started_at: DateTime<Utc>,
    pub duration_ms: Option<u64>,
    pub input: String,            // full input params (JSON or formatted)
    pub output: Option<String>,   // full output
    pub thinking: Option<String>,
    pub subagent_id: Option<String>, // which subagent spawned this, if any
}

/// An MCP request/response log entry.
pub struct McpLogEntry {
    pub id: String,
    pub server_name: String,
    pub method: String,           // e.g. "tools/call", "resources/read"
    pub request_summary: String,
    pub request_body: String,     // full JSON
    pub response_body: Option<String>,
    pub status: RunStatus,
    pub timestamp: DateTime<Utc>,
    pub duration_ms: Option<u64>,
}

/// A structured session event for the Logs tab.
pub struct SessionEvent {
    pub timestamp: DateTime<Utc>,
    pub level: EventLevel,        // Info, Warn, Error
    pub event_type: String,       // "model_switch", "agent_spawn", "approval", etc.
    pub summary: String,
    pub detail: Option<String>,
}

pub enum RunStatus {
    Running,
    Success,
    Failed,
    Cancelled,
}

pub enum EventLevel {
    Info,
    Warn,
    Error,
}
```

### Session storage

```rust
pub struct Session {
    // ... existing fields ...
    pub transcript: Vec<Message>,

    // NEW — operational metadata
    pub subagent_runs: Vec<SubagentRun>,
    pub tool_runs: Vec<ToolRun>,
    pub mcp_logs: Vec<McpLogEntry>,
    pub event_log: Vec<SessionEvent>,
}
```

All serialized into the same session JSON file. If size becomes a problem later, options: gzip compression, separate sidecar files, or pruning old entries.

---

## Input box changes

### Current

```
┌─────────────────────────────────┐
│ text input area                 │
└─────────────────────────────────┘
```

### New

```
┌─────────────────────────────────┐
│ text input area                 │
│                                 │
│ @coder   claude-opus-4-6   Anthropic │
└─────────────────────────────────┘
```

- The info line is always visible, even with no provider connected: `@coder   [not connected]`
- Agent name uses the agent's auto-generated color.
- Tab key cycles the agent name (existing behavior).
- Model name and provider update when `/connect` or `/models` changes them.

---

## Status bar simplification

### Current (too much)

```
[ses_2026...] │ ^P ^O ^E │ INFO │ main │ ●off │ @coder │ [unknown] │ $0.0000 │ 0/0
```

### New

```
tab agents   ctrl+p commands   ctrl+t models   ◐ 42 tok/s
```

Only keybind hints + optional streaming progress. All other info moves to sidebar sections.

---

## Session picker popup

Triggered by: `ucode --continue`, `ucode --resume`, or `/resume` command.

Opens a modal (similar to `/models` modal pattern):
- Filter line at top
- List of recent sessions: title, date, message count, last model used
- Keyboard navigation (Up/Down, Enter to select, Esc to cancel)
- Selected session loads all data (transcript + subagent_runs + tool_runs + mcp_logs + event_log)

---

## Keybinds

| Key | Action |
|-----|--------|
| `Tab` | Cycle active agent (when no autocomplete) |
| `Ctrl+1..5` or `g1..g5` | Switch to tab 1-5 |
| `gt` / `gT` | Next/prev tab (neovim style) |
| `Ctrl+B` | Toggle sidebar |
| `Ctrl+P` | Command palette |
| `Ctrl+T` | Models modal |

---

## Demo strategy

Build the full UI in `crates/ucode-tui/examples/demo.rs` first:
- Fake subagent runs, tool calls, MCP entries, log events
- All tabs navigable with real list+buffer interaction
- Sidebar with fake sections
- Input box with agent/model/provider line
- Simplified status bar

This lets us validate the look and feel before wiring real data.
