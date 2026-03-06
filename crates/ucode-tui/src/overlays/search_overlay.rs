use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use crate::app::TranscriptEntry;
use crate::keybinds::KeybindPreset;
use crate::theme::UcodeTheme;

// ---------------------------------------------------------------------------
// SearchMatch
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchMatch {
    pub transcript_index: usize,
    pub byte_offset: usize,
    pub length: usize,
}

// ---------------------------------------------------------------------------
// SearchOverlayState
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SearchOverlayState {
    pub visible: bool,
    pub query: String,
    pub cursor_pos: usize,
    pub matches: Vec<SearchMatch>,
    pub current_match: usize,
    pub case_sensitive: bool,
    pub preset: KeybindPreset,
}

impl SearchOverlayState {
    pub fn new() -> Self {
        Self {
            visible: false,
            query: String::new(),
            cursor_pos: 0,
            matches: Vec::new(),
            current_match: 0,
            case_sensitive: false,
            preset: KeybindPreset::default(),
        }
    }

    pub fn open(&mut self, preset: KeybindPreset) {
        self.visible = true;
        self.preset = preset;
        self.query.clear();
        self.cursor_pos = 0;
        self.matches.clear();
        self.current_match = 0;
    }

    pub fn hint_text(&self) -> &str {
        match self.preset {
            KeybindPreset::Vscode => "Esc: close  Enter/n: next  N: prev",
            KeybindPreset::Vim => "Esc/Enter: close  n: next  N: prev",
            KeybindPreset::Emacs => "C-g: close  C-s: next  C-r: prev",
        }
    }

    pub fn close(&mut self) {
        self.visible = false;
    }

    pub fn insert_char(&mut self, c: char) {
        let byte_pos = self
            .query
            .char_indices()
            .nth(self.cursor_pos)
            .map(|(i, _)| i)
            .unwrap_or(self.query.len());
        self.query.insert(byte_pos, c);
        self.cursor_pos += 1;
    }

    pub fn delete_char(&mut self) {
        if self.cursor_pos == 0 {
            return;
        }
        self.cursor_pos -= 1;
        let byte_pos = self
            .query
            .char_indices()
            .nth(self.cursor_pos)
            .map(|(i, _)| i)
            .unwrap_or(self.query.len());
        self.query.remove(byte_pos);
    }

    /// Find all occurrences of the current query in the transcript.
    /// Case-insensitive by default (unless `case_sensitive` is true).
    pub fn search(&mut self, transcript: &[TranscriptEntry]) {
        self.matches.clear();
        self.current_match = 0;

        if self.query.is_empty() {
            return;
        }

        let needle_lower = self.query.to_lowercase();

        for (idx, entry) in transcript.iter().enumerate() {
            let text = entry_text(entry);
            let haystack = if self.case_sensitive {
                text.to_owned()
            } else {
                text.to_lowercase()
            };
            let needle = if self.case_sensitive {
                self.query.as_str()
            } else {
                needle_lower.as_str()
            };

            let mut search_start = 0;
            while let Some(pos) = haystack[search_start..].find(needle) {
                let byte_offset = search_start + pos;
                self.matches.push(SearchMatch {
                    transcript_index: idx,
                    byte_offset,
                    length: needle.len(),
                });
                search_start = byte_offset + needle.len();
                if search_start >= haystack.len() {
                    break;
                }
            }
        }
    }

    pub fn next_match(&mut self) {
        if self.matches.is_empty() {
            return;
        }
        self.current_match = (self.current_match + 1) % self.matches.len();
    }

    pub fn prev_match(&mut self) {
        if self.matches.is_empty() {
            return;
        }
        if self.current_match == 0 {
            self.current_match = self.matches.len() - 1;
        } else {
            self.current_match -= 1;
        }
    }

    pub fn match_count(&self) -> usize {
        self.matches.len()
    }

    pub fn current_match_info(&self) -> Option<&SearchMatch> {
        if self.matches.is_empty() {
            return None;
        }
        self.matches.get(self.current_match)
    }

    pub fn status_text(&self) -> String {
        if self.query.is_empty() {
            return String::new();
        }
        if self.matches.is_empty() {
            return "No matches".to_owned();
        }
        format!("{}/{} matches", self.current_match + 1, self.matches.len())
    }
}

impl Default for SearchOverlayState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Helper: extract searchable text from a TranscriptEntry
// ---------------------------------------------------------------------------

fn entry_text(entry: &TranscriptEntry) -> &str {
    match entry {
        TranscriptEntry::UserMessage(s) => s.as_str(),
        TranscriptEntry::AssistantMessage(s) => s.as_str(),
        TranscriptEntry::Streaming(msg) => msg.content.as_str(),
        TranscriptEntry::ToolCall { name, .. } => name.as_str(),
        TranscriptEntry::RouterEvent(s) => s.as_str(),
        TranscriptEntry::SystemMessage(s) => s.as_str(),
        TranscriptEntry::PatchProposed { file_path, .. } => file_path.as_str(),
    }
}

// ---------------------------------------------------------------------------
// SearchOverlay widget
// ---------------------------------------------------------------------------

pub struct SearchOverlay<'a> {
    pub state: &'a SearchOverlayState,
    pub theme: &'a UcodeTheme,
}

impl<'a> SearchOverlay<'a> {
    pub fn new(state: &'a SearchOverlayState, theme: &'a UcodeTheme) -> Self {
        Self { state, theme }
    }
}

impl Widget for SearchOverlay<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        let status = self.state.status_text();
        let hints = self.state.hint_text();

        let line = Line::from(vec![
            Span::styled("\u{1f50d} ", self.theme.accent_style()),
            Span::styled(self.state.query.clone(), self.theme.text_style()),
            Span::raw("  "),
            Span::styled(status, self.theme.dim_style()),
            Span::raw("  "),
            Span::styled(hints, self.theme.muted_style()),
        ]);

        Paragraph::new(line).render(area, buf);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{StreamingMessage, TranscriptEntry};
    use crate::keybinds::KeybindPreset;

    fn make_transcript() -> Vec<TranscriptEntry> {
        vec![
            TranscriptEntry::UserMessage("Hello world".to_owned()),
            TranscriptEntry::AssistantMessage("The quick brown fox".to_owned()),
            TranscriptEntry::RouterEvent("hello from router".to_owned()),
            TranscriptEntry::SystemMessage("System hello".to_owned()),
        ]
    }

    // -----------------------------------------------------------------------
    // SearchOverlayState::new
    // -----------------------------------------------------------------------

    #[test]
    fn new_defaults() {
        let state = SearchOverlayState::new();
        assert!(!state.visible);
        assert!(state.query.is_empty());
        assert_eq!(state.cursor_pos, 0);
        assert!(state.matches.is_empty());
        assert_eq!(state.current_match, 0);
        assert!(!state.case_sensitive);
    }

    // -----------------------------------------------------------------------
    // open / close
    // -----------------------------------------------------------------------

    #[test]
    fn open_sets_visible_and_clears_query() {
        let mut state = SearchOverlayState::new();
        state.query = "old query".to_owned();
        state.cursor_pos = 5;
        state.open(KeybindPreset::default());
        assert!(state.visible);
        assert!(state.query.is_empty());
        assert_eq!(state.cursor_pos, 0);
        assert!(state.matches.is_empty());
    }

    #[test]
    fn close_hides_overlay() {
        let mut state = SearchOverlayState::new();
        state.open(KeybindPreset::default());
        assert!(state.visible);
        state.close();
        assert!(!state.visible);
    }

    // -----------------------------------------------------------------------
    // preset / hint_text
    // -----------------------------------------------------------------------

    #[test]
    fn open_stores_preset() {
        let mut state = SearchOverlayState::new();
        state.open(KeybindPreset::Vim);
        assert_eq!(state.preset, KeybindPreset::Vim);
        state.open(KeybindPreset::Emacs);
        assert_eq!(state.preset, KeybindPreset::Emacs);
    }

    #[test]
    fn hint_text_vscode() {
        let mut state = SearchOverlayState::new();
        state.open(KeybindPreset::Vscode);
        assert!(state.hint_text().contains("Enter"));
        assert!(state.hint_text().contains("next"));
    }

    #[test]
    fn hint_text_vim() {
        let mut state = SearchOverlayState::new();
        state.open(KeybindPreset::Vim);
        assert!(state.hint_text().contains("Enter"));
        assert!(state.hint_text().contains("close"));
    }

    #[test]
    fn hint_text_emacs() {
        let mut state = SearchOverlayState::new();
        state.open(KeybindPreset::Emacs);
        assert!(state.hint_text().contains("C-s"));
        assert!(state.hint_text().contains("C-r"));
        assert!(state.hint_text().contains("C-g"));
    }

    // -----------------------------------------------------------------------
    // insert_char / delete_char
    // -----------------------------------------------------------------------

    #[test]
    fn insert_char_appends_and_advances_cursor() {
        let mut state = SearchOverlayState::new();
        state.insert_char('h');
        state.insert_char('i');
        assert_eq!(state.query, "hi");
        assert_eq!(state.cursor_pos, 2);
    }

    #[test]
    fn delete_char_removes_last_char() {
        let mut state = SearchOverlayState::new();
        state.insert_char('h');
        state.insert_char('i');
        state.delete_char();
        assert_eq!(state.query, "h");
        assert_eq!(state.cursor_pos, 1);
    }

    #[test]
    fn delete_char_at_start_is_noop() {
        let mut state = SearchOverlayState::new();
        state.delete_char(); // should not panic
        assert!(state.query.is_empty());
        assert_eq!(state.cursor_pos, 0);
    }

    #[test]
    fn delete_char_empties_single_char_query() {
        let mut state = SearchOverlayState::new();
        state.insert_char('x');
        state.delete_char();
        assert!(state.query.is_empty());
        assert_eq!(state.cursor_pos, 0);
    }

    // -----------------------------------------------------------------------
    // search — basic
    // -----------------------------------------------------------------------

    #[test]
    fn search_finds_matches_in_transcript() {
        let mut state = SearchOverlayState::new();
        state.insert_char('h');
        state.insert_char('e');
        state.insert_char('l');
        state.insert_char('l');
        state.insert_char('o');
        let transcript = make_transcript();
        state.search(&transcript);
        // "hello" appears in: UserMessage("Hello world"), RouterEvent("hello from router"),
        // SystemMessage("System hello") — 3 matches (case-insensitive)
        assert_eq!(state.match_count(), 3);
    }

    #[test]
    fn search_case_insensitive_by_default() {
        let mut state = SearchOverlayState::new();
        // Query lowercase, transcript has uppercase "Hello"
        state.insert_char('h');
        state.insert_char('e');
        state.insert_char('l');
        state.insert_char('l');
        state.insert_char('o');
        let transcript = vec![TranscriptEntry::UserMessage("Hello World".to_owned())];
        state.search(&transcript);
        assert_eq!(state.match_count(), 1);
    }

    #[test]
    fn search_with_no_matches_returns_empty() {
        let mut state = SearchOverlayState::new();
        state.insert_char('z');
        state.insert_char('z');
        state.insert_char('z');
        let transcript = make_transcript();
        state.search(&transcript);
        assert_eq!(state.match_count(), 0);
    }

    #[test]
    fn search_empty_query_returns_no_matches() {
        let mut state = SearchOverlayState::new();
        let transcript = make_transcript();
        state.search(&transcript);
        assert_eq!(state.match_count(), 0);
    }

    #[test]
    fn search_resets_current_match_to_zero() {
        let mut state = SearchOverlayState::new();
        state.insert_char('h');
        state.insert_char('e');
        state.insert_char('l');
        state.insert_char('l');
        state.insert_char('o');
        let transcript = make_transcript();
        state.search(&transcript);
        state.next_match();
        assert_eq!(state.current_match, 1);
        // Re-search resets to 0
        state.search(&transcript);
        assert_eq!(state.current_match, 0);
    }

    #[test]
    fn search_all_entry_variants() {
        let mut msg = StreamingMessage::new();
        msg.push_token("streaming needle");
        let transcript = vec![
            TranscriptEntry::UserMessage("needle in user".to_owned()),
            TranscriptEntry::AssistantMessage("needle in assistant".to_owned()),
            TranscriptEntry::Streaming(msg),
            TranscriptEntry::ToolCall {
                name: "needle_tool".to_owned(),
                status: crate::app::ToolCallStatus::Running,
                duration_ms: None,
                summary: None,
                thinking: None,
                output: None,
            },
            TranscriptEntry::RouterEvent("needle router".to_owned()),
            TranscriptEntry::SystemMessage("needle system".to_owned()),
            TranscriptEntry::PatchProposed {
                file_path: "needle/path.rs".to_owned(),
                raw_diff: "+added".to_owned(),
                patch_id: None,
            },
        ];
        let mut state = SearchOverlayState::new();
        for c in "needle".chars() {
            state.insert_char(c);
        }
        state.search(&transcript);
        // One match per entry variant = 7
        assert_eq!(state.match_count(), 7);
    }

    #[test]
    fn search_stores_correct_byte_offset_and_length() {
        let mut state = SearchOverlayState::new();
        for c in "fox".chars() {
            state.insert_char(c);
        }
        let transcript = vec![TranscriptEntry::AssistantMessage(
            "The quick brown fox".to_owned(),
        )];
        state.search(&transcript);
        assert_eq!(state.match_count(), 1);
        let m = &state.matches[0];
        assert_eq!(m.transcript_index, 0);
        assert_eq!(m.byte_offset, 16); // "The quick brown " = 16 bytes
        assert_eq!(m.length, 3); // "fox"
    }

    // -----------------------------------------------------------------------
    // next_match / prev_match
    // -----------------------------------------------------------------------

    #[test]
    fn next_match_advances() {
        let mut state = SearchOverlayState::new();
        for c in "hello".chars() {
            state.insert_char(c);
        }
        let transcript = make_transcript();
        state.search(&transcript);
        assert_eq!(state.current_match, 0);
        state.next_match();
        assert_eq!(state.current_match, 1);
    }

    #[test]
    fn next_match_wraps_around() {
        let mut state = SearchOverlayState::new();
        for c in "hello".chars() {
            state.insert_char(c);
        }
        let transcript = make_transcript();
        state.search(&transcript);
        let count = state.match_count();
        // Advance to last match
        for _ in 0..count - 1 {
            state.next_match();
        }
        assert_eq!(state.current_match, count - 1);
        // One more wraps to 0
        state.next_match();
        assert_eq!(state.current_match, 0);
    }

    #[test]
    fn prev_match_goes_back() {
        let mut state = SearchOverlayState::new();
        for c in "hello".chars() {
            state.insert_char(c);
        }
        let transcript = make_transcript();
        state.search(&transcript);
        state.next_match();
        assert_eq!(state.current_match, 1);
        state.prev_match();
        assert_eq!(state.current_match, 0);
    }

    #[test]
    fn prev_match_wraps_around() {
        let mut state = SearchOverlayState::new();
        for c in "hello".chars() {
            state.insert_char(c);
        }
        let transcript = make_transcript();
        state.search(&transcript);
        let count = state.match_count();
        // At 0, prev wraps to last
        state.prev_match();
        assert_eq!(state.current_match, count - 1);
    }

    #[test]
    fn next_match_noop_when_no_matches() {
        let mut state = SearchOverlayState::new();
        state.next_match(); // should not panic
        assert_eq!(state.current_match, 0);
    }

    #[test]
    fn prev_match_noop_when_no_matches() {
        let mut state = SearchOverlayState::new();
        state.prev_match(); // should not panic
        assert_eq!(state.current_match, 0);
    }

    // -----------------------------------------------------------------------
    // current_match_info
    // -----------------------------------------------------------------------

    #[test]
    fn current_match_info_returns_none_when_no_matches() {
        let state = SearchOverlayState::new();
        assert!(state.current_match_info().is_none());
    }

    #[test]
    fn current_match_info_returns_correct_match() {
        let mut state = SearchOverlayState::new();
        for c in "hello".chars() {
            state.insert_char(c);
        }
        let transcript = make_transcript();
        state.search(&transcript);
        let info = state.current_match_info().expect("should have a match");
        assert_eq!(info.transcript_index, 0); // first "hello" is in UserMessage at index 0
    }

    // -----------------------------------------------------------------------
    // status_text
    // -----------------------------------------------------------------------

    #[test]
    fn status_text_empty_when_no_query() {
        let state = SearchOverlayState::new();
        assert_eq!(state.status_text(), "");
    }

    #[test]
    fn status_text_no_matches() {
        let mut state = SearchOverlayState::new();
        for c in "zzz".chars() {
            state.insert_char(c);
        }
        let transcript = make_transcript();
        state.search(&transcript);
        assert_eq!(state.status_text(), "No matches");
    }

    #[test]
    fn status_text_with_matches() {
        let mut state = SearchOverlayState::new();
        for c in "hello".chars() {
            state.insert_char(c);
        }
        let transcript = make_transcript();
        state.search(&transcript);
        // current_match=0, total=3
        assert_eq!(state.status_text(), "1/3 matches");
    }

    #[test]
    fn status_text_updates_after_next_match() {
        let mut state = SearchOverlayState::new();
        for c in "hello".chars() {
            state.insert_char(c);
        }
        let transcript = make_transcript();
        state.search(&transcript);
        state.next_match();
        assert_eq!(state.status_text(), "2/3 matches");
    }

    // -----------------------------------------------------------------------
    // Widget rendering
    // -----------------------------------------------------------------------

    #[test]
    fn search_overlay_renders_without_panic() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let backend = TestBackend::new(120, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = SearchOverlayState::new();
        let theme = UcodeTheme::default();

        terminal
            .draw(|f| {
                f.render_widget(SearchOverlay::new(&state, &theme), f.area());
            })
            .unwrap();
    }

    #[test]
    fn search_overlay_renders_with_query_without_panic() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let backend = TestBackend::new(120, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = SearchOverlayState::new();
        state.open(KeybindPreset::default());
        for c in "hello".chars() {
            state.insert_char(c);
        }
        let transcript = make_transcript();
        state.search(&transcript);
        let theme = UcodeTheme::default();

        terminal
            .draw(|f| {
                f.render_widget(SearchOverlay::new(&state, &theme), f.area());
            })
            .unwrap();
    }

    #[test]
    fn search_overlay_renders_zero_size_without_panic() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let backend = TestBackend::new(1, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = SearchOverlayState::new();
        let theme = UcodeTheme::default();

        terminal
            .draw(|f| {
                let zero = Rect::new(0, 0, 0, 0);
                f.render_widget(SearchOverlay::new(&state, &theme), zero);
            })
            .unwrap();
    }
}
