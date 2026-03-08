use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Widget;
use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

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
// PanelFocus
// ---------------------------------------------------------------------------

/// Which pane within a master-detail panel has focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PanelFocus {
    #[default]
    Master,
    Detail,
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
    /// Which pane (list or buffer) currently has focus.
    pub focus: PanelFocus,
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
            focus: PanelFocus::default(),
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

    pub fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            PanelFocus::Master => PanelFocus::Detail,
            PanelFocus::Detail => PanelFocus::Master,
        };
    }

    pub fn focus_master(&mut self) {
        self.focus = PanelFocus::Master;
    }

    pub fn focus_detail(&mut self) {
        self.focus = PanelFocus::Detail;
    }

    /// Select an item by absolute index, clamped to the visible item count.
    pub fn select_index(&mut self, idx: usize) {
        let count = self.visible_items().len();
        self.selected = if count == 0 { 0 } else { idx.min(count - 1) };
    }

    pub fn scroll_buffer_up(&mut self, n: usize) {
        self.buffer_scroll = self.buffer_scroll.saturating_sub(n);
    }

    pub fn scroll_buffer_down(&mut self, n: usize) {
        self.buffer_scroll = self.buffer_scroll.saturating_add(n);
    }

    /// Clamp `buffer_scroll` so the last line of content stays visible.
    ///
    /// `viewport_height` is the number of rows available to the buffer pane.
    pub fn clamp_buffer_scroll(&mut self, viewport_height: usize) {
        let total_lines = self.buffer.lines().count();
        let max_scroll = total_lines.saturating_sub(viewport_height);
        self.buffer_scroll = self.buffer_scroll.min(max_scroll);
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

        // Vertical border — accent color when detail pane is focused so the
        // user can tell which side is active.
        let border_style = match self.state.focus {
            PanelFocus::Detail => Style::new().fg(self.theme.accent),
            PanelFocus::Master => Style::new().fg(self.theme.border),
        };
        for y in border_area.y..border_area.y + border_area.height {
            buf[(border_area.x, y)]
                .set_char('│')
                .set_style(border_style);
        }

        // Split each pane: 1-row header + remaining content.
        let [list_header_area, list_content_area] =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(list_area);
        let [buf_header_area, buf_content_area] =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(buffer_area);

        // Render pane headers — focused pane gets accent background.
        self.render_pane_header(
            list_header_area,
            buf,
            " LIST ",
            self.state.focus == PanelFocus::Master,
        );
        self.render_pane_header(
            buf_header_area,
            buf,
            " DETAIL ",
            self.state.focus == PanelFocus::Detail,
        );

        // --- List panel ---
        self.render_list(list_content_area, buf);

        // --- Buffer panel ---
        self.render_buffer(buf_content_area, buf);
    }
}

impl MasterDetail<'_> {
    /// Render a 1-row pane header. Focused pane: accent bg + dark fg (inverted).
    /// Unfocused pane: dim text on default bg.
    fn render_pane_header(&self, area: Rect, buf: &mut Buffer, label: &str, focused: bool) {
        let style = if focused {
            Style::new()
                .fg(self.theme.background)
                .bg(self.theme.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(self.theme.text_dim)
        };
        // Fill the entire header row first so the bg covers the full width.
        for x in area.x..area.x + area.width {
            buf[(x, area.y)].set_char(' ').set_style(style);
        }
        for (i, ch) in label.chars().enumerate() {
            let x = area.x + i as u16;
            if x < area.x + area.width {
                buf[(x, area.y)].set_char(ch).set_style(style);
            }
        }
    }

    /// Render a single line of text using unicode-aware column tracking.
    ///
    /// Returns the display column reached after writing (relative to `area.x`).
    fn render_text_line(
        &self,
        area: Rect,
        buf: &mut Buffer,
        y: u16,
        text: &str,
        style: Style,
        x_offset: u16,
    ) -> usize {
        let max_w = area.width.saturating_sub(x_offset) as usize;
        let mut col = 0usize;
        for ch in text.chars() {
            if col >= max_w {
                break;
            }
            let ch_w = UnicodeWidthChar::width(ch).unwrap_or(1);
            let x = area.x + x_offset + col as u16;
            if x < area.x + area.width {
                buf[(x, y)].set_char(ch).set_style(style);
            }
            col += ch_w;
        }
        col
    }

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
            self.render_text_line(area, buf, y, &filter_text, style, 1);
            y += 1;
            // Blank separator line.
            y += 1;
        }

        // List items — each item occupies: label row, detail row, blank row.
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

            let prefix = if is_selected { "▸ " } else { "  " };
            let line = format!("{}{} {}", prefix, item.status_icon, item.label);
            let max_w = area.width as usize;

            // Render label using display-width-aware loop.
            let col = self.render_text_line(area, buf, y, &line, style, 0);
            // Fill remainder of row for selected highlight.
            if is_selected {
                let display_w = UnicodeWidthStr::width(line.as_str()).min(max_w);
                for x in (area.x + display_w as u16)..area.x + area.width {
                    buf[(x, y)].set_style(style);
                }
                let _ = col; // col == display_w for BMP chars; use display_w for fill
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
                self.render_text_line(area, buf, y, &detail, detail_style, 0);
                if is_selected {
                    let display_w = UnicodeWidthStr::width(detail.as_str()).min(max_w);
                    for x in (area.x + display_w as u16)..area.x + area.width {
                        buf[(x, y)].set_style(detail_style);
                    }
                }
                y += 1;
            }

            // Blank separator row between items.
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

        for (line_idx, line) in content.lines().enumerate() {
            if line_idx < scroll {
                continue;
            }
            if y >= area.y + area.height {
                break;
            }
            // 1-char left padding; render with display-width-aware loop.
            self.render_text_line(area, buf, y, line, style, 1);
            y += 1;
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
    fn panel_focus_toggle() {
        let mut state = MasterDetailState::default();
        assert_eq!(state.focus, PanelFocus::Master);
        state.toggle_focus();
        assert_eq!(state.focus, PanelFocus::Detail);
        state.toggle_focus();
        assert_eq!(state.focus, PanelFocus::Master);
    }

    #[test]
    fn buffer_scroll() {
        let mut state = MasterDetailState::default();
        state.set_buffer("line1\nline2\nline3\nline4\nline5".into());
        assert_eq!(state.buffer_scroll, 0);
        state.scroll_buffer_down(2);
        assert_eq!(state.buffer_scroll, 2);
        state.scroll_buffer_up(1);
        assert_eq!(state.buffer_scroll, 1);
        state.scroll_buffer_up(10); // clamps to 0
        assert_eq!(state.buffer_scroll, 0);
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

    #[test]
    fn select_index_basic() {
        let items = vec![
            ListItem::new("Read", "✓", "45ms"),
            ListItem::new("Edit", "✓", "12ms"),
            ListItem::new("Bash", "✗", "1.2s"),
        ];
        let mut state = MasterDetailState::new(items);
        state.select_index(2);
        assert_eq!(state.selected_index(), 2);
        assert_eq!(state.selected_item().unwrap().label, "Bash");
    }

    #[test]
    fn select_index_clamps_to_last() {
        let items = vec![
            ListItem::new("Read", "✓", "45ms"),
            ListItem::new("Edit", "✓", "12ms"),
        ];
        let mut state = MasterDetailState::new(items);
        state.select_index(99);
        assert_eq!(state.selected_index(), 1);
    }

    #[test]
    fn select_index_on_empty_list() {
        let mut state = MasterDetailState::new(Vec::new());
        state.select_index(5); // must not panic
        assert_eq!(state.selected_index(), 0);
    }

    #[test]
    fn clamp_buffer_scroll_prevents_overscroll() {
        let mut state = MasterDetailState::default();
        // 5 lines of content, viewport of 3 rows → max scroll = 2.
        state.set_buffer("a\nb\nc\nd\ne".into());
        state.scroll_buffer_down(100);
        assert_eq!(state.buffer_scroll, 100); // unclamped yet
        state.clamp_buffer_scroll(3);
        assert_eq!(state.buffer_scroll, 2);
    }

    #[test]
    fn clamp_buffer_scroll_viewport_larger_than_content() {
        let mut state = MasterDetailState::default();
        state.set_buffer("a\nb".into());
        state.scroll_buffer_down(5);
        state.clamp_buffer_scroll(10); // viewport bigger than content → clamp to 0
        assert_eq!(state.buffer_scroll, 0);
    }

    #[test]
    fn vim_insert_mode_tab_switching_not_suppressed() {
        use crate::keybinds::{Action, InputMode, KeybindPreset, KeybindResolver};
        use crossterm::event::{KeyCode, KeyEventKind, KeyEventState, KeyModifiers};

        let mut resolver = KeybindResolver::new(KeybindPreset::Vim);
        resolver.set_mode(InputMode::Insert);

        let make_press = |code, mods| crossterm::event::KeyEvent {
            code,
            modifiers: mods,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };

        // Alt+1..5 and Alt+h/l must resolve even in insert mode.
        assert_eq!(
            resolver.resolve(&make_press(KeyCode::Char('1'), KeyModifiers::ALT)),
            Some(Action::SelectTab1)
        );
        assert_eq!(
            resolver.resolve(&make_press(KeyCode::Char('5'), KeyModifiers::ALT)),
            Some(Action::SelectTab5)
        );
        assert_eq!(
            resolver.resolve(&make_press(KeyCode::Char('l'), KeyModifiers::ALT)),
            Some(Action::NextTab)
        );
        assert_eq!(
            resolver.resolve(&make_press(KeyCode::Char('h'), KeyModifiers::ALT)),
            Some(Action::PrevTab)
        );
    }
}
