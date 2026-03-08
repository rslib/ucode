# Tabbed Panel UI Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Restructure the TUI into a tabbed panel system with master-detail views, simplified status bar, enriched input box with agent/model/provider info, full-height collapsible sidebar, and persistent session data for subagent runs, tool calls, MCP logs, and structured events. Build the demo first.

**Architecture:** The tab system is a new `TabBar` component with a `TabId` enum. Each non-Chat tab renders a `MasterDetail` widget (reusable list+buffer split). The layout engine gains a `tab_bar` row and the sidebar becomes full-height (spanning tab bar through input). Session gains 4 new `Vec` fields for operational metadata. The input box grows an info line at the bottom. The status bar is stripped to keybind hints + streaming progress only.

**Tech Stack:** Rust, ratatui, serde, chrono, existing ucode-tui component patterns.

**Design doc:** `docs/plans/2026-03-07-tabbed-panel-ui-design.md`

---

## Milestone ordering

```
Phase 1: Data model types (ucode-core)
Phase 2: Tab bar + layout restructure (ucode-tui)
Phase 3: Master-detail widget (ucode-tui)
Phase 4: Input box info line (ucode-tui)
Phase 5: Status bar simplification (ucode-tui)
Phase 6: Sidebar restructure (ucode-tui)
Phase 7: Demo (ucode-tui/examples/demo.rs)
Phase 8: Wire real data (ucode-agent, ucode-tui)
Phase 9: Session persistence (ucode-core)
Phase 10: Session picker popup (ucode-tui)
```

**Demo-first strategy:** Phases 1-7 build the full UI with fake data in `demo.rs`. Phases 8-10 wire real data and persistence. This lets you validate the look and feel before committing to the plumbing.

---

### Task 1: Data model types for operational metadata

**Files:**
- Create: `crates/ucode-core/src/operational.rs`
- Modify: `crates/ucode-core/src/lib.rs`

**Step 1: Write the failing test**

In `crates/ucode-core/src/operational.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_status_default_is_running() {
        let s = RunStatus::Running;
        assert_eq!(s.icon(), '⟳');
        assert!(!s.is_terminal());
    }

    #[test]
    fn run_status_terminal_states() {
        assert!(RunStatus::Success.is_terminal());
        assert!(RunStatus::Failed.is_terminal());
        assert!(RunStatus::Cancelled.is_terminal());
        assert!(!RunStatus::Running.is_terminal());
    }

    #[test]
    fn event_level_badge() {
        assert_eq!(EventLevel::Info.badge(), "INFO");
        assert_eq!(EventLevel::Warn.badge(), "WARN");
        assert_eq!(EventLevel::Error.badge(), "ERROR");
    }

    #[test]
    fn subagent_run_roundtrip() {
        let run = SubagentRun {
            id: "sa-001".into(),
            agent_name: "rust-expert".into(),
            task_description: "Fix tests".into(),
            status: RunStatus::Success,
            started_at: chrono::Utc::now(),
            completed_at: Some(chrono::Utc::now()),
            duration_ms: Some(1200),
            token_count: Some(890),
            output: "# Summary\nAll tests pass.".into(),
            tool_call_ids: vec!["tc-001".into(), "tc-002".into()],
        };
        let json = serde_json::to_string(&run).unwrap();
        let decoded: SubagentRun = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.id, "sa-001");
        assert_eq!(decoded.agent_name, "rust-expert");
        assert_eq!(decoded.tool_call_ids.len(), 2);
    }

    #[test]
    fn tool_run_roundtrip() {
        let run = ToolRun {
            id: "tc-001".into(),
            tool_name: "Read".into(),
            args_summary: "file=src/main.rs".into(),
            status: RunStatus::Success,
            started_at: chrono::Utc::now(),
            duration_ms: Some(45),
            input: r#"{"file":"src/main.rs","offset":1}"#.into(),
            output: Some("fn main() {}".into()),
            thinking: None,
            subagent_id: Some("sa-001".into()),
        };
        let json = serde_json::to_string(&run).unwrap();
        let decoded: ToolRun = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.tool_name, "Read");
    }

    #[test]
    fn mcp_log_entry_roundtrip() {
        let entry = McpLogEntry {
            id: "mcp-001".into(),
            server_name: "context7".into(),
            method: "tools/call".into(),
            request_summary: "query-docs next.js".into(),
            request_body: "{}".into(),
            response_body: Some("{}".into()),
            status: RunStatus::Success,
            timestamp: chrono::Utc::now(),
            duration_ms: Some(120),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let decoded: McpLogEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.server_name, "context7");
    }

    #[test]
    fn session_event_roundtrip() {
        let event = SessionEvent {
            timestamp: chrono::Utc::now(),
            level: EventLevel::Warn,
            event_type: "budget_warning".into(),
            summary: "Token budget at 75%".into(),
            detail: Some("Used 150k of 200k tokens".into()),
        };
        let json = serde_json::to_string(&event).unwrap();
        let decoded: SessionEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.event_type, "budget_warning");
        assert_eq!(decoded.level, EventLevel::Warn);
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p ucode-core -- operational`
Expected: FAIL — module doesn't exist

**Step 3: Implement the types**

In `crates/ucode-core/src/operational.rs`:

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Status of a run (subagent, tool call, MCP request).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Success,
    Failed,
    Cancelled,
}

impl RunStatus {
    pub fn icon(self) -> char {
        match self {
            Self::Running => '⟳',
            Self::Success => '✓',
            Self::Failed => '✗',
            Self::Cancelled => '○',
        }
    }

    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Running)
    }
}

/// Severity level for session events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventLevel {
    Info,
    Warn,
    Error,
}

impl EventLevel {
    pub fn badge(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
}

/// A single subagent invocation and its output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentRun {
    pub id: String,
    pub agent_name: String,
    pub task_description: String,
    pub status: RunStatus,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub duration_ms: Option<u64>,
    pub token_count: Option<u64>,
    /// Full output (markdown).
    pub output: String,
    /// Tool call IDs cross-referencing ToolRun entries.
    pub tool_call_ids: Vec<String>,
}

/// A single tool invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolRun {
    pub id: String,
    pub tool_name: String,
    /// e.g. "file=src/main.rs, offset=1"
    pub args_summary: String,
    pub status: RunStatus,
    pub started_at: DateTime<Utc>,
    pub duration_ms: Option<u64>,
    /// Full input parameters (JSON or formatted string).
    pub input: String,
    pub output: Option<String>,
    pub thinking: Option<String>,
    /// Which subagent spawned this, if any.
    pub subagent_id: Option<String>,
}

/// An MCP request/response log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpLogEntry {
    pub id: String,
    pub server_name: String,
    /// e.g. "tools/call", "resources/read"
    pub method: String,
    pub request_summary: String,
    pub request_body: String,
    pub response_body: Option<String>,
    pub status: RunStatus,
    pub timestamp: DateTime<Utc>,
    pub duration_ms: Option<u64>,
}

/// A structured session event for the Logs tab.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEvent {
    pub timestamp: DateTime<Utc>,
    pub level: EventLevel,
    /// e.g. "model_switch", "agent_spawn", "approval", "budget_warning"
    pub event_type: String,
    pub summary: String,
    pub detail: Option<String>,
}
```

Add `pub mod operational;` to `crates/ucode-core/src/lib.rs`.

**Step 4: Run test to verify it passes**

Run: `cargo test -p ucode-core -- operational`
Expected: PASS

**Step 5: Commit**

```
feat(core): add operational metadata types (SubagentRun, ToolRun, McpLogEntry, SessionEvent)
```

---

### Task 2: Tab bar component

**Files:**
- Create: `crates/ucode-tui/src/components/tab_bar.rs`
- Modify: `crates/ucode-tui/src/components/mod.rs`

**Step 1: Write the failing test**

In `crates/ucode-tui/src/components/tab_bar.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_id_all_returns_five() {
        assert_eq!(TabId::all().len(), 5);
    }

    #[test]
    fn tab_id_labels() {
        assert_eq!(TabId::Chat.label(), "Chat");
        assert_eq!(TabId::Subagents.label(), "Subagents");
        assert_eq!(TabId::Tools.label(), "Tools");
        assert_eq!(TabId::Mcp.label(), "MCP");
        assert_eq!(TabId::Logs.label(), "Logs");
    }

    #[test]
    fn tab_state_next_wraps() {
        let mut state = TabBarState::new();
        assert_eq!(state.active, TabId::Chat);
        state.next();
        assert_eq!(state.active, TabId::Subagents);
        state.next();
        state.next();
        state.next(); // Logs
        state.next(); // wraps to Chat
        assert_eq!(state.active, TabId::Chat);
    }

    #[test]
    fn tab_state_prev_wraps() {
        let mut state = TabBarState::new();
        state.prev(); // wraps to Logs
        assert_eq!(state.active, TabId::Logs);
    }

    #[test]
    fn tab_state_select_by_index() {
        let mut state = TabBarState::new();
        state.select(2); // Tools (0-indexed)
        assert_eq!(state.active, TabId::Tools);
    }

    #[test]
    fn tab_state_select_out_of_bounds_noop() {
        let mut state = TabBarState::new();
        state.select(99);
        assert_eq!(state.active, TabId::Chat); // unchanged
    }

    #[test]
    fn tab_state_badge_counts() {
        let mut state = TabBarState::new();
        state.set_badge(TabId::Subagents, 3);
        state.set_badge(TabId::Logs, 1);
        assert_eq!(state.badge(TabId::Subagents), Some(3));
        assert_eq!(state.badge(TabId::Logs), Some(1));
        assert_eq!(state.badge(TabId::Chat), None);
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p ucode-tui -- tab_bar`
Expected: FAIL — module doesn't exist

**Step 3: Implement**

In `crates/ucode-tui/src/components/tab_bar.rs`:

```rust
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::theme::UcodeTheme;

// ---------------------------------------------------------------------------
// TabId
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TabId {
    Chat,
    Subagents,
    Tools,
    Mcp,
    Logs,
}

impl TabId {
    pub fn all() -> &'static [TabId] {
        &[
            TabId::Chat,
            TabId::Subagents,
            TabId::Tools,
            TabId::Mcp,
            TabId::Logs,
        ]
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Chat => "Chat",
            Self::Subagents => "Subagents",
            Self::Tools => "Tools",
            Self::Mcp => "MCP",
            Self::Logs => "Logs",
        }
    }

    pub fn index(self) -> usize {
        TabId::all().iter().position(|&t| t == self).unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------
// TabBarState
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct TabBarState {
    pub active: TabId,
    badges: [Option<usize>; 5],
}

impl TabBarState {
    pub fn new() -> Self {
        Self {
            active: TabId::Chat,
            badges: [None; 5],
        }
    }

    pub fn next(&mut self) {
        let tabs = TabId::all();
        let idx = self.active.index();
        self.active = tabs[(idx + 1) % tabs.len()];
    }

    pub fn prev(&mut self) {
        let tabs = TabId::all();
        let idx = self.active.index();
        self.active = tabs[(idx + tabs.len() - 1) % tabs.len()];
    }

    pub fn select(&mut self, index: usize) {
        let tabs = TabId::all();
        if index < tabs.len() {
            self.active = tabs[index];
        }
    }

    pub fn set_badge(&mut self, tab: TabId, count: usize) {
        self.badges[tab.index()] = if count > 0 { Some(count) } else { None };
    }

    pub fn badge(&self, tab: TabId) -> Option<usize> {
        self.badges[tab.index()]
    }
}

impl Default for TabBarState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// TabBar widget
// ---------------------------------------------------------------------------

pub struct TabBar<'a> {
    pub state: &'a TabBarState,
    pub theme: &'a UcodeTheme,
}

impl<'a> TabBar<'a> {
    pub fn new(state: &'a TabBarState, theme: &'a UcodeTheme) -> Self {
        Self { state, theme }
    }
}

impl Widget for TabBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        // Background.
        let bg_style = Style::new()
            .bg(self.theme.surface)
            .fg(self.theme.text_dim);
        for x in area.x..area.x + area.width {
            buf[(x, area.y)].set_style(bg_style).set_char(' ');
        }

        let mut x = area.x + 1; // 1-char left padding
        let tabs = TabId::all();

        for &tab in tabs {
            if x >= area.x + area.width {
                break;
            }

            let is_active = tab == self.state.active;
            let label = tab.label();
            let badge = self.state.badge(tab);

            // Style: active tab gets accent + bold, inactive gets dim.
            let style = if is_active {
                Style::new()
                    .fg(self.theme.accent)
                    .bg(self.theme.surface)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::new()
                    .fg(self.theme.text_dim)
                    .bg(self.theme.surface)
            };

            // Render " Label "
            let text = if let Some(count) = badge {
                format!(" {} ({}) ", label, count)
            } else {
                format!(" {} ", label)
            };

            for ch in text.chars() {
                if x >= area.x + area.width {
                    break;
                }
                buf[(x, area.y)].set_char(ch).set_style(style);
                x += 1;
            }

            // Separator
            if x < area.x + area.width {
                buf[(x, area.y)]
                    .set_char('│')
                    .set_style(Style::new().fg(self.theme.border).bg(self.theme.surface));
                x += 1;
            }
        }
    }
}
```

Add `pub mod tab_bar;` to `crates/ucode-tui/src/components/mod.rs`.

**Step 4: Run test to verify it passes**

Run: `cargo test -p ucode-tui -- tab_bar`
Expected: PASS

**Step 5: Commit**

```
feat(tui): add TabBar component with TabId, TabBarState, and neovim-style tab rendering
```

---

### Task 3: Master-detail widget

**Files:**
- Create: `crates/ucode-tui/src/components/master_detail.rs`
- Modify: `crates/ucode-tui/src/components/mod.rs`

A reusable split-panel widget: list on the left (~30%), buffer on the right (~70%). Used by Subagents, Tools, MCP, and Logs tabs.

**Step 1: Write the failing test**

In `crates/ucode-tui/src/components/master_detail.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_item_new() {
        let item = ListItem::new("Read", "✓", "45ms");
        assert_eq!(item.label, "Read");
        assert_eq!(item.status_icon, "✓");
        assert_eq!(item.detail, "45ms");
    }

    #[test]
    fn master_detail_state_navigation() {
        let items = vec![
            ListItem::new("Read", "✓", "45ms"),
            ListItem::new("Edit", "✓", "12ms"),
            ListItem::new("Bash", "✗", "1.2s"),
        ];
        let mut state = MasterDetailState::new(items);
        assert_eq!(state.selected_index(), 0);

        state.select_next();
        assert_eq!(state.selected_index(), 1);

        state.select_next();
        state.select_next(); // clamps at last
        assert_eq!(state.selected_index(), 2);

        state.select_prev();
        assert_eq!(state.selected_index(), 1);
    }

    #[test]
    fn master_detail_state_empty() {
        let state = MasterDetailState::new(Vec::new());
        assert_eq!(state.selected_index(), 0);
        assert!(state.selected_item().is_none());
    }

    #[test]
    fn master_detail_state_selected_item() {
        let items = vec![
            ListItem::new("Read", "✓", "45ms"),
            ListItem::new("Edit", "✓", "12ms"),
        ];
        let mut state = MasterDetailState::new(items);
        state.select_next();
        let item = state.selected_item().unwrap();
        assert_eq!(item.label, "Edit");
    }

    #[test]
    fn master_detail_state_set_buffer() {
        let items = vec![ListItem::new("Read", "✓", "45ms")];
        let mut state = MasterDetailState::new(items);
        state.set_buffer("# Output\nHello world".into());
        assert_eq!(state.buffer_content(), "# Output\nHello world");
    }

    #[test]
    fn master_detail_state_filter() {
        let items = vec![
            ListItem::new("Read", "✓", "45ms"),
            ListItem::new("Edit", "✓", "12ms"),
            ListItem::new("Bash", "✗", "1.2s"),
        ];
        let mut state = MasterDetailState::new(items);
        state.set_filter("ba");
        let visible = state.visible_items();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].label, "Bash");
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p ucode-tui -- master_detail`
Expected: FAIL

**Step 3: Implement**

In `crates/ucode-tui/src/components/master_detail.rs`:

```rust
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::theme::UcodeTheme;

// ---------------------------------------------------------------------------
// ListItem
// ---------------------------------------------------------------------------

/// A single entry in the master list.
#[derive(Debug, Clone)]
pub struct ListItem {
    pub label: String,
    pub status_icon: String,
    pub detail: String,
    /// Optional secondary line (e.g. args summary).
    pub subtitle: Option<String>,
}

impl ListItem {
    pub fn new(
        label: impl Into<String>,
        status_icon: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            label: label.into(),
            status_icon: status_icon.into(),
            detail: detail.into(),
            subtitle: None,
        }
    }

    pub fn with_subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }
}

// ---------------------------------------------------------------------------
// MasterDetailState
// ---------------------------------------------------------------------------

/// State for a master-detail (list + buffer) panel.
#[derive(Debug, Clone)]
pub struct MasterDetailState {
    items: Vec<ListItem>,
    selected: usize,
    buffer: String,
    /// Scroll offset within the buffer panel.
    pub buffer_scroll: usize,
    /// Scroll offset within the list panel.
    pub list_scroll: usize,
    filter: String,
}

impl MasterDetailState {
    pub fn new(items: Vec<ListItem>) -> Self {
        Self {
            items,
            selected: 0,
            buffer: String::new(),
            buffer_scroll: 0,
            list_scroll: 0,
            filter: String::new(),
        }
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn selected_item(&self) -> Option<&ListItem> {
        let visible = self.visible_items();
        visible.into_iter().nth(self.selected)
    }

    pub fn select_next(&mut self) {
        let count = self.visible_items().len();
        if count > 0 {
            self.selected = (self.selected + 1).min(count - 1);
        }
    }

    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn set_buffer(&mut self, content: String) {
        self.buffer = content;
        self.buffer_scroll = 0;
    }

    pub fn buffer_content(&self) -> &str {
        &self.buffer
    }

    pub fn set_filter(&mut self, filter: impl Into<String>) {
        self.filter = filter.into().to_lowercase();
        self.selected = 0;
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    pub fn visible_items(&self) -> Vec<&ListItem> {
        if self.filter.is_empty() {
            self.items.iter().collect()
        } else {
            self.items
                .iter()
                .filter(|item| item.label.to_lowercase().contains(&self.filter))
                .collect()
        }
    }

    pub fn set_items(&mut self, items: Vec<ListItem>) {
        self.items = items;
        let count = self.visible_items().len();
        if self.selected >= count {
            self.selected = count.saturating_sub(1);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl Default for MasterDetailState {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

// ---------------------------------------------------------------------------
// MasterDetail widget
// ---------------------------------------------------------------------------

pub struct MasterDetail<'a> {
    pub state: &'a MasterDetailState,
    pub theme: &'a UcodeTheme,
    /// Label shown when the list is empty.
    pub empty_message: &'a str,
    /// Whether to show the filter input at the top of the list.
    pub show_filter: bool,
}

impl<'a> MasterDetail<'a> {
    pub fn new(state: &'a MasterDetailState, theme: &'a UcodeTheme) -> Self {
        Self {
            state,
            theme,
            empty_message: "No items",
            show_filter: false,
        }
    }

    pub fn empty_message(mut self, msg: &'a str) -> Self {
        self.empty_message = msg;
        self
    }

    pub fn show_filter(mut self, show: bool) -> Self {
        self.show_filter = show;
        self
    }
}

impl Widget for MasterDetail<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width < 20 {
            return;
        }

        // Empty state: centered message.
        if self.state.is_empty() {
            let msg = self.empty_message;
            let y = area.y + area.height / 2;
            let x = area.x + area.width.saturating_sub(msg.len() as u16) / 2;
            let style = Style::new().fg(self.theme.text_dim);
            for (i, ch) in msg.chars().enumerate() {
                let px = x + i as u16;
                if px < area.x + area.width {
                    buf[(px, y)].set_char(ch).set_style(style);
                }
            }
            return;
        }

        // Split: 30% list, 1 border, 69% buffer.
        let list_width = (area.width * 30 / 100).max(15).min(area.width - 5);
        let [list_area, border_area, buffer_area] = Layout::horizontal([
            Constraint::Length(list_width),
            Constraint::Length(1),
            Constraint::Fill(1),
        ])
        .areas(area);

        // Vertical border between list and buffer.
        let border_style = Style::new().fg(self.theme.border);
        for y in border_area.y..border_area.y + border_area.height {
            buf[(border_area.x, y)]
                .set_char('│')
                .set_style(border_style);
        }

        // --- List panel ---
        self.render_list(list_area, buf);

        // --- Buffer panel ---
        self.render_buffer(buffer_area, buf);
    }
}

impl MasterDetail<'_> {
    fn render_list(&self, area: Rect, buf: &mut Buffer) {
        let items = self.state.visible_items();
        let mut y = area.y;

        // Optional filter line.
        if self.show_filter && y < area.y + area.height {
            let filter_text = if self.state.filter().is_empty() {
                "Filter: ________".to_string()
            } else {
                format!("Filter: {}", self.state.filter())
            };
            let style = Style::new().fg(self.theme.text_dim);
            for (i, ch) in filter_text.chars().enumerate() {
                let x = area.x + 1 + i as u16;
                if x < area.x + area.width {
                    buf[(x, y)].set_char(ch).set_style(style);
                }
            }
            y += 1;
            // Blank separator line.
            y += 1;
        }

        // List items.
        for (i, item) in items.iter().enumerate() {
            if y >= area.y + area.height {
                break;
            }

            let is_selected = i == self.state.selected_index();
            let style = if is_selected {
                Style::new()
                    .fg(self.theme.text)
                    .bg(self.theme.select_cursor)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(self.theme.text)
            };

            // Selection indicator.
            let prefix = if is_selected { "▸ " } else { "  " };

            // Line: "▸ icon label"
            let line = format!(
                "{}{} {}",
                prefix, item.status_icon, item.label
            );
            let max_w = area.width as usize;
            for (j, ch) in line.chars().take(max_w).enumerate() {
                let x = area.x + j as u16;
                if x < area.x + area.width {
                    buf[(x, y)].set_char(ch).set_style(style);
                }
            }
            // Fill rest of line with bg for selected.
            if is_selected {
                for x in (area.x + line.len().min(max_w) as u16)..area.x + area.width {
                    buf[(x, y)].set_style(style);
                }
            }
            y += 1;

            // Detail line (dimmed, indented).
            if y < area.y + area.height {
                let detail = format!("  {}", item.detail);
                let detail_style = if is_selected {
                    Style::new()
                        .fg(self.theme.text_dim)
                        .bg(self.theme.select_cursor)
                } else {
                    Style::new().fg(self.theme.text_dim)
                };
                for (j, ch) in detail.chars().take(max_w).enumerate() {
                    let x = area.x + j as u16;
                    if x < area.x + area.width {
                        buf[(x, y)].set_char(ch).set_style(detail_style);
                    }
                }
                if is_selected {
                    for x in (area.x + detail.len().min(max_w) as u16)..area.x + area.width {
                        buf[(x, y)].set_style(detail_style);
                    }
                }
                y += 1;
            }

            // Blank line between items.
            y += 1;
        }
    }

    fn render_buffer(&self, area: Rect, buf: &mut Buffer) {
        let content = self.state.buffer_content();
        if content.is_empty() {
            return;
        }

        let style = Style::new().fg(self.theme.text);
        let mut y = area.y;
        let scroll = self.state.buffer_scroll;
        let max_w = (area.width.saturating_sub(2)) as usize; // 1-char padding each side

        for (line_idx, line) in content.lines().enumerate() {
            if line_idx < scroll {
                continue;
            }
            if y >= area.y + area.height {
                break;
            }
            for (j, ch) in line.chars().take(max_w).enumerate() {
                let x = area.x + 1 + j as u16;
                if x < area.x + area.width {
                    buf[(x, y)].set_char(ch).set_style(style);
                }
            }
            y += 1;
        }
    }
}
```

Add `pub mod master_detail;` to `crates/ucode-tui/src/components/mod.rs`.

**Step 4: Run test to verify it passes**

Run: `cargo test -p ucode-tui -- master_detail`
Expected: PASS

**Step 5: Commit**

```
feat(tui): add MasterDetail reusable list+buffer split widget
```

---

### Task 4: Layout restructure — tab bar row + full-height sidebar

**Files:**
- Modify: `crates/ucode-tui/src/layout.rs`

The layout changes from:

```
title_bar(1) | middle(fill) [transcript | sidebar] | input(N) | gap(1) | status_bar(1)
```

to:

```
tab_bar(1) | middle(fill) [content | sidebar] | input(N+1 for info line) | status_bar(1)
```

Key changes:
- `title_bar` renamed to `tab_bar` (same 1-row slot, different content)
- Sidebar spans full height (tab_bar through input) — achieved by splitting BEFORE the horizontal split
- Input box gains 1 extra row for the agent/model/provider info line
- `input_gap` removed (info line replaces it as visual separator)
- `LayoutAreas` gains `tab_bar` field (replaces `title_bar`)

**Step 1: Write the failing test**

```rust
#[test]
fn layout_has_tab_bar() {
    let area = make_area(200, 50);
    let sidebar = SidebarState::new(SidebarMode::Full);
    let input = InputState::default();
    let layout = compute_layout(area, &sidebar, &input);

    // Tab bar: row 0, height 1
    assert_eq!(layout.tab_bar.height, 1);
    assert_eq!(layout.tab_bar.y, 0);
}

#[test]
fn sidebar_spans_full_height() {
    let area = make_area(200, 50);
    let sidebar = SidebarState::new(SidebarMode::Full);
    let input = InputState::default();
    let layout = compute_layout(area, &sidebar, &input);

    // Sidebar should span from tab_bar.y to status_bar.y (exclusive).
    // i.e. from row 0 to row 48 (status_bar at 49).
    assert_eq!(layout.sidebar.y, 0);
    assert_eq!(layout.sidebar.height, 49); // everything except status bar
}

#[test]
fn input_includes_info_line() {
    let area = make_area(200, 50);
    let sidebar = SidebarState::new(SidebarMode::Hidden);
    let input = InputState::default(); // 1 line
    let layout = compute_layout(area, &sidebar, &input);

    // Input height: 1 content + 1 top pad + 1 bottom pad + 1 info line = 4
    assert_eq!(layout.input.height, 4);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p ucode-tui -- layout`
Expected: FAIL — `tab_bar` field doesn't exist

**Step 3: Implement**

Update `LayoutAreas`:

```rust
pub struct LayoutAreas {
    pub tab_bar: Rect,
    pub content: Rect,    // was `transcript` — now depends on active tab
    pub sidebar: Rect,
    pub input: Rect,
    pub status_bar: Rect,
}
```

Update `InputState::height()`:

```rust
/// Height: content lines + 1 top padding + 1 info line + 1 bottom padding.
pub fn height(&self) -> u16 {
    self.line_count + 3  // was +2, now +3 for info line
}
```

Update `compute_layout`:

```rust
pub fn compute_layout(area: Rect, sidebar: &SidebarState, input: &InputState) -> LayoutAreas {
    let pad = LAYOUT_HORIZONTAL_PAD;
    let padded = if area.width > pad * 2 + MIN_WIDTH {
        Rect::new(area.x + pad, area.y, area.width - pad * 2, area.height)
    } else {
        area
    };

    let sidebar_width = sidebar.effective_width();

    // First: split off sidebar (full height) from the right.
    let [main_col, sidebar_area] = if sidebar_width > 0 {
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(sidebar_width)])
            .areas(padded)
    } else {
        let [main_col] = Layout::horizontal([Constraint::Fill(1)]).areas(padded);
        [main_col, Rect::default()]
    };

    // Then: split main column vertically.
    let [tab_bar, content, input_area, status_bar] = Layout::vertical([
        Constraint::Length(1),       // tab bar
        Constraint::Fill(1),         // content area
        Constraint::Length(input.height()), // input box (now includes info line)
        Constraint::Length(1),       // status bar
    ])
    .areas(main_col);

    LayoutAreas {
        tab_bar,
        content,
        sidebar: sidebar_area,
        input: input_area,
        status_bar,
    }
}
```

Update all existing tests to use `tab_bar` instead of `title_bar` and remove `input_gap` references. Update `render_frame` to use `areas.tab_bar` and `areas.content`.

**Step 4: Run test to verify it passes**

Run: `cargo test -p ucode-tui -- layout`
Expected: PASS

**Step 5: Commit**

```
feat(tui): restructure layout — tab bar row, full-height sidebar, input info line
```

---

### Task 5: Input box info line

**Files:**
- Modify: `crates/ucode-tui/src/components/input.rs`

Add an info line at the bottom of the input box showing `@agent_name   model-name   Provider`.

**Step 1: Write the failing test**

```rust
#[test]
fn input_box_info_line_renders() {
    let state = InputBoxState::new();
    let theme = UcodeTheme::default();
    let info = InputInfoLine {
        agent_name: "coder".into(),
        model_name: Some("claude-opus-4-6".into()),
        provider_name: Some("Anthropic".into()),
    };
    let widget = InputBox::new(&state, &theme, true).with_info_line(&info);
    // Just verify it doesn't panic when rendered.
    let area = Rect::new(0, 0, 80, 5);
    let mut buf = Buffer::empty(area);
    widget.render(area, &mut buf);
}

#[test]
fn input_box_info_line_not_connected() {
    let info = InputInfoLine {
        agent_name: "coder".into(),
        model_name: None,
        provider_name: None,
    };
    assert_eq!(info.display_text(), "@coder   [not connected]");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p ucode-tui -- input_box_info`
Expected: FAIL

**Step 3: Implement**

Add `InputInfoLine` struct and integrate into the `InputBox` widget. The info line renders on the last row of the input area, left-aligned, with the agent name in accent color and model/provider in dim text.

```rust
/// Data for the info line at the bottom of the input box.
#[derive(Debug, Clone, Default)]
pub struct InputInfoLine {
    pub agent_name: String,
    pub model_name: Option<String>,
    pub provider_name: Option<String>,
}

impl InputInfoLine {
    pub fn display_text(&self) -> String {
        match (&self.model_name, &self.provider_name) {
            (Some(model), Some(provider)) => {
                format!("@{}   {}   {}", self.agent_name, model, provider)
            }
            (Some(model), None) => format!("@{}   {}", self.agent_name, model),
            _ => format!("@{}   [not connected]", self.agent_name),
        }
    }
}
```

Modify the `InputBox` widget to accept an optional `InputInfoLine` and render it on the last row before the bottom border.

**Step 4: Run test to verify it passes**

Run: `cargo test -p ucode-tui -- input_box_info`
Expected: PASS

**Step 5: Commit**

```
feat(tui): add agent/model/provider info line to input box
```

---

### Task 6: Status bar simplification

**Files:**
- Modify: `crates/ucode-tui/src/components/status_bar.rs`

Strip the status bar down to: keybind hints + optional streaming progress. Remove session title, log level, branch, sandbox tier, agent pill, model badge, diff stats, cost, tokens.

**Step 1: Write the failing test**

```rust
#[test]
fn simplified_status_bar_only_keybinds() {
    let state = StatusBarState {
        keybind_hints: vec!["tab agents".into(), "ctrl+p commands".into(), "ctrl+t models".into()],
        streaming: false,
        ..StatusBarState::default()
    };
    let theme = UcodeTheme::default();
    let area = Rect::new(0, 0, 80, 1);
    let mut buf = Buffer::empty(area);
    StatusBar::new(&state, &theme).render(area, &mut buf);
    let text: String = (0..80).map(|x| buf[(x, 0u16)].symbol().chars().next().unwrap_or(' ')).collect();
    assert!(text.contains("tab agents"));
    assert!(text.contains("ctrl+p commands"));
    // Should NOT contain old segments.
    assert!(!text.contains("INFO"));
    assert!(!text.contains("main"));
    assert!(!text.contains("$0"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p ucode-tui -- simplified_status_bar`
Expected: FAIL

**Step 3: Implement**

Simplify `StatusBarState` to only the fields needed:

```rust
pub struct StatusBarState {
    pub keybind_hints: Vec<String>,
    pub hint_message: Option<String>,
    pub streaming: bool,
    pub stream_tok_per_sec: Option<f64>,
    pub copy_mode_label: Option<String>,
}
```

Simplify `build_segments` to only render:
1. Copy-mode indicator (when active)
2. Keybind hints
3. Transient hint message
4. Streaming indicator (when streaming)

Remove all other segments. Move the removed data (session title, branch, cost, tokens, etc.) to sidebar sections (handled in Task 7).

Update all code that sets the removed fields. Search for `status_bar.model_name`, `status_bar.branch`, `status_bar.cost`, etc. and remove those assignments.

**Step 4: Run test to verify it passes**

Run: `cargo test -p ucode-tui -- status_bar`
Expected: PASS (all status bar tests, updated)

**Step 5: Commit**

```
feat(tui): simplify status bar to keybind hints + streaming progress only
```

---

### Task 7: Sidebar restructure — full height with sections and fixed footer

**Files:**
- Modify: `crates/ucode-tui/src/components/sidebar/mod.rs`
- Modify: `crates/ucode-tui/src/components/sidebar/sections.rs`

The sidebar now spans full terminal height. It has:
- Scrollable body with toggleable sections (TODO, Sessions, Git, Cost/Tokens)
- Fixed footer: working directory + version

**Step 1: Write the failing test**

```rust
#[test]
fn sidebar_footer_renders() {
    let mut data = SidebarData::new();
    data.footer_dir = "~/code/ucode".into();
    data.footer_version = "ucode v0.1.0".into();
    // Verify footer fields exist and are set.
    assert_eq!(data.footer_dir, "~/code/ucode");
    assert_eq!(data.footer_version, "ucode v0.1.0");
}

#[test]
fn sidebar_section_toggle() {
    let mut data = SidebarData::new();
    assert!(!data.is_collapsed(SectionId::Context));
    data.toggle_section(SectionId::Context);
    assert!(data.is_collapsed(SectionId::Context));
    data.toggle_section(SectionId::Context);
    assert!(!data.is_collapsed(SectionId::Context));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p ucode-tui -- sidebar_footer`
Expected: FAIL

**Step 3: Implement**

Add `footer_dir: String` and `footer_version: String` fields to `SidebarData`. Add `collapsed: HashSet<SectionId>` for section toggling. Update the `Sidebar` widget to:
1. Reserve 2 rows at the bottom for the fixed footer
2. Render scrollable sections in the remaining space
3. Render footer (directory + version) at the bottom

**Step 4: Run test to verify it passes**

Run: `cargo test -p ucode-tui -- sidebar`
Expected: PASS

**Step 5: Commit**

```
feat(tui): restructure sidebar with toggleable sections and fixed footer
```

---

### Task 8: Wire tab bar into render_frame and event loop

**Files:**
- Modify: `crates/ucode-tui/src/app.rs`
- Modify: `crates/ucode-tui/src/event_loop.rs`
- Modify: `crates/ucode-tui/src/keybinds.rs`

Add `TabBarState` to `AppState`. Add `NextTab`/`PrevTab`/`SelectTab` actions. Wire tab rendering into `render_frame`. Route tab content based on `app.tab_bar.active`.

**Step 1: Add state and actions**

In `app.rs`, add:
```rust
pub tab_bar: crate::components::tab_bar::TabBarState,
```

In `keybinds.rs`, add to `Action` enum:
```rust
NextTab,
PrevTab,
```

Add keybindings: `gt` for NextTab, `gT` for PrevTab (in Normal mode, matching neovim).

**Step 2: Wire render_frame**

In `render_frame`, replace the title bar rendering with tab bar rendering. Replace the transcript rendering with tab-dependent content:

```rust
// Tab bar.
f.render_widget(TabBar::new(&app.tab_bar, &app.theme), areas.tab_bar);

// Content area — depends on active tab.
match app.tab_bar.active {
    TabId::Chat => {
        // Existing transcript rendering.
        let transcript_widget = TranscriptView::new(/* ... */);
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
```

Add `MasterDetailState` fields to `AppState`:
```rust
pub subagents_panel: MasterDetailState,
pub tools_panel: MasterDetailState,
pub mcp_panel: MasterDetailState,
pub logs_panel: MasterDetailState,
```

**Step 3: Wire dispatch_action**

```rust
Action::NextTab => {
    app.tab_bar.next();
    app.mark_dirty();
}
Action::PrevTab => {
    app.tab_bar.prev();
    app.mark_dirty();
}
```

**Step 4: Run tests**

Run: `cargo test -p ucode-tui`
Expected: PASS (update render_frame tests as needed)

**Step 5: Commit**

```
feat(tui): wire tab bar into render_frame and event loop with gt/gT navigation
```

---

### Task 9: Update demo.rs with full tabbed UI

**Files:**
- Modify: `crates/ucode-tui/examples/demo.rs`

This is the key validation step. Update the demo to:
1. Populate fake subagent runs, tool calls, MCP entries, and log events
2. Show the tab bar with badge counts
3. Show the input box with agent/model/provider info line
4. Show the simplified status bar
5. Show the sidebar with sections and footer
6. All tabs navigable with `gt`/`gT`

**Step 1: Implement**

Add fake data population after `AppState::new()`:

```rust
// Fake subagent runs.
app.subagents_panel.set_items(vec![
    ListItem::new("rust-expert", "✓", "1.2s  890 tok")
        .with_subtitle("C3+D1: @mention routing"),
    ListItem::new("explore", "✓", "0.4s  210 tok")
        .with_subtitle("Find layout test files"),
    ListItem::new("quick-fix", "⟳", "running...")
        .with_subtitle("Fix layout test assertions"),
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
     ```".into()
);

// Fake tool calls.
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
     Output:\n  176: #[test]\n  177: fn terminal_size_minimum() {\n  178:     assert!(...".into()
);

// Fake MCP servers.
app.mcp_panel.set_items(vec![
    ListItem::new("context7", "●", "12 tools"),
    ListItem::new("git-docs", "●", "3 tools"),
    ListItem::new("arxiv", "○", "disconnected"),
]);
app.mcp_panel.set_buffer(
    "context7\nStatus: connected\nTools: 12\n\n\
     ── Tool Catalog ──\n\
     resolve-library-id\n  Resolve package name to library ID\n\
     query-docs\n  Query documentation and examples\n\n\
     ── Request Log (14 calls) ──\n\n\
     14:23:05 query-docs         ✓  45ms\n  lib=/vercel/next.js\n\n\
     14:22:58 resolve-library-id ✓  120ms\n  name=\"next.js\"".into()
);

// Fake log events.
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
     Detail:\n  Spawned subagent rust-expert for implementing\n  AgentMessage struct expansion.".into()
);

// Badge counts.
app.tab_bar.set_badge(TabId::Subagents, 3);
app.tab_bar.set_badge(TabId::Tools, 5);
app.tab_bar.set_badge(TabId::Logs, 4);

// Sidebar footer.
sidebar_data.footer_dir = std::env::current_dir()
    .map(|p| p.display().to_string())
    .unwrap_or_else(|_| "~/code/ucode".into());
sidebar_data.footer_version = format!("ucode v{}", env!("CARGO_PKG_VERSION"));

// Status bar: keybind hints only.
app.status_bar = StatusBarState {
    keybind_hints: vec![
        "tab agents".into(),
        "ctrl+p commands".into(),
        "ctrl+t models".into(),
        "gt/gT tabs".into(),
    ],
    ..StatusBarState::default()
};
```

**Step 2: Run the demo**

Run: `cargo run -p ucode-tui --example demo`
Expected: Full tabbed UI visible. Navigate tabs with `gt`/`gT`. All tabs show fake data with list+buffer split.

**Step 3: Commit**

```
feat(tui): update demo with full tabbed panel UI and fake operational data
```

---

### Task 10: Add operational metadata fields to Session

**Files:**
- Modify: `crates/ucode-core/src/session.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn session_with_operational_data_roundtrip() {
    use crate::operational::*;

    let mut session = Session::new_for_test();
    session.subagent_runs.push(SubagentRun {
        id: "sa-001".into(),
        agent_name: "explore".into(),
        task_description: "Find files".into(),
        status: RunStatus::Success,
        started_at: chrono::Utc::now(),
        completed_at: Some(chrono::Utc::now()),
        duration_ms: Some(400),
        token_count: Some(210),
        output: "Found 3 files".into(),
        tool_call_ids: vec![],
    });
    session.tool_runs.push(ToolRun {
        id: "tc-001".into(),
        tool_name: "Grep".into(),
        args_summary: "pattern=TODO".into(),
        status: RunStatus::Success,
        started_at: chrono::Utc::now(),
        duration_ms: Some(120),
        input: "{}".into(),
        output: Some("3 matches".into()),
        thinking: None,
        subagent_id: Some("sa-001".into()),
    });

    let json = serde_json::to_string(&session).unwrap();
    let decoded: Session = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.subagent_runs.len(), 1);
    assert_eq!(decoded.tool_runs.len(), 1);
    assert_eq!(decoded.subagent_runs[0].agent_name, "explore");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p ucode-core -- session_with_operational`
Expected: FAIL — fields don't exist

**Step 3: Implement**

Add to `Session`:

```rust
use crate::operational::{SubagentRun, ToolRun, McpLogEntry, SessionEvent};

pub struct Session {
    pub meta: SessionMeta,
    pub transcript: Vec<Message>,
    pub tool_audit: Vec<ToolAuditEntry>,
    #[serde(default)]
    pub compaction_log: Vec<CompactionRecord>,
    #[serde(default)]
    pub usage: SessionUsage,

    // Operational metadata for tabbed panel UI.
    #[serde(default)]
    pub subagent_runs: Vec<SubagentRun>,
    #[serde(default)]
    pub tool_runs: Vec<ToolRun>,
    #[serde(default)]
    pub mcp_logs: Vec<McpLogEntry>,
    #[serde(default)]
    pub event_log: Vec<SessionEvent>,
}
```

The `#[serde(default)]` ensures backward compatibility — old session files without these fields will deserialize with empty vecs.

**Step 4: Run test to verify it passes**

Run: `cargo test -p ucode-core -- session`
Expected: PASS

**Step 5: Commit**

```
feat(core): add operational metadata fields to Session for tabbed panel persistence
```

---

### Task 11: Session picker popup (/resume command)

**Files:**
- Modify: `crates/ucode-tui/src/overlays/session_picker.rs` (already exists)
- Modify: `crates/ucode-tui/src/command_registry.rs`
- Modify: `crates/ucode-tui/src/event_loop.rs`

Wire the `/resume` command to open the session picker modal. The session picker already exists as `SessionPickerState` — it needs to be populated with session list data and wired to the overlay queue.

**Step 1: Register /resume command**

In `CommandRegistry::with_builtins()`, add:

```rust
CommandDef {
    name: "/resume".to_owned(),
    description: "Resume a previous session".to_owned(),
    category: CommandCategory::Session,
    source: CommandSource::BuiltIn,
    args_hint: None,
    action: Some(Action::OpenSessionPicker),
}
```

Add `OpenSessionPicker` to the `Action` enum.

**Step 2: Wire dispatch_action**

```rust
Action::OpenSessionPicker => {
    if let Some(ref store) = app.session_store {
        let sessions = store.list_sessions().unwrap_or_default();
        app.session_picker.open(sessions);
        app.focus = FocusTarget::Overlay;
    }
    app.mark_dirty();
}
```

**Step 3: Handle session selection**

When the user selects a session in the picker, load it and restore all data (transcript + operational metadata).

**Step 4: Run tests**

Run: `cargo test -p ucode-tui`
Expected: PASS

**Step 5: Commit**

```
feat(tui): wire /resume command to session picker popup
```

---

## Summary

| Task | What | Crate | Phase |
|------|------|-------|-------|
| 1 | Operational metadata types | ucode-core | 1 (data model) |
| 2 | TabBar component | ucode-tui | 2 (tab bar) |
| 3 | MasterDetail widget | ucode-tui | 3 (master-detail) |
| 4 | Layout restructure | ucode-tui | 2 (layout) |
| 5 | Input box info line | ucode-tui | 4 (input) |
| 6 | Status bar simplification | ucode-tui | 5 (status bar) |
| 7 | Sidebar restructure | ucode-tui | 6 (sidebar) |
| 8 | Wire tabs into render + events | ucode-tui | 2 (wiring) |
| 9 | Demo with full UI | ucode-tui | 7 (demo) |
| 10 | Session operational fields | ucode-core | 9 (persistence) |
| 11 | Session picker /resume | ucode-tui | 10 (session) |

**Dependency order:** 1 → 2,3 (parallel) → 4 → 5,6,7 (parallel) → 8 → 9 → 10 → 11
