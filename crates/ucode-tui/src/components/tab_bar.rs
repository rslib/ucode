use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
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

    /// Given a click at column `x` within the tab bar area, return the tab
    /// that was clicked, if any.
    pub fn hit_test(&self, x: u16, area_x: u16) -> Option<TabId> {
        let mut col = area_x + 1; // 1-char left padding
        for &tab in TabId::all() {
            let badge = self.badge(tab);
            let text_len = if let Some(count) = badge {
                // " Label (N) " — label + " (" + digits + ") " + surrounding spaces
                format!(" {} ({}) ", tab.label(), count).len() as u16
            } else {
                // " Label "
                (tab.label().len() as u16) + 2
            };
            let end = col + text_len + 1; // +1 for separator
            if x >= col && x < end {
                return Some(tab);
            }
            col = end;
        }
        None
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
        let bg_style = Style::new().bg(self.theme.surface).fg(self.theme.text_dim);
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
                Style::new().fg(self.theme.text_dim).bg(self.theme.surface)
            };

            // Render " Label " or " Label (N) "
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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

    #[test]
    fn tab_state_hit_test() {
        let mut state = TabBarState::new();
        state.set_badge(TabId::Subagents, 3);
        // Layout: 1-pad, " Chat "(7), "│"(1), " Subagents (3) "(16), "│"(1), ...
        // area_x = 0
        // Chat: cols 1..8 (text) + 8 (sep) = 1..9
        // Subagents: cols 9..25 (text) + 25 (sep) = 9..26
        assert_eq!(state.hit_test(1, 0), Some(TabId::Chat));
        assert_eq!(state.hit_test(5, 0), Some(TabId::Chat));
        assert_eq!(state.hit_test(9, 0), Some(TabId::Subagents));
        assert_eq!(state.hit_test(0, 0), None); // in the padding
    }
}
