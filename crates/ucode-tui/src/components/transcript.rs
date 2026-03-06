use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Widget,
};
use unicode_width::UnicodeWidthStr;

use crate::app::{StreamingMessage, ToolCallStatus, TranscriptEntry};
use crate::theme::UcodeTheme;

// ---------------------------------------------------------------------------
// TranscriptView widget
// ---------------------------------------------------------------------------

pub struct TranscriptView<'a> {
    pub entries: &'a [TranscriptEntry],
    pub scroll_offset: usize,
    pub auto_scroll: bool,
    pub theme: &'a UcodeTheme,
    /// When true the streaming cursor `▌` is visible (caller toggles for blink).
    pub show_cursor: bool,
}

impl<'a> TranscriptView<'a> {
    pub fn new(
        entries: &'a [TranscriptEntry],
        scroll_offset: usize,
        auto_scroll: bool,
        theme: &'a UcodeTheme,
        show_cursor: bool,
    ) -> Self {
        Self {
            entries,
            scroll_offset,
            auto_scroll,
            theme,
            show_cursor,
        }
    }
}

impl Widget for TranscriptView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        let width = area.width;
        let viewport_height = area.height as usize;

        // Pre-compute all lines for all entries so we can apply scroll_offset
        // as a line-level offset (not entry-level).
        let all_lines: Vec<Line<'_>> = self
            .entries
            .iter()
            .flat_map(|entry| entry_lines(entry, self.theme, width, self.show_cursor))
            .collect();

        let total_lines = all_lines.len();

        // Clamp scroll_offset to valid range.
        let max_offset = total_lines.saturating_sub(viewport_height);
        let offset = self.scroll_offset.min(max_offset);

        // Reserve the last row for the "new content" indicator when needed.
        let indicator_needed = !self.auto_scroll && total_lines > offset + viewport_height;
        let render_rows = if indicator_needed {
            viewport_height.saturating_sub(1)
        } else {
            viewport_height
        };

        // Render visible lines into the buffer.
        let visible = all_lines.iter().skip(offset).take(render_rows);
        for (row, line) in visible.enumerate() {
            let y = area.y + row as u16;
            let row_area = Rect::new(area.x, y, width, 1);
            line.clone().render(row_area, buf);
        }

        // Render the "new content" indicator at the bottom row.
        if indicator_needed {
            let indicator = Line::from(Span::styled(
                " \u{2193} New content below ",
                self.theme.accent_style(),
            ));
            let y = area.y + viewport_height as u16 - 1;
            let row_area = Rect::new(area.x, y, width, 1);
            indicator.render(row_area, buf);
        }
    }
}

// ---------------------------------------------------------------------------
// Entry → lines dispatch
// ---------------------------------------------------------------------------

fn entry_lines<'a>(
    entry: &'a TranscriptEntry,
    theme: &'a UcodeTheme,
    width: u16,
    show_cursor: bool,
) -> Vec<Line<'a>> {
    match entry {
        TranscriptEntry::UserMessage(text) => render_user_message(text, theme, width),
        TranscriptEntry::AssistantMessage(text) => render_assistant_message(text, theme, width),
        TranscriptEntry::Streaming(msg) => render_streaming_message(msg, theme, width, show_cursor),
        TranscriptEntry::ToolCall {
            name,
            status,
            duration_ms,
            summary,
        } => render_tool_call(name, status, *duration_ms, summary.as_deref(), theme, width),
        TranscriptEntry::RouterEvent(text) => render_router_event(text, theme),
        TranscriptEntry::SystemMessage(text) => render_system_message(text, theme),
    }
}

// ---------------------------------------------------------------------------
// Per-variant renderers
// ---------------------------------------------------------------------------

fn render_user_message<'a>(text: &str, theme: &'a UcodeTheme, width: u16) -> Vec<Line<'a>> {
    // "  You: {text}"
    // The prefix "  You: " is 7 chars; wrap the text portion at (width - 7).
    let prefix = "  You: ";
    let text_width = width.saturating_sub(prefix.width() as u16);
    let wrapped = wrap_text(text, text_width);

    let mut lines = Vec::with_capacity(wrapped.len().max(1));
    for (i, chunk) in wrapped.iter().enumerate() {
        if i == 0 {
            lines.push(Line::from(vec![
                Span::styled(prefix, theme.accent_style()),
                Span::styled(chunk.clone(), theme.text_style()),
            ]));
        } else {
            // Continuation lines: indent by prefix width.
            let indent = " ".repeat(prefix.width());
            lines.push(Line::from(vec![
                Span::raw(indent),
                Span::styled(chunk.clone(), theme.text_style()),
            ]));
        }
    }

    if lines.is_empty() {
        lines.push(Line::from(Span::styled(prefix, theme.accent_style())));
    }

    lines
}

fn render_assistant_message<'a>(text: &str, theme: &'a UcodeTheme, width: u16) -> Vec<Line<'a>> {
    // "  Assistant:\n  {text}"
    let header = Line::from(Span::styled("  Assistant:", theme.accent_style()));
    let mut lines = vec![header];
    lines.extend(render_indented_text(text, theme, width));
    lines
}

fn render_streaming_message<'a>(
    msg: &'a StreamingMessage,
    theme: &'a UcodeTheme,
    width: u16,
    show_cursor: bool,
) -> Vec<Line<'a>> {
    let header = Line::from(Span::styled("  Assistant:", theme.accent_style()));
    let mut lines = vec![header];

    // Wrap the content, then append cursor to the last line when visible.
    let content_width = width.saturating_sub(2); // 2-space indent
    let wrapped = wrap_text(&msg.content, content_width);

    if wrapped.is_empty() {
        // No content yet — show cursor on a blank indented line.
        let cursor = if show_cursor { "\u{258c}" } else { "" };
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(cursor, theme.text_style()),
        ]));
    } else {
        for (i, chunk) in wrapped.iter().enumerate() {
            let is_last = i == wrapped.len() - 1;
            if is_last && show_cursor {
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(chunk.clone(), theme.text_style()),
                    Span::styled("\u{258c}", theme.text_style()),
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(chunk.clone(), theme.text_style()),
                ]));
            }
        }
    }

    lines
}

fn render_tool_call<'a>(
    name: &str,
    status: &ToolCallStatus,
    duration_ms: Option<u64>,
    summary: Option<&str>,
    theme: &'a UcodeTheme,
    width: u16,
) -> Vec<Line<'a>> {
    let (icon, icon_style) = status_icon(status);

    // Top border: "  ┌ tool: {name} ────────"
    // We fill with ─ to reach `width` columns.
    let top_prefix = format!("  \u{250c} tool: {} ", name);
    let top_prefix_w = top_prefix.width();
    let fill_count = (width as usize).saturating_sub(top_prefix_w);
    let top_line = format!("{}{}", top_prefix, "\u{2500}".repeat(fill_count));

    // Body: "  │ {summary}  {icon}  {duration}"
    let duration_str = match duration_ms {
        Some(ms) if ms >= 1000 => format!("{:.1}s", ms as f64 / 1000.0),
        Some(ms) => format!("{}ms", ms),
        None => String::new(),
    };
    let summary_text = summary.unwrap_or("");

    // Bottom border: "  └──────────────────────"
    let bottom_prefix = "  \u{2514}";
    let bottom_fill = (width as usize).saturating_sub(bottom_prefix.width());
    let bottom_line = format!("{}{}", bottom_prefix, "\u{2500}".repeat(bottom_fill));

    let border_style = Style::new().fg(theme.border);

    let mut body_spans: Vec<Span<'a>> = vec![
        Span::styled("  \u{2502} ", border_style),
        Span::styled(summary_text.to_owned(), theme.text_style()),
    ];
    if !summary_text.is_empty() {
        body_spans.push(Span::raw("  "));
    }
    body_spans.push(Span::styled(icon, icon_style));
    if !duration_str.is_empty() {
        body_spans.push(Span::raw("  "));
        body_spans.push(Span::styled(duration_str, theme.dim_style()));
    }

    vec![
        Line::from(Span::styled(top_line, border_style)),
        Line::from(body_spans),
        Line::from(Span::styled(bottom_line, border_style)),
    ]
}

fn render_router_event<'a>(text: &str, theme: &'a UcodeTheme) -> Vec<Line<'a>> {
    // "  ↪ router: {text}"
    vec![Line::from(vec![
        Span::styled("  \u{21aa} router: ", theme.warning_style()),
        Span::styled(text.to_owned(), theme.warning_style()),
    ])]
}

fn render_system_message<'a>(text: &str, theme: &'a UcodeTheme) -> Vec<Line<'a>> {
    // "  ─ {text}"
    vec![Line::from(vec![
        Span::styled("  \u{2500} ", theme.muted_style()),
        Span::styled(text.to_owned(), theme.muted_style()),
    ])]
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns `(icon_str, style)` for the given tool call status.
fn status_icon(status: &ToolCallStatus) -> (&'static str, Style) {
    match status {
        ToolCallStatus::Success => ("\u{2713}", Style::new()), // ✓ — caller uses safe color
        ToolCallStatus::Failed => ("\u{2717}", Style::new()),  // ✗
        ToolCallStatus::Running => ("\u{27f3}", Style::new()), // ⟳
        ToolCallStatus::PendingApproval => ("\u{26a0}", Style::new()), // ⚠
    }
}

/// Wrap `text` to fit within `width` terminal columns using Unicode-aware
/// width measurement. Words that are wider than `width` are placed on their
/// own line without splitting.
fn wrap_text(text: &str, width: u16) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let max = width as usize;
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_width: usize = 0;

    for word in text.split_whitespace() {
        let word_w = word.width();
        if current.is_empty() {
            current.push_str(word);
            current_width = word_w;
        } else if current_width + 1 + word_w <= max {
            current.push(' ');
            current.push_str(word);
            current_width += 1 + word_w;
        } else {
            lines.push(current.clone());
            current.clear();
            current.push_str(word);
            current_width = word_w;
        }
    }

    if !current.is_empty() {
        lines.push(current);
    }

    lines
}

/// Render `text` as indented lines (2-space indent), wrapping at `width`.
fn render_indented_text<'a>(text: &str, theme: &'a UcodeTheme, width: u16) -> Vec<Line<'a>> {
    let content_width = width.saturating_sub(2);
    let wrapped = wrap_text(text, content_width);
    if wrapped.is_empty() {
        return vec![Line::from(Span::raw("  "))];
    }
    wrapped
        .into_iter()
        .map(|chunk| {
            Line::from(vec![
                Span::raw("  "),
                Span::styled(chunk, theme.text_style()),
            ])
        })
        .collect()
}

// ---------------------------------------------------------------------------
// entry_height — virtual scrolling helper
// ---------------------------------------------------------------------------

/// Compute how many terminal rows `entry` occupies when rendered at `width`.
pub fn entry_height(entry: &TranscriptEntry, width: u16) -> usize {
    match entry {
        TranscriptEntry::UserMessage(text) => {
            let prefix_w = "  You: ".width() as u16;
            let text_w = width.saturating_sub(prefix_w);
            wrap_text(text, text_w).len().max(1)
        }
        TranscriptEntry::AssistantMessage(text) => {
            // 1 header line + wrapped body lines
            let body_w = width.saturating_sub(2);
            1 + wrap_text(text, body_w).len().max(1)
        }
        TranscriptEntry::Streaming(msg) => {
            let body_w = width.saturating_sub(2);
            1 + wrap_text(&msg.content, body_w).len().max(1)
        }
        TranscriptEntry::ToolCall { name, summary, .. } => {
            // Always 3 lines: top border, body, bottom border.
            // (We don't wrap the body for height estimation — keep it simple.)
            let _ = (name, summary);
            3
        }
        TranscriptEntry::RouterEvent(_) => 1,
        TranscriptEntry::SystemMessage(_) => 1,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::StreamingMessage;
    use ratatui::{buffer::Buffer, layout::Rect};

    fn theme() -> UcodeTheme {
        UcodeTheme::default()
    }

    fn lines_text(lines: &[Line<'_>]) -> String {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    // -----------------------------------------------------------------------
    // render_user_message_format
    // -----------------------------------------------------------------------

    #[test]
    fn render_user_message_format() {
        let t = theme();
        let lines = render_user_message("hello world", &t, 80);
        let text = lines_text(&lines);
        assert!(
            text.contains("You:"),
            "expected 'You:' prefix, got: {text:?}"
        );
        assert!(
            text.contains("hello world"),
            "expected message text, got: {text:?}"
        );
    }

    // -----------------------------------------------------------------------
    // render_assistant_message_format
    // -----------------------------------------------------------------------

    #[test]
    fn render_assistant_message_format() {
        let t = theme();
        let lines = render_assistant_message("hello world", &t, 80);
        let text = lines_text(&lines);
        assert!(
            text.contains("Assistant:"),
            "expected 'Assistant:' prefix, got: {text:?}"
        );
        assert!(
            text.contains("hello world"),
            "expected message text, got: {text:?}"
        );
    }

    // -----------------------------------------------------------------------
    // render_streaming_with_cursor
    // -----------------------------------------------------------------------

    #[test]
    fn render_streaming_with_cursor() {
        let t = theme();
        let mut msg = StreamingMessage::new();
        msg.push_token("hello");
        let lines = render_streaming_message(&msg, &t, 80, true);
        let text = lines_text(&lines);
        assert!(
            text.contains('\u{258c}'),
            "expected cursor char ▌ when show_cursor=true, got: {text:?}"
        );
    }

    // -----------------------------------------------------------------------
    // render_streaming_without_cursor
    // -----------------------------------------------------------------------

    #[test]
    fn render_streaming_without_cursor() {
        let t = theme();
        let mut msg = StreamingMessage::new();
        msg.push_token("hello");
        let lines = render_streaming_message(&msg, &t, 80, false);
        let text = lines_text(&lines);
        assert!(
            !text.contains('\u{258c}'),
            "expected no cursor char when show_cursor=false, got: {text:?}"
        );
    }

    // -----------------------------------------------------------------------
    // render_tool_call_success
    // -----------------------------------------------------------------------

    #[test]
    fn render_tool_call_success() {
        let t = theme();
        let lines = render_tool_call(
            "read_file",
            &ToolCallStatus::Success,
            Some(42),
            Some("read 10 lines"),
            &t,
            80,
        );
        let text = lines_text(&lines);
        assert!(
            text.contains('\u{2713}'),
            "expected ✓ icon for Success, got: {text:?}"
        );
        assert!(
            text.contains("read_file"),
            "expected tool name, got: {text:?}"
        );
    }

    // -----------------------------------------------------------------------
    // render_tool_call_pending
    // -----------------------------------------------------------------------

    #[test]
    fn render_tool_call_pending() {
        let t = theme();
        let lines = render_tool_call(
            "write_file",
            &ToolCallStatus::PendingApproval,
            None,
            None,
            &t,
            80,
        );
        let text = lines_text(&lines);
        assert!(
            text.contains('\u{26a0}'),
            "expected ⚠ icon for PendingApproval, got: {text:?}"
        );
    }

    // -----------------------------------------------------------------------
    // render_router_event_format
    // -----------------------------------------------------------------------

    #[test]
    fn render_router_event_format() {
        let t = theme();
        let lines = render_router_event("anthropic -> openai", &t);
        let text = lines_text(&lines);
        assert!(
            text.contains('\u{21aa}'),
            "expected ↪ prefix, got: {text:?}"
        );
        assert!(
            text.contains("anthropic -> openai"),
            "expected event text, got: {text:?}"
        );
    }

    // -----------------------------------------------------------------------
    // render_system_message_format
    // -----------------------------------------------------------------------

    #[test]
    fn render_system_message_format() {
        let t = theme();
        let lines = render_system_message("rate limit hit", &t);
        let text = lines_text(&lines);
        assert!(
            text.contains('\u{2500}'),
            "expected ─ prefix, got: {text:?}"
        );
        assert!(
            text.contains("rate limit hit"),
            "expected message text, got: {text:?}"
        );
    }

    // -----------------------------------------------------------------------
    // entry_height_single_line
    // -----------------------------------------------------------------------

    #[test]
    fn entry_height_single_line() {
        let entry = TranscriptEntry::UserMessage("hi".to_owned());
        let h = entry_height(&entry, 80);
        // Short message: 1 line
        assert_eq!(h, 1, "short user message should be 1 line");

        let entry2 = TranscriptEntry::RouterEvent("event".to_owned());
        assert_eq!(entry_height(&entry2, 80), 1);

        let entry3 = TranscriptEntry::SystemMessage("sys".to_owned());
        assert_eq!(entry_height(&entry3, 80), 1);
    }

    // -----------------------------------------------------------------------
    // entry_height_wrapping
    // -----------------------------------------------------------------------

    #[test]
    fn entry_height_wrapping() {
        // A message that definitely wraps at width=20.
        let long = "word ".repeat(20);
        let entry = TranscriptEntry::UserMessage(long.trim().to_owned());
        let h = entry_height(&entry, 20);
        assert!(
            h > 1,
            "long message should wrap to multiple lines, got h={h}"
        );
    }

    // -----------------------------------------------------------------------
    // transcript_renders_to_buffer
    // -----------------------------------------------------------------------

    #[test]
    fn transcript_renders_to_buffer() {
        let t = theme();
        let mut msg = StreamingMessage::new();
        msg.push_token("streaming content");

        let entries = vec![
            TranscriptEntry::UserMessage("hello".to_owned()),
            TranscriptEntry::AssistantMessage("world".to_owned()),
            TranscriptEntry::Streaming(msg),
            TranscriptEntry::ToolCall {
                name: "my_tool".to_owned(),
                status: ToolCallStatus::Success,
                duration_ms: Some(100),
                summary: Some("did stuff".to_owned()),
            },
            TranscriptEntry::RouterEvent("rerouted".to_owned()),
            TranscriptEntry::SystemMessage("info".to_owned()),
        ];

        let widget = TranscriptView::new(&entries, 0, true, &t, true);
        let area = Rect::new(0, 0, 80, 30);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);

        // Collect all rendered text from the buffer.
        let rendered: String = (0..30u16)
            .flat_map(|y| (0..80u16).map(move |x| (x, y)))
            .map(|(x, y)| buf[(x, y)].symbol().to_owned())
            .collect();

        assert!(rendered.contains("You:"), "user message prefix missing");
        assert!(rendered.contains("hello"), "user message text missing");
        assert!(rendered.contains("Assistant:"), "assistant prefix missing");
        assert!(rendered.contains("world"), "assistant text missing");
        assert!(rendered.contains("my_tool"), "tool call name missing");
        assert!(rendered.contains("rerouted"), "router event text missing");
        assert!(rendered.contains("info"), "system message text missing");
    }

    // -----------------------------------------------------------------------
    // wrap_text_basic
    // -----------------------------------------------------------------------

    #[test]
    fn wrap_text_basic() {
        // "hello world foo" at width=11 → ["hello world", "foo"]
        let result = wrap_text("hello world foo", 11);
        assert_eq!(result, vec!["hello world", "foo"]);
    }

    #[test]
    fn wrap_text_single_word_fits() {
        let result = wrap_text("hello", 10);
        assert_eq!(result, vec!["hello"]);
    }

    #[test]
    fn wrap_text_empty() {
        let result = wrap_text("", 80);
        assert!(result.is_empty());
    }

    #[test]
    fn wrap_text_zero_width() {
        let result = wrap_text("hello world", 0);
        assert!(result.is_empty());
    }

    // -----------------------------------------------------------------------
    // status_icon coverage
    // -----------------------------------------------------------------------

    #[test]
    fn status_icon_variants() {
        assert_eq!(status_icon(&ToolCallStatus::Success).0, "\u{2713}");
        assert_eq!(status_icon(&ToolCallStatus::Failed).0, "\u{2717}");
        assert_eq!(status_icon(&ToolCallStatus::Running).0, "\u{27f3}");
        assert_eq!(status_icon(&ToolCallStatus::PendingApproval).0, "\u{26a0}");
    }

    // -----------------------------------------------------------------------
    // auto_scroll indicator
    // -----------------------------------------------------------------------

    #[test]
    fn transcript_shows_indicator_when_not_auto_scroll() {
        let t = theme();
        // Fill with enough entries to overflow a small viewport.
        let entries: Vec<TranscriptEntry> = (0..20)
            .map(|i| TranscriptEntry::UserMessage(format!("message {i}")))
            .collect();

        // scroll_offset=0, auto_scroll=false → indicator should appear.
        let widget = TranscriptView::new(&entries, 0, false, &t, false);
        let area = Rect::new(0, 0, 80, 5);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);

        let rendered: String = (0..5u16)
            .flat_map(|y| (0..80u16).map(move |x| (x, y)))
            .map(|(x, y)| buf[(x, y)].symbol().to_owned())
            .collect();

        assert!(
            rendered.contains('\u{2193}'),
            "expected ↓ indicator when not auto_scroll, got: {rendered:?}"
        );
    }

    #[test]
    fn transcript_no_indicator_when_auto_scroll() {
        let t = theme();
        let entries: Vec<TranscriptEntry> = (0..20)
            .map(|i| TranscriptEntry::UserMessage(format!("message {i}")))
            .collect();

        let widget = TranscriptView::new(&entries, 0, true, &t, false);
        let area = Rect::new(0, 0, 80, 5);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);

        let rendered: String = (0..5u16)
            .flat_map(|y| (0..80u16).map(move |x| (x, y)))
            .map(|(x, y)| buf[(x, y)].symbol().to_owned())
            .collect();

        assert!(
            !rendered.contains('\u{2193}'),
            "expected no ↓ indicator when auto_scroll=true"
        );
    }
}
