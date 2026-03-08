use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

use crate::keybinds::{Action, KeyCombo, KeybindPreset, KeybindResolver};
use crate::theme::UcodeTheme;

// ---------------------------------------------------------------------------
// KeybindEntry
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct KeybindEntry {
    pub key_label: String,
    pub description: String,
}

// ---------------------------------------------------------------------------
// KeybindGroup
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct KeybindGroup {
    pub category: String,
    pub bindings: Vec<KeybindEntry>,
}

// ---------------------------------------------------------------------------
// KeybindOverlayState
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct KeybindOverlayState {
    pub visible: bool,
    pub scroll_offset: usize,
    pub entries: Vec<KeybindGroup>,
    pub preset_name: String,
}

impl KeybindOverlayState {
    pub fn new() -> Self {
        Self {
            visible: false,
            scroll_offset: 0,
            entries: Vec::new(),
            preset_name: String::new(),
        }
    }

    pub fn open(&mut self, resolver: &KeybindResolver) {
        self.entries = build_groups(resolver);
        self.preset_name = match resolver.preset {
            KeybindPreset::Vscode => "vscode".to_owned(),
            KeybindPreset::Vim => "vim".to_owned(),
            KeybindPreset::Emacs => "emacs".to_owned(),
        };
        self.scroll_offset = 0;
        self.visible = true;
    }

    pub fn close(&mut self) {
        self.visible = false;
    }

    pub fn scroll_up(&mut self, n: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(n);
    }

    pub fn scroll_down(&mut self, n: usize) {
        let max = self.total_lines().saturating_sub(1);
        self.scroll_offset = (self.scroll_offset + n).min(max);
    }

    /// Total renderable lines: one header per group + one per entry + one blank between groups.
    pub fn total_lines(&self) -> usize {
        if self.entries.is_empty() {
            return 0;
        }
        let mut count = 0;
        for (i, group) in self.entries.iter().enumerate() {
            count += 1; // category header
            count += group.bindings.len();
            if i + 1 < self.entries.len() {
                count += 1; // blank separator
            }
        }
        count
    }
}

impl Default for KeybindOverlayState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Category ordering
// ---------------------------------------------------------------------------

fn category_order(cat: &str) -> usize {
    match cat {
        "Navigation" => 0,
        "Editing" => 1,
        "Overlays" => 2,
        "Sidebar" => 3,
        "Copy & Search" => 4,
        "Appearance" => 5,
        "Approval" => 6,
        "Mode" => 7,
        _ => 8,
    }
}

fn action_category(action: Action) -> &'static str {
    match action {
        Action::ScrollUp
        | Action::ScrollDown
        | Action::PageUp
        | Action::PageDown
        | Action::ScrollToTop
        | Action::ScrollToBottom
        | Action::HalfPageUp
        | Action::HalfPageDown => "Navigation",

        Action::SendMessage
        | Action::NewlineInInput
        | Action::AcceptAutocomplete
        | Action::Dismiss
        | Action::CancelGeneration => "Editing",

        Action::OpenPalette
        | Action::OpenSessionSwitcher
        | Action::SearchTranscript
        | Action::ShowKeybindOverlay
        | Action::ShowDiff => "Overlays",

        Action::ToggleSidebar | Action::GrowSidebar | Action::ShrinkSidebar => "Sidebar",

        Action::EnterCopyMode
        | Action::EnterSelectionMode
        | Action::YankSelection
        | Action::NextSearchMatch
        | Action::PrevSearchMatch
        | Action::ReverseSearch
        | Action::SetMark
        | Action::CopySelection => "Copy & Search",

        Action::ToggleTheme | Action::ToggleDensity => "Appearance",

        Action::ApproveAction | Action::RejectAction => "Approval",

        Action::EnterInsertMode
        | Action::EnterNormalMode
        | Action::ClearTranscript
        | Action::Exit => "Mode",

        Action::OpenConnect | Action::OpenModels | Action::OpenImagePopup => "Overlays",

        Action::SelectTab1
        | Action::SelectTab2
        | Action::SelectTab3
        | Action::SelectTab4
        | Action::SelectTab5
        | Action::NextTab
        | Action::PrevTab => "Navigation",
    }
}

// ---------------------------------------------------------------------------
// action_description
// ---------------------------------------------------------------------------

pub fn action_description(action: Action) -> &'static str {
    match action {
        Action::OpenPalette => "Open command palette",
        Action::OpenSessionSwitcher => "Open session switcher",
        Action::ToggleSidebar => "Toggle sidebar",
        Action::SearchTranscript => "Search transcript",
        Action::ClearTranscript => "Clear transcript",
        Action::CancelGeneration => "Cancel generation",
        Action::Exit => "Exit",
        Action::Dismiss => "Dismiss / cancel",
        Action::AcceptAutocomplete => "Accept autocomplete",
        Action::SendMessage => "Send message",
        Action::NewlineInInput => "Insert newline",
        Action::ScrollUp => "Scroll up",
        Action::ScrollDown => "Scroll down",
        Action::PageUp => "Page up",
        Action::PageDown => "Page down",
        Action::ScrollToTop => "Scroll to top",
        Action::ScrollToBottom => "Scroll to bottom",
        Action::HalfPageUp => "Half page up",
        Action::HalfPageDown => "Half page down",
        Action::GrowSidebar => "Grow sidebar",
        Action::ShrinkSidebar => "Shrink sidebar",
        Action::EnterCopyMode => "Enter copy mode",
        Action::EnterSelectionMode => "Select mode (v=char V=line Ctrl+V=block)",
        Action::YankSelection => "Yank selection",
        Action::NextSearchMatch => "Next search match",
        Action::PrevSearchMatch => "Previous search match",
        Action::ShowKeybindOverlay => "Show keybind reference",
        Action::ApproveAction => "Approve action",
        Action::RejectAction => "Reject action",
        Action::ShowDiff => "Show diff",
        Action::EnterInsertMode => "Enter insert mode",
        Action::EnterNormalMode => "Enter normal mode",
        Action::ReverseSearch => "Reverse search",
        Action::SetMark => "Set mark",
        Action::CopySelection => "Copy selection",
        Action::ToggleTheme => "Toggle theme",
        Action::ToggleDensity => "Toggle density",
        Action::OpenConnect => "Open connect modal",
        Action::OpenModels => "List available models",
        Action::OpenImagePopup => "Open image popup",
        Action::SelectTab1 => "Chat tab",
        Action::SelectTab2 => "Subagents tab",
        Action::SelectTab3 => "Tools tab",
        Action::SelectTab4 => "MCP tab",
        Action::SelectTab5 => "Logs tab",
        Action::NextTab => "Next tab",
        Action::PrevTab => "Previous tab",
    }
}

// ---------------------------------------------------------------------------
// format_key_combo
// ---------------------------------------------------------------------------

pub fn format_key_combo(combo: &KeyCombo) -> String {
    let mut parts = String::new();

    if combo.modifiers.contains(KeyModifiers::CONTROL) {
        parts.push_str("Ctrl+");
    }
    if combo.modifiers.contains(KeyModifiers::ALT) {
        parts.push_str("Alt+");
    }
    if combo.modifiers.contains(KeyModifiers::SHIFT) {
        parts.push_str("Shift+");
    }

    let key = match combo.code {
        KeyCode::Char(c) => c.to_uppercase().to_string(),
        KeyCode::Esc => "Esc".to_owned(),
        KeyCode::Enter => "Enter".to_owned(),
        KeyCode::Tab => "Tab".to_owned(),
        KeyCode::Backspace => "Backspace".to_owned(),
        KeyCode::Delete => "Delete".to_owned(),
        KeyCode::Up => "Up".to_owned(),
        KeyCode::Down => "Down".to_owned(),
        KeyCode::Left => "Left".to_owned(),
        KeyCode::Right => "Right".to_owned(),
        KeyCode::Home => "Home".to_owned(),
        KeyCode::End => "End".to_owned(),
        KeyCode::PageUp => "PgUp".to_owned(),
        KeyCode::PageDown => "PgDn".to_owned(),
        KeyCode::F(n) => format!("F{n}"),
        _ => "?".to_owned(),
    };

    parts.push_str(&key);
    parts
}

// ---------------------------------------------------------------------------
// build_groups
// ---------------------------------------------------------------------------

pub fn build_groups(resolver: &KeybindResolver) -> Vec<KeybindGroup> {
    use std::collections::HashMap;

    // action → list of key labels
    let mut action_keys: HashMap<Action, Vec<String>> = HashMap::new();
    for (combo, &action) in resolver.bindings() {
        action_keys
            .entry(action)
            .or_default()
            .push(format_key_combo(combo));
    }

    // category name → entries
    let mut cat_map: HashMap<&'static str, Vec<KeybindEntry>> = HashMap::new();
    for (action, mut keys) in action_keys {
        if keys.is_empty() {
            continue;
        }
        keys.sort();
        let key_label = keys.join(", ");
        let description = action_description(action).to_owned();
        let category = action_category(action);
        cat_map.entry(category).or_default().push(KeybindEntry {
            key_label,
            description,
        });
    }

    // Sort entries within each category alphabetically by description.
    for entries in cat_map.values_mut() {
        entries.sort_by(|a, b| a.description.cmp(&b.description));
    }

    // Collect and sort categories by canonical order.
    let mut groups: Vec<KeybindGroup> = cat_map
        .into_iter()
        .map(|(cat, bindings)| KeybindGroup {
            category: cat.to_owned(),
            bindings,
        })
        .collect();
    groups.sort_by_key(|g| category_order(&g.category));

    groups
}

// ---------------------------------------------------------------------------
// KeybindOverlay widget
// ---------------------------------------------------------------------------

pub struct KeybindOverlay<'a> {
    pub state: &'a KeybindOverlayState,
    pub theme: &'a UcodeTheme,
}

impl<'a> KeybindOverlay<'a> {
    pub fn new(state: &'a KeybindOverlayState, theme: &'a UcodeTheme) -> Self {
        Self { state, theme }
    }
}

impl Widget for KeybindOverlay<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let modal_area = centered_rect(80, 80, area);
        Clear.render(modal_area, buf);

        let title = format!(" Keybind Reference ({}) ", self.state.preset_name);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border_style(true))
            .title(title);
        let inner = block.inner(modal_area);
        block.render(modal_area, buf);

        if inner.height < 3 || inner.width < 20 {
            return;
        }

        // Reserve last line for footer hint.
        let content_height = inner.height.saturating_sub(1) as usize;
        let footer_y = inner.y + inner.height.saturating_sub(1);

        // Footer.
        let footer = Line::from(vec![Span::styled(
            "Esc to close, \u{2191}\u{2193} to scroll",
            self.theme.dim_style(),
        )]);
        let footer_rect = Rect::new(inner.x, footer_y, inner.width, 1);
        Paragraph::new(footer).render(footer_rect, buf);

        // Build all display lines.
        let mut lines: Vec<Line> = Vec::new();
        for (i, group) in self.state.entries.iter().enumerate() {
            // Category header.
            lines.push(Line::from(vec![Span::styled(
                group.category.clone(),
                self.theme.accent_style().add_modifier(Modifier::BOLD),
            )]));
            // Entries.
            for entry in &group.bindings {
                let key_col = format!("{:>12}", entry.key_label);
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(key_col, self.theme.accent_style()),
                    Span::raw("  "),
                    Span::styled(entry.description.clone(), self.theme.text_style()),
                ]));
            }
            // Blank separator between groups.
            if i + 1 < self.state.entries.len() {
                lines.push(Line::raw(""));
            }
        }

        // Render visible slice.
        let offset = self.state.scroll_offset;
        let visible = lines
            .into_iter()
            .skip(offset)
            .take(content_height)
            .collect::<Vec<_>>();

        let content_rect = Rect::new(inner.x, inner.y, inner.width, content_height as u16);
        Paragraph::new(visible).render(content_rect, buf);
    }
}

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

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
    use crate::keybinds::{KeybindPreset, KeybindResolver};

    // -----------------------------------------------------------------------
    // format_key_combo
    // -----------------------------------------------------------------------

    #[test]
    fn format_ctrl_p() {
        let combo = KeyCombo::new(KeyCode::Char('p'), KeyModifiers::CONTROL);
        assert_eq!(format_key_combo(&combo), "Ctrl+P");
    }

    #[test]
    fn format_shift_n() {
        let combo = KeyCombo::new(KeyCode::Char('N'), KeyModifiers::SHIFT);
        assert_eq!(format_key_combo(&combo), "Shift+N");
    }

    #[test]
    fn format_alt_x() {
        let combo = KeyCombo::new(KeyCode::Char('x'), KeyModifiers::ALT);
        assert_eq!(format_key_combo(&combo), "Alt+X");
    }

    #[test]
    fn format_esc() {
        let combo = KeyCombo::new(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(format_key_combo(&combo), "Esc");
    }

    #[test]
    fn format_enter() {
        let combo = KeyCombo::new(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(format_key_combo(&combo), "Enter");
    }

    #[test]
    fn format_tab() {
        let combo = KeyCombo::new(KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(format_key_combo(&combo), "Tab");
    }

    #[test]
    fn format_f6() {
        let combo = KeyCombo::new(KeyCode::F(6), KeyModifiers::NONE);
        assert_eq!(format_key_combo(&combo), "F6");
    }

    #[test]
    fn format_page_up() {
        let combo = KeyCombo::new(KeyCode::PageUp, KeyModifiers::NONE);
        assert_eq!(format_key_combo(&combo), "PgUp");
    }

    #[test]
    fn format_page_down() {
        let combo = KeyCombo::new(KeyCode::PageDown, KeyModifiers::NONE);
        assert_eq!(format_key_combo(&combo), "PgDn");
    }

    #[test]
    fn format_bare_char() {
        let combo = KeyCombo::new(KeyCode::Char('?'), KeyModifiers::NONE);
        assert_eq!(format_key_combo(&combo), "?");
    }

    #[test]
    fn format_ctrl_home() {
        let combo = KeyCombo::new(KeyCode::Home, KeyModifiers::CONTROL);
        assert_eq!(format_key_combo(&combo), "Ctrl+Home");
    }

    // -----------------------------------------------------------------------
    // action_description
    // -----------------------------------------------------------------------

    #[test]
    fn action_description_non_empty_for_all_variants() {
        let all_actions = [
            Action::OpenPalette,
            Action::OpenSessionSwitcher,
            Action::ToggleSidebar,
            Action::SearchTranscript,
            Action::ClearTranscript,
            Action::CancelGeneration,
            Action::Exit,
            Action::Dismiss,
            Action::AcceptAutocomplete,
            Action::SendMessage,
            Action::NewlineInInput,
            Action::ScrollUp,
            Action::ScrollDown,
            Action::PageUp,
            Action::PageDown,
            Action::ScrollToTop,
            Action::ScrollToBottom,
            Action::HalfPageUp,
            Action::HalfPageDown,
            Action::GrowSidebar,
            Action::ShrinkSidebar,
            Action::EnterCopyMode,
            Action::YankSelection,
            Action::NextSearchMatch,
            Action::PrevSearchMatch,
            Action::ShowKeybindOverlay,
            Action::ApproveAction,
            Action::RejectAction,
            Action::ShowDiff,
            Action::EnterInsertMode,
            Action::EnterNormalMode,
            Action::ReverseSearch,
            Action::SetMark,
            Action::CopySelection,
            Action::ToggleTheme,
            Action::ToggleDensity,
        ];
        for action in all_actions {
            let desc = action_description(action);
            assert!(
                !desc.is_empty(),
                "action_description returned empty for {action:?}"
            );
        }
    }

    #[test]
    fn action_description_open_palette() {
        assert_eq!(
            action_description(Action::OpenPalette),
            "Open command palette"
        );
    }

    #[test]
    fn action_description_toggle_theme() {
        assert_eq!(action_description(Action::ToggleTheme), "Toggle theme");
    }

    // -----------------------------------------------------------------------
    // build_groups
    // -----------------------------------------------------------------------

    #[test]
    fn build_groups_vscode_non_empty() {
        let resolver = KeybindResolver::new(KeybindPreset::Vscode);
        let groups = build_groups(&resolver);
        assert!(!groups.is_empty(), "vscode should produce non-empty groups");
    }

    #[test]
    fn build_groups_vim_non_empty() {
        let resolver = KeybindResolver::new(KeybindPreset::Vim);
        let groups = build_groups(&resolver);
        assert!(!groups.is_empty(), "vim should produce non-empty groups");
    }

    #[test]
    fn build_groups_emacs_non_empty() {
        let resolver = KeybindResolver::new(KeybindPreset::Emacs);
        let groups = build_groups(&resolver);
        assert!(!groups.is_empty(), "emacs should produce non-empty groups");
    }

    #[test]
    fn build_groups_sorted_by_category_order() {
        let resolver = KeybindResolver::new(KeybindPreset::Vscode);
        let groups = build_groups(&resolver);
        let orders: Vec<usize> = groups.iter().map(|g| category_order(&g.category)).collect();
        let mut sorted = orders.clone();
        sorted.sort();
        assert_eq!(orders, sorted, "groups should be sorted by category order");
    }

    #[test]
    fn build_groups_entries_sorted_alphabetically() {
        let resolver = KeybindResolver::new(KeybindPreset::Vscode);
        let groups = build_groups(&resolver);
        for group in &groups {
            let descs: Vec<&str> = group
                .bindings
                .iter()
                .map(|e| e.description.as_str())
                .collect();
            let mut sorted = descs.clone();
            sorted.sort();
            assert_eq!(
                descs, sorted,
                "entries in '{}' should be sorted alphabetically",
                group.category
            );
        }
    }

    #[test]
    fn build_groups_navigation_category_exists_for_vscode() {
        let resolver = KeybindResolver::new(KeybindPreset::Vscode);
        let groups = build_groups(&resolver);
        let nav = groups.iter().find(|g| g.category == "Navigation");
        assert!(nav.is_some(), "Navigation category should exist for vscode");
        assert!(
            !nav.unwrap().bindings.is_empty(),
            "Navigation should have bindings"
        );
    }

    // -----------------------------------------------------------------------
    // KeybindOverlayState
    // -----------------------------------------------------------------------

    #[test]
    fn state_new_not_visible() {
        let state = KeybindOverlayState::new();
        assert!(!state.visible);
        assert!(state.entries.is_empty());
        assert_eq!(state.scroll_offset, 0);
    }

    #[test]
    fn state_open_populates_entries_and_sets_visible() {
        let mut state = KeybindOverlayState::new();
        let resolver = KeybindResolver::new(KeybindPreset::Vscode);
        state.open(&resolver);
        assert!(state.visible);
        assert!(!state.entries.is_empty());
        assert_eq!(state.preset_name, "vscode");
        assert_eq!(state.scroll_offset, 0);
    }

    #[test]
    fn state_open_vim_sets_preset_name() {
        let mut state = KeybindOverlayState::new();
        let resolver = KeybindResolver::new(KeybindPreset::Vim);
        state.open(&resolver);
        assert_eq!(state.preset_name, "vim");
    }

    #[test]
    fn state_open_emacs_sets_preset_name() {
        let mut state = KeybindOverlayState::new();
        let resolver = KeybindResolver::new(KeybindPreset::Emacs);
        state.open(&resolver);
        assert_eq!(state.preset_name, "emacs");
    }

    #[test]
    fn state_close_hides_overlay() {
        let mut state = KeybindOverlayState::new();
        let resolver = KeybindResolver::new(KeybindPreset::Vscode);
        state.open(&resolver);
        assert!(state.visible);
        state.close();
        assert!(!state.visible);
    }

    #[test]
    fn state_scroll_up_saturates_at_zero() {
        let mut state = KeybindOverlayState::new();
        assert_eq!(state.scroll_offset, 0);
        state.scroll_up(5);
        assert_eq!(state.scroll_offset, 0);
    }

    #[test]
    fn state_scroll_down_increases_offset() {
        let mut state = KeybindOverlayState::new();
        let resolver = KeybindResolver::new(KeybindPreset::Vscode);
        state.open(&resolver);
        let before = state.scroll_offset;
        state.scroll_down(3);
        assert!(state.scroll_offset > before);
    }

    #[test]
    fn state_scroll_down_clamps_at_max() {
        let mut state = KeybindOverlayState::new();
        let resolver = KeybindResolver::new(KeybindPreset::Vscode);
        state.open(&resolver);
        let total = state.total_lines();
        state.scroll_down(total + 1000);
        assert!(state.scroll_offset < total);
    }

    #[test]
    fn state_scroll_up_after_down() {
        let mut state = KeybindOverlayState::new();
        let resolver = KeybindResolver::new(KeybindPreset::Vscode);
        state.open(&resolver);
        state.scroll_down(5);
        assert_eq!(state.scroll_offset, 5);
        state.scroll_up(3);
        assert_eq!(state.scroll_offset, 2);
    }

    #[test]
    fn state_total_lines_counts_correctly() {
        let mut state = KeybindOverlayState::new();
        // Manually set entries to verify counting.
        state.entries = vec![
            KeybindGroup {
                category: "A".to_owned(),
                bindings: vec![
                    KeybindEntry {
                        key_label: "X".to_owned(),
                        description: "x".to_owned(),
                    },
                    KeybindEntry {
                        key_label: "Y".to_owned(),
                        description: "y".to_owned(),
                    },
                ],
            },
            KeybindGroup {
                category: "B".to_owned(),
                bindings: vec![KeybindEntry {
                    key_label: "Z".to_owned(),
                    description: "z".to_owned(),
                }],
            },
        ];
        // Group A: 1 header + 2 entries = 3
        // Blank separator: 1
        // Group B: 1 header + 1 entry = 2
        // Total: 6
        assert_eq!(state.total_lines(), 6);
    }

    #[test]
    fn state_total_lines_empty() {
        let state = KeybindOverlayState::new();
        assert_eq!(state.total_lines(), 0);
    }

    // -----------------------------------------------------------------------
    // Widget rendering
    // -----------------------------------------------------------------------

    #[test]
    fn keybind_overlay_renders_without_panic() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = KeybindOverlayState::new();
        let resolver = KeybindResolver::new(KeybindPreset::Vscode);
        state.open(&resolver);
        let theme = UcodeTheme::default();

        terminal
            .draw(|f| {
                f.render_widget(KeybindOverlay::new(&state, &theme), f.area());
            })
            .unwrap();
    }

    #[test]
    fn keybind_overlay_renders_vim_without_panic() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = KeybindOverlayState::new();
        let resolver = KeybindResolver::new(KeybindPreset::Vim);
        state.open(&resolver);
        let theme = UcodeTheme::default();

        terminal
            .draw(|f| {
                f.render_widget(KeybindOverlay::new(&state, &theme), f.area());
            })
            .unwrap();
    }
}
