use ratatui::{
    buffer::Buffer,
    layout::Rect,
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
    /// Copy-mode state for highlighting selected entries.
    pub copy_mode: &'a crate::overlays::copy_mode::CopyModeState,
}

impl<'a> TranscriptView<'a> {
    pub fn new(
        entries: &'a [TranscriptEntry],
        scroll_offset: usize,
        auto_scroll: bool,
        theme: &'a UcodeTheme,
        show_cursor: bool,
        copy_mode: &'a crate::overlays::copy_mode::CopyModeState,
    ) -> Self {
        Self {
            entries,
            scroll_offset,
            auto_scroll,
            theme,
            show_cursor,
            copy_mode,
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

        let mut all_lines = compute_all_lines(self.entries, self.theme, width, self.show_cursor);

        // Apply copy-mode highlighting by visual line index.
        if self.copy_mode.active {
            use ratatui::style::{Modifier, Style};
            let cursor_col = self.copy_mode.cursor.col;

            for (line_idx, line) in all_lines.iter_mut().enumerate() {
                if self.copy_mode.selecting {
                    // Phase 2: column-aware highlighting based on visual mode.
                    let line_width: usize = line
                        .spans
                        .iter()
                        .map(|s| unicode_width::UnicodeWidthStr::width(s.content.as_ref()))
                        .sum();
                    if let Some((sc, ec)) = self.copy_mode.line_col_range(line_idx, line_width) {
                        let hl = Style::default()
                            .bg(self.theme.select)
                            .add_modifier(Modifier::BOLD);
                        *line = highlight_line_range(line, sc, ec, hl);
                    }
                } else if self.copy_mode.is_cursor_line(line_idx) {
                    // Phase 1: cursor line indicator — subtle dedicated background.
                    let hl = Style::default().bg(self.theme.select_cursor);
                    let spans: Vec<_> = line
                        .spans
                        .iter()
                        .map(|span| {
                            let mut s = span.clone();
                            s.style = s.style.patch(hl);
                            s
                        })
                        .collect();
                    *line = Line::from(spans);
                }

                // Character cursor: highlight the single cell at (cursor.line, cursor.col)
                // with REVERSED style so the user can see their exact column position.
                // Applied after selection highlighting so it overlays on top.
                if self.copy_mode.is_cursor_line(line_idx) {
                    let cursor_hl = Style::default().add_modifier(Modifier::REVERSED);
                    *line = highlight_line_range(line, cursor_col, cursor_col + 1, cursor_hl);
                }
            }
        }

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
// compute_all_lines — public helper for line-level copy mode
// ---------------------------------------------------------------------------

/// Compute all visual lines for the transcript entries.
///
/// Returns the flat list of rendered lines with no copy-mode highlighting
/// applied. The blinking cursor only appears on the last `Streaming` entry
/// when it is the very last transcript entry.
pub fn compute_all_lines<'a>(
    entries: &'a [TranscriptEntry],
    theme: &'a UcodeTheme,
    width: u16,
    show_cursor: bool,
) -> Vec<Line<'a>> {
    let last_entry_idx = entries.len().checked_sub(1);
    let cursor_idx =
        last_entry_idx.filter(|&idx| matches!(entries[idx], TranscriptEntry::Streaming(_)));

    entries
        .iter()
        .enumerate()
        .flat_map(|(i, entry)| {
            let cursor = show_cursor && Some(i) == cursor_idx;
            entry_lines(entry, theme, width, cursor)
        })
        .collect()
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
            thinking,
            output,
        } => render_tool_call(
            name,
            status,
            *duration_ms,
            summary.as_deref(),
            thinking.as_deref(),
            output.as_deref(),
            theme,
            width,
        ),
        TranscriptEntry::RouterEvent(text) => render_router_event(text, theme),
        TranscriptEntry::SystemMessage(text) => render_system_message(text, theme),
        TranscriptEntry::PatchProposed { file_path, .. } => {
            render_system_message(&format!("Patch proposed: {file_path}"), theme)
        }
    }
}

// ---------------------------------------------------------------------------
// Per-variant renderers
// ---------------------------------------------------------------------------

fn render_user_message<'a>(text: &str, theme: &'a UcodeTheme, width: u16) -> Vec<Line<'a>> {
    // Left accent border: "│ {text}"
    let prefix = "│ ";
    let text_width = width.saturating_sub(prefix.width() as u16);
    let wrapped = wrap_text(text, text_width);

    let mut lines = Vec::with_capacity(wrapped.len().max(1) + 1);

    for chunk in &wrapped {
        lines.push(Line::from(vec![
            Span::styled(prefix, theme.accent_style()),
            Span::styled(chunk.clone(), theme.text_style()),
        ]));
    }

    if lines.is_empty() {
        lines.push(Line::from(Span::styled(prefix, theme.accent_style())));
    }

    // Blank line after user message for visual separation.
    lines.push(Line::from(""));

    lines
}

fn render_assistant_message<'a>(text: &str, theme: &'a UcodeTheme, width: u16) -> Vec<Line<'a>> {
    let content_width = width.saturating_sub(2);
    let md_lines = super::markdown::render_markdown(text, theme, content_width);

    let mut lines: Vec<Line<'a>> = md_lines
        .into_iter()
        .map(|line| {
            let mut spans = Vec::with_capacity(line.spans.len() + 1);
            spans.push(Span::raw("  "));
            spans.extend(line.spans);
            Line::from(spans)
        })
        .collect();

    if lines.is_empty() {
        lines.push(Line::from(Span::raw("  ")));
    }

    // Blank line after for visual separation.
    lines.push(Line::from(""));
    lines
}

fn render_streaming_message<'a>(
    msg: &'a StreamingMessage,
    theme: &'a UcodeTheme,
    width: u16,
    show_cursor: bool,
) -> Vec<Line<'a>> {
    let content_width = width.saturating_sub(2);
    let md_lines = super::markdown::render_markdown(&msg.content, theme, content_width);

    if md_lines.is_empty() {
        let cursor = if show_cursor { "\u{258c}" } else { "" };
        return vec![Line::from(vec![
            Span::raw("  "),
            Span::styled(cursor.to_owned(), theme.text_style()),
        ])];
    }

    let mut lines: Vec<Line<'a>> = Vec::with_capacity(md_lines.len());
    let last_idx = md_lines.len() - 1;

    for (i, line) in md_lines.into_iter().enumerate() {
        let mut spans = Vec::with_capacity(line.spans.len() + 2);
        spans.push(Span::raw("  "));
        spans.extend(line.spans);
        if i == last_idx && show_cursor {
            spans.push(Span::styled("\u{258c}".to_owned(), theme.text_style()));
        }
        lines.push(Line::from(spans));
    }

    lines
}

#[allow(clippy::too_many_arguments)]
fn render_tool_call<'a>(
    name: &str,
    status: &ToolCallStatus,
    duration_ms: Option<u64>,
    summary: Option<&str>,
    thinking: Option<&str>,
    output: Option<&str>,
    theme: &'a UcodeTheme,
    width: u16,
) -> Vec<Line<'a>> {
    let (icon, icon_style) = match status {
        ToolCallStatus::Running => ("\u{27f3}", theme.accent_style()),
        ToolCallStatus::Success => ("\u{2713}", theme.safe_style()),
        ToolCallStatus::Failed => ("\u{2717}", theme.danger_style()),
        ToolCallStatus::PendingApproval => ("\u{26a0}", theme.warning_style()),
    };

    let duration_str = match duration_ms {
        Some(ms) if ms >= 1000 => format!(" {:.1}s", ms as f64 / 1000.0),
        Some(ms) => format!(" {}ms", ms),
        None => String::new(),
    };

    let mut spans: Vec<Span<'a>> = vec![
        Span::styled("  \u{2192} ", theme.accent_style()), // → arrow
        Span::styled(name.to_owned(), theme.text_style()),
    ];

    // Show summary as bracketed args if present.
    if let Some(sum) = summary.filter(|s| !s.is_empty()) {
        spans.push(Span::styled(format!(" [{sum}]"), theme.dim_style()));
    }

    // Status icon.
    spans.push(Span::raw(" "));
    spans.push(Span::styled(icon, icon_style));

    // Duration.
    if !duration_str.is_empty() {
        spans.push(Span::styled(duration_str, theme.dim_style()));
    }

    // Pending approval gets extra label.
    if *status == ToolCallStatus::PendingApproval {
        spans.push(Span::styled(" pending approval", theme.warning_style()));
    }

    let mut lines = vec![Line::from(spans)];

    // Thinking summary line (dim). Prefix "    ⊙⊙ " = 7 chars.
    if let Some(thought) = thinking.filter(|s| !s.is_empty()) {
        let max_len = (width as usize).saturating_sub(7);
        let first_line = thought.lines().next().unwrap_or("");
        let truncated: String = if first_line.chars().count() > max_len {
            let mut s: String = first_line.chars().take(max_len.saturating_sub(3)).collect();
            s.push_str("...");
            s
        } else {
            first_line.to_owned()
        };
        lines.push(Line::from(vec![
            Span::styled("    \u{2299}\u{2299} ", theme.dim_style()),
            Span::styled(truncated, theme.dim_style()),
        ]));
    }

    // Output summary line (muted). Prefix "    ▮▮ " = 7 chars.
    if let Some(out) = output.filter(|s| !s.is_empty()) {
        let max_len = (width as usize).saturating_sub(7);
        let first_line = out.lines().next().unwrap_or("");
        let truncated: String = if first_line.chars().count() > max_len {
            let mut s: String = first_line.chars().take(max_len.saturating_sub(3)).collect();
            s.push_str("...");
            s
        } else {
            first_line.to_owned()
        };
        lines.push(Line::from(vec![
            Span::styled("    \u{25ae}\u{25ae} ", theme.muted_style()),
            Span::styled(truncated, theme.muted_style()),
        ]));
    }

    // Blank line after tool call for visual separation.
    lines.push(Line::from(""));

    lines
}

fn render_router_event<'a>(text: &str, theme: &'a UcodeTheme) -> Vec<Line<'a>> {
    // "  ↪ router: {text}"
    vec![
        Line::from(vec![
            Span::styled("  \u{21aa} router: ", theme.warning_style()),
            Span::styled(text.to_owned(), theme.warning_style()),
        ]),
        Line::from(""),
    ]
}

fn render_system_message<'a>(text: &str, theme: &'a UcodeTheme) -> Vec<Line<'a>> {
    // "  ─ {text}"
    vec![
        Line::from(vec![
            Span::styled("  \u{2500} ", theme.muted_style()),
            Span::styled(text.to_owned(), theme.muted_style()),
        ]),
        Line::from(""),
    ]
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Wrap `text` to fit within `width` terminal columns using Unicode-aware
/// width measurement. Words that are wider than `width` are placed on their
/// own line without splitting.
fn wrap_text(text: &str, width: u16) -> Vec<String> {
    if width == 0 || text.is_empty() {
        return Vec::new();
    }
    let max = width as usize;
    let mut lines: Vec<String> = Vec::new();

    // Split on explicit newlines first to preserve line breaks,
    // then word-wrap each paragraph independently.
    for paragraph in text.split('\n') {
        let mut current = String::new();
        let mut current_width: usize = 0;

        for word in paragraph.split_whitespace() {
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

        // Always push the line (even if empty) to preserve blank lines.
        lines.push(current);
    }

    lines
}

// ---------------------------------------------------------------------------
// highlight_line_range — column-aware span highlighting
// ---------------------------------------------------------------------------

/// Apply `hl` style to the column range `[start_col, end_col)` within `line`.
///
/// Walks spans tracking cumulative column position (Unicode-width-aware),
/// splits spans at the selection boundaries, and returns a new `Line` with
/// the selected segment styled.
pub fn highlight_line_range(
    line: &Line<'_>,
    start_col: usize,
    end_col: usize,
    hl: ratatui::style::Style,
) -> Line<'static> {
    use unicode_width::UnicodeWidthChar;

    if start_col >= end_col {
        // Nothing to highlight — clone as-is.
        let spans: Vec<ratatui::text::Span<'static>> = line
            .spans
            .iter()
            .map(|s| ratatui::text::Span::styled(s.content.to_string(), s.style))
            .collect();
        return Line::from(spans);
    }

    let mut result: Vec<ratatui::text::Span<'static>> = Vec::new();
    let mut col = 0usize;

    for span in &line.spans {
        let text = span.content.as_ref();
        let base_style = span.style;

        // Fast path: span is entirely before or after the selection.
        let span_width: usize = text
            .chars()
            .map(|c| UnicodeWidthChar::width(c).unwrap_or(0))
            .sum();
        let span_end = col + span_width;

        if span_end <= start_col || col >= end_col {
            // Entirely outside selection.
            result.push(ratatui::text::Span::styled(text.to_owned(), base_style));
            col = span_end;
            continue;
        }

        // The span overlaps the selection — split into up to three segments.
        let mut pre = String::new();
        let mut selected = String::new();
        let mut post = String::new();
        let mut c = col;

        for ch in text.chars() {
            let w = UnicodeWidthChar::width(ch).unwrap_or(0);
            if c < start_col {
                pre.push(ch);
            } else if c < end_col {
                selected.push(ch);
            } else {
                post.push(ch);
            }
            c += w;
        }

        if !pre.is_empty() {
            result.push(ratatui::text::Span::styled(pre, base_style));
        }
        if !selected.is_empty() {
            result.push(ratatui::text::Span::styled(selected, base_style.patch(hl)));
        }
        if !post.is_empty() {
            result.push(ratatui::text::Span::styled(post, base_style));
        }

        col = span_end;
    }

    Line::from(result)
}

// ---------------------------------------------------------------------------
// entry_height — virtual scrolling helper
// ---------------------------------------------------------------------------

/// Compute how many terminal rows `entry` occupies when rendered at `width`.
pub fn entry_height(entry: &TranscriptEntry, width: u16) -> usize {
    match entry {
        TranscriptEntry::UserMessage(text) => {
            let prefix_w = "│ ".width() as u16;
            let text_w = width.saturating_sub(prefix_w);
            wrap_text(text, text_w).len().max(1) + 1 // +1 for blank separator
        }
        TranscriptEntry::AssistantMessage(text) => {
            let body_w = width.saturating_sub(2);
            super::markdown::markdown_height(text, body_w).max(1) + 1 // +1 for blank separator
        }
        TranscriptEntry::Streaming(msg) => {
            let body_w = width.saturating_sub(2);
            super::markdown::markdown_height(&msg.content, body_w).max(1)
        }
        TranscriptEntry::ToolCall {
            thinking, output, ..
        } => {
            1 + thinking
                .as_ref()
                .map_or(0, |s| if s.is_empty() { 0 } else { 1 })
                + output
                    .as_ref()
                    .map_or(0, |s| if s.is_empty() { 0 } else { 1 })
                + 1 // blank separator
        }
        TranscriptEntry::RouterEvent(_) => 2, // content + blank separator
        TranscriptEntry::SystemMessage(_) => 2, // content + blank separator
        TranscriptEntry::PatchProposed { .. } => 2, // content + blank separator
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
            text.contains('│'),
            "expected left border '│', got: {text:?}"
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
            !text.contains("Assistant:"),
            "should not contain 'Assistant:' label, got: {text:?}"
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
            "expected cursor char when show_cursor=true, got: {text:?}"
        );
        assert!(
            !text.contains("Assistant:"),
            "should not contain 'Assistant:' label"
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
        assert!(
            !text.contains("Assistant:"),
            "should not contain 'Assistant:' label"
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
            None,
            None,
            &t,
            80,
        );
        let text = lines_text(&lines);
        assert!(text.contains('\u{2192}'), "expected → arrow, got: {text:?}");
        assert!(
            text.contains("read_file"),
            "expected tool name, got: {text:?}"
        );
        assert!(
            text.contains('\u{2713}'),
            "expected ✓ icon for Success, got: {text:?}"
        );
        assert!(text.contains("42ms"), "expected duration, got: {text:?}");
    }

    // -----------------------------------------------------------------------
    // render_tool_call_pending
    // -----------------------------------------------------------------------

    #[test]
    fn render_tool_call_pending() {
        let t = theme();
        let lines = render_tool_call(
            "run_cmd",
            &ToolCallStatus::PendingApproval,
            None,
            None,
            None,
            None,
            &t,
            80,
        );
        let text = lines_text(&lines);
        assert!(text.contains('\u{2192}'), "expected → arrow, got: {text:?}");
        assert!(
            text.contains('\u{26a0}'),
            "expected ⚠ icon for PendingApproval, got: {text:?}"
        );
        assert!(
            text.contains("pending approval"),
            "expected 'pending approval' label, got: {text:?}"
        );
    }

    // -----------------------------------------------------------------------
    // render_tool_call_running
    // -----------------------------------------------------------------------

    #[test]
    fn render_tool_call_running() {
        let t = theme();
        let lines = render_tool_call(
            "search",
            &ToolCallStatus::Running,
            None,
            None,
            None,
            None,
            &t,
            80,
        );
        let text = lines_text(&lines);
        assert!(
            text.contains('\u{27f3}'),
            "expected ⟳ icon for Running, got: {text:?}"
        );
        assert_eq!(lines.len(), 2, "tool call should be 1 content + 1 blank");
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
        // Short user message: 1 text line + 1 blank separator = 2
        assert_eq!(
            h, 2,
            "short user message should be 2 lines (text + separator)"
        );

        let entry2 = TranscriptEntry::RouterEvent("event".to_owned());
        assert_eq!(entry_height(&entry2, 80), 2);

        let entry3 = TranscriptEntry::SystemMessage("sys".to_owned());
        assert_eq!(entry_height(&entry3, 80), 2);

        let entry4 = TranscriptEntry::ToolCall {
            name: "read".to_owned(),
            status: ToolCallStatus::Success,
            duration_ms: None,
            summary: None,
            thinking: None,
            output: None,
        };
        assert_eq!(
            entry_height(&entry4, 80),
            2,
            "tool call should be 1 content + 1 blank"
        );
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
                thinking: None,
                output: None,
            },
            TranscriptEntry::RouterEvent("rerouted".to_owned()),
            TranscriptEntry::SystemMessage("info".to_owned()),
        ];

        let copy_mode = crate::overlays::copy_mode::CopyModeState::new();
        let widget = TranscriptView::new(&entries, 0, true, &t, true, &copy_mode);
        let area = Rect::new(0, 0, 80, 30);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);

        // Collect all rendered text from the buffer.
        let rendered: String = (0..30u16)
            .flat_map(|y| (0..80u16).map(move |x| (x, y)))
            .map(|(x, y)| buf[(x, y)].symbol().to_owned())
            .collect();

        assert!(rendered.contains("hello"), "user message text missing");
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
        let copy_mode = crate::overlays::copy_mode::CopyModeState::new();
        let widget = TranscriptView::new(&entries, 0, false, &t, false, &copy_mode);
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

        let copy_mode = crate::overlays::copy_mode::CopyModeState::new();
        let widget = TranscriptView::new(&entries, 0, true, &t, false, &copy_mode);
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

    // -----------------------------------------------------------------------
    // render_tool_call with thinking/output blocks
    // -----------------------------------------------------------------------

    #[test]
    fn render_tool_call_with_thinking() {
        let t = theme();
        let lines = render_tool_call(
            "sequential_thinking",
            &ToolCallStatus::Success,
            Some(100),
            Some("analyzing problem"),
            Some("Let me analyze the code structure..."),
            None,
            &t,
            80,
        );
        let text = lines_text(&lines);
        assert!(
            text.contains('\u{2299}'),
            "expected ⊙ thinking icon, got: {text:?}"
        );
        assert!(
            text.contains("Let me analyze"),
            "expected thinking content, got: {text:?}"
        );
        assert_eq!(
            lines.len(),
            3,
            "tool call with thinking should be 2 content + 1 blank"
        );
    }

    #[test]
    fn render_tool_call_with_output() {
        let t = theme();
        let lines = render_tool_call(
            "read_file",
            &ToolCallStatus::Success,
            Some(50),
            Some("path=README.md"),
            None,
            Some("# Project Title\nSome content here"),
            &t,
            80,
        );
        let text = lines_text(&lines);
        assert!(
            text.contains('\u{25ae}'),
            "expected ▮ output icon, got: {text:?}"
        );
        assert!(
            text.contains("# Project Title"),
            "expected output first line, got: {text:?}"
        );
        assert_eq!(
            lines.len(),
            3,
            "tool call with output should be 2 content + 1 blank"
        );
    }

    #[test]
    fn render_tool_call_with_thinking_and_output() {
        let t = theme();
        let lines = render_tool_call(
            "search",
            &ToolCallStatus::Success,
            Some(200),
            None,
            Some("I need to find the config file"),
            Some("Found 3 matches in src/"),
            &t,
            80,
        );
        assert_eq!(
            lines.len(),
            4,
            "tool call with both thinking and output should be 3 content + 1 blank"
        );
    }

    #[test]
    fn render_tool_call_without_thinking_or_output() {
        let t = theme();
        let lines = render_tool_call(
            "read_file",
            &ToolCallStatus::Success,
            Some(42),
            Some("path=foo.rs"),
            None,
            None,
            &t,
            80,
        );
        assert_eq!(
            lines.len(),
            2,
            "tool call without thinking/output should be 1 content + 1 blank"
        );
    }

    // -----------------------------------------------------------------------
    // entry_height with thinking/output
    // -----------------------------------------------------------------------

    #[test]
    fn entry_height_tool_call_with_thinking_and_output() {
        let entry = TranscriptEntry::ToolCall {
            name: "search".to_owned(),
            status: ToolCallStatus::Success,
            duration_ms: Some(100),
            summary: None,
            thinking: Some("analyzing...".to_owned()),
            output: Some("found 3 results".to_owned()),
        };
        assert_eq!(
            entry_height(&entry, 80),
            4,
            "1 main + 1 thinking + 1 output + 1 blank"
        );
    }

    // -----------------------------------------------------------------------
    // only_last_streaming_entry_shows_cursor
    // -----------------------------------------------------------------------

    #[test]
    fn cursor_only_when_last_entry_is_streaming() {
        let t = theme();
        const CURSOR: &str = "\u{258c}";

        // Last entry IS Streaming → cursor should appear exactly once.
        let mut msg = StreamingMessage::new();
        msg.push_token("hello");

        let entries = vec![
            TranscriptEntry::UserMessage("hi".to_owned()),
            TranscriptEntry::Streaming(msg),
        ];

        let copy_mode = crate::overlays::copy_mode::CopyModeState::new();
        let widget = TranscriptView::new(&entries, 0, true, &t, true, &copy_mode);
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);

        let rendered: String = (0..40u16)
            .flat_map(|y| (0..80u16).map(move |x| (x, y)))
            .map(|(x, y)| buf[(x, y)].symbol().to_owned())
            .collect();

        let cursor_count = rendered.matches(CURSOR).count();
        assert_eq!(
            cursor_count, 1,
            "expected 1 cursor when last entry is Streaming, got {cursor_count}"
        );
    }

    #[test]
    fn no_cursor_when_entries_follow_streaming() {
        let t = theme();
        const CURSOR: &str = "\u{258c}";

        // Streaming entry followed by tool calls — no cursor should appear.
        // This is the pattern when start_streaming() runs, then tool calls
        // arrive before any stream tokens.
        let msg = StreamingMessage::new(); // empty

        let entries = vec![
            TranscriptEntry::Streaming(msg),
            TranscriptEntry::ToolCall {
                name: "Read".to_owned(),
                status: ToolCallStatus::Success,
                duration_ms: Some(100),
                summary: None,
                thinking: None,
                output: None,
            },
        ];

        let copy_mode = crate::overlays::copy_mode::CopyModeState::new();
        let widget = TranscriptView::new(&entries, 0, true, &t, true, &copy_mode);
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);

        let rendered: String = (0..40u16)
            .flat_map(|y| (0..80u16).map(move |x| (x, y)))
            .map(|(x, y)| buf[(x, y)].symbol().to_owned())
            .collect();

        let cursor_count = rendered.matches(CURSOR).count();
        assert_eq!(
            cursor_count, 0,
            "expected 0 cursors when entries follow Streaming, got {cursor_count}"
        );
    }
}
