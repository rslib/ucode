use crate::app::TranscriptEntry;

// ---------------------------------------------------------------------------
// CopyModeState
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct CopyModeState {
    pub active: bool,
    /// Index of the anchor (start) transcript entry.
    pub anchor: usize,
    /// Index of the cursor (current) transcript entry.
    pub cursor: usize,
}

impl CopyModeState {
    pub fn new() -> Self {
        Self {
            active: false,
            anchor: 0,
            cursor: 0,
        }
    }

    /// Enter copy mode, anchoring at the given transcript index.
    pub fn enter(&mut self, transcript_index: usize) {
        self.active = true;
        self.anchor = transcript_index;
        self.cursor = transcript_index;
    }

    /// Exit copy mode.
    pub fn exit(&mut self) {
        self.active = false;
    }

    /// Move cursor up (toward index 0).
    pub fn move_up(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    /// Move cursor down (toward end of transcript).
    pub fn move_down(&mut self, max_index: usize) {
        if self.cursor < max_index {
            self.cursor += 1;
        }
    }

    /// Returns the inclusive range of selected transcript indices (start, end).
    pub fn selection_range(&self) -> (usize, usize) {
        let start = self.anchor.min(self.cursor);
        let end = self.anchor.max(self.cursor);
        (start, end)
    }

    /// Returns true if the given transcript index is within the selection.
    pub fn is_selected(&self, index: usize) -> bool {
        let (start, end) = self.selection_range();
        index >= start && index <= end
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

    // --- CopyModeState ---

    #[test]
    fn new_defaults() {
        let state = CopyModeState::new();
        assert!(!state.active);
        assert_eq!(state.anchor, 0);
        assert_eq!(state.cursor, 0);
    }

    #[test]
    fn enter_activates_and_sets_anchor() {
        let mut state = CopyModeState::new();
        state.enter(5);
        assert!(state.active);
        assert_eq!(state.anchor, 5);
        assert_eq!(state.cursor, 5);
    }

    #[test]
    fn exit_deactivates() {
        let mut state = CopyModeState::new();
        state.enter(3);
        state.exit();
        assert!(!state.active);
    }

    #[test]
    fn move_up_decrements_cursor() {
        let mut state = CopyModeState::new();
        state.enter(5);
        state.move_up();
        assert_eq!(state.cursor, 4);
        assert_eq!(state.anchor, 5); // anchor unchanged
    }

    #[test]
    fn move_up_stops_at_zero() {
        let mut state = CopyModeState::new();
        state.enter(0);
        state.move_up();
        assert_eq!(state.cursor, 0);
    }

    #[test]
    fn move_down_increments_cursor() {
        let mut state = CopyModeState::new();
        state.enter(3);
        state.move_down(10);
        assert_eq!(state.cursor, 4);
    }

    #[test]
    fn move_down_stops_at_max() {
        let mut state = CopyModeState::new();
        state.enter(10);
        state.move_down(10);
        assert_eq!(state.cursor, 10);
    }

    #[test]
    fn selection_range_anchor_before_cursor() {
        let mut state = CopyModeState::new();
        state.enter(2);
        state.move_down(10);
        state.move_down(10);
        assert_eq!(state.selection_range(), (2, 4));
    }

    #[test]
    fn selection_range_cursor_before_anchor() {
        let mut state = CopyModeState::new();
        state.enter(5);
        state.move_up();
        state.move_up();
        assert_eq!(state.selection_range(), (3, 5));
    }

    #[test]
    fn selection_range_same_position() {
        let mut state = CopyModeState::new();
        state.enter(3);
        assert_eq!(state.selection_range(), (3, 3));
    }

    #[test]
    fn is_selected_within_range() {
        let mut state = CopyModeState::new();
        state.enter(2);
        state.move_down(10);
        state.move_down(10);
        // Range is 2..=4
        assert!(!state.is_selected(1));
        assert!(state.is_selected(2));
        assert!(state.is_selected(3));
        assert!(state.is_selected(4));
        assert!(!state.is_selected(5));
    }

    // --- entry_to_copy_text ---

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

    // --- collect_selection_text ---

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
        // Entries separated by double newline
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
        // end=10 but transcript has only 1 entry
        let text = collect_selection_text(&transcript, 0, 10);
        assert_eq!(text, "User: only");
    }
}
