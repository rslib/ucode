use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Widget,
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
    /// Optional argument hint, e.g. "<name>"
    pub args_hint: Option<String>,
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
            args_hint: None,
        }
    }

    pub fn with_args_hint(mut self, hint: impl Into<String>) -> Self {
        self.args_hint = Some(hint.into());
        self
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
    /// Single-entry kill ring for readline-style cut/paste.
    pub kill_ring: String,
    /// Previously sent messages (oldest first).
    pub history: Vec<String>,
    /// Current position in history. `None` means not browsing history.
    /// When `Some(i)`, `history[i]` is displayed.
    history_index: Option<usize>,
    /// The in-progress text saved when the user starts browsing history,
    /// so it can be restored when they navigate past the newest entry.
    history_stash: String,
    /// Line index of the first visible line when content exceeds max visible lines.
    pub scroll_offset: usize,
    /// Byte offset of the selection anchor.
    pub selection_anchor: usize,
    /// True when a text selection is active.
    pub selecting: bool,
}

impl InputBoxState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert `c` at the current cursor position and advance the cursor.
    pub fn insert_char(&mut self, c: char) {
        self.content.insert(self.cursor_pos, c);
        self.cursor_pos += c.len_utf8();
        if c == '\n' {
            self.cursor_col = 0;
        } else {
            self.cursor_col += c.width().unwrap_or(1);
        }
    }

    /// Insert a string at the current cursor position and advance the cursor.
    pub fn insert_str(&mut self, s: &str) {
        for c in s.chars() {
            self.insert_char(c);
        }
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
    }

    /// Clear content and reset cursor.
    pub fn clear(&mut self) {
        self.content.clear();
        self.cursor_pos = 0;
        self.cursor_col = 0;
        self.scroll_offset = 0;
        self.autocomplete.hide();
        self.selecting = false;
    }

    /// Take the content, clear state, and return the taken string.
    pub fn take_content(&mut self) -> String {
        let taken = std::mem::take(&mut self.content);
        self.cursor_pos = 0;
        self.cursor_col = 0;
        self.scroll_offset = 0;
        self.autocomplete.hide();
        self.selecting = false;
        taken
    }

    /// Adjust `scroll_offset` so the cursor line is within the visible window.
    pub fn ensure_cursor_visible(&mut self, max_visible_lines: usize) {
        let (cursor_line, _) = cursor_line_and_col(&self.content, self.cursor_pos);
        if cursor_line < self.scroll_offset {
            self.scroll_offset = cursor_line;
        } else if cursor_line >= self.scroll_offset + max_visible_lines {
            self.scroll_offset = cursor_line + 1 - max_visible_lines;
        }
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
    // Readline-style editing
    // ------------------------------------------------------------------

    /// Move cursor to the start of the previous word (Alt+B).
    ///
    /// A word is a contiguous run of alphanumeric/underscore characters.
    pub fn move_word_left(&mut self) {
        if self.cursor_pos == 0 {
            return;
        }
        let prefix = &self.content[..self.cursor_pos];
        // Skip non-word chars, then skip word chars.
        let mut chars: Vec<(usize, char)> = prefix.char_indices().collect();
        chars.reverse();
        let mut pos = self.cursor_pos;
        let mut iter = chars.iter();
        // Skip trailing non-word chars.
        for &(i, c) in iter.by_ref() {
            if is_word_char(c) {
                pos = i;
                break;
            }
        }
        // Skip the word itself.
        for &(i, c) in iter {
            if !is_word_char(c) {
                break;
            }
            pos = i;
        }
        let delta_bytes = self.cursor_pos - pos;
        let delta_cols = display_width(&self.content[pos..self.cursor_pos]);
        self.cursor_pos = pos;
        self.cursor_col = self.cursor_col.saturating_sub(delta_cols);
        let _ = delta_bytes;
    }

    /// Move cursor to the end of the next word (Alt+F).
    ///
    /// A word is a contiguous run of alphanumeric/underscore characters.
    pub fn move_word_right(&mut self) {
        if self.cursor_pos >= self.content.len() {
            return;
        }
        let suffix = &self.content[self.cursor_pos..];
        let mut new_offset = self.cursor_pos;
        let mut chars = suffix.char_indices().peekable();
        // Skip leading non-word chars.
        while let Some(&(i, c)) = chars.peek() {
            if is_word_char(c) {
                break;
            }
            new_offset = self.cursor_pos + i + c.len_utf8();
            chars.next();
        }
        // Skip the word itself.
        while let Some(&(i, c)) = chars.peek() {
            if !is_word_char(c) {
                break;
            }
            new_offset = self.cursor_pos + i + c.len_utf8();
            chars.next();
        }
        let added = display_width(&self.content[self.cursor_pos..new_offset]);
        self.cursor_col += added;
        self.cursor_pos = new_offset;
    }

    /// Kill (cut) from cursor to end of current line, storing in kill_ring (Ctrl+K).
    pub fn kill_to_end(&mut self) {
        let line_end = self.content[self.cursor_pos..]
            .find('\n')
            .map(|i| self.cursor_pos + i)
            .unwrap_or(self.content.len());
        self.kill_ring = self.content[self.cursor_pos..line_end].to_owned();
        self.content.drain(self.cursor_pos..line_end);
        // cursor_pos and cursor_col stay the same.
    }

    /// Kill from cursor to start of current line, storing in kill_ring (Ctrl+U).
    pub fn kill_to_start(&mut self) {
        let line_start = self.content[..self.cursor_pos]
            .rfind('\n')
            .map(|i| i + 1)
            .unwrap_or(0);
        self.kill_ring = self.content[line_start..self.cursor_pos].to_owned();
        self.content.drain(line_start..self.cursor_pos);
        self.cursor_pos = line_start;
        self.cursor_col = 0;
    }

    /// Delete word before cursor, storing in kill_ring (Ctrl+W).
    pub fn delete_word_back(&mut self) {
        if self.cursor_pos == 0 {
            return;
        }
        let prefix = &self.content[..self.cursor_pos];
        let mut chars: Vec<(usize, char)> = prefix.char_indices().collect();
        chars.reverse();
        let mut word_start = self.cursor_pos;
        let mut iter = chars.iter();
        // Skip trailing non-word chars.
        for &(i, c) in iter.by_ref() {
            if is_word_char(c) {
                word_start = i;
                break;
            }
        }
        // Skip the word itself.
        for &(i, c) in iter {
            if !is_word_char(c) {
                break;
            }
            word_start = i;
        }
        let removed = self.content[word_start..self.cursor_pos].to_owned();
        let removed_cols = display_width(&removed);
        self.kill_ring = removed;
        self.content.drain(word_start..self.cursor_pos);
        self.cursor_pos = word_start;
        self.cursor_col = self.cursor_col.saturating_sub(removed_cols);
    }

    /// Delete word after cursor, storing in kill_ring (Alt+D).
    pub fn delete_word_forward(&mut self) {
        if self.cursor_pos >= self.content.len() {
            return;
        }
        let suffix = &self.content[self.cursor_pos..];
        let mut end_offset = self.cursor_pos;
        let mut chars = suffix.char_indices().peekable();
        // Skip leading non-word chars.
        while let Some(&(i, c)) = chars.peek() {
            if is_word_char(c) {
                break;
            }
            end_offset = self.cursor_pos + i + c.len_utf8();
            chars.next();
        }
        // Skip the word itself.
        while let Some(&(i, c)) = chars.peek() {
            if !is_word_char(c) {
                break;
            }
            end_offset = self.cursor_pos + i + c.len_utf8();
            chars.next();
        }
        self.kill_ring = self.content[self.cursor_pos..end_offset].to_owned();
        self.content.drain(self.cursor_pos..end_offset);
        // cursor_pos and cursor_col stay the same.
    }

    /// Swap the two characters before the cursor (Ctrl+T).
    ///
    /// If at end of line, swaps the last two characters.
    pub fn transpose_chars(&mut self) {
        // Determine the effective "cursor" position for transposition.
        // If at end (or past last char), use the last char boundary.
        let pos = if self.cursor_pos >= self.content.len() {
            // Find the last char boundary.
            let mut p = self.content.len();
            while p > 0 && !self.content.is_char_boundary(p) {
                p -= 1;
            }
            p
        } else {
            self.cursor_pos
        };

        if pos == 0 {
            return;
        }

        // Find the char before pos.
        let prev1 = self.prev_char_boundary(pos);
        if prev1 == 0 && !self.content.is_char_boundary(0) {
            return;
        }
        let prev2 = if prev1 > 0 {
            self.prev_char_boundary(prev1)
        } else {
            return; // need at least two chars
        };

        // Extract the two chars.
        let c1: char = self.content[prev2..prev1].chars().next().unwrap_or('\0');
        let c2: char = self.content[prev1..pos].chars().next().unwrap_or('\0');

        // Rebuild: replace [prev2..pos] with c2 then c1.
        let mut new_content = String::with_capacity(self.content.len());
        new_content.push_str(&self.content[..prev2]);
        new_content.push(c2);
        new_content.push(c1);
        new_content.push_str(&self.content[pos..]);
        self.content = new_content;
        // Move cursor past the transposed pair if we were at end.
        if self.cursor_pos >= self.content.len() {
            self.cursor_pos = self.content.len();
        }
    }

    /// Swap the word before the cursor with the word after the cursor (Alt+T).
    ///
    /// If the cursor is at the end of the string, swaps the last two words.
    /// Moves the cursor to the end of the second (now-later) word.
    /// Does nothing if fewer than two words exist.
    pub fn transpose_words(&mut self) {
        // Collect all word spans as (start_byte, end_byte) pairs.
        let words: Vec<(usize, usize)> = {
            let mut v = Vec::new();
            let mut chars = self.content.char_indices().peekable();
            while let Some(&(i, c)) = chars.peek() {
                if is_word_char(c) {
                    let start = i;
                    let mut end = i + c.len_utf8();
                    chars.next();
                    while let Some(&(j, d)) = chars.peek() {
                        if !is_word_char(d) {
                            break;
                        }
                        end = j + d.len_utf8();
                        chars.next();
                    }
                    v.push((start, end));
                } else {
                    chars.next();
                }
            }
            v
        };

        if words.len() < 2 {
            return;
        }

        // Determine which two words to swap.
        // If cursor is at or past the end of the last word, swap the last two.
        // Otherwise, swap the word whose end is at/after the cursor with the
        // word immediately before it.
        let (w1, w2) = if self.cursor_pos >= words[words.len() - 1].1 {
            (words.len() - 2, words.len() - 1)
        } else {
            // Find the first word whose end is >= cursor_pos.
            let idx = words
                .iter()
                .position(|&(_, end)| end >= self.cursor_pos)
                .unwrap_or(words.len() - 1);
            if idx == 0 { (0, 1) } else { (idx - 1, idx) }
        };

        let (s1, e1) = words[w1];
        let (s2, e2) = words[w2];

        // Build new content: swap the two word slices, keep everything else.
        let word1 = self.content[s1..e1].to_owned();
        let word2 = self.content[s2..e2].to_owned();

        let mut new_content = String::with_capacity(self.content.len());
        new_content.push_str(&self.content[..s1]);
        new_content.push_str(&word2);
        new_content.push_str(&self.content[e1..s2]);
        new_content.push_str(&word1);
        new_content.push_str(&self.content[e2..]);

        // New cursor position: end of the second word in its new location.
        // The second word now starts at s1 + word2.len() + gap, ends at s1 + word2.len() + gap + word1.len().
        // But we want cursor at end of word1 in its new position.
        let gap = &self.content[e1..s2];
        let new_cursor = s1 + word2.len() + gap.len() + word1.len();

        self.content = new_content;
        self.cursor_pos = new_cursor;
        self.cursor_col = display_width(&self.content[..self.cursor_pos]);
    }

    /// Uppercase the word from cursor to end of word (Alt+U).
    ///
    /// Moves cursor to end of word.
    pub fn upcase_word(&mut self) {
        if self.cursor_pos >= self.content.len() {
            return;
        }
        let suffix = &self.content[self.cursor_pos..];
        let mut end_offset = self.cursor_pos;
        let mut chars = suffix.char_indices().peekable();
        // Skip leading non-word chars.
        while let Some(&(_, c)) = chars.peek() {
            if is_word_char(c) {
                break;
            }
            chars.next();
        }
        // Find end of word.
        while let Some(&(i, c)) = chars.peek() {
            if !is_word_char(c) {
                break;
            }
            end_offset = self.cursor_pos + i + c.len_utf8();
            chars.next();
        }
        if end_offset == self.cursor_pos {
            return;
        }
        // Find the actual start of the word (skip non-word prefix from cursor).
        let word_start = self.content[self.cursor_pos..]
            .char_indices()
            .find(|&(_, c)| is_word_char(c))
            .map(|(i, _)| self.cursor_pos + i)
            .unwrap_or(self.cursor_pos);
        let upcased: String = self.content[word_start..end_offset]
            .chars()
            .map(|c| c.to_uppercase().next().unwrap_or(c))
            .collect();
        self.content.replace_range(word_start..end_offset, &upcased);
        self.cursor_pos = end_offset;
        self.cursor_col = display_width(&self.content[..self.cursor_pos]);
    }

    /// Lowercase the word from cursor to end of word (Alt+L).
    ///
    /// Moves cursor to end of word.
    pub fn downcase_word(&mut self) {
        if self.cursor_pos >= self.content.len() {
            return;
        }
        let suffix = &self.content[self.cursor_pos..];
        let mut end_offset = self.cursor_pos;
        let mut chars = suffix.char_indices().peekable();
        // Skip leading non-word chars.
        while let Some(&(_, c)) = chars.peek() {
            if is_word_char(c) {
                break;
            }
            chars.next();
        }
        // Find end of word.
        while let Some(&(i, c)) = chars.peek() {
            if !is_word_char(c) {
                break;
            }
            end_offset = self.cursor_pos + i + c.len_utf8();
            chars.next();
        }
        if end_offset == self.cursor_pos {
            return;
        }
        let word_start = self.content[self.cursor_pos..]
            .char_indices()
            .find(|&(_, c)| is_word_char(c))
            .map(|(i, _)| self.cursor_pos + i)
            .unwrap_or(self.cursor_pos);
        let downcased: String = self.content[word_start..end_offset]
            .chars()
            .map(|c| c.to_lowercase().next().unwrap_or(c))
            .collect();
        self.content
            .replace_range(word_start..end_offset, &downcased);
        self.cursor_pos = end_offset;
        self.cursor_col = display_width(&self.content[..self.cursor_pos]);
    }

    /// Capitalize the first character of the current word, lowercase the rest (Alt+C).
    ///
    /// Moves cursor to end of word.
    pub fn capitalize_word(&mut self) {
        if self.cursor_pos >= self.content.len() {
            return;
        }
        let suffix = &self.content[self.cursor_pos..];
        let mut end_offset = self.cursor_pos;
        let mut chars = suffix.char_indices().peekable();
        // Skip leading non-word chars.
        while let Some(&(_, c)) = chars.peek() {
            if is_word_char(c) {
                break;
            }
            chars.next();
        }
        // Find end of word.
        while let Some(&(i, c)) = chars.peek() {
            if !is_word_char(c) {
                break;
            }
            end_offset = self.cursor_pos + i + c.len_utf8();
            chars.next();
        }
        if end_offset == self.cursor_pos {
            return;
        }
        let word_start = self.content[self.cursor_pos..]
            .char_indices()
            .find(|&(_, c)| is_word_char(c))
            .map(|(i, _)| self.cursor_pos + i)
            .unwrap_or(self.cursor_pos);
        let capitalized: String = {
            let mut word_chars = self.content[word_start..end_offset].chars();
            match word_chars.next() {
                None => String::new(),
                Some(first) => {
                    let upper: String = first.to_uppercase().collect();
                    let rest: String = word_chars
                        .map(|c| c.to_lowercase().next().unwrap_or(c))
                        .collect();
                    upper + &rest
                }
            }
        };
        self.content
            .replace_range(word_start..end_offset, &capitalized);
        self.cursor_pos = end_offset;
        self.cursor_col = display_width(&self.content[..self.cursor_pos]);
    }

    /// Move cursor up one line, maintaining display column (Ctrl+P / Up arrow).
    ///
    /// If already on the first line, does nothing. The column is clamped to
    /// the length of the target line.
    pub fn move_up(&mut self) {
        let (line_idx, col) = cursor_line_and_col(&self.content, self.cursor_pos);
        if line_idx == 0 {
            return;
        }
        // Collect line byte-start positions.
        let line_starts = line_byte_starts(&self.content);
        let target_line = line_idx - 1;
        let target_start = line_starts[target_line];
        // End of target line = start of current line minus 1 (the '\n').
        let target_end = line_starts[line_idx] - 1;
        let target_line_text = &self.content[target_start..target_end];
        self.cursor_pos = byte_pos_at_col(target_line_text, target_start, col);
        self.cursor_col = display_width(&self.content[target_start..self.cursor_pos]);
    }

    /// Move cursor down one line, maintaining display column (Ctrl+N / Down arrow).
    ///
    /// If already on the last line, does nothing. The column is clamped to
    /// the length of the target line.
    pub fn move_down(&mut self) {
        let (line_idx, col) = cursor_line_and_col(&self.content, self.cursor_pos);
        let line_starts = line_byte_starts(&self.content);
        let last_line = line_starts.len() - 1;
        if line_idx == last_line {
            return;
        }
        let target_line = line_idx + 1;
        let target_start = line_starts[target_line];
        let target_end = if target_line < last_line {
            line_starts[target_line + 1] - 1
        } else {
            self.content.len()
        };
        let target_line_text = &self.content[target_start..target_end];
        self.cursor_pos = byte_pos_at_col(target_line_text, target_start, col);
        self.cursor_col = display_width(&self.content[target_start..self.cursor_pos]);
    }

    /// Insert kill_ring contents at cursor (Ctrl+Y).
    pub fn yank(&mut self) {
        if self.kill_ring.is_empty() {
            return;
        }
        let yanked = self.kill_ring.clone();
        let col_delta = display_width(&yanked);
        self.content.insert_str(self.cursor_pos, &yanked);
        self.cursor_pos += yanked.len();
        self.cursor_col += col_delta;
    }

    /// Record a sent message in the history ring.
    pub fn push_history(&mut self, msg: String) {
        // Avoid consecutive duplicates.
        if self.history.last().is_none_or(|last| *last != msg) {
            self.history.push(msg);
        }
        self.history_index = None;
        self.history_stash.clear();
    }

    /// Returns `true` if the cursor is on the first line of the input.
    pub fn cursor_on_first_line(&self) -> bool {
        let prefix = &self.content[..self.cursor_pos.min(self.content.len())];
        !prefix.contains('\n')
    }

    /// Returns `true` if the cursor is on the last line of the input.
    pub fn cursor_on_last_line(&self) -> bool {
        let suffix = &self.content[self.cursor_pos.min(self.content.len())..];
        !suffix.contains('\n')
    }

    /// Navigate to the previous history entry. Returns `true` if the
    /// content changed (caller should mark dirty).
    pub fn history_prev(&mut self) -> bool {
        if self.history.is_empty() {
            return false;
        }
        let new_idx = match self.history_index {
            None => {
                // Entering history — stash current input.
                self.history_stash = self.content.clone();
                self.history.len() - 1
            }
            Some(0) => return false, // already at oldest
            Some(i) => i - 1,
        };
        self.history_index = Some(new_idx);
        self.set_content_from_history(&self.history[new_idx].clone());
        true
    }

    /// Navigate to the next history entry, or restore the stashed input
    /// if past the newest entry. Returns `true` if content changed.
    pub fn history_next(&mut self) -> bool {
        let idx = match self.history_index {
            None => return false, // not browsing history
            Some(i) => i,
        };
        if idx + 1 < self.history.len() {
            self.history_index = Some(idx + 1);
            self.set_content_from_history(&self.history[idx + 1].clone());
        } else {
            // Past newest — restore stashed input.
            self.history_index = None;
            let stash = self.history_stash.clone();
            self.set_content_from_history(&stash);
            self.history_stash.clear();
        }
        true
    }

    /// Replace content and move cursor to end.
    fn set_content_from_history(&mut self, text: &str) {
        self.content = text.to_owned();
        self.cursor_pos = self.content.len();
        self.cursor_col =
            display_width(&self.content[self.content.rfind('\n').map(|i| i + 1).unwrap_or(0)..]);
    }

    // ------------------------------------------------------------------
    // Selection
    // ------------------------------------------------------------------

    /// Start a selection at the current cursor position.
    pub fn start_selecting(&mut self) {
        self.selecting = true;
        self.selection_anchor = self.cursor_pos;
    }

    /// Cancel the active selection.
    pub fn cancel_selecting(&mut self) {
        self.selecting = false;
    }

    /// Ensure we are in selection mode. If not selecting, starts it.
    /// If already selecting, does nothing (preserves anchor).
    pub fn ensure_selecting(&mut self) {
        if !self.selecting {
            self.start_selecting();
        }
    }

    /// Returns the selected byte range, or None if not selecting or anchor == cursor.
    pub fn selected_range(&self) -> Option<std::ops::Range<usize>> {
        if !self.selecting {
            return None;
        }
        let start = self.selection_anchor.min(self.cursor_pos);
        let end = self.selection_anchor.max(self.cursor_pos);
        if start == end {
            return None;
        }
        Some(start..end)
    }

    /// Returns the selected text slice, or None.
    pub fn selected_text(&self) -> Option<&str> {
        self.selected_range().map(|r| &self.content[r])
    }

    /// Delete the selected text, place cursor at the start of the range,
    /// and cancel the selection.
    pub fn delete_selected(&mut self) {
        if let Some(range) = self.selected_range() {
            let start = range.start;
            self.content.drain(range);
            self.cursor_pos = start;
            self.cursor_col = display_width(
                &self.content[self.content[..start]
                    .rfind('\n')
                    .map(|i| i + 1)
                    .unwrap_or(0)..start],
            );
            self.selecting = false;
        }
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

/// True for alphanumeric and underscore — readline word boundary definition.
fn is_word_char(c: char) -> bool {
    crate::overlays::copy_mode::is_word_char(c)
}

/// Return the byte offset of each line's first character in `text`.
///
/// Always contains at least one entry (0 for the first line).
fn line_byte_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0usize];
    for (i, c) in text.char_indices() {
        if c == '\n' {
            starts.push(i + 1);
        }
    }
    starts
}

/// Given a line's text slice and its absolute byte start in the full buffer,
/// return the absolute byte position corresponding to display column `col`.
///
/// If `col` exceeds the line's display width the position is clamped to the
/// end of the line.
fn byte_pos_at_col(line_text: &str, line_start: usize, col: usize) -> usize {
    let mut current_col = 0usize;
    for (i, c) in line_text.char_indices() {
        if current_col >= col {
            return line_start + i;
        }
        current_col += c.width().unwrap_or(1);
    }
    // col >= line width: clamp to end of line.
    line_start + line_text.len()
}

// ---------------------------------------------------------------------------
// InputInfoLine
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// InputBox widget
// ---------------------------------------------------------------------------

pub struct InputBox<'a> {
    pub state: &'a InputBoxState,
    pub theme: &'a UcodeTheme,
    pub focused: bool,
    pub info_line: Option<&'a InputInfoLine>,
}

impl<'a> InputBox<'a> {
    pub fn new(state: &'a InputBoxState, theme: &'a UcodeTheme, focused: bool) -> Self {
        Self {
            state,
            theme,
            focused,
            info_line: None,
        }
    }

    pub fn with_info_line(mut self, info: &'a InputInfoLine) -> Self {
        self.info_line = Some(info);
        self
    }
}

impl Widget for InputBox<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        // Fill entire area with surface background.
        let bg_style = Style::new().bg(self.theme.surface);
        for row in area.y..area.y + area.height {
            for col in area.x..area.x + area.width {
                buf[(col, row)].set_style(bg_style);
            }
        }

        // Left accent bar: 1 column, full height.
        let accent_style = Style::new().fg(self.theme.accent).bg(self.theme.surface);
        for row in area.y..area.y + area.height {
            let cell = &mut buf[(area.x, row)];
            cell.set_symbol("\u{2502}"); // "│"
            cell.set_style(accent_style);
        }

        // Text area: after accent bar (1 col) + 1 col padding,
        // inset 1 row top and 1 row bottom for vertical padding.
        let text_x = area.x + 2;
        let text_width = area.width.saturating_sub(2); // 2 left (bar + pad)
        let text_y = area.y + 1;
        let text_height = area.height.saturating_sub(2);
        if text_width == 0 || text_height == 0 {
            return;
        }
        let text_area = Rect {
            x: text_x,
            y: text_y,
            width: text_width,
            height: text_height,
        };

        let lines: Vec<&str> = self.state.content.split('\n').collect();
        let visible_rows = text_area.height as usize;

        // Determine which line the cursor is on and its column.
        let (cursor_line, cursor_line_col) =
            cursor_line_and_col(&self.state.content, self.state.cursor_pos);

        // Render each visible line, skipping lines above the scroll viewport.
        for (row_idx, line_text) in lines
            .iter()
            .skip(self.state.scroll_offset)
            .take(visible_rows)
            .enumerate()
        {
            let y = text_area.y + row_idx as u16;
            let content_span = Span::styled(*line_text, self.theme.text_style());
            let line = Line::from(vec![content_span]);
            let row_area = Rect {
                y,
                height: 1,
                ..text_area
            };
            line.render(row_area, buf);
        }

        // Render selection highlight.
        if let Some(sel_range) = self.state.selected_range() {
            let sel_style = ratatui::style::Style::default().add_modifier(Modifier::REVERSED);

            for (row_idx, line_text) in lines
                .iter()
                .skip(self.state.scroll_offset)
                .take(visible_rows)
                .enumerate()
            {
                let actual_line_idx = self.state.scroll_offset + row_idx;
                let y = text_area.y + row_idx as u16;

                // Byte offset of this line's first character within content.
                let line_byte_start: usize = self
                    .state
                    .content
                    .split('\n')
                    .take(actual_line_idx)
                    .map(|l| l.len() + 1) // +1 for the '\n' separator
                    .sum();
                let line_byte_end = line_byte_start + line_text.len();

                // Intersect the selection range with this line's byte range.
                let hl_start = sel_range.start.max(line_byte_start);
                let hl_end = sel_range.end.min(line_byte_end);

                if hl_start >= hl_end {
                    // Selection may span past this line's end (covers the '\n').
                    if sel_range.start <= line_byte_end && sel_range.end > line_byte_end {
                        let start_in_line = sel_range.start.max(line_byte_start) - line_byte_start;
                        let col_start = display_width(&line_text[..start_in_line]);
                        let col_end = display_width(line_text);
                        for col in col_start..=col_end {
                            let x = text_area.x + col as u16;
                            if x < text_area.x + text_area.width {
                                let cell = &mut buf[(x, y)];
                                cell.set_style(cell.style().patch(sel_style));
                            }
                        }
                    }
                    continue;
                }

                let start_in_line = hl_start - line_byte_start;
                let end_in_line = hl_end - line_byte_start;
                let col_start = display_width(&line_text[..start_in_line]);
                let col_end = display_width(&line_text[..end_in_line]);

                for col in col_start..col_end {
                    let x = text_area.x + col as u16;
                    if x < text_area.x + text_area.width {
                        let cell = &mut buf[(x, y)];
                        cell.set_style(cell.style().patch(sel_style));
                    }
                }
            }
        }

        // Render cursor highlight.
        let visible_cursor_line = cursor_line.saturating_sub(self.state.scroll_offset);
        let cursor_x = text_area.x + cursor_line_col as u16;
        let cursor_y = text_area.y + visible_cursor_line as u16;

        if !self.state.selecting
            && cursor_x < text_area.x + text_area.width
            && cursor_y < text_area.y + text_area.height
        {
            let cell = &mut buf[(cursor_x, cursor_y)];
            let current_style = cell.style();
            cell.set_style(current_style.add_modifier(Modifier::REVERSED));
        }

        // Render info line on the bottom row of the input area.
        if let Some(info) = self.info_line {
            let info_y = area.y + area.height - 1;
            let info_x = area.x + 2; // after accent bar + padding
            let info_width = area.width.saturating_sub(3) as usize; // leave 1 right margin

            let display = info.display_text();
            let agent_part = format!("@{}", info.agent_name);
            let rest = &display[agent_part.len()..];

            // Render agent name in accent color.
            let accent_style = Style::new().fg(self.theme.accent).bg(self.theme.surface);
            for (i, ch) in agent_part.chars().enumerate() {
                let x = info_x + i as u16;
                if x < area.x + area.width && (i < info_width) {
                    buf[(x, info_y)].set_char(ch).set_style(accent_style);
                }
            }

            // Render rest (model + provider) in dim style.
            let dim_style = Style::new().fg(self.theme.text_dim).bg(self.theme.surface);
            let offset = agent_part.len();
            for (i, ch) in rest.chars().enumerate() {
                let x = info_x + (offset + i) as u16;
                if x < area.x + area.width && (offset + i < info_width) {
                    buf[(x, info_y)].set_char(ch).set_style(dim_style);
                }
            }
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

            let hint_style = if is_selected {
                bg_style
            } else {
                self.theme.dim_style()
            };

            let mut spans = vec![
                Span::styled("  ", bg_style),
                Span::styled(entry.name.clone(), name_style),
            ];
            if let Some(hint) = &entry.args_hint {
                spans.push(Span::styled(format!(" {hint}"), hint_style));
            }
            spans.push(Span::styled("  ", bg_style));
            spans.push(Span::styled(entry.description.clone(), desc_style));
            spans.push(Span::styled("  ", bg_style));
            spans.push(Span::styled(entry.source.clone(), src_style));

            let line = Line::from(spans);
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
    // Readline method tests (Part 1)
    // ------------------------------------------------------------------

    #[test]
    fn input_box_kill_ring_default_empty() {
        let state = InputBoxState::new();
        assert!(state.kill_ring.is_empty());
    }

    #[test]
    fn input_box_move_word_left() {
        let mut state = InputBoxState::new();
        for c in "hello world".chars() {
            state.insert_char(c);
        }
        // cursor is at end (pos 11); move_word_left should land at start of "world" (pos 6)
        state.move_word_left();
        assert_eq!(state.cursor_pos, 6);
    }

    #[test]
    fn input_box_move_word_right() {
        let mut state = InputBoxState::new();
        for c in "hello world".chars() {
            state.insert_char(c);
        }
        // move to start
        state.move_home();
        // move_word_right should land after "hello" (pos 5)
        state.move_word_right();
        assert_eq!(state.cursor_pos, 5);
    }

    #[test]
    fn input_box_kill_to_end() {
        let mut state = InputBoxState::new();
        for c in "hello world".chars() {
            state.insert_char(c);
        }
        // move to position 5 (after "hello")
        state.move_home();
        for _ in 0..5 {
            state.move_right();
        }
        state.kill_to_end();
        assert_eq!(state.content, "hello");
        assert_eq!(state.kill_ring, " world");
    }

    #[test]
    fn input_box_kill_to_start() {
        let mut state = InputBoxState::new();
        for c in "hello world".chars() {
            state.insert_char(c);
        }
        // move to position 5 (after "hello")
        state.move_home();
        for _ in 0..5 {
            state.move_right();
        }
        state.kill_to_start();
        assert_eq!(state.content, " world");
        assert_eq!(state.kill_ring, "hello");
    }

    #[test]
    fn input_box_delete_word_back() {
        let mut state = InputBoxState::new();
        for c in "hello world".chars() {
            state.insert_char(c);
        }
        // cursor at end; delete_word_back should remove "world"
        state.delete_word_back();
        assert_eq!(state.content, "hello ");
        assert_eq!(state.kill_ring, "world");
    }

    #[test]
    fn input_box_delete_word_forward() {
        let mut state = InputBoxState::new();
        for c in "hello world".chars() {
            state.insert_char(c);
        }
        // move to start; delete_word_forward should remove "hello"
        state.move_home();
        state.delete_word_forward();
        assert_eq!(state.content, " world");
        assert_eq!(state.kill_ring, "hello");
    }

    #[test]
    fn input_box_transpose_chars() {
        let mut state = InputBoxState::new();
        state.insert_char('a');
        state.insert_char('b');
        // cursor at end; transpose swaps last two chars
        state.transpose_chars();
        assert_eq!(state.content, "ba");
    }

    #[test]
    fn input_box_yank() {
        let mut state = InputBoxState::new();
        for c in "hello world".chars() {
            state.insert_char(c);
        }
        // kill to end from position 5 → kill_ring = " world", content = "hello"
        state.move_home();
        for _ in 0..5 {
            state.move_right();
        }
        state.kill_to_end();
        assert_eq!(state.content, "hello");

        // move to start and yank
        state.move_home();
        state.yank();
        assert_eq!(state.content, " worldhello");
    }

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

    #[test]
    fn input_box_transpose_words() {
        let mut state = InputBoxState::new();
        for c in "hello world".chars() {
            state.insert_char(c);
        }
        // cursor at end; transpose_words swaps "hello" and "world"
        state.transpose_words();
        assert_eq!(state.content, "world hello");
        // cursor should be at end of "hello" in its new position
        assert_eq!(state.cursor_pos, state.content.len());
    }

    #[test]
    fn input_box_upcase_word() {
        let mut state = InputBoxState::new();
        for c in "hello world".chars() {
            state.insert_char(c);
        }
        state.move_home();
        state.upcase_word();
        assert_eq!(state.content, "HELLO world");
        // cursor should be after "HELLO" (pos 5)
        assert_eq!(state.cursor_pos, 5);
    }

    #[test]
    fn input_box_downcase_word() {
        let mut state = InputBoxState::new();
        for c in "HELLO world".chars() {
            state.insert_char(c);
        }
        state.move_home();
        state.downcase_word();
        assert_eq!(state.content, "hello world");
        // cursor should be after "hello" (pos 5)
        assert_eq!(state.cursor_pos, 5);
    }

    #[test]
    fn input_box_capitalize_word() {
        let mut state = InputBoxState::new();
        for c in "hello world".chars() {
            state.insert_char(c);
        }
        state.move_home();
        state.capitalize_word();
        assert_eq!(state.content, "Hello world");
        // cursor should be after "Hello" (pos 5)
        assert_eq!(state.cursor_pos, 5);
    }

    // ------------------------------------------------------------------
    // move_up / move_down tests
    // ------------------------------------------------------------------

    #[test]
    fn input_box_move_up() {
        let mut state = InputBoxState::new();
        for c in "abc\ndef\nghi".chars() {
            if c == '\n' {
                state.insert_newline();
            } else {
                state.insert_char(c);
            }
        }
        // cursor is at end of line 2 (after 'i'), col 3
        assert_eq!(state.cursor_pos, 11);

        state.move_up();
        // should be on line 1 at col 3 (after 'f')
        assert_eq!(state.cursor_pos, 7); // "abc\ndef" → 'f' is at byte 6, after it = 7
        assert_eq!(state.cursor_col, 3);

        state.move_up();
        // should be on line 0 at col 3 (after 'c')
        assert_eq!(state.cursor_pos, 3);
        assert_eq!(state.cursor_col, 3);

        // already on first line — no-op
        state.move_up();
        assert_eq!(state.cursor_pos, 3);
    }

    #[test]
    fn input_box_move_down() {
        let mut state = InputBoxState::new();
        for c in "abc\ndef\nghi".chars() {
            if c == '\n' {
                state.insert_newline();
            } else {
                state.insert_char(c);
            }
        }
        // move cursor to start (line 0, col 0)
        state.move_home();
        state.move_home(); // second call is a no-op but harmless
        // manually reset to absolute start
        state.cursor_pos = 0;
        state.cursor_col = 0;

        state.move_down();
        // should be on line 1 at col 0 (before 'd')
        assert_eq!(state.cursor_pos, 4); // "abc\n" = 4 bytes
        assert_eq!(state.cursor_col, 0);

        state.move_down();
        // should be on line 2 at col 0 (before 'g')
        assert_eq!(state.cursor_pos, 8); // "abc\ndef\n" = 8 bytes
        assert_eq!(state.cursor_col, 0);
    }

    #[test]
    fn input_box_move_up_clamps_column() {
        let mut state = InputBoxState::new();
        for c in "abcdef\nhi".chars() {
            if c == '\n' {
                state.insert_newline();
            } else {
                state.insert_char(c);
            }
        }
        // cursor is at end of line 1 (after 'i'), col 2
        // move_up should clamp to end of line 0 (col 6, after 'f')
        // but we want to test clamping: start at end of line 0 (col 6), move_down clamps to end of line 1 (col 2)
        state.cursor_pos = 6; // end of "abcdef"
        state.cursor_col = 6;

        state.move_down();
        // line 1 is "hi" (len 2), col 6 clamped to col 2
        assert_eq!(state.cursor_pos, 9); // "abcdef\nhi" = 9 bytes, end of "hi"
        assert_eq!(state.cursor_col, 2);
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
    fn test_autocomplete_entry_with_args_hint() {
        let entry = AutocompleteEntry::new("/session rename", "Rename session", "[builtin]")
            .with_args_hint("<name>");
        assert_eq!(entry.name, "/session rename");
        assert_eq!(entry.args_hint, Some("<name>".to_owned()));
    }

    #[test]
    fn test_autocomplete_entry_without_args_hint() {
        let entry = AutocompleteEntry::new("/session list", "List sessions", "[builtin]");
        assert_eq!(entry.args_hint, None);
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
    // insert_char / insert_str newline tests
    // ------------------------------------------------------------------

    #[test]
    fn insert_char_newline_resets_cursor_col() {
        let mut state = InputBoxState::new();
        state.insert_str("hello");
        assert_eq!(state.cursor_col, 5);
        state.insert_char('\n');
        assert_eq!(state.cursor_col, 0);
        assert_eq!(state.cursor_pos, 6); // "hello\n"
    }

    #[test]
    fn insert_str_with_newlines_tracks_cursor_col() {
        let mut state = InputBoxState::new();
        state.insert_str("abc\ndef");
        assert_eq!(state.cursor_col, 3); // on "def", col 3
        assert_eq!(state.line_count(), 2);
    }

    #[test]
    fn insert_str_multiple_newlines() {
        let mut state = InputBoxState::new();
        state.insert_str("a\nb\nc");
        assert_eq!(state.cursor_col, 1); // on "c", col 1
        assert_eq!(state.line_count(), 3);
    }

    // ------------------------------------------------------------------
    // scroll_offset / ensure_cursor_visible tests
    // ------------------------------------------------------------------

    #[test]
    fn ensure_cursor_visible_scrolls_down() {
        let mut state = InputBoxState::new();
        state.insert_str("1\n2\n3\n4\n5\n6\n7\n8");
        // Cursor is on line 7 (0-indexed). With max_visible=6, scroll_offset should be 2.
        state.scroll_offset = 0;
        state.ensure_cursor_visible(6);
        assert_eq!(state.scroll_offset, 2);
    }

    #[test]
    fn ensure_cursor_visible_scrolls_up() {
        let mut state = InputBoxState::new();
        state.insert_str("1\n2\n3\n4\n5\n6\n7\n8");
        state.scroll_offset = 5;
        state.move_home(); // cursor goes to start of current line
        // Move to first line
        for _ in 0..7 {
            state.move_up();
        }
        state.ensure_cursor_visible(6);
        assert_eq!(state.scroll_offset, 0);
    }

    #[test]
    fn scroll_offset_reset_on_clear() {
        let mut state = InputBoxState::new();
        state.insert_str("1\n2\n3\n4\n5\n6\n7");
        state.scroll_offset = 3;
        state.clear();
        assert_eq!(state.scroll_offset, 0);
    }

    // ------------------------------------------------------------------
    // Selection tests
    // ------------------------------------------------------------------

    #[test]
    fn selection_start_and_cancel() {
        let mut state = InputBoxState::new();
        state.insert_str("hello");
        assert!(!state.selecting);
        state.start_selecting();
        assert!(state.selecting);
        assert_eq!(state.selection_anchor, 5);
        state.cancel_selecting();
        assert!(!state.selecting);
    }

    #[test]
    fn ensure_selecting_idempotent() {
        let mut state = InputBoxState::new();
        state.insert_str("hello");
        state.ensure_selecting(); // starts at pos 5
        assert_eq!(state.selection_anchor, 5);
        state.move_left(); // cursor moves to 4
        state.ensure_selecting(); // should NOT reset anchor
        assert_eq!(state.selection_anchor, 5); // anchor unchanged
        assert!(state.selecting);
    }

    #[test]
    fn selected_range_none_when_not_selecting() {
        let mut state = InputBoxState::new();
        state.insert_str("hello");
        assert!(state.selected_range().is_none());
    }

    #[test]
    fn selected_range_none_when_anchor_equals_cursor() {
        let mut state = InputBoxState::new();
        state.insert_str("hello");
        state.start_selecting();
        // anchor == cursor_pos == 5
        assert!(state.selected_range().is_none());
    }

    #[test]
    fn selected_range_forward() {
        let mut state = InputBoxState::new();
        state.insert_str("hello world");
        // Position cursor at 5, start selecting, move to end
        state.move_home();
        for _ in 0..5 {
            state.move_right();
        }
        state.start_selecting(); // anchor at 5
        state.move_end(); // cursor at 11
        let range = state.selected_range().unwrap();
        assert_eq!(range, 5..11);
        assert_eq!(state.selected_text().unwrap(), " world");
    }

    #[test]
    fn selected_range_backward() {
        let mut state = InputBoxState::new();
        state.insert_str("hello world");
        state.start_selecting(); // anchor at 11 (end)
        state.move_home(); // cursor at 0
        let range = state.selected_range().unwrap();
        assert_eq!(range, 0..11);
        assert_eq!(state.selected_text().unwrap(), "hello world");
    }

    #[test]
    fn delete_selected_removes_text() {
        let mut state = InputBoxState::new();
        state.insert_str("hello world");
        state.move_home();
        for _ in 0..5 {
            state.move_right();
        }
        state.start_selecting();
        state.move_end();
        state.delete_selected();
        assert_eq!(state.content, "hello");
        assert_eq!(state.cursor_pos, 5);
        assert!(!state.selecting);
    }

    #[test]
    fn delete_selected_multiline() {
        let mut state = InputBoxState::new();
        state.insert_str("abc\ndef\nghi");
        // Select "def\n" (bytes 4..8)
        state.move_home();
        for _ in 0..3 {
            state.move_up();
        } // won't go past line 0
        state.move_home();
        // cursor at 0, move to byte 4
        for _ in 0..4 {
            state.move_right();
        }
        state.start_selecting(); // anchor at 4
        for _ in 0..4 {
            state.move_right();
        } // cursor at 8
        state.delete_selected();
        assert_eq!(state.content, "abc\nghi");
        assert_eq!(state.cursor_pos, 4);
        assert_eq!(state.cursor_col, 0); // start of "ghi" line
    }

    #[test]
    fn clear_cancels_selection() {
        let mut state = InputBoxState::new();
        state.insert_str("hello");
        state.start_selecting();
        state.clear();
        assert!(!state.selecting);
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
        let widget = InputBox::new(&state, &theme, true);

        // 1 line + 2 padding = 3
        let area = Rect::new(0, 0, 40, 3);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);

        // Collect all cell symbols into a single string.
        let rendered: String = (0..40u16)
            .flat_map(|x| (0..3u16).map(move |y| (x, y)))
            .map(|(x, y)| buf[(x, y)].symbol().to_string())
            .collect();

        assert!(
            rendered.contains('h'),
            "text 'hello' missing in rendered output"
        );
    }

    #[test]
    fn input_info_line_display_text_full() {
        let info = InputInfoLine {
            agent_name: "coder".into(),
            model_name: Some("claude-opus-4-6".into()),
            provider_name: Some("Anthropic".into()),
        };
        assert_eq!(info.display_text(), "@coder   claude-opus-4-6   Anthropic");
    }

    #[test]
    fn input_info_line_display_text_no_provider() {
        let info = InputInfoLine {
            agent_name: "coder".into(),
            model_name: Some("claude-opus-4-6".into()),
            provider_name: None,
        };
        assert_eq!(info.display_text(), "@coder   claude-opus-4-6");
    }

    #[test]
    fn input_info_line_display_text_not_connected() {
        let info = InputInfoLine {
            agent_name: "coder".into(),
            model_name: None,
            provider_name: None,
        };
        assert_eq!(info.display_text(), "@coder   [not connected]");
    }

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
        let area = Rect::new(0, 0, 80, 5);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);
        // Verify it doesn't panic and the info line content appears on the last row.
        let last_row: String = (0..80)
            .map(|x| buf[(x, 4u16)].symbol().to_owned())
            .collect::<String>();
        assert!(
            last_row.contains("@coder"),
            "expected @coder in last row: {last_row:?}"
        );
    }
}
