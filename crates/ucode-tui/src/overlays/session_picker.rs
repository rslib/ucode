use chrono::{DateTime, Utc};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

use ucode_core::SessionMeta;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// State for the session picker overlay.
#[derive(Debug, Clone)]
pub struct SessionPickerState {
    pub visible: bool,
    pub entries: Vec<SessionMeta>,
    pub selected: usize,
    pub filter: String,
    pub filter_cursor: usize,
    pub filtered_indices: Vec<usize>,
    /// The currently active session ID (highlighted in the list).
    pub current_session_id: Option<String>,
}

impl SessionPickerState {
    pub fn new() -> Self {
        Self {
            visible: false,
            entries: Vec::new(),
            selected: 0,
            filter: String::new(),
            filter_cursor: 0,
            filtered_indices: Vec::new(),
            current_session_id: None,
        }
    }

    /// Open the picker with a list of sessions.
    pub fn open(&mut self, sessions: Vec<SessionMeta>, current_id: Option<&str>) {
        self.visible = true;
        self.entries = sessions;
        self.filter.clear();
        self.filter_cursor = 0;
        self.selected = 0;
        self.current_session_id = current_id.map(str::to_owned);
        self.update_filter();
    }

    pub fn close(&mut self) {
        self.visible = false;
    }

    // -- filter text editing -------------------------------------------------

    pub fn insert_char(&mut self, c: char) {
        self.filter.insert(self.filter_cursor, c);
        self.filter_cursor += c.len_utf8();
        self.update_filter();
    }

    pub fn delete_char(&mut self) {
        if self.filter_cursor == 0 {
            return;
        }
        let before = &self.filter[..self.filter_cursor];
        let char_start = before
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.filter.drain(char_start..self.filter_cursor);
        self.filter_cursor = char_start;
        self.update_filter();
    }

    // -- navigation ----------------------------------------------------------

    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn move_down(&mut self) {
        if self.filtered_indices.is_empty() {
            return;
        }
        let max = self.filtered_indices.len() - 1;
        if self.selected < max {
            self.selected += 1;
        }
    }

    // -- queries -------------------------------------------------------------

    pub fn selected_entry(&self) -> Option<&SessionMeta> {
        let idx = *self.filtered_indices.get(self.selected)?;
        self.entries.get(idx)
    }

    pub fn selected_session_id(&self) -> Option<&str> {
        self.selected_entry().map(|e| e.id.as_str())
    }

    // -- internal ------------------------------------------------------------

    pub fn update_filter(&mut self) {
        if self.filter.is_empty() {
            self.filtered_indices = (0..self.entries.len()).collect();
        } else {
            let needle = self.filter.to_lowercase();
            self.filtered_indices = self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, e)| {
                    e.id.to_lowercase().contains(&needle)
                        || e.title
                            .as_deref()
                            .map(|t| t.to_lowercase().contains(&needle))
                            .unwrap_or(false)
                })
                .map(|(i, _)| i)
                .collect();
        }
        if self.filtered_indices.is_empty() {
            self.selected = 0;
        } else {
            let max = self.filtered_indices.len() - 1;
            if self.selected > max {
                self.selected = max;
            }
        }
    }
}

impl Default for SessionPickerState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_width = area.width * percent_x / 100;
    let popup_height = area.height * percent_y / 100;
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    Rect::new(x, y, popup_width, popup_height)
}

/// Format a timestamp as a relative time string (e.g. "2m ago", "3h ago").
fn relative_time(dt: &DateTime<Utc>) -> String {
    let now = Utc::now();
    let delta = now.signed_duration_since(*dt);

    if delta.num_seconds() < 60 {
        "just now".to_owned()
    } else if delta.num_minutes() < 60 {
        format!("{}m ago", delta.num_minutes())
    } else if delta.num_hours() < 24 {
        format!("{}h ago", delta.num_hours())
    } else if delta.num_days() < 30 {
        format!("{}d ago", delta.num_days())
    } else {
        dt.format("%Y-%m-%d").to_string()
    }
}

// ---------------------------------------------------------------------------
// Widget
// ---------------------------------------------------------------------------

pub struct SessionPicker<'a> {
    state: &'a SessionPickerState,
    theme: &'a crate::theme::UcodeTheme,
}

impl<'a> SessionPicker<'a> {
    pub fn new(state: &'a SessionPickerState, theme: &'a crate::theme::UcodeTheme) -> Self {
        Self { state, theme }
    }
}

impl Widget for SessionPicker<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if !self.state.visible {
            return;
        }

        let popup = centered_rect(60, 60, area);
        Clear.render(popup, buf);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border_style(true))
            .title(" Sessions ");
        let inner = block.inner(popup);
        block.render(popup, buf);

        if inner.height < 4 || inner.width < 10 {
            return;
        }

        // Layout: filter line, separator, list body, separator, footer
        let footer_height = 1u16;
        let list_height = inner
            .height
            .saturating_sub(1 + 1 + 1 + footer_height) // filter + sep + sep + footer
            .max(1);

        let filter_rect = Rect::new(inner.x, inner.y, inner.width, 1);
        let sep1_rect = Rect::new(inner.x, inner.y + 1, inner.width, 1);
        let list_rect = Rect::new(inner.x, inner.y + 2, inner.width, list_height);
        let sep2_y = inner.y + 2 + list_height;
        let sep2_rect = Rect::new(inner.x, sep2_y, inner.width, 1);
        let footer_rect = Rect::new(inner.x, sep2_y + 1, inner.width, footer_height);

        // Filter line
        let filter_line = Line::from(vec![
            Span::styled("Filter: ", self.theme.dim_style()),
            Span::styled(&self.state.filter, self.theme.text_style()),
        ]);
        Paragraph::new(filter_line).render(filter_rect, buf);

        // Separator
        let sep = "\u{2500}".repeat(inner.width as usize);
        Paragraph::new(sep.as_str())
            .style(self.theme.dim_style())
            .render(sep1_rect, buf);

        // List body
        if self.state.filtered_indices.is_empty() {
            let msg = if self.state.entries.is_empty() {
                "No sessions."
            } else {
                "No matches."
            };
            Paragraph::new(msg)
                .style(self.theme.dim_style())
                .render(list_rect, buf);
        } else {
            self.render_session_list(list_rect, buf);
        }

        // Bottom separator
        if sep2_rect.y < inner.y + inner.height {
            Paragraph::new(sep.as_str())
                .style(self.theme.dim_style())
                .render(sep2_rect, buf);
        }

        // Footer
        if footer_rect.y < inner.y + inner.height {
            let footer = Line::from(vec![
                Span::styled("\u{2191}\u{2193}", self.theme.accent_style()),
                Span::styled(" navigate  ", self.theme.dim_style()),
                Span::styled("type", self.theme.accent_style()),
                Span::styled(" to filter  ", self.theme.dim_style()),
                Span::styled("Enter", self.theme.accent_style()),
                Span::styled(" load  ", self.theme.dim_style()),
                Span::styled("Esc", self.theme.accent_style()),
                Span::styled(" close", self.theme.dim_style()),
            ]);
            Paragraph::new(footer).render(footer_rect, buf);
        }
    }
}

impl SessionPicker<'_> {
    fn render_session_list(&self, area: Rect, buf: &mut Buffer) {
        let visible_height = area.height as usize;

        // Compute scroll offset to keep selected row visible.
        let scroll_offset = if self.state.selected >= visible_height {
            self.state.selected - visible_height + 1
        } else {
            0
        };

        for (display_idx, &entry_idx) in self.state.filtered_indices.iter().enumerate() {
            if display_idx < scroll_offset {
                continue;
            }
            let y_offset = display_idx - scroll_offset;
            if y_offset >= visible_height {
                break;
            }
            let y = area.y + y_offset as u16;

            let entry = &self.state.entries[entry_idx];
            let is_selected = display_idx == self.state.selected;
            let is_current = self
                .state
                .current_session_id
                .as_deref()
                .is_some_and(|cid| cid == entry.id);

            let prefix = if is_current && is_selected {
                "* > "
            } else if is_current {
                "*   "
            } else if is_selected {
                "  > "
            } else {
                "    "
            };

            let title = entry.title.as_deref().unwrap_or("Untitled");

            // Truncate session ID to 12 chars for display.
            let id_display = if entry.id.len() > 12 {
                &entry.id[..12]
            } else {
                &entry.id
            };

            let time_str = relative_time(&entry.updated_at);

            // Build the row: "prefix title  (id_display)  time_str"
            let available = area.width as usize;
            let fixed_len = prefix.len() + id_display.len() + time_str.len() + 6; // spaces + parens
            let title_max = available.saturating_sub(fixed_len);
            let title_truncated = if title.len() > title_max {
                &title[..title_max.saturating_sub(1).max(1)]
            } else {
                title
            };

            let style = if is_selected {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                self.theme.text_style()
            };

            let line = Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(title_truncated.to_owned(), style),
                Span::styled("  ", style),
                Span::styled(format!("({id_display})"), self.theme.dim_style()),
                Span::styled("  ", style),
                Span::styled(time_str, self.theme.dim_style()),
            ]);
            Paragraph::new(line).render(Rect::new(area.x, y, area.width, 1), buf);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_session_meta(id: &str, title: Option<&str>, mins_ago: i64) -> SessionMeta {
        let now = Utc::now();
        let updated = now - chrono::Duration::minutes(mins_ago);
        SessionMeta {
            id: id.to_owned(),
            created_at: updated,
            updated_at: updated,
            active_model: None,
            active_skill: None,
            working_dir: PathBuf::from("/tmp"),
            title: title.map(str::to_owned),
            title_source: ucode_core::TitleSource::Auto,
            archived: false,
            parent_session_id: None,
            fork_source_index: None,
        }
    }

    #[test]
    fn test_new_is_not_visible() {
        let state = SessionPickerState::new();
        assert!(!state.visible);
        assert!(state.entries.is_empty());
        assert!(state.filter.is_empty());
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn test_open_sets_visible() {
        let mut state = SessionPickerState::new();
        let sessions = vec![
            make_session_meta("ses_001", Some("First"), 5),
            make_session_meta("ses_002", Some("Second"), 10),
        ];
        state.open(sessions, Some("ses_001"));
        assert!(state.visible);
        assert_eq!(state.entries.len(), 2);
        assert_eq!(state.filtered_indices.len(), 2);
        assert_eq!(state.current_session_id.as_deref(), Some("ses_001"));
    }

    #[test]
    fn test_close() {
        let mut state = SessionPickerState::new();
        state.open(vec![make_session_meta("s1", None, 0)], None);
        assert!(state.visible);
        state.close();
        assert!(!state.visible);
    }

    #[test]
    fn test_filter_narrows_list() {
        let mut state = SessionPickerState::new();
        let sessions = vec![
            make_session_meta("ses_001", Some("Debug session"), 5),
            make_session_meta("ses_002", Some("Feature work"), 10),
            make_session_meta("ses_003", Some("Debug tools"), 15),
        ];
        state.open(sessions, None);
        assert_eq!(state.filtered_indices.len(), 3);

        for c in "debug".chars() {
            state.insert_char(c);
        }
        assert_eq!(state.filtered_indices.len(), 2);
    }

    #[test]
    fn test_navigate_up_down() {
        let mut state = SessionPickerState::new();
        let sessions = vec![
            make_session_meta("a", None, 0),
            make_session_meta("b", None, 1),
            make_session_meta("c", None, 2),
        ];
        state.open(sessions, None);

        assert_eq!(state.selected, 0);
        state.move_up(); // saturates at 0
        assert_eq!(state.selected, 0);

        state.move_down();
        state.move_down();
        assert_eq!(state.selected, 2);

        state.move_up();
        assert_eq!(state.selected, 1);

        // Clamp at last.
        for _ in 0..10 {
            state.move_down();
        }
        assert_eq!(state.selected, 2);
    }

    #[test]
    fn test_selected_entry() {
        let mut state = SessionPickerState::new();
        let sessions = vec![
            make_session_meta("s1", Some("First"), 0),
            make_session_meta("s2", Some("Second"), 1),
        ];
        state.open(sessions, None);

        assert_eq!(state.selected_session_id(), Some("s1"));
        state.move_down();
        assert_eq!(state.selected_session_id(), Some("s2"));
    }

    #[test]
    fn test_delete_char() {
        let mut state = SessionPickerState::new();
        let sessions = vec![
            make_session_meta("s1", Some("abc"), 0),
            make_session_meta("s2", Some("xyz"), 1),
        ];
        state.open(sessions, None);

        for c in "abc".chars() {
            state.insert_char(c);
        }
        assert_eq!(state.filtered_indices.len(), 1);

        state.delete_char();
        assert_eq!(state.filter, "ab");
        // "ab" still matches "abc"
        assert_eq!(state.filtered_indices.len(), 1);

        state.delete_char();
        state.delete_char();
        assert_eq!(state.filter, "");
        assert_eq!(state.filtered_indices.len(), 2);
    }

    #[test]
    fn test_relative_time() {
        let now = Utc::now();
        assert_eq!(relative_time(&now), "just now");
        assert_eq!(
            relative_time(&(now - chrono::Duration::minutes(5))),
            "5m ago"
        );
        assert_eq!(relative_time(&(now - chrono::Duration::hours(3))), "3h ago");
        assert_eq!(relative_time(&(now - chrono::Duration::days(7))), "7d ago");
    }

    // -- widget render smoke tests ------------------------------------------

    fn make_terminal() -> ratatui::Terminal<ratatui::backend::TestBackend> {
        let backend = ratatui::backend::TestBackend::new(120, 40);
        ratatui::Terminal::new(backend).unwrap()
    }

    #[test]
    fn session_picker_hidden_renders_nothing() {
        let mut terminal = make_terminal();
        let state = SessionPickerState::new();
        let theme = crate::theme::UcodeTheme::default();
        terminal
            .draw(|f| f.render_widget(SessionPicker::new(&state, &theme), f.area()))
            .unwrap();
    }

    #[test]
    fn session_picker_with_entries_renders() {
        let mut terminal = make_terminal();
        let mut state = SessionPickerState::new();
        let sessions = vec![
            make_session_meta("ses_001", Some("Debug session"), 5),
            make_session_meta("ses_002", Some("Feature work"), 60),
        ];
        state.open(sessions, Some("ses_001"));
        let theme = crate::theme::UcodeTheme::default();
        terminal
            .draw(|f| f.render_widget(SessionPicker::new(&state, &theme), f.area()))
            .unwrap();
    }

    #[test]
    fn session_picker_empty_renders() {
        let mut terminal = make_terminal();
        let mut state = SessionPickerState::new();
        state.open(vec![], None);
        let theme = crate::theme::UcodeTheme::default();
        terminal
            .draw(|f| f.render_widget(SessionPicker::new(&state, &theme), f.area()))
            .unwrap();
    }
}
