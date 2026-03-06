use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
    widgets::Widget,
};
use unicode_width::UnicodeWidthStr;

use crate::theme::UcodeTheme;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleBarState {
    /// Human-readable session name, e.g. "main".
    pub session_title: String,
    /// Full session ID; only the first 6 chars are displayed.
    pub session_id: String,
    /// Set when this session was forked from another; holds the parent title.
    pub parent_title: Option<String>,
    /// Pre-formatted date string, e.g. "2026-03-05".
    pub date: String,
    /// Terminal multiplexer name ("tmux", "zellij", …) when running inside one.
    pub in_multiplexer: Option<String>,
}

impl TitleBarState {
    pub fn new(
        session_title: impl Into<String>,
        session_id: impl Into<String>,
        date: impl Into<String>,
    ) -> Self {
        Self {
            session_title: session_title.into(),
            session_id: session_id.into(),
            parent_title: None,
            date: date.into(),
            in_multiplexer: None,
        }
    }

    /// Returns at most 6 characters of the session ID.
    fn short_id(&self) -> &str {
        let s = self.session_id.as_str();
        // Find the byte offset of the 6th character boundary (handles ASCII IDs fine).
        let end = s.char_indices().nth(6).map(|(i, _)| i).unwrap_or(s.len());
        &s[..end]
    }
}

// ---------------------------------------------------------------------------
// Widget
// ---------------------------------------------------------------------------

pub struct TitleBar<'a> {
    pub state: &'a TitleBarState,
    pub theme: &'a UcodeTheme,
}

impl<'a> TitleBar<'a> {
    pub fn new(state: &'a TitleBarState, theme: &'a UcodeTheme) -> Self {
        Self { state, theme }
    }

    fn left_spans(&self) -> Vec<Span<'a>> {
        let mut spans = Vec::with_capacity(6);

        // "ucode" in accent color
        spans.push(Span::styled(" ucode", self.theme.accent_style()));

        // space + session title
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            self.state.session_title.clone(),
            self.theme.text_style(),
        ));

        // ⎇ parent_title when forked
        if let Some(ref parent) = self.state.parent_title {
            spans.push(Span::styled(
                format!(" \u{2387} {parent}"),
                self.theme.dim_style(),
            ));
        }

        // [multiplexer] indicator
        if let Some(ref mux) = self.state.in_multiplexer {
            spans.push(Span::styled(format!(" [{mux}]"), self.theme.muted_style()));
        }

        spans
    }

    fn right_text(&self) -> String {
        format!("{}  {} ", self.state.short_id(), self.state.date)
    }
}

impl Widget for TitleBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        let right_text = self.right_text();
        let right_width = right_text.width() as u16;

        // Build the left line and render it.
        let left_spans = self.left_spans();
        let left_line = Line::from(left_spans);
        left_line.render(area, buf);

        // Render right side right-aligned, overwriting whatever the left line
        // placed there (the right side is always shorter than the full width).
        let right_style = self.theme.dim_style();
        if right_width <= area.width {
            let right_x = area.x + area.width - right_width;
            let right_area = Rect::new(right_x, area.y, right_width, 1);
            Line::from(Span::styled(right_text, right_style)).render(right_area, buf);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    fn default_state() -> TitleBarState {
        TitleBarState::new("main", "abc123def456", "2026-03-05")
    }

    fn default_theme() -> UcodeTheme {
        UcodeTheme::default()
    }

    #[test]
    fn title_bar_state_default() {
        let state = default_state();
        assert_eq!(state.session_title, "main");
        assert_eq!(state.session_id, "abc123def456");
        assert_eq!(state.date, "2026-03-05");
        assert!(state.parent_title.is_none());
        assert!(state.in_multiplexer.is_none());
    }

    #[test]
    fn title_bar_short_session_id() {
        let state = TitleBarState::new("main", "ab", "2026-03-05");
        assert_eq!(state.short_id(), "ab");

        let state2 = TitleBarState::new("main", "abc123def", "2026-03-05");
        assert_eq!(state2.short_id(), "abc123");
    }

    #[test]
    fn title_bar_with_fork() {
        let mut state = default_state();
        state.parent_title = Some("root".to_string());
        assert_eq!(state.parent_title.as_deref(), Some("root"));

        let theme = default_theme();
        let widget = TitleBar::new(&state, &theme);
        let spans = widget.left_spans();
        let combined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(combined.contains('\u{2387}'), "fork indicator missing");
        assert!(combined.contains("root"), "parent title missing");
    }

    #[test]
    fn title_bar_with_multiplexer() {
        let mut state = default_state();
        state.in_multiplexer = Some("tmux".to_string());

        let theme = default_theme();
        let widget = TitleBar::new(&state, &theme);
        let spans = widget.left_spans();
        let combined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(combined.contains("[tmux]"), "multiplexer indicator missing");
    }

    #[test]
    fn title_bar_renders() {
        let state = default_state();
        let theme = default_theme();
        let widget = TitleBar::new(&state, &theme);

        let area = Rect::new(0, 0, 80, 1);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);

        // Collect all cell content into a single string for easy assertion.
        let rendered: String = (0..80).map(|x| buf[(x, 0)].symbol().to_string()).collect();

        assert!(rendered.contains("ucode"), "brand name missing");
        assert!(rendered.contains("main"), "session title missing");
        assert!(rendered.contains("abc123"), "short session id missing");
        assert!(rendered.contains("2026-03-05"), "date missing");
    }
}
