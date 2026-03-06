use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

use crate::theme::UcodeTheme;

// ---------------------------------------------------------------------------
// DiffLine
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffLine {
    Header(String),
    Context(String),
    Added(String),
    Removed(String),
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

pub fn parse_unified_diff(raw: &str) -> Vec<DiffLine> {
    raw.lines()
        .map(|line| {
            if line.starts_with("@@") || line.starts_with("---") || line.starts_with("+++") {
                DiffLine::Header(line.to_owned())
            } else if let Some(rest) = line.strip_prefix('+') {
                DiffLine::Added(rest.to_owned())
            } else if let Some(rest) = line.strip_prefix('-') {
                DiffLine::Removed(rest.to_owned())
            } else {
                // Strip a single leading space if present (unified diff context lines).
                let content = line.strip_prefix(' ').unwrap_or(line);
                DiffLine::Context(content.to_owned())
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// ApprovalDecision
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    Approved,
    Rejected,
}

// ---------------------------------------------------------------------------
// DiffModalState
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct DiffModalState {
    pub visible: bool,
    pub file_path: String,
    pub diff_lines: Vec<DiffLine>,
    pub scroll_offset: usize,
    pub patch_id: Option<String>,
}

impl DiffModalState {
    pub fn new() -> Self {
        Self {
            visible: false,
            file_path: String::new(),
            diff_lines: Vec::new(),
            scroll_offset: 0,
            patch_id: None,
        }
    }

    pub fn open(&mut self, file_path: String, raw_diff: &str, patch_id: Option<String>) {
        self.file_path = file_path;
        self.diff_lines = parse_unified_diff(raw_diff);
        self.scroll_offset = 0;
        self.patch_id = patch_id;
        self.visible = true;
    }

    /// Close without a decision.
    pub fn close(&mut self) -> Option<ApprovalDecision> {
        self.visible = false;
        None
    }

    pub fn approve(&mut self) -> Option<ApprovalDecision> {
        self.visible = false;
        Some(ApprovalDecision::Approved)
    }

    pub fn reject(&mut self) -> Option<ApprovalDecision> {
        self.visible = false;
        Some(ApprovalDecision::Rejected)
    }

    pub fn scroll_up(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
    }

    pub fn scroll_down(&mut self, lines: usize) {
        let max = self.total_lines().saturating_sub(1);
        self.scroll_offset = (self.scroll_offset + lines).min(max);
    }

    pub fn total_lines(&self) -> usize {
        self.diff_lines.len()
    }
}

impl Default for DiffModalState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// DiffModal widget
// ---------------------------------------------------------------------------

pub struct DiffModal<'a> {
    state: &'a DiffModalState,
    theme: &'a UcodeTheme,
}

impl<'a> DiffModal<'a> {
    pub fn new(state: &'a DiffModalState, theme: &'a UcodeTheme) -> Self {
        Self { state, theme }
    }
}

impl Widget for DiffModal<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let modal_area = centered_rect(80, 70, area);

        Clear.render(modal_area, buf);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border_style(true));
        let inner = block.inner(modal_area);
        block.render(modal_area, buf);

        if inner.height < 4 || inner.width < 10 {
            return;
        }

        // Layout: title row | separator | diff content | separator | action bar
        let chunks = Layout::vertical([
            Constraint::Length(1), // title
            Constraint::Length(1), // separator
            Constraint::Min(1),    // diff content
            Constraint::Length(1), // separator
            Constraint::Length(1), // action bar
        ])
        .split(inner);

        // --- Title row ---
        let title_text = format!("apply_patch — {}", self.state.file_path);
        let warning_label = "⚠ approval";
        let title_width = chunks[0].width as usize;
        let pad = title_width
            .saturating_sub(title_text.len() + warning_label.len())
            .max(1);

        let title_line = Line::from(vec![
            Span::styled(
                title_text,
                self.theme.text_style().add_modifier(Modifier::BOLD),
            ),
            Span::raw(" ".repeat(pad)),
            Span::styled(warning_label, self.theme.warning_style()),
        ]);
        Paragraph::new(title_line).render(chunks[0], buf);

        // --- Top separator ---
        let sep = "─".repeat(chunks[1].width as usize);
        Paragraph::new(sep.as_str())
            .style(self.theme.dim_style())
            .render(chunks[1], buf);

        // --- Diff content ---
        let content_area = chunks[2];
        let visible_height = content_area.height as usize;
        let offset = self.state.scroll_offset;

        for (i, diff_line) in self
            .state
            .diff_lines
            .iter()
            .enumerate()
            .skip(offset)
            .take(visible_height)
        {
            let y = content_area.y + (i - offset) as u16;
            let row_rect = Rect::new(content_area.x, y, content_area.width, 1);

            let line = match diff_line {
                DiffLine::Header(text) => {
                    Line::from(Span::styled(text.as_str(), self.theme.dim_style()))
                }
                DiffLine::Added(text) => Line::from(vec![
                    Span::styled("+", self.theme.safe_style()),
                    Span::styled(text.as_str(), self.theme.safe_style()),
                ]),
                DiffLine::Removed(text) => Line::from(vec![
                    Span::styled("-", self.theme.danger_style()),
                    Span::styled(text.as_str(), self.theme.danger_style()),
                ]),
                DiffLine::Context(text) => Line::from(vec![
                    Span::raw(" "),
                    Span::styled(text.as_str(), self.theme.text_style()),
                ]),
            };

            Paragraph::new(line).render(row_rect, buf);
        }

        // --- Bottom separator ---
        let sep2 = "─".repeat(chunks[3].width as usize);
        Paragraph::new(sep2.as_str())
            .style(self.theme.dim_style())
            .render(chunks[3], buf);

        // --- Action bar ---
        let action_line = Line::from(vec![
            Span::styled("[a]", self.theme.accent_style()),
            Span::styled(" apply    ", self.theme.muted_style()),
            Span::styled("[r]", self.theme.accent_style()),
            Span::styled(" reject    ", self.theme.muted_style()),
            Span::styled("esc cancel", self.theme.muted_style()),
        ]);
        Paragraph::new(action_line).render(chunks[4], buf);
    }
}

// ---------------------------------------------------------------------------
// Layout helper (pub(crate) so diff_modal and palette can share it later)
// ---------------------------------------------------------------------------

/// Compute a centered rectangle of the given percentage width/height within `area`.
pub(crate) fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_width = area.width * percent_x / 100;
    let popup_height = area.height * percent_y / 100;
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    Rect::new(x, y, popup_width, popup_height)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_diff() {
        assert!(parse_unified_diff("").is_empty());
    }

    #[test]
    fn parse_simple_diff() {
        let raw = "\
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,4 +1,5 @@
 fn foo() {
-    let x = 1;
+    let x = 2;
+    let y = 3;
 }";
        let lines = parse_unified_diff(raw);
        assert_eq!(lines[0], DiffLine::Header("--- a/src/lib.rs".to_owned()));
        assert_eq!(lines[1], DiffLine::Header("+++ b/src/lib.rs".to_owned()));
        assert_eq!(lines[2], DiffLine::Header("@@ -1,4 +1,5 @@".to_owned()));
        assert_eq!(lines[3], DiffLine::Context("fn foo() {".to_owned()));
        assert_eq!(lines[4], DiffLine::Removed("    let x = 1;".to_owned()));
        assert_eq!(lines[5], DiffLine::Added("    let x = 2;".to_owned()));
        assert_eq!(lines[6], DiffLine::Added("    let y = 3;".to_owned()));
        assert_eq!(lines[7], DiffLine::Context("}".to_owned()));
    }

    #[test]
    fn parse_diff_strips_prefixes() {
        let raw = "+added line\n-removed line\n context line";
        let lines = parse_unified_diff(raw);
        assert_eq!(lines[0], DiffLine::Added("added line".to_owned()));
        assert_eq!(lines[1], DiffLine::Removed("removed line".to_owned()));
        assert_eq!(lines[2], DiffLine::Context("context line".to_owned()));
    }

    #[test]
    fn parse_diff_file_headers() {
        let raw = "--- a/foo.rs\n+++ b/foo.rs";
        let lines = parse_unified_diff(raw);
        assert_eq!(lines[0], DiffLine::Header("--- a/foo.rs".to_owned()));
        assert_eq!(lines[1], DiffLine::Header("+++ b/foo.rs".to_owned()));
    }

    #[test]
    fn diff_modal_state_open_close() {
        let mut state = DiffModalState::new();
        assert!(!state.visible);

        state.open(
            "src/lib.rs".to_owned(),
            "+added\n-removed",
            Some("patch-1".to_owned()),
        );
        assert!(state.visible);
        assert_eq!(state.file_path, "src/lib.rs");
        assert_eq!(state.patch_id.as_deref(), Some("patch-1"));
        assert_eq!(state.scroll_offset, 0);
        assert_eq!(state.total_lines(), 2);

        let decision = state.close();
        assert!(!state.visible);
        assert!(decision.is_none());
    }

    #[test]
    fn diff_modal_state_approve() {
        let mut state = DiffModalState::new();
        state.open("src/lib.rs".to_owned(), "+line", None);
        let decision = state.approve();
        assert!(!state.visible);
        assert_eq!(decision, Some(ApprovalDecision::Approved));
    }

    #[test]
    fn diff_modal_state_reject() {
        let mut state = DiffModalState::new();
        state.open("src/lib.rs".to_owned(), "-line", None);
        let decision = state.reject();
        assert!(!state.visible);
        assert_eq!(decision, Some(ApprovalDecision::Rejected));
    }

    #[test]
    fn diff_modal_state_scroll() {
        let raw = (0..20)
            .map(|i| format!("+line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut state = DiffModalState::new();
        state.open("f.rs".to_owned(), &raw, None);
        assert_eq!(state.total_lines(), 20);

        // scroll_up at zero stays at zero
        state.scroll_up(5);
        assert_eq!(state.scroll_offset, 0);

        // scroll_down moves forward
        state.scroll_down(5);
        assert_eq!(state.scroll_offset, 5);

        // scroll_up moves back
        state.scroll_up(3);
        assert_eq!(state.scroll_offset, 2);

        // scroll_down clamps at max (total_lines - 1 = 19)
        state.scroll_down(100);
        assert_eq!(state.scroll_offset, 19);
    }

    #[test]
    fn diff_modal_renders_without_panic() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = DiffModalState::new();
        let theme = UcodeTheme::default();

        terminal
            .draw(|f| {
                f.render_widget(DiffModal::new(&state, &theme), f.area());
            })
            .unwrap();
    }

    #[test]
    fn diff_modal_renders_with_content() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let raw = "\
--- a/crates/ucode-auth/src/lib.rs
+++ b/crates/ucode-auth/src/lib.rs
@@ -42,8 +42,24 @@ impl AuthClient {
-    async fn refresh_token(&self) -> Result<Token> {
+    async fn refresh_token(&self) -> Result<Token> {
+        let refresher = TokenRefresher::new(self.http.clone());
     }";

        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = DiffModalState::new();
        state.open(
            "crates/ucode-auth/src/lib.rs".to_owned(),
            raw,
            Some("patch-42".to_owned()),
        );
        let theme = UcodeTheme::default();

        terminal
            .draw(|f| {
                f.render_widget(DiffModal::new(&state, &theme), f.area());
            })
            .unwrap();
    }
}
