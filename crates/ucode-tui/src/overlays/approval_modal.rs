use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

use crate::theme::UcodeTheme;

// ---------------------------------------------------------------------------
// ApprovalType
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalType {
    RunCmd,
    ApplyPatch,
    FileAccess,
    NetworkAccess,
}

impl ApprovalType {
    pub fn label(self) -> &'static str {
        match self {
            Self::RunCmd => "run_cmd",
            Self::ApplyPatch => "apply_patch",
            Self::FileAccess => "file access",
            Self::NetworkAccess => "network access",
        }
    }
}

// ---------------------------------------------------------------------------
// ApprovalScope
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalScope {
    Once,
    Session,
    Denied,
}

// ---------------------------------------------------------------------------
// ApprovalModalState
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ApprovalModalState {
    pub visible: bool,
    pub approval_type: ApprovalType,
    pub tool_name: String,
    pub command: Option<String>,
    pub cwd: Option<String>,
    pub file_path: Option<String>,
    pub sandbox_label: String,
    pub tool_call_index: Option<usize>,
}

impl ApprovalModalState {
    pub fn new() -> Self {
        Self {
            visible: false,
            approval_type: ApprovalType::RunCmd,
            tool_name: String::new(),
            command: None,
            cwd: None,
            file_path: None,
            sandbox_label: String::new(),
            tool_call_index: None,
        }
    }

    pub fn open_run_cmd(
        &mut self,
        tool_name: String,
        command: String,
        cwd: String,
        sandbox_label: String,
        tool_call_index: Option<usize>,
    ) {
        self.approval_type = ApprovalType::RunCmd;
        self.tool_name = tool_name;
        self.command = Some(command);
        self.cwd = Some(cwd);
        self.file_path = None;
        self.sandbox_label = sandbox_label;
        self.tool_call_index = tool_call_index;
        self.visible = true;
    }

    pub fn open_file_access(
        &mut self,
        tool_name: String,
        file_path: String,
        sandbox_label: String,
        tool_call_index: Option<usize>,
    ) {
        self.approval_type = ApprovalType::FileAccess;
        self.tool_name = tool_name;
        self.command = None;
        self.cwd = None;
        self.file_path = Some(file_path);
        self.sandbox_label = sandbox_label;
        self.tool_call_index = tool_call_index;
        self.visible = true;
    }

    pub fn open_network_access(
        &mut self,
        tool_name: String,
        sandbox_label: String,
        tool_call_index: Option<usize>,
    ) {
        self.approval_type = ApprovalType::NetworkAccess;
        self.tool_name = tool_name;
        self.command = None;
        self.cwd = None;
        self.file_path = None;
        self.sandbox_label = sandbox_label;
        self.tool_call_index = tool_call_index;
        self.visible = true;
    }

    pub fn close(&mut self) {
        self.visible = false;
    }

    pub fn approve_once(&mut self) -> ApprovalScope {
        self.visible = false;
        ApprovalScope::Once
    }

    pub fn approve_session(&mut self) -> ApprovalScope {
        self.visible = false;
        ApprovalScope::Session
    }

    pub fn deny(&mut self) -> ApprovalScope {
        self.visible = false;
        ApprovalScope::Denied
    }
}

impl Default for ApprovalModalState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// ApprovalModal widget
// ---------------------------------------------------------------------------

pub struct ApprovalModal<'a> {
    state: &'a ApprovalModalState,
    theme: &'a UcodeTheme,
}

impl<'a> ApprovalModal<'a> {
    pub fn new(state: &'a ApprovalModalState, theme: &'a UcodeTheme) -> Self {
        Self { state, theme }
    }
}

impl Widget for ApprovalModal<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let modal_area = crate::overlays::diff_modal::centered_rect(60, 40, area);

        Clear.render(modal_area, buf);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border_style(true));
        let inner = block.inner(modal_area);
        block.render(modal_area, buf);

        if inner.height < 4 || inner.width < 10 {
            return;
        }

        // Count detail lines to size the content area correctly.
        let detail_count = self.detail_line_count();

        let chunks = Layout::vertical([
            Constraint::Length(1),                   // title
            Constraint::Length(1),                   // separator
            Constraint::Length(detail_count as u16), // detail lines
            Constraint::Length(1),                   // separator
            Constraint::Length(1),                   // action bar
        ])
        .split(inner);

        // --- Title row ---
        let title_text = format!("{} — approval required", self.state.approval_type.label());
        let warning_label = "⚠";
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

        // --- Detail lines ---
        let detail_area = chunks[2];
        let mut row = 0u16;

        if let Some(cmd) = &self.state.command {
            let line = Line::from(vec![
                Span::styled("  command:  ", self.theme.dim_style()),
                Span::styled(cmd.as_str(), self.theme.text_style()),
            ]);
            Paragraph::new(line).render(
                Rect::new(detail_area.x, detail_area.y + row, detail_area.width, 1),
                buf,
            );
            row += 1;
        }

        if let Some(cwd) = &self.state.cwd {
            let line = Line::from(vec![
                Span::styled("  cwd:      ", self.theme.dim_style()),
                Span::styled(cwd.as_str(), self.theme.dim_style()),
            ]);
            Paragraph::new(line).render(
                Rect::new(detail_area.x, detail_area.y + row, detail_area.width, 1),
                buf,
            );
            row += 1;
        }

        if let Some(fp) = &self.state.file_path {
            let line = Line::from(vec![
                Span::styled("  file:     ", self.theme.dim_style()),
                Span::styled(fp.as_str(), self.theme.text_style()),
            ]);
            Paragraph::new(line).render(
                Rect::new(detail_area.x, detail_area.y + row, detail_area.width, 1),
                buf,
            );
            row += 1;
        }

        // sandbox line always shown
        let sandbox_line = Line::from(vec![
            Span::styled("  sandbox:  ", self.theme.dim_style()),
            Span::styled(self.state.sandbox_label.as_str(), self.theme.dim_style()),
        ]);
        Paragraph::new(sandbox_line).render(
            Rect::new(detail_area.x, detail_area.y + row, detail_area.width, 1),
            buf,
        );

        // --- Bottom separator ---
        let sep2 = "─".repeat(chunks[3].width as usize);
        Paragraph::new(sep2.as_str())
            .style(self.theme.dim_style())
            .render(chunks[3], buf);

        // --- Action bar ---
        let action_line = Line::from(vec![
            Span::styled("[o]", self.theme.accent_style()),
            Span::styled(" approve once    ", self.theme.muted_style()),
            Span::styled("[s]", self.theme.accent_style()),
            Span::styled(" approve session    ", self.theme.muted_style()),
            Span::styled("[d]", self.theme.accent_style()),
            Span::styled(" deny    ", self.theme.muted_style()),
            Span::styled("esc cancel", self.theme.muted_style()),
        ]);
        Paragraph::new(action_line).render(chunks[4], buf);
    }
}

impl ApprovalModal<'_> {
    fn detail_line_count(&self) -> usize {
        let mut count = 1; // sandbox always shown
        if self.state.command.is_some() {
            count += 1;
        }
        if self.state.cwd.is_some() {
            count += 1;
        }
        if self.state.file_path.is_some() {
            count += 1;
        }
        count
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_type_labels() {
        assert_eq!(ApprovalType::RunCmd.label(), "run_cmd");
        assert_eq!(ApprovalType::ApplyPatch.label(), "apply_patch");
        assert_eq!(ApprovalType::FileAccess.label(), "file access");
        assert_eq!(ApprovalType::NetworkAccess.label(), "network access");
    }

    #[test]
    fn approval_modal_state_new_hidden() {
        let state = ApprovalModalState::new();
        assert!(!state.visible);
    }

    #[test]
    fn approval_modal_open_run_cmd() {
        let mut state = ApprovalModalState::new();
        state.open_run_cmd(
            "run_cmd".to_owned(),
            "cargo test --workspace".to_owned(),
            "/home/user/code/ucode".to_owned(),
            "●ws workspace".to_owned(),
            Some(3),
        );
        assert!(state.visible);
        assert_eq!(state.approval_type, ApprovalType::RunCmd);
        assert_eq!(state.tool_name, "run_cmd");
        assert_eq!(state.command.as_deref(), Some("cargo test --workspace"));
        assert_eq!(state.cwd.as_deref(), Some("/home/user/code/ucode"));
        assert!(state.file_path.is_none());
        assert_eq!(state.sandbox_label, "●ws workspace");
        assert_eq!(state.tool_call_index, Some(3));
    }

    #[test]
    fn approval_modal_open_file_access() {
        let mut state = ApprovalModalState::new();
        state.open_file_access(
            "read_file".to_owned(),
            "/etc/passwd".to_owned(),
            "●strict".to_owned(),
            None,
        );
        assert!(state.visible);
        assert_eq!(state.approval_type, ApprovalType::FileAccess);
        assert_eq!(state.tool_name, "read_file");
        assert!(state.command.is_none());
        assert!(state.cwd.is_none());
        assert_eq!(state.file_path.as_deref(), Some("/etc/passwd"));
        assert_eq!(state.sandbox_label, "●strict");
        assert_eq!(state.tool_call_index, None);
    }

    #[test]
    fn approval_modal_approve_once() {
        let mut state = ApprovalModalState::new();
        state.open_run_cmd(
            "run_cmd".to_owned(),
            "ls".to_owned(),
            "/tmp".to_owned(),
            "●ws".to_owned(),
            None,
        );
        let scope = state.approve_once();
        assert!(!state.visible);
        assert_eq!(scope, ApprovalScope::Once);
    }

    #[test]
    fn approval_modal_approve_session() {
        let mut state = ApprovalModalState::new();
        state.open_run_cmd(
            "run_cmd".to_owned(),
            "ls".to_owned(),
            "/tmp".to_owned(),
            "●ws".to_owned(),
            None,
        );
        let scope = state.approve_session();
        assert!(!state.visible);
        assert_eq!(scope, ApprovalScope::Session);
    }

    #[test]
    fn approval_modal_deny() {
        let mut state = ApprovalModalState::new();
        state.open_network_access("fetch".to_owned(), "●net".to_owned(), Some(7));
        let scope = state.deny();
        assert!(!state.visible);
        assert_eq!(scope, ApprovalScope::Denied);
    }

    #[test]
    fn approval_modal_close() {
        let mut state = ApprovalModalState::new();
        state.open_run_cmd(
            "run_cmd".to_owned(),
            "echo hi".to_owned(),
            "/".to_owned(),
            "●off".to_owned(),
            None,
        );
        assert!(state.visible);
        state.close();
        assert!(!state.visible);
    }

    #[test]
    fn approval_modal_renders_without_panic() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = ApprovalModalState::new();
        let theme = UcodeTheme::default();

        terminal
            .draw(|f| {
                f.render_widget(ApprovalModal::new(&state, &theme), f.area());
            })
            .unwrap();
    }

    #[test]
    fn approval_modal_renders_run_cmd() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = ApprovalModalState::new();
        state.open_run_cmd(
            "run_cmd".to_owned(),
            "cargo test --workspace".to_owned(),
            "/home/user/code/ucode".to_owned(),
            "●ws workspace".to_owned(),
            Some(0),
        );
        let theme = UcodeTheme::default();

        terminal
            .draw(|f| {
                f.render_widget(ApprovalModal::new(&state, &theme), f.area());
            })
            .unwrap();
    }
}
