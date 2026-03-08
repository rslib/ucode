use crate::app::TranscriptEntry;

// ---------------------------------------------------------------------------
// Position / VisualMode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    pub line: usize,
    pub col: usize,
}

impl Position {
    pub fn new(line: usize, col: usize) -> Self {
        Self { line, col }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualMode {
    /// `v` — character-level selection (partial lines).
    Char,
    /// `V` — line-level selection (full lines, original behaviour).
    Line,
    /// `Ctrl+V` — block/rectangle selection.
    Block,
}

impl VisualMode {
    /// Status-bar label for this mode.
    pub fn label(self) -> &'static str {
        match self {
            VisualMode::Char => "VISUAL",
            VisualMode::Line => "V-LINE",
            VisualMode::Block => "V-BLOCK",
        }
    }
}

// ---------------------------------------------------------------------------
// CopyModeState
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct CopyModeState {
    /// True when copy mode is active (phase 1 or 2).
    pub active: bool,
    /// True when visual selection is active (phase 2).
    pub selecting: bool,
    /// Visual mode (only meaningful when selecting=true).
    pub mode: VisualMode,
    /// Anchor position (start of selection). Only meaningful when selecting=true.
    pub anchor: Position,
    /// Cursor position (current position).
    pub cursor: Position,
    /// Total rendered lines in the transcript. Updated each render frame.
    pub total_lines: usize,
}

impl CopyModeState {
    pub fn new() -> Self {
        Self {
            active: false,
            selecting: false,
            mode: VisualMode::Line,
            anchor: Position::new(0, 0),
            cursor: Position::new(0, 0),
            total_lines: 0,
        }
    }

    /// Enter copy mode at the given visual line and column.
    pub fn enter(&mut self, line: usize, col: usize) {
        self.active = true;
        self.selecting = false;
        self.cursor = Position::new(line, col);
        self.anchor = Position::new(0, 0);
    }

    /// Begin visual selection with the given mode; anchor at current cursor.
    pub fn start_selecting_with_mode(&mut self, mode: VisualMode) {
        self.selecting = true;
        self.mode = mode;
        self.anchor = self.cursor;
    }

    /// Exit copy mode entirely.
    pub fn exit(&mut self) {
        self.active = false;
        self.selecting = false;
    }

    /// Exit visual selection but stay in phase 1 (active remains true).
    pub fn exit_selecting(&mut self) {
        self.selecting = false;
    }

    /// Move cursor up (toward line 0), preserving column.
    pub fn move_up(&mut self) {
        if self.cursor.line > 0 {
            self.cursor.line -= 1;
        }
    }

    /// Move cursor down (toward end of transcript), preserving column.
    pub fn move_down(&mut self) {
        let max = self.total_lines.saturating_sub(1);
        if self.cursor.line < max {
            self.cursor.line += 1;
        }
    }

    /// Move cursor left (toward col 0).
    pub fn move_left(&mut self) {
        if self.cursor.col > 0 {
            self.cursor.col -= 1;
        }
    }

    /// Move cursor right (no upper bound — renderer handles clamping).
    pub fn move_right(&mut self) {
        self.cursor.col += 1;
    }

    /// Move cursor to column 0.
    pub fn move_to_line_start(&mut self) {
        self.cursor.col = 0;
    }

    /// Move cursor to end of line. `line_width` is the display width of the
    /// current line. Cursor lands on the last character (line_width - 1),
    /// or stays at 0 if the line is empty.
    pub fn move_to_line_end(&mut self, line_width: usize) {
        self.cursor.col = line_width.saturating_sub(1);
    }

    /// Move cursor to first line (line 0), preserving column.
    pub fn move_to_first_line(&mut self) {
        self.cursor.line = 0;
    }

    /// Move cursor to last line, preserving column.
    pub fn move_to_last_line(&mut self) {
        self.cursor.line = self.total_lines.saturating_sub(1);
    }

    /// Move cursor to a specific column.
    pub fn move_to_col(&mut self, col: usize) {
        self.cursor.col = col;
    }

    /// Swap anchor and cursor (extend selection in the other direction).
    pub fn swap_anchor(&mut self) {
        std::mem::swap(&mut self.anchor, &mut self.cursor);
    }

    /// Returns `Some((min_line, max_line))` when visual selection is active.
    pub fn selection_line_range(&self) -> Option<(usize, usize)> {
        if !self.selecting {
            return None;
        }
        let start = self.anchor.line.min(self.cursor.line);
        let end = self.anchor.line.max(self.cursor.line);
        Some((start, end))
    }

    /// For a given visual line index, return the selected column range
    /// `Some((start_col, end_col))` based on the current visual mode.
    ///
    /// `end_col` is **exclusive** (consistent with `highlight_line_range` and
    /// `line_text_substr`). The character under the cursor is always included
    /// in the selection (vim semantics), so the returned `end_col` is one past
    /// the rightmost selected column.
    ///
    /// `line_width` is the actual rendered width of the line (sum of span
    /// widths), NOT the terminal width.
    pub fn line_col_range(&self, line_idx: usize, line_width: usize) -> Option<(usize, usize)> {
        let (min_line, max_line) = self.selection_line_range()?;
        if line_idx < min_line || line_idx > max_line {
            return None;
        }

        match self.mode {
            VisualMode::Line => Some((0, line_width)),

            VisualMode::Block => {
                let min_col = self.anchor.col.min(self.cursor.col);
                let max_col = self.anchor.col.max(self.cursor.col);
                // +1: end_col is exclusive; include the character at max_col.
                Some((min_col, max_col + 1))
            }

            VisualMode::Char => {
                if min_line == max_line {
                    // Single line: select between the two columns (inclusive).
                    let min_col = self.anchor.col.min(self.cursor.col);
                    let max_col = self.anchor.col.max(self.cursor.col);
                    Some((min_col, max_col + 1))
                } else {
                    // Determine which end is "first" (anchor vs cursor order).
                    let (first, last) = if self.anchor.line <= self.cursor.line {
                        (self.anchor, self.cursor)
                    } else {
                        (self.cursor, self.anchor)
                    };

                    if line_idx == first.line {
                        Some((first.col, line_width))
                    } else if line_idx == last.line {
                        // +1: include the character at last.col.
                        Some((0, last.col + 1))
                    } else {
                        // Middle lines: full width.
                        Some((0, line_width))
                    }
                }
            }
        }
    }

    /// True if `line` is the current cursor line.
    pub fn is_cursor_line(&self, line: usize) -> bool {
        line == self.cursor.line
    }

    /// True if `line` falls within the active visual selection.
    pub fn is_line_in_selection(&self, line: usize) -> bool {
        match self.selection_line_range() {
            Some((start, end)) => line >= start && line <= end,
            None => false,
        }
    }

    /// Status-bar label: `None` when inactive, `"COPY"` in phase 1,
    /// mode label (`"VISUAL"` / `"V-LINE"` / `"V-BLOCK"`) in phase 2.
    pub fn status_label(&self) -> Option<&'static str> {
        if !self.active {
            return None;
        }
        if self.selecting {
            Some(self.mode.label())
        } else {
            Some("COPY")
        }
    }

    // -----------------------------------------------------------------------
    // Backward-compat aliases
    // -----------------------------------------------------------------------

    /// Alias kept for call-sites that haven't been migrated yet.
    #[inline]
    pub fn is_line_selected(&self, line: usize) -> bool {
        self.is_line_in_selection(line)
    }

    /// Alias kept for call-sites that haven't been migrated yet.
    #[inline]
    pub fn selection_range(&self) -> Option<(usize, usize)> {
        self.selection_line_range()
    }
}

impl Default for CopyModeState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Text extraction helpers
// ---------------------------------------------------------------------------

/// Extract plain text from a single rendered `Line` (concatenate span contents).
pub fn line_to_text(line: &ratatui::text::Line<'_>) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

/// Extract a substring of a `Line` by column range [start_col, end_col).
///
/// Uses Unicode-aware column widths so multi-byte / wide characters are
/// handled correctly.
pub fn line_text_substr(
    line: &ratatui::text::Line<'_>,
    start_col: usize,
    end_col: usize,
) -> String {
    use unicode_width::UnicodeWidthChar;

    if start_col >= end_col {
        return String::new();
    }

    let full = line_to_text(line);
    let mut result = String::new();
    let mut col = 0usize;

    for ch in full.chars() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if col >= end_col {
            break;
        }
        if col >= start_col {
            result.push(ch);
        }
        col += w;
    }

    result
}

/// Collect text from a range of visual lines, joined by newlines.
pub fn collect_lines_text(lines: &[ratatui::text::Line<'_>], start: usize, end: usize) -> String {
    lines[start..=end.min(lines.len().saturating_sub(1))]
        .iter()
        .map(|l| line_to_text(l))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Word character predicate — matches alphanumeric and underscore.
/// Shared with input.rs for consistency.
pub fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Find the display-column of the next word start from `col` in `text`.
///
/// Skips non-word chars, then skips word chars. Returns the column where
/// the next word begins, or the end of the string.
pub fn next_word_start(text: &str, col: usize) -> usize {
    use unicode_width::UnicodeWidthChar;
    let mut cur = 0usize;
    let mut chars = text.chars().peekable();

    // Advance to `col`.
    while cur < col {
        if let Some(c) = chars.next() {
            cur += UnicodeWidthChar::width(c).unwrap_or(0);
        } else {
            return cur;
        }
    }

    // If on a word char, skip the rest of the current word first.
    let on_word = chars.peek().is_some_and(|c| is_word_char(*c));
    if on_word {
        while let Some(&c) = chars.peek() {
            if !is_word_char(c) {
                break;
            }
            cur += UnicodeWidthChar::width(c).unwrap_or(0);
            chars.next();
        }
    }

    // Skip non-word chars to reach the next word.
    while let Some(&c) = chars.peek() {
        if is_word_char(c) {
            break;
        }
        cur += UnicodeWidthChar::width(c).unwrap_or(0);
        chars.next();
    }

    cur
}

/// Find the display-column of the previous word start from `col` in `text`.
///
/// Moves backward: skips non-word chars, then skips word chars.
/// Returns the column where the previous word begins, or 0.
pub fn prev_word_start(text: &str, col: usize) -> usize {
    use unicode_width::UnicodeWidthChar;

    // Build (col_offset, char) pairs up to `col`.
    let mut pairs: Vec<(usize, char)> = Vec::new();
    let mut cur = 0usize;
    for c in text.chars() {
        if cur >= col {
            break;
        }
        pairs.push((cur, c));
        cur += UnicodeWidthChar::width(c).unwrap_or(0);
    }

    if pairs.is_empty() {
        return 0;
    }

    let mut idx = pairs.len();

    // Skip trailing non-word chars.
    while idx > 0 && !is_word_char(pairs[idx - 1].1) {
        idx -= 1;
    }
    // Skip the word itself.
    while idx > 0 && is_word_char(pairs[idx - 1].1) {
        idx -= 1;
    }

    if idx < pairs.len() { pairs[idx].0 } else { 0 }
}

/// Find the display-column of the next word end from `col` in `text`.
///
/// Skips non-word chars, then advances through word chars.
/// Returns the column of the last character of the next word.
pub fn next_word_end(text: &str, col: usize) -> usize {
    use unicode_width::UnicodeWidthChar;
    let mut cur = 0usize;
    let mut chars = text.chars().peekable();

    // Advance past `col` (move at least one position).
    while cur <= col {
        if let Some(c) = chars.next() {
            cur += UnicodeWidthChar::width(c).unwrap_or(0);
        } else {
            return col;
        }
    }

    // Skip non-word chars.
    while let Some(&c) = chars.peek() {
        if is_word_char(c) {
            break;
        }
        cur += UnicodeWidthChar::width(c).unwrap_or(0);
        chars.next();
    }

    // Advance through word chars; track the last word-char column.
    let mut last_word_col = cur;
    while let Some(&c) = chars.peek() {
        if !is_word_char(c) {
            break;
        }
        last_word_col = cur;
        cur += UnicodeWidthChar::width(c).unwrap_or(0);
        chars.next();
    }

    last_word_col
}

/// Collect selected text from visual lines according to the current
/// `CopyModeState` (handles all three visual modes).
pub fn collect_selected_text(lines: &[ratatui::text::Line<'_>], state: &CopyModeState) -> String {
    use unicode_width::UnicodeWidthStr;

    let (min_line, max_line) = match state.selection_line_range() {
        Some(r) => r,
        None => return String::new(),
    };

    let end = max_line.min(lines.len().saturating_sub(1));
    let mut parts: Vec<String> = Vec::new();

    for (line_idx, line) in lines.iter().enumerate().take(end + 1).skip(min_line) {
        let line_width: usize = line.spans.iter().map(|s| s.content.width()).sum();
        if let Some((sc, ec)) = state.line_col_range(line_idx, line_width) {
            parts.push(line_text_substr(line, sc, ec));
        }
    }

    parts.join("\n")
}

// ---------------------------------------------------------------------------
// Entry-level text extraction helpers (kept for backward compatibility)
// ---------------------------------------------------------------------------

/// Extract the displayable text from a transcript entry for clipboard copy.
pub fn entry_to_copy_text(entry: &TranscriptEntry) -> String {
    match entry {
        TranscriptEntry::UserMessage(s) => format!("User: {s}"),
        TranscriptEntry::AssistantMessage(s) => format!("Assistant: {s}"),
        TranscriptEntry::Streaming(msg) => format!("Assistant: {}", msg.content),
        TranscriptEntry::ToolCall {
            name,
            summary,
            output,
            ..
        } => {
            let mut text = format!("Tool: {name}");
            if let Some(s) = summary {
                text.push_str(&format!("\n  Summary: {s}"));
            }
            if let Some(o) = output {
                text.push_str(&format!("\n  Output: {o}"));
            }
            text
        }
        TranscriptEntry::RouterEvent(s) => format!("Router: {s}"),
        TranscriptEntry::SystemMessage(s) => format!("System: {s}"),
        TranscriptEntry::PatchProposed {
            file_path,
            raw_diff,
            ..
        } => {
            format!("Patch: {file_path}\n{raw_diff}")
        }
    }
}

/// Collect text from selected transcript entries into a single string.
pub fn collect_selection_text(transcript: &[TranscriptEntry], start: usize, end: usize) -> String {
    transcript[start..=end.min(transcript.len().saturating_sub(1))]
        .iter()
        .map(entry_to_copy_text)
        .collect::<Vec<_>>()
        .join("\n\n")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{StreamingMessage, ToolCallStatus, TranscriptEntry};
    use ratatui::text::{Line, Span};

    // --- CopyModeState ---

    #[test]
    fn new_defaults() {
        let state = CopyModeState::new();
        assert!(!state.active);
        assert!(!state.selecting);
        assert_eq!(state.anchor, Position::new(0, 0));
        assert_eq!(state.cursor, Position::new(0, 0));
        assert_eq!(state.total_lines, 0);
    }

    #[test]
    fn enter_sets_active_not_selecting() {
        let mut state = CopyModeState::new();
        state.enter(5, 3);
        assert!(state.active);
        assert!(!state.selecting);
        assert_eq!(state.cursor, Position::new(5, 3));
        assert_eq!(state.anchor, Position::new(0, 0));
    }

    #[test]
    fn start_selecting_with_mode_sets_anchor_at_cursor() {
        let mut state = CopyModeState::new();
        state.enter(7, 2);
        state.start_selecting_with_mode(VisualMode::Char);
        assert!(state.selecting);
        assert_eq!(state.mode, VisualMode::Char);
        assert_eq!(state.anchor, Position::new(7, 2));
        assert_eq!(state.cursor, Position::new(7, 2));
    }

    #[test]
    fn exit_selecting_keeps_active() {
        let mut state = CopyModeState::new();
        state.enter(3, 0);
        state.start_selecting_with_mode(VisualMode::Line);
        state.exit_selecting();
        assert!(state.active);
        assert!(!state.selecting);
    }

    #[test]
    fn exit_clears_both_flags() {
        let mut state = CopyModeState::new();
        state.enter(3, 0);
        state.start_selecting_with_mode(VisualMode::Line);
        state.exit();
        assert!(!state.active);
        assert!(!state.selecting);
    }

    #[test]
    fn swap_anchor_swaps_positions() {
        let mut state = CopyModeState::new();
        state.enter(2, 0);
        state.start_selecting_with_mode(VisualMode::Char);
        state.cursor = Position::new(8, 5);
        state.swap_anchor();
        assert_eq!(state.anchor, Position::new(8, 5));
        assert_eq!(state.cursor, Position::new(2, 0));
    }

    #[test]
    fn selection_line_range_none_when_not_selecting() {
        let mut state = CopyModeState::new();
        state.enter(3, 0);
        assert_eq!(state.selection_line_range(), None);
    }

    #[test]
    fn selection_line_range_some_when_selecting() {
        let mut state = CopyModeState::new();
        state.enter(2, 0);
        state.start_selecting_with_mode(VisualMode::Line);
        state.cursor = Position::new(5, 0);
        assert_eq!(state.selection_line_range(), Some((2, 5)));
    }

    #[test]
    fn selection_line_range_normalises_reversed() {
        let mut state = CopyModeState::new();
        state.enter(8, 0);
        state.start_selecting_with_mode(VisualMode::Line);
        state.cursor = Position::new(3, 0);
        assert_eq!(state.selection_line_range(), Some((3, 8)));
    }

    #[test]
    fn is_line_in_selection_false_when_not_selecting() {
        let mut state = CopyModeState::new();
        state.enter(3, 0);
        assert!(!state.is_line_in_selection(3));
    }

    #[test]
    fn is_line_in_selection_within_range() {
        let mut state = CopyModeState::new();
        state.enter(2, 0);
        state.start_selecting_with_mode(VisualMode::Line);
        state.cursor = Position::new(5, 0);
        assert!(!state.is_line_in_selection(1));
        assert!(state.is_line_in_selection(2));
        assert!(state.is_line_in_selection(3));
        assert!(state.is_line_in_selection(5));
        assert!(!state.is_line_in_selection(6));
    }

    #[test]
    fn is_cursor_line() {
        let mut state = CopyModeState::new();
        state.enter(4, 0);
        assert!(state.is_cursor_line(4));
        assert!(!state.is_cursor_line(3));
        assert!(!state.is_cursor_line(5));
    }

    #[test]
    fn move_up_decrements_cursor_line() {
        let mut state = CopyModeState::new();
        state.enter(5, 3);
        state.total_lines = 10;
        state.move_up();
        assert_eq!(state.cursor.line, 4);
        assert_eq!(state.cursor.col, 3); // col preserved
    }

    #[test]
    fn move_up_stops_at_zero() {
        let mut state = CopyModeState::new();
        state.enter(0, 0);
        state.total_lines = 10;
        state.move_up();
        assert_eq!(state.cursor.line, 0);
    }

    #[test]
    fn move_down_increments_cursor_line() {
        let mut state = CopyModeState::new();
        state.enter(3, 2);
        state.total_lines = 10;
        state.move_down();
        assert_eq!(state.cursor.line, 4);
        assert_eq!(state.cursor.col, 2); // col preserved
    }

    #[test]
    fn move_down_stops_at_total_lines_minus_one() {
        let mut state = CopyModeState::new();
        state.total_lines = 5;
        state.enter(4, 0);
        state.move_down();
        assert_eq!(state.cursor.line, 4);
    }

    #[test]
    fn move_down_zero_total_lines_stays_at_zero() {
        let mut state = CopyModeState::new();
        state.total_lines = 0;
        state.enter(0, 0);
        state.move_down();
        assert_eq!(state.cursor.line, 0);
    }

    #[test]
    fn move_left_decrements_col() {
        let mut state = CopyModeState::new();
        state.enter(0, 5);
        state.move_left();
        assert_eq!(state.cursor.col, 4);
    }

    #[test]
    fn move_left_stops_at_zero() {
        let mut state = CopyModeState::new();
        state.enter(0, 0);
        state.move_left();
        assert_eq!(state.cursor.col, 0);
    }

    #[test]
    fn move_right_increments_col() {
        let mut state = CopyModeState::new();
        state.enter(0, 3);
        state.move_right();
        assert_eq!(state.cursor.col, 4);
    }

    #[test]
    fn move_to_line_start_resets_col() {
        let mut state = CopyModeState::new();
        state.enter(5, 10);
        state.move_to_line_start();
        assert_eq!(state.cursor.col, 0);
        assert_eq!(state.cursor.line, 5); // line unchanged
    }

    #[test]
    fn move_to_line_end_sets_last_col() {
        let mut state = CopyModeState::new();
        state.enter(3, 0);
        state.move_to_line_end(40);
        assert_eq!(state.cursor.col, 39);
    }

    #[test]
    fn move_to_line_end_empty_line() {
        let mut state = CopyModeState::new();
        state.enter(3, 5);
        state.move_to_line_end(0);
        assert_eq!(state.cursor.col, 0);
    }

    #[test]
    fn move_to_first_line_preserves_col() {
        let mut state = CopyModeState::new();
        state.total_lines = 100;
        state.enter(50, 7);
        state.move_to_first_line();
        assert_eq!(state.cursor.line, 0);
        assert_eq!(state.cursor.col, 7);
    }

    #[test]
    fn move_to_last_line_preserves_col() {
        let mut state = CopyModeState::new();
        state.total_lines = 100;
        state.enter(10, 3);
        state.move_to_last_line();
        assert_eq!(state.cursor.line, 99);
        assert_eq!(state.cursor.col, 3);
    }

    #[test]
    fn move_to_col_sets_exact_col() {
        let mut state = CopyModeState::new();
        state.enter(0, 0);
        state.move_to_col(15);
        assert_eq!(state.cursor.col, 15);
    }

    // --- line_col_range: Line mode ---

    #[test]
    fn line_col_range_line_mode_full_width() {
        let mut state = CopyModeState::new();
        state.enter(2, 0);
        state.start_selecting_with_mode(VisualMode::Line);
        state.cursor = Position::new(4, 0);
        // Every line in [2,4] gets (0, line_width).
        assert_eq!(state.line_col_range(2, 80), Some((0, 80)));
        assert_eq!(state.line_col_range(3, 40), Some((0, 40)));
        assert_eq!(state.line_col_range(4, 60), Some((0, 60)));
        // Outside range → None.
        assert_eq!(state.line_col_range(1, 80), None);
        assert_eq!(state.line_col_range(5, 80), None);
    }

    // --- line_col_range: Block mode ---

    #[test]
    fn line_col_range_block_mode() {
        let mut state = CopyModeState::new();
        state.enter(1, 5);
        state.start_selecting_with_mode(VisualMode::Block);
        state.cursor = Position::new(3, 10);
        // All lines in [1,3] get (5, 11) — end_col is exclusive, includes col 10.
        assert_eq!(state.line_col_range(1, 80), Some((5, 11)));
        assert_eq!(state.line_col_range(2, 80), Some((5, 11)));
        assert_eq!(state.line_col_range(3, 80), Some((5, 11)));
        assert_eq!(state.line_col_range(0, 80), None);
    }

    #[test]
    fn line_col_range_block_mode_reversed_cols() {
        let mut state = CopyModeState::new();
        state.enter(1, 10);
        state.start_selecting_with_mode(VisualMode::Block);
        state.cursor = Position::new(3, 5);
        // min/max normalised: (5, 11) — includes col 10.
        assert_eq!(state.line_col_range(2, 80), Some((5, 11)));
    }

    // --- line_col_range: Char mode ---

    #[test]
    fn line_col_range_char_mode_single_line() {
        let mut state = CopyModeState::new();
        state.enter(3, 4);
        state.start_selecting_with_mode(VisualMode::Char);
        state.cursor = Position::new(3, 9);
        // end_col exclusive: includes col 9 → (4, 10).
        assert_eq!(state.line_col_range(3, 80), Some((4, 10)));
        assert_eq!(state.line_col_range(2, 80), None);
    }

    #[test]
    fn line_col_range_char_mode_single_line_reversed() {
        let mut state = CopyModeState::new();
        state.enter(3, 9);
        state.start_selecting_with_mode(VisualMode::Char);
        state.cursor = Position::new(3, 4);
        // Normalised: includes col 9 → (4, 10).
        assert_eq!(state.line_col_range(3, 80), Some((4, 10)));
    }

    #[test]
    fn line_col_range_char_mode_multi_line() {
        let mut state = CopyModeState::new();
        state.enter(2, 5);
        state.start_selecting_with_mode(VisualMode::Char);
        state.cursor = Position::new(4, 8);
        // First line: (anchor.col, line_width).
        assert_eq!(state.line_col_range(2, 80), Some((5, 80)));
        // Middle line: (0, line_width).
        assert_eq!(state.line_col_range(3, 60), Some((0, 60)));
        // Last line: (0, cursor.col+1) — includes col 8.
        assert_eq!(state.line_col_range(4, 80), Some((0, 9)));
    }

    #[test]
    fn line_col_range_char_mode_multi_line_reversed() {
        // anchor is below cursor — same result after normalisation.
        let mut state = CopyModeState::new();
        state.enter(4, 8);
        state.start_selecting_with_mode(VisualMode::Char);
        state.cursor = Position::new(2, 5);
        // first (lower line) = cursor(2,5), last (higher line) = anchor(4,8)
        assert_eq!(state.line_col_range(2, 80), Some((5, 80)));
        assert_eq!(state.line_col_range(3, 60), Some((0, 60)));
        // Last line: includes col 8 → (0, 9).
        assert_eq!(state.line_col_range(4, 80), Some((0, 9)));
    }

    // --- line_to_text ---

    #[test]
    fn line_to_text_concatenates_spans() {
        let line = Line::from(vec![Span::raw("hello"), Span::raw(" "), Span::raw("world")]);
        assert_eq!(line_to_text(&line), "hello world");
    }

    #[test]
    fn line_to_text_empty_line() {
        let line = Line::from("");
        assert_eq!(line_to_text(&line), "");
    }

    // --- line_text_substr ---

    #[test]
    fn line_text_substr_basic() {
        let line = Line::from("hello world");
        assert_eq!(line_text_substr(&line, 6, 11), "world");
    }

    #[test]
    fn line_text_substr_full() {
        let line = Line::from("hello");
        assert_eq!(line_text_substr(&line, 0, 5), "hello");
    }

    #[test]
    fn line_text_substr_empty_range() {
        let line = Line::from("hello");
        assert_eq!(line_text_substr(&line, 3, 3), "");
    }

    #[test]
    fn line_text_substr_inverted_range() {
        let line = Line::from("hello");
        assert_eq!(line_text_substr(&line, 5, 3), "");
    }

    // --- is_word_char ---

    #[test]
    fn word_char_alpha() {
        assert!(is_word_char('a'));
        assert!(is_word_char('Z'));
        assert!(is_word_char('_'));
        assert!(is_word_char('5'));
    }

    #[test]
    fn word_char_non_word() {
        assert!(!is_word_char(' '));
        assert!(!is_word_char('.'));
        assert!(!is_word_char('-'));
        assert!(!is_word_char('('));
    }

    // --- next_word_start ---

    #[test]
    fn next_word_start_basic() {
        // "hello world"
        //  01234 56789A
        assert_eq!(next_word_start("hello world", 0), 6);
    }

    #[test]
    fn next_word_start_from_space() {
        assert_eq!(next_word_start("hello world", 5), 6);
    }

    #[test]
    fn next_word_start_at_end() {
        assert_eq!(next_word_start("hello", 3), 5);
    }

    #[test]
    fn next_word_start_multiple_spaces() {
        assert_eq!(next_word_start("foo   bar", 0), 6);
    }

    // --- prev_word_start ---

    #[test]
    fn prev_word_start_basic() {
        assert_eq!(prev_word_start("hello world", 8), 6);
    }

    #[test]
    fn prev_word_start_from_word_start() {
        assert_eq!(prev_word_start("hello world", 6), 0);
    }

    #[test]
    fn prev_word_start_at_zero() {
        assert_eq!(prev_word_start("hello", 0), 0);
    }

    #[test]
    fn prev_word_start_from_space() {
        assert_eq!(prev_word_start("hello world", 5), 0);
    }

    // --- next_word_end ---

    #[test]
    fn next_word_end_basic() {
        // "hello world" — from col 0, advance past 'h' then through "ello"; last word col = 4 ('o').
        assert_eq!(next_word_end("hello world", 0), 4);
    }

    #[test]
    fn next_word_end_from_space() {
        // From col 5 (space), advance past ' ' lands on 'w' (cur=6); advance through "world";
        // last word col = 10 ('d').
        assert_eq!(next_word_end("hello world", 5), 10);
    }

    // --- collect_lines_text ---

    #[test]
    fn collect_lines_text_basic() {
        let lines = vec![
            Line::from("first"),
            Line::from("second"),
            Line::from("third"),
        ];
        assert_eq!(collect_lines_text(&lines, 0, 2), "first\nsecond\nthird");
    }

    #[test]
    fn collect_lines_text_partial_range() {
        let lines = vec![Line::from("a"), Line::from("b"), Line::from("c")];
        assert_eq!(collect_lines_text(&lines, 1, 1), "b");
    }

    #[test]
    fn collect_lines_text_clamps_end() {
        let lines = vec![Line::from("only")];
        assert_eq!(collect_lines_text(&lines, 0, 10), "only");
    }

    // --- collect_selected_text ---

    #[test]
    fn collect_selected_text_line_mode() {
        let lines = vec![
            Line::from("first"),
            Line::from("second"),
            Line::from("third"),
        ];
        let mut state = CopyModeState::new();
        state.enter(0, 0);
        state.start_selecting_with_mode(VisualMode::Line);
        state.cursor = Position::new(1, 0);
        let text = collect_selected_text(&lines, &state);
        assert_eq!(text, "first\nsecond");
    }

    #[test]
    fn collect_selected_text_char_mode_single_line() {
        let lines = vec![Line::from("hello world")];
        let mut state = CopyModeState::new();
        state.enter(0, 6);
        state.start_selecting_with_mode(VisualMode::Char);
        state.cursor = Position::new(0, 11);
        let text = collect_selected_text(&lines, &state);
        assert_eq!(text, "world");
    }

    #[test]
    fn collect_selected_text_block_mode() {
        // Columns 6..10 (exclusive) of each line — cursor at col 9 is inclusive:
        //   "hello world" → 'w'(6) 'o'(7) 'r'(8) 'l'(9) → "worl"
        //   "foo   bar  " → 'b'(6) 'a'(7) 'r'(8) ' '(9) → "bar "
        //   "abcde fghij" → 'f'(6) 'g'(7) 'h'(8) 'i'(9) → "fghi"
        let lines = vec![
            Line::from("hello world"),
            Line::from("foo   bar  "),
            Line::from("abcde fghij"),
        ];
        let mut state = CopyModeState::new();
        state.enter(0, 6);
        state.start_selecting_with_mode(VisualMode::Block);
        state.cursor = Position::new(2, 9);
        // line_col_range returns (6, 10); line_text_substr extracts [6, 10).
        let text = collect_selected_text(&lines, &state);
        let parts: Vec<&str> = text.split('\n').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0], "worl");
        assert_eq!(parts[1], "bar ");
        assert_eq!(parts[2], "fghi");
    }

    #[test]
    fn collect_selected_text_none_when_not_selecting() {
        let lines = vec![Line::from("hello")];
        let mut state = CopyModeState::new();
        state.enter(0, 0);
        // Not selecting.
        let text = collect_selected_text(&lines, &state);
        assert_eq!(text, "");
    }

    // --- entry_to_copy_text (kept for backward compat) ---

    #[test]
    fn entry_text_user_message() {
        let entry = TranscriptEntry::UserMessage("hello".to_owned());
        assert_eq!(entry_to_copy_text(&entry), "User: hello");
    }

    #[test]
    fn entry_text_assistant_message() {
        let entry = TranscriptEntry::AssistantMessage("world".to_owned());
        assert_eq!(entry_to_copy_text(&entry), "Assistant: world");
    }

    #[test]
    fn entry_text_streaming() {
        let mut msg = StreamingMessage::new();
        msg.push_token("streaming content");
        let entry = TranscriptEntry::Streaming(msg);
        assert_eq!(entry_to_copy_text(&entry), "Assistant: streaming content");
    }

    #[test]
    fn entry_text_tool_call_basic() {
        let entry = TranscriptEntry::ToolCall {
            name: "read_file".to_owned(),
            status: ToolCallStatus::Success,
            duration_ms: Some(100),
            summary: None,
            thinking: None,
            output: None,
        };
        assert_eq!(entry_to_copy_text(&entry), "Tool: read_file");
    }

    #[test]
    fn entry_text_tool_call_with_summary_and_output() {
        let entry = TranscriptEntry::ToolCall {
            name: "search".to_owned(),
            status: ToolCallStatus::Success,
            duration_ms: None,
            summary: Some("Found 3 results".to_owned()),
            thinking: None,
            output: Some("result1\nresult2\nresult3".to_owned()),
        };
        let text = entry_to_copy_text(&entry);
        assert!(text.contains("Tool: search"));
        assert!(text.contains("Summary: Found 3 results"));
        assert!(text.contains("Output: result1"));
    }

    #[test]
    fn entry_text_router_event() {
        let entry = TranscriptEntry::RouterEvent("routed".to_owned());
        assert_eq!(entry_to_copy_text(&entry), "Router: routed");
    }

    #[test]
    fn entry_text_system_message() {
        let entry = TranscriptEntry::SystemMessage("system info".to_owned());
        assert_eq!(entry_to_copy_text(&entry), "System: system info");
    }

    #[test]
    fn entry_text_patch_proposed() {
        let entry = TranscriptEntry::PatchProposed {
            file_path: "src/main.rs".to_owned(),
            raw_diff: "+new line".to_owned(),
            patch_id: None,
        };
        let text = entry_to_copy_text(&entry);
        assert!(text.contains("Patch: src/main.rs"));
        assert!(text.contains("+new line"));
    }

    // --- collect_selection_text (kept for backward compat) ---

    #[test]
    fn collect_single_entry() {
        let transcript = vec![TranscriptEntry::UserMessage("hello".to_owned())];
        let text = collect_selection_text(&transcript, 0, 0);
        assert_eq!(text, "User: hello");
    }

    #[test]
    fn collect_multiple_entries() {
        let transcript = vec![
            TranscriptEntry::UserMessage("hello".to_owned()),
            TranscriptEntry::AssistantMessage("world".to_owned()),
            TranscriptEntry::SystemMessage("info".to_owned()),
        ];
        let text = collect_selection_text(&transcript, 0, 2);
        assert!(text.contains("User: hello"));
        assert!(text.contains("Assistant: world"));
        assert!(text.contains("System: info"));
        assert!(text.contains("\n\n"));
    }

    #[test]
    fn collect_partial_range() {
        let transcript = vec![
            TranscriptEntry::UserMessage("a".to_owned()),
            TranscriptEntry::AssistantMessage("b".to_owned()),
            TranscriptEntry::SystemMessage("c".to_owned()),
        ];
        let text = collect_selection_text(&transcript, 1, 1);
        assert_eq!(text, "Assistant: b");
    }

    #[test]
    fn collect_clamps_end_to_transcript_len() {
        let transcript = vec![TranscriptEntry::UserMessage("only".to_owned())];
        let text = collect_selection_text(&transcript, 0, 10);
        assert_eq!(text, "User: only");
    }
}
