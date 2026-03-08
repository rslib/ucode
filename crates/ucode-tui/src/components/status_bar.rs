use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::theme::UcodeTheme;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct StatusBarState {
    pub keybind_hints: Vec<String>,
    /// Transient hint message shown temporarily (e.g. "Press Ctrl+C again to exit").
    pub hint_message: Option<String>,
    pub streaming: bool,
    pub stream_tok_per_sec: Option<f64>,
    /// Copy-mode indicator (e.g. "COPY", "VISUAL", "V-LINE", "V-BLOCK").
    pub copy_mode_label: Option<String>,
}

impl Default for StatusBarState {
    fn default() -> Self {
        Self {
            keybind_hints: vec!["^P".into(), "^O".into(), "^E".into()],
            hint_message: None,
            streaming: false,
            stream_tok_per_sec: None,
            copy_mode_label: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Widget
// ---------------------------------------------------------------------------

pub struct StatusBar<'a> {
    pub state: &'a StatusBarState,
    pub theme: &'a UcodeTheme,
}

impl<'a> StatusBar<'a> {
    pub fn new(state: &'a StatusBarState, theme: &'a UcodeTheme) -> Self {
        Self { state, theme }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn sep(theme: &UcodeTheme) -> Span<'static> {
    Span::styled(" │ ", Style::new().fg(theme.border))
}

/// Build the ordered list of segments.  Each segment is a small `Vec<Span>`
/// so that multi-span segments stay together when the truncation logic drops
/// rightmost segments.
fn build_segments<'a>(state: &'a StatusBarState, theme: &'a UcodeTheme) -> Vec<Vec<Span<'a>>> {
    let mut segments: Vec<Vec<Span<'a>>> = Vec::new();

    // 1. Copy-mode indicator (high-priority — always visible when active).
    if let Some(label) = &state.copy_mode_label {
        segments.push(vec![Span::styled(
            format!("-- {} --", label),
            theme.warning_style(),
        )]);
    }

    // 2. Keybind hints
    if !state.keybind_hints.is_empty() {
        let hints = state.keybind_hints.join(" ");
        segments.push(vec![Span::styled(hints, theme.muted_style())]);
    }

    // 3. Transient hint message
    if let Some(hint) = &state.hint_message {
        segments.push(vec![Span::styled(hint.clone(), theme.warning_style())]);
    }

    // 4. Streaming indicator
    if state.streaming {
        let label = match state.stream_tok_per_sec {
            Some(tps) => format!("streaming... ● {:.0} tok/s", tps),
            None => "streaming... ●".into(),
        };
        segments.push(vec![Span::styled(label, theme.warning_style())]);
    }

    segments
}

impl Widget for StatusBar<'_> {
    fn render(self, area: ratatui::layout::Rect, buf: &mut ratatui::buffer::Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        let segments = build_segments(self.state, self.theme);
        let sep_width = 3u16; // " │ "

        // Measure each segment's display width (sum of content byte lengths;
        // segments are ASCII-dominant so this is a safe approximation).
        let seg_widths: Vec<u16> = segments
            .iter()
            .map(|spans| spans.iter().map(|s| s.content.len() as u16).sum())
            .collect();

        // Greedily include segments from left to right until the area is full.
        let mut used_width: u16 = 0;
        let mut visible_count = 0usize;
        for (i, &w) in seg_widths.iter().enumerate() {
            let needed = if i == 0 { w } else { sep_width + w };
            if used_width + needed > area.width {
                break;
            }
            used_width += needed;
            visible_count += 1;
        }
        // Always show at least one segment so the bar is never blank.
        visible_count = visible_count.max(1);

        // Flatten visible segments with separators into a single Line.
        let mut spans: Vec<Span<'_>> = Vec::new();
        for (i, seg) in segments.into_iter().take(visible_count).enumerate() {
            if i > 0 {
                spans.push(sep(self.theme));
            }
            spans.extend(seg);
        }

        let line = Line::from(spans);
        let row_area = ratatui::layout::Rect { height: 1, ..area };
        line.render(row_area, buf);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    fn render_to_buf(state: &StatusBarState, width: u16) -> Buffer {
        let theme = UcodeTheme::default();
        let widget = StatusBar::new(state, &theme);
        let area = Rect::new(0, 0, width, 1);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);
        buf
    }

    fn buf_text(buf: &Buffer) -> String {
        let area = buf.area;
        (0..area.width)
            .map(|x| buf[(x, 0)].symbol().to_owned())
            .collect()
    }

    #[test]
    fn status_bar_state_default() {
        let state = StatusBarState::default();
        assert_eq!(state.keybind_hints, vec!["^P", "^O", "^E"]);
        assert!(!state.streaming);
        assert!(state.stream_tok_per_sec.is_none());
        assert!(state.hint_message.is_none());
        assert!(state.copy_mode_label.is_none());
    }

    #[test]
    fn status_bar_renders_keybind_hints() {
        let state = StatusBarState::default();
        let buf = render_to_buf(&state, 120);
        let text = buf_text(&buf);
        assert!(text.contains("^P"), "expected ^P in: {text:?}");
        assert!(text.contains("^O"), "expected ^O in: {text:?}");
        assert!(text.contains("^E"), "expected ^E in: {text:?}");
    }

    #[test]
    fn status_bar_streaming_mode() {
        let state = StatusBarState {
            streaming: true,
            stream_tok_per_sec: Some(42.0),
            ..StatusBarState::default()
        };
        let buf = render_to_buf(&state, 120);
        let text = buf_text(&buf);
        assert!(
            text.contains("streaming"),
            "expected 'streaming' in: {text:?}"
        );
        assert!(text.contains("42"), "expected tok/s rate in: {text:?}");
    }

    #[test]
    fn status_bar_streaming_no_rate() {
        let state = StatusBarState {
            streaming: true,
            stream_tok_per_sec: None,
            ..StatusBarState::default()
        };
        let buf = render_to_buf(&state, 120);
        let text = buf_text(&buf);
        assert!(
            text.contains("streaming"),
            "expected 'streaming' in: {text:?}"
        );
    }

    #[test]
    fn status_bar_hint_message_rendered() {
        let state = StatusBarState {
            hint_message: Some("Press Ctrl+C again to exit".into()),
            ..StatusBarState::default()
        };
        let buf = render_to_buf(&state, 120);
        let text = buf_text(&buf);
        assert!(
            text.contains("Press Ctrl+C again to exit"),
            "expected hint in: {text:?}"
        );
    }

    #[test]
    fn status_bar_copy_mode_rendered() {
        let state = StatusBarState {
            copy_mode_label: Some("VISUAL".into()),
            ..StatusBarState::default()
        };
        let buf = render_to_buf(&state, 120);
        let text = buf_text(&buf);
        assert!(text.contains("VISUAL"), "expected VISUAL in: {text:?}");
    }

    #[test]
    fn status_bar_narrow_does_not_panic() {
        let state = StatusBarState::default();
        let _ = render_to_buf(&state, 10);
    }

    #[test]
    fn status_bar_zero_width_does_not_panic() {
        let state = StatusBarState::default();
        let _ = render_to_buf(&state, 0);
    }

    #[test]
    fn status_bar_no_old_segments() {
        let state = StatusBarState::default();
        let buf = render_to_buf(&state, 120);
        let text = buf_text(&buf);
        // Should NOT contain old segments that were removed.
        assert!(
            !text.contains("INFO"),
            "should not contain log level: {text:?}"
        );
        assert!(!text.contains("$0"), "should not contain cost: {text:?}");
        assert!(
            !text.contains("claude"),
            "should not contain model name: {text:?}"
        );
    }
}
