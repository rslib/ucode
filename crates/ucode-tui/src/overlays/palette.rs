use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

use crate::command_registry::{CommandCategory, CommandDef, CommandRegistry};
use crate::theme::UcodeTheme;

// ---------------------------------------------------------------------------
// PaletteState
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct PaletteState {
    pub visible: bool,
    pub input: String,
    pub cursor: usize,
    pub commands: Vec<CommandDef>,
    pub filtered_indices: Vec<usize>,
    pub selected: usize,
}

impl PaletteState {
    pub fn new() -> Self {
        let reg = CommandRegistry::with_builtins();
        Self::from_registry(&reg)
    }

    pub fn from_registry(reg: &CommandRegistry) -> Self {
        let commands: Vec<CommandDef> = reg.list().to_vec();
        let filtered_indices = (0..commands.len()).collect();
        Self {
            visible: false,
            input: String::new(),
            cursor: 0,
            commands,
            filtered_indices,
            selected: 0,
        }
    }

    pub fn open(&mut self) {
        self.visible = true;
        self.input.clear();
        self.cursor = 0;
        self.selected = 0;
        self.update_filter();
    }

    pub fn close(&mut self) {
        self.visible = false;
    }

    pub fn insert_char(&mut self, c: char) {
        self.input.insert(self.cursor, c);
        self.cursor += c.len_utf8();
        self.update_filter();
    }

    pub fn delete_char(&mut self) {
        if self.cursor == 0 {
            return;
        }
        // Find the char boundary before cursor.
        let before = &self.input[..self.cursor];
        let char_start = before
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.input.drain(char_start..self.cursor);
        self.cursor = char_start;
        self.update_filter();
    }

    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn move_down(&mut self) {
        if self.filtered_indices.is_empty() {
            return;
        }
        let max = self.filtered_indices.len() - 1;
        if self.selected < max {
            self.selected += 1;
        }
    }

    pub fn selected_command(&self) -> Option<&CommandDef> {
        let idx = *self.filtered_indices.get(self.selected)?;
        self.commands.get(idx)
    }

    pub fn execute_selected(&mut self) -> Option<CommandDef> {
        let cmd = self.selected_command()?.clone();
        self.close();
        Some(cmd)
    }

    pub fn update_filter(&mut self) {
        if self.input.is_empty() {
            self.filtered_indices = (0..self.commands.len()).collect();
        } else {
            let needle = self.input.to_lowercase();
            self.filtered_indices = self
                .commands
                .iter()
                .enumerate()
                .filter(|(_, cmd)| {
                    cmd.name.to_lowercase().contains(&needle)
                        || cmd.description.to_lowercase().contains(&needle)
                })
                .map(|(i, _)| i)
                .collect();
        }
        // Clamp selected to valid range.
        if self.filtered_indices.is_empty() {
            self.selected = 0;
        } else {
            let max = self.filtered_indices.len() - 1;
            if self.selected > max {
                self.selected = max;
            }
        }
    }
}

impl Default for PaletteState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// PaletteOverlay widget
// ---------------------------------------------------------------------------

pub struct PaletteOverlay<'a> {
    state: &'a PaletteState,
    theme: &'a UcodeTheme,
}

impl<'a> PaletteOverlay<'a> {
    pub fn new(state: &'a PaletteState, theme: &'a UcodeTheme) -> Self {
        Self { state, theme }
    }
}

impl Widget for PaletteOverlay<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let palette_area = centered_rect(60, 50, area);

        Clear.render(palette_area, buf);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border_style(true))
            .title(" Command Palette ");
        let inner = block.inner(palette_area);
        block.render(palette_area, buf);

        if inner.height < 3 || inner.width < 10 {
            return;
        }

        let chunks = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .split(inner);

        let input_line = Line::from(vec![
            Span::styled("> ", self.theme.accent_style()),
            Span::raw(&self.state.input),
        ]);
        Paragraph::new(input_line).render(chunks[0], buf);

        let sep = "─".repeat(chunks[1].width as usize);
        Paragraph::new(sep)
            .style(self.theme.dim_style())
            .render(chunks[1], buf);

        let visible_height = chunks[2].height as usize;
        let list_area = chunks[2];

        let scroll_offset = if self.state.selected >= visible_height {
            self.state.selected - visible_height + 1
        } else {
            0
        };

        let mut current_category: Option<CommandCategory> = None;
        let mut row = 0usize;

        for (display_idx, &cmd_idx) in self.state.filtered_indices.iter().enumerate() {
            if row >= scroll_offset + visible_height {
                break;
            }

            let cmd = &self.state.commands[cmd_idx];

            if current_category != Some(cmd.category) {
                current_category = Some(cmd.category);
                if row >= scroll_offset {
                    let y = list_area.y + (row - scroll_offset) as u16;
                    if y < list_area.y + list_area.height {
                        let label = cmd.category.label();
                        let header = Line::from(vec![
                            Span::styled(format!("── {} ", label), self.theme.dim_style()),
                            Span::styled(
                                "─"
                                    .repeat(list_area.width.saturating_sub(label.len() as u16 + 4)
                                        as usize),
                                self.theme.dim_style(),
                            ),
                        ]);
                        let header_rect = Rect::new(list_area.x, y, list_area.width, 1);
                        Paragraph::new(header).render(header_rect, buf);
                    }
                }
                row += 1;
                if row >= scroll_offset + visible_height {
                    break;
                }
            }

            if row >= scroll_offset {
                let y = list_area.y + (row - scroll_offset) as u16;
                if y < list_area.y + list_area.height {
                    let is_selected = display_idx == self.state.selected;
                    let name_style = if is_selected {
                        self.theme.accent_style().add_modifier(Modifier::BOLD)
                    } else {
                        self.theme.text_style()
                    };
                    let desc_style = if is_selected {
                        self.theme.text_style()
                    } else {
                        self.theme.muted_style()
                    };
                    let badge_style = self.theme.dim_style();

                    let hint_str = cmd
                        .args_hint
                        .as_deref()
                        .map(|h| format!(" {h}"))
                        .unwrap_or_default();
                    let name_display_len = cmd.name.len() + hint_str.len();
                    let badge = cmd.source.badge();
                    let badge_len = badge.len();
                    let spacing = 2usize;
                    let desc_max = (list_area.width as usize)
                        .saturating_sub(name_display_len + 2 + badge_len + spacing);

                    let desc_truncated = if cmd.description.len() > desc_max {
                        format!("{}…", &cmd.description[..desc_max.saturating_sub(1)])
                    } else {
                        cmd.description.clone()
                    };

                    let prefix = if is_selected { "▸ " } else { "  " };
                    let hint_style = self.theme.dim_style();
                    let displayed_desc_len = desc_truncated.len().min(desc_max);
                    let used =
                        prefix.len() + name_display_len + spacing + displayed_desc_len + badge_len;
                    let pad = (list_area.width as usize).saturating_sub(used);

                    let mut spans = vec![Span::styled(format!("{prefix}{}", cmd.name), name_style)];
                    if !hint_str.is_empty() {
                        spans.push(Span::styled(hint_str, hint_style));
                    }
                    spans.push(Span::raw("  "));
                    spans.push(Span::styled(desc_truncated, desc_style));
                    spans.push(Span::raw(" ".repeat(pad)));
                    spans.push(Span::styled(badge, badge_style));

                    let line = Line::from(spans);

                    let row_style = if is_selected {
                        Style::default().bg(self.theme.surface)
                    } else {
                        Style::default()
                    };

                    let row_rect = Rect::new(list_area.x, y, list_area.width, 1);
                    Paragraph::new(line).style(row_style).render(row_rect, buf);
                }
            }
            row += 1;
        }
    }
}

/// Compute a centered rectangle of the given percentage width/height within `area`.
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
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
    use crate::command_registry::{CommandCategory, CommandSource};

    #[test]
    fn builtin_commands_count() {
        let state = PaletteState::new();
        assert_eq!(state.commands.len(), 10);
    }

    #[test]
    fn test_palette_commands_have_args_hint() {
        let state = PaletteState::new();
        let rename = state
            .commands
            .iter()
            .find(|c| c.name == "/session rename")
            .expect("/session rename must exist");
        assert_eq!(rename.args_hint.as_deref(), Some("<name>"));
    }

    #[test]
    fn category_labels() {
        assert_eq!(CommandCategory::Recent.label(), "recent");
        assert_eq!(CommandCategory::Session.label(), "session");
        assert_eq!(CommandCategory::Tools.label(), "tools");
        assert_eq!(CommandCategory::Plugins.label(), "plugins");
    }

    #[test]
    fn source_badges() {
        assert_eq!(CommandSource::Builtin.badge(), "[builtin]");
        assert_eq!(
            CommandSource::Plugin("my-plugin".to_owned()).badge(),
            "[plugin]"
        );
    }

    #[test]
    fn new_state_all_filtered_not_visible() {
        let state = PaletteState::new();
        assert!(!state.visible);
        assert_eq!(state.filtered_indices.len(), state.commands.len());
        assert_eq!(state.selected, 0);
        assert!(state.input.is_empty());
    }

    #[test]
    fn open_sets_visible_clears_input() {
        let mut state = PaletteState::new();
        state.input = "leftover".to_owned();
        state.cursor = 8;
        state.open();
        assert!(state.visible);
        assert!(state.input.is_empty());
        assert_eq!(state.cursor, 0);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn insert_char_filters_session_commands() {
        let mut state = PaletteState::new();
        state.open();
        for c in "session".chars() {
            state.insert_char(c);
        }
        // Only session commands should remain.
        assert!(!state.filtered_indices.is_empty());
        for &idx in &state.filtered_indices {
            let cmd = &state.commands[idx];
            let haystack = format!("{} {}", cmd.name, cmd.description).to_lowercase();
            assert!(
                haystack.contains("session"),
                "unexpected command: {}",
                cmd.name
            );
        }
        // Exactly 3 session commands.
        assert_eq!(state.filtered_indices.len(), 3);
    }

    #[test]
    fn move_up_saturates_at_zero() {
        let mut state = PaletteState::new();
        state.open();
        assert_eq!(state.selected, 0);
        state.move_up();
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn move_down_clamps_at_last() {
        let mut state = PaletteState::new();
        state.open();
        let last = state.filtered_indices.len() - 1;
        for _ in 0..last + 5 {
            state.move_down();
        }
        assert_eq!(state.selected, last);
    }

    #[test]
    fn move_up_down_navigation() {
        let mut state = PaletteState::new();
        state.open();
        state.move_down();
        state.move_down();
        assert_eq!(state.selected, 2);
        state.move_up();
        assert_eq!(state.selected, 1);
    }

    #[test]
    fn selected_command_returns_correct() {
        let mut state = PaletteState::new();
        state.open();
        // First command should be /connect (index 0 in builtin_commands).
        let cmd = state.selected_command().unwrap();
        assert_eq!(cmd.name, "/connect");
    }

    #[test]
    fn execute_selected_returns_command_and_closes() {
        let mut state = PaletteState::new();
        state.open();
        let cmd = state.execute_selected().unwrap();
        assert_eq!(cmd.name, "/connect");
        assert!(!state.visible);
    }

    #[test]
    fn delete_char_refilters() {
        let mut state = PaletteState::new();
        state.open();
        for c in "session".chars() {
            state.insert_char(c);
        }
        assert_eq!(state.filtered_indices.len(), 3);
        // Delete all chars one by one.
        for _ in 0.."session".len() {
            state.delete_char();
        }
        assert!(state.input.is_empty());
        // Should show all commands again.
        assert_eq!(state.filtered_indices.len(), state.commands.len());
    }

    #[test]
    fn empty_filter_shows_all() {
        let mut state = PaletteState::new();
        state.open();
        state.update_filter();
        assert_eq!(state.filtered_indices.len(), state.commands.len());
    }

    #[test]
    fn filter_no_match_gives_empty() {
        let mut state = PaletteState::new();
        state.open();
        for c in "zzznomatch".chars() {
            state.insert_char(c);
        }
        assert!(state.filtered_indices.is_empty());
        // selected_command returns None when nothing matches.
        assert!(state.selected_command().is_none());
    }

    #[test]
    fn selected_clamped_after_filter_narrows() {
        let mut state = PaletteState::new();
        state.open();
        // Move to last item.
        let last = state.filtered_indices.len() - 1;
        for _ in 0..last {
            state.move_down();
        }
        assert_eq!(state.selected, last);
        // Now filter to only 3 items.
        for c in "session".chars() {
            state.insert_char(c);
        }
        // selected must be within [0, 2].
        assert!(state.selected <= 2);
    }

    #[test]
    fn palette_overlay_renders_without_panic() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = PaletteState::new();
        state.open();
        let theme = UcodeTheme::default();

        terminal
            .draw(|f| {
                f.render_widget(PaletteOverlay::new(&state, &theme), f.area());
            })
            .unwrap();
    }

    #[test]
    fn palette_overlay_with_filter_renders() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = PaletteState::new();
        state.open();
        for c in "session".chars() {
            state.insert_char(c);
        }
        let theme = UcodeTheme::default();

        terminal
            .draw(|f| {
                f.render_widget(PaletteOverlay::new(&state, &theme), f.area());
            })
            .unwrap();
    }
}
