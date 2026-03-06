use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Widget},
};
use unicode_width::UnicodeWidthChar;

use crate::theme::UcodeTheme;

// ---------------------------------------------------------------------------
// AutocompleteEntry
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutocompleteEntry {
    /// Display name, e.g. "/session fork"
    pub name: String,
    /// Short description, e.g. "Fork current session"
    pub description: String,
    /// Source badge, e.g. "[builtin]", "[user]", "[plugin]"
    pub source: String,
}

impl AutocompleteEntry {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        source: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            source: source.into(),
        }
    }
}

// ---------------------------------------------------------------------------
// AutocompleteState
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct AutocompleteState {
    pub entries: Vec<AutocompleteEntry>,
    pub selected: usize,
    pub visible: bool,
}

impl AutocompleteState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn show(&mut self, entries: Vec<AutocompleteEntry>) {
        self.entries = entries;
        self.selected = 0;
        self.visible = !self.entries.is_empty();
    }

    pub fn hide(&mut self) {
        self.visible = false;
    }

    /// Move selection down, wrapping at the end.
    pub fn next(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        self.selected = (self.selected + 1) % self.entries.len();
    }

    /// Move selection up, wrapping at the start.
    pub fn prev(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        self.selected = self
            .selected
            .checked_sub(1)
            .unwrap_or(self.entries.len() - 1);
    }

    pub fn selected_entry(&self) -> Option<&AutocompleteEntry> {
        if self.visible {
            self.entries.get(self.selected)
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// InputBoxState
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct InputBoxState {
    /// Raw text content.
    pub content: String,
    /// Byte offset of the cursor within `content`.
    pub cursor_pos: usize,
    /// Display column of the cursor (accounts for unicode width).
    pub cursor_col: usize,
    pub autocomplete: AutocompleteState,
}

impl InputBoxState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert `c` at the current cursor position and advance the cursor.
    pub fn insert_char(&mut self, c: char) {
        self.content.insert(self.cursor_pos, c);
        self.cursor_pos += c.len_utf8();
        self.cursor_col += c.width().unwrap_or(1);
    }

    /// Delete the character immediately before the cursor (backspace).
    pub fn delete_char(&mut self) {
        if self.cursor_pos == 0 {
            return;
        }
        // Walk back to the previous char boundary.
        let prev = self.prev_char_boundary(self.cursor_pos);
        let removed: char = self.content[prev..self.cursor_pos]
            .chars()
            .next()
            .unwrap_or('\0');
        self.content.remove(prev);
        self.cursor_pos = prev;
        self.cursor_col = self.cursor_col.saturating_sub(removed.width().unwrap_or(1));
    }

    /// Delete the character at the cursor position (delete key).
    pub fn delete_forward(&mut self) {
        if self.cursor_pos >= self.content.len() {
            return;
        }
        self.content.remove(self.cursor_pos);
        // cursor_pos and cursor_col stay the same.
    }

    /// Move cursor one character to the left.
    pub fn move_left(&mut self) {
        if self.cursor_pos == 0 {
            return;
        }
        let prev = self.prev_char_boundary(self.cursor_pos);
        let c = self.content[prev..self.cursor_pos]
            .chars()
            .next()
            .unwrap_or('\0');
        self.cursor_pos = prev;
        self.cursor_col = self.cursor_col.saturating_sub(c.width().unwrap_or(1));
    }

    /// Move cursor one character to the right.
    pub fn move_right(&mut self) {
        if self.cursor_pos >= self.content.len() {
            return;
        }
        let c = self.content[self.cursor_pos..]
            .chars()
            .next()
            .unwrap_or('\0');
        self.cursor_pos += c.len_utf8();
        self.cursor_col += c.width().unwrap_or(1);
    }

    /// Move cursor to the start of the current line.
    pub fn move_home(&mut self) {
        // Find the start of the current line by scanning back for '\n'.
        let line_start = self.content[..self.cursor_pos]
            .rfind('\n')
            .map(|i| i + 1)
            .unwrap_or(0);
        // Recompute cursor_col for the new position.
        self.cursor_col = display_width(&self.content[line_start..self.cursor_pos]);
        // Actually move to line start.
        self.cursor_pos = line_start;
        self.cursor_col = 0;
    }

    /// Move cursor to the end of the current line.
    pub fn move_end(&mut self) {
        let line_end = self.content[self.cursor_pos..]
            .find('\n')
            .map(|i| self.cursor_pos + i)
            .unwrap_or(self.content.len());
        // Advance cursor_col by the width of chars from current pos to line_end.
        let added = display_width(&self.content[self.cursor_pos..line_end]);
        self.cursor_pos = line_end;
        self.cursor_col += added;
    }

    /// Insert a newline at the cursor position.
    pub fn insert_newline(&mut self) {
        self.insert_char('\n');
        // After the newline the column resets to 0.
        self.cursor_col = 0;
    }

    /// Clear content and reset cursor.
    pub fn clear(&mut self) {
        self.content.clear();
        self.cursor_pos = 0;
        self.cursor_col = 0;
        self.autocomplete.hide();
    }

    /// Take the content, clear state, and return the taken string.
    pub fn take_content(&mut self) -> String {
        let taken = std::mem::take(&mut self.content);
        self.cursor_pos = 0;
        self.cursor_col = 0;
        self.autocomplete.hide();
        taken
    }

    pub fn is_empty(&self) -> bool {
        self.content.is_empty()
    }

    /// Number of lines (newlines + 1).
    pub fn line_count(&self) -> usize {
        self.content.chars().filter(|&c| c == '\n').count() + 1
    }

    /// True when the content starts with '/'.
    pub fn has_slash_prefix(&self) -> bool {
        self.content.starts_with('/')
    }

    /// True when the content starts with '@'.
    pub fn has_mention_prefix(&self) -> bool {
        self.content.starts_with('@')
    }

    // ------------------------------------------------------------------
    // Private helpers
    // ------------------------------------------------------------------

    /// Return the byte offset of the character boundary immediately before `pos`.
    fn prev_char_boundary(&self, pos: usize) -> usize {
        let mut p = pos.saturating_sub(1);
        while p > 0 && !self.content.is_char_boundary(p) {
            p -= 1;
        }
        p
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Sum of display widths of all chars in `s`.
fn display_width(s: &str) -> usize {
    s.chars().map(|c| c.width().unwrap_or(1)).sum()
}

// ---------------------------------------------------------------------------
// InputBox widget
// ---------------------------------------------------------------------------

pub struct InputBox<'a> {
    pub state: &'a InputBoxState,
    pub theme: &'a UcodeTheme,
}

impl<'a> InputBox<'a> {
    pub fn new(state: &'a InputBoxState, theme: &'a UcodeTheme) -> Self {
        Self { state, theme }
    }
}

impl Widget for InputBox<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        // Outer bordered block.
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border_style(true));
        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height == 0 || inner.width == 0 {
            return;
        }

        // Build the visible content for the first (and usually only) line.
        // We render line-by-line, one line per row inside the inner area.
        let lines: Vec<&str> = self.state.content.split('\n').collect();
        let visible_rows = inner.height as usize;

        // Determine which line the cursor is on and its column.
        let (cursor_line, cursor_line_col) =
            cursor_line_and_col(&self.state.content, self.state.cursor_pos);

        // Render each visible line.
        for (row_idx, line_text) in lines.iter().take(visible_rows).enumerate() {
            let y = inner.y + row_idx as u16;

            if row_idx == 0 {
                // First row: prepend the "> " prompt in accent color.
                let prompt = Span::styled("> ", self.theme.accent_style());
                let content_span = Span::styled(*line_text, self.theme.text_style());
                let line = Line::from(vec![prompt, content_span]);
                let row_area = Rect {
                    y,
                    height: 1,
                    ..inner
                };
                line.render(row_area, buf);
            } else {
                let content_span = Span::styled(*line_text, self.theme.text_style());
                let line = Line::from(vec![content_span]);
                let row_area = Rect {
                    y,
                    height: 1,
                    ..inner
                };
                line.render(row_area, buf);
            }
        }

        // Render cursor highlight.
        // The prompt "> " occupies 2 columns on row 0.
        let prompt_width: u16 = if cursor_line == 0 { 2 } else { 0 };
        let cursor_x = inner.x + prompt_width + cursor_line_col as u16;
        let cursor_y = inner.y + cursor_line as u16;

        if cursor_x < inner.x + inner.width && cursor_y < inner.y + inner.height {
            let cell = &mut buf[(cursor_x, cursor_y)];
            let current_style = cell.style();
            cell.set_style(current_style.add_modifier(Modifier::REVERSED));
        }
    }
}

/// Return `(line_index, column_within_line)` for a byte offset in `text`.
///
/// `column_within_line` is the display-width column, not a byte offset.
fn cursor_line_and_col(text: &str, byte_pos: usize) -> (usize, usize) {
    let clamped = byte_pos.min(text.len());
    let prefix = &text[..clamped];
    let line_idx = prefix.chars().filter(|&c| c == '\n').count();
    let line_start = prefix.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let col = display_width(&prefix[line_start..]);
    (line_idx, col)
}

// ---------------------------------------------------------------------------
// AutocompleteDropdown widget
// ---------------------------------------------------------------------------

pub struct AutocompleteDropdown<'a> {
    pub state: &'a AutocompleteState,
    pub theme: &'a UcodeTheme,
}

impl<'a> AutocompleteDropdown<'a> {
    pub fn new(state: &'a AutocompleteState, theme: &'a UcodeTheme) -> Self {
        Self { state, theme }
    }
}

impl Widget for AutocompleteDropdown<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if !self.state.visible || area.height == 0 || area.width == 0 {
            return;
        }

        let entries = &self.state.entries;
        let visible_rows = (area.height as usize).min(entries.len());

        for (i, entry) in entries.iter().take(visible_rows).enumerate() {
            let y = area.y + i as u16;
            let is_selected = i == self.state.selected;

            let row_area = Rect {
                y,
                height: 1,
                ..area
            };

            let (name_style, desc_style, src_style, bg_style) = if is_selected {
                let bg = Style::default()
                    .bg(self.theme.accent)
                    .fg(self.theme.background);
                (bg, bg, bg, bg)
            } else {
                (
                    self.theme.text_style(),
                    self.theme.dim_style(),
                    self.theme.muted_style(),
                    Style::default().bg(self.theme.surface),
                )
            };

            // Fill the row background.
            buf.set_style(row_area, bg_style);

            let line = Line::from(vec![
                Span::styled("  ", bg_style),
                Span::styled(entry.name.clone(), name_style),
                Span::styled("  ", bg_style),
                Span::styled(entry.description.clone(), desc_style),
                Span::styled("  ", bg_style),
                Span::styled(entry.source.clone(), src_style),
            ]);
            line.render(row_area, buf);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{buffer::Buffer, layout::Rect};

    // ------------------------------------------------------------------
    // InputBoxState tests
    // ------------------------------------------------------------------

    #[test]
    fn input_box_insert_char() {
        let mut state = InputBoxState::new();
        state.insert_char('h');
        state.insert_char('i');
        assert_eq!(state.content, "hi");
        assert_eq!(state.cursor_pos, 2);
    }

    #[test]
    fn input_box_delete_char() {
        let mut state = InputBoxState::new();
        for c in "hello".chars() {
            state.insert_char(c);
        }
        state.delete_char();
        assert_eq!(state.content, "hell");
        assert_eq!(state.cursor_pos, 4);
    }

    #[test]
    fn input_box_move_cursor() {
        let mut state = InputBoxState::new();
        for c in "abc".chars() {
            state.insert_char(c);
        }
        // cursor is after 'c' (pos 3); move left once → between 'b' and 'c'
        state.move_left();
        assert_eq!(state.cursor_pos, 2);
        state.insert_char('x');
        assert_eq!(state.content, "abxc");
    }

    #[test]
    fn input_box_clear() {
        let mut state = InputBoxState::new();
        for c in "hello world".chars() {
            state.insert_char(c);
        }
        state.clear();
        assert!(state.is_empty());
        assert_eq!(state.cursor_pos, 0);
        assert_eq!(state.cursor_col, 0);
    }

    #[test]
    fn input_box_take_content() {
        let mut state = InputBoxState::new();
        for c in "hello".chars() {
            state.insert_char(c);
        }
        let taken = state.take_content();
        assert_eq!(taken, "hello");
        assert!(state.is_empty());
        assert_eq!(state.cursor_pos, 0);
    }

    #[test]
    fn input_box_newline() {
        let mut state = InputBoxState::new();
        state.insert_char('a');
        state.insert_newline();
        state.insert_char('b');
        assert_eq!(state.line_count(), 2);
        assert_eq!(state.content, "a\nb");
    }

    #[test]
    fn input_box_slash_prefix() {
        let mut state = InputBoxState::new();
        state.insert_char('/');
        assert!(state.has_slash_prefix());
        assert!(!state.has_mention_prefix());
    }

    #[test]
    fn input_box_mention_prefix() {
        let mut state = InputBoxState::new();
        state.insert_char('@');
        assert!(state.has_mention_prefix());
        assert!(!state.has_slash_prefix());
    }

    // ------------------------------------------------------------------
    // AutocompleteState tests
    // ------------------------------------------------------------------

    fn sample_entries() -> Vec<AutocompleteEntry> {
        vec![
            AutocompleteEntry::new("/session fork", "Fork current session", "[builtin]"),
            AutocompleteEntry::new("/session list", "List sessions", "[builtin]"),
            AutocompleteEntry::new("/help", "Show help", "[builtin]"),
        ]
    }

    #[test]
    fn autocomplete_navigation() {
        let mut state = AutocompleteState::new();
        state.show(sample_entries());
        assert_eq!(state.selected, 0);

        state.next();
        assert_eq!(state.selected, 1);

        state.next();
        assert_eq!(state.selected, 2);

        // Wrap around.
        state.next();
        assert_eq!(state.selected, 0);

        // Prev wraps the other way.
        state.prev();
        assert_eq!(state.selected, 2);
    }

    #[test]
    fn autocomplete_selected_entry() {
        let mut state = AutocompleteState::new();
        state.show(sample_entries());
        state.next(); // select index 1

        let entry = state.selected_entry().expect("should have entry");
        assert_eq!(entry.name, "/session list");
    }

    #[test]
    fn autocomplete_hide() {
        let mut state = AutocompleteState::new();
        state.show(sample_entries());
        assert!(state.visible);

        state.hide();
        assert!(!state.visible);
        // selected_entry returns None when hidden.
        assert!(state.selected_entry().is_none());
    }

    // ------------------------------------------------------------------
    // Widget render test
    // ------------------------------------------------------------------

    #[test]
    fn input_box_renders() {
        let mut state = InputBoxState::new();
        for c in "hello".chars() {
            state.insert_char(c);
        }
        let theme = UcodeTheme::default();
        let widget = InputBox::new(&state, &theme);

        let area = Rect::new(0, 0, 40, 3);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);

        // Collect all cell symbols into a single string.
        let rendered: String = (0..40u16)
            .flat_map(|x| (0..3u16).map(move |y| (x, y)))
            .map(|(x, y)| buf[(x, y)].symbol().to_string())
            .collect();

        assert!(
            rendered.contains('>'),
            "prompt '>' missing in rendered output"
        );
    }
}
