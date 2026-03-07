use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// A single model entry in the modal list.
#[derive(Debug, Clone)]
pub struct ModelEntry {
    pub provider: String,
    pub model_id: String,
    pub display_name: Option<String>,
}

impl ModelEntry {
    /// Label shown in the list: "model_id (display_name)" or just "model_id".
    pub fn label(&self) -> String {
        match &self.display_name {
            Some(name) if name != &self.model_id => format!("{} ({})", self.model_id, name),
            _ => self.model_id.clone(),
        }
    }
}

/// State for the models modal overlay.
#[derive(Debug, Clone)]
pub struct ModelsModalState {
    pub visible: bool,
    pub loading: bool,
    /// Number of providers we're still waiting for.
    pub pending_providers: usize,
    pub entries: Vec<ModelEntry>,
    pub filter: String,
    pub filter_cursor: usize,
    pub filtered_indices: Vec<usize>,
    pub selected: usize,
    /// The currently active model id (highlighted in the list).
    pub current_model: Option<String>,
}

// ---------------------------------------------------------------------------
// ModelsModalState impl
// ---------------------------------------------------------------------------

impl ModelsModalState {
    pub fn new() -> Self {
        Self {
            visible: false,
            loading: false,
            pending_providers: 0,
            entries: Vec::new(),
            filter: String::new(),
            filter_cursor: 0,
            filtered_indices: Vec::new(),
            selected: 0,
            current_model: None,
        }
    }

    /// Open the modal, clearing any previous state and initiating a load.
    pub fn open(&mut self, current_model: Option<String>, provider_count: usize) {
        self.visible = true;
        self.loading = true;
        self.pending_providers = provider_count;
        self.entries.clear();
        self.filter.clear();
        self.filter_cursor = 0;
        self.selected = 0;
        self.current_model = current_model;
        self.update_filter();
    }

    pub fn close(&mut self) {
        self.visible = false;
    }

    // -- filter text editing -------------------------------------------------

    pub fn insert_char(&mut self, c: char) {
        self.filter.insert(self.filter_cursor, c);
        self.filter_cursor += c.len_utf8();
        self.update_filter();
    }

    pub fn delete_char(&mut self) {
        if self.filter_cursor == 0 {
            return;
        }
        let before = &self.filter[..self.filter_cursor];
        let char_start = before
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.filter.drain(char_start..self.filter_cursor);
        self.filter_cursor = char_start;
        self.update_filter();
    }

    // -- navigation ----------------------------------------------------------

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

    // -- queries -------------------------------------------------------------

    pub fn selected_entry(&self) -> Option<&ModelEntry> {
        let idx = *self.filtered_indices.get(self.selected)?;
        self.entries.get(idx)
    }

    // -- data ingestion ------------------------------------------------------

    /// Append models from a provider response and decrement the pending count.
    pub fn add_models(&mut self, provider: &str, models: &[ucode_providers::ModelInfo]) {
        for m in models {
            self.entries.push(ModelEntry {
                provider: provider.to_owned(),
                model_id: m.id.clone(),
                display_name: m.name.clone(),
            });
        }
        self.pending_providers = self.pending_providers.saturating_sub(1);
        if self.pending_providers == 0 {
            self.loading = false;
        }
        self.update_filter();
    }

    /// Record a provider error: decrement pending count, clear loading when done.
    ///
    /// The caller is responsible for surfacing the error message to the user.
    pub fn add_error(&mut self, _error: &str) {
        self.pending_providers = self.pending_providers.saturating_sub(1);
        if self.pending_providers == 0 {
            self.loading = false;
        }
    }

    // -- internal ------------------------------------------------------------

    pub fn update_filter(&mut self) {
        if self.filter.is_empty() {
            self.filtered_indices = (0..self.entries.len()).collect();
        } else {
            let needle = self.filter.to_lowercase();
            self.filtered_indices = self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, e)| {
                    e.model_id.to_lowercase().contains(&needle)
                        || e.display_name
                            .as_deref()
                            .map(|n| n.to_lowercase().contains(&needle))
                            .unwrap_or(false)
                })
                .map(|(i, _)| i)
                .collect();
        }
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

impl Default for ModelsModalState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// centered_rect helper
// ---------------------------------------------------------------------------

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_width = area.width * percent_x / 100;
    let popup_height = area.height * percent_y / 100;
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    Rect::new(x, y, popup_width, popup_height)
}

// ---------------------------------------------------------------------------
// ModelsModal widget
// ---------------------------------------------------------------------------

pub struct ModelsModal<'a> {
    state: &'a ModelsModalState,
    theme: &'a crate::theme::UcodeTheme,
}

impl<'a> ModelsModal<'a> {
    pub fn new(state: &'a ModelsModalState, theme: &'a crate::theme::UcodeTheme) -> Self {
        Self { state, theme }
    }
}

impl Widget for ModelsModal<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if !self.state.visible {
            return;
        }

        let popup = centered_rect(60, 70, area);
        Clear.render(popup, buf);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border_style(true))
            .title(" Models ");
        let inner = block.inner(popup);
        block.render(popup, buf);

        if inner.height < 4 || inner.width < 10 {
            return;
        }

        // Layout: filter line, separator, list body, separator, footer (1 line)
        let footer_height = 1u16;
        let list_height = inner
            .height
            .saturating_sub(1 + 1 + 1 + footer_height) // filter + sep + sep + footer
            .max(1);

        let filter_rect = Rect::new(inner.x, inner.y, inner.width, 1);
        let sep1_rect = Rect::new(inner.x, inner.y + 1, inner.width, 1);
        let list_rect = Rect::new(inner.x, inner.y + 2, inner.width, list_height);
        let sep2_y = inner.y + 2 + list_height;
        let sep2_rect = Rect::new(inner.x, sep2_y, inner.width, 1);
        let footer_rect = Rect::new(inner.x, sep2_y + 1, inner.width, footer_height);

        // Filter line
        let filter_line = Line::from(vec![
            Span::styled("Filter: ", self.theme.dim_style()),
            Span::styled(&self.state.filter, self.theme.text_style()),
        ]);
        Paragraph::new(filter_line).render(filter_rect, buf);

        // Separator
        let sep = "─".repeat(inner.width as usize);
        Paragraph::new(sep.as_str())
            .style(self.theme.dim_style())
            .render(sep1_rect, buf);

        // List body — loading state or model entries grouped by provider
        if self.state.loading && self.state.entries.is_empty() {
            Paragraph::new("Fetching models...")
                .style(self.theme.dim_style())
                .render(list_rect, buf);
        } else if self.state.filtered_indices.is_empty() {
            let msg = if self.state.entries.is_empty() {
                "No models available."
            } else {
                "No matches."
            };
            Paragraph::new(msg)
                .style(self.theme.dim_style())
                .render(list_rect, buf);
        } else {
            self.render_model_list(list_rect, buf);
        }

        // Bottom separator
        if sep2_rect.y < inner.y + inner.height {
            Paragraph::new(sep.as_str())
                .style(self.theme.dim_style())
                .render(sep2_rect, buf);
        }

        // Footer with keybind hints
        if footer_rect.y < inner.y + inner.height {
            let footer = Line::from(vec![
                Span::styled("↑↓", self.theme.accent_style()),
                Span::styled(" navigate  ", self.theme.dim_style()),
                Span::styled("type", self.theme.accent_style()),
                Span::styled(" to filter  ", self.theme.dim_style()),
                Span::styled("Enter", self.theme.accent_style()),
                Span::styled(" select  ", self.theme.dim_style()),
                Span::styled("Esc", self.theme.accent_style()),
                Span::styled(" close", self.theme.dim_style()),
            ]);
            Paragraph::new(footer).render(footer_rect, buf);
        }
    }
}

impl ModelsModal<'_> {
    fn render_model_list(&self, area: Rect, buf: &mut Buffer) {
        let visible_height = area.height as usize;

        // Build flat display rows: (is_header, display_idx_or_usize::MAX)
        // We need to figure out which "display row" the selected entry maps to
        // so we can compute scroll offset.
        let mut rows: Vec<(bool, usize)> = Vec::new();
        let mut current_provider: Option<&str> = None;

        for (display_idx, &entry_idx) in self.state.filtered_indices.iter().enumerate() {
            let entry = &self.state.entries[entry_idx];
            if current_provider != Some(&entry.provider) {
                current_provider = Some(&entry.provider);
                rows.push((true, usize::MAX)); // header row
            }
            rows.push((false, display_idx)); // model row
        }

        // Find the display row of the selected entry.
        let selected_row = rows
            .iter()
            .position(|(is_header, didx)| !is_header && *didx == self.state.selected)
            .unwrap_or(0);

        // Compute scroll offset to keep selected row visible.
        let scroll_offset = if selected_row >= visible_height {
            selected_row - visible_height + 1
        } else {
            0
        };

        // Render visible rows.
        for (row_idx, (is_header, display_idx)) in rows.iter().enumerate() {
            if row_idx < scroll_offset {
                continue;
            }
            let y_offset = row_idx - scroll_offset;
            if y_offset >= visible_height {
                break;
            }
            let y = area.y + y_offset as u16;

            if *is_header {
                // Find the provider name from the next model entry.
                let provider_name = rows
                    .iter()
                    .skip(row_idx + 1)
                    .find(|(h, _)| !h)
                    .and_then(|(_, didx)| {
                        self.state
                            .filtered_indices
                            .get(*didx)
                            .map(|&ei| self.state.entries[ei].provider.as_str())
                    })
                    .unwrap_or("Unknown");
                let header = Line::from(Span::styled(
                    provider_name,
                    self.theme.dim_style().add_modifier(Modifier::BOLD),
                ));
                Paragraph::new(header).render(Rect::new(area.x, y, area.width, 1), buf);
            } else {
                let entry_idx = self.state.filtered_indices[*display_idx];
                let entry = &self.state.entries[entry_idx];
                let is_selected = *display_idx == self.state.selected;
                let is_current = self
                    .state
                    .current_model
                    .as_deref()
                    .is_some_and(|cm| cm == entry.model_id);

                let prefix = if is_current && is_selected {
                    "* > "
                } else if is_current {
                    "*   "
                } else if is_selected {
                    "  > "
                } else {
                    "    "
                };

                let label = entry.label();
                let style = if is_selected {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    self.theme.text_style()
                };

                let line = Line::from(Span::styled(format!("{prefix}{label}"), style));
                Paragraph::new(line).render(Rect::new(area.x, y, area.width, 1), buf);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_model(id: &str, name: Option<&str>) -> ucode_providers::ModelInfo {
        ucode_providers::ModelInfo {
            id: id.to_owned(),
            name: name.map(str::to_owned),
        }
    }

    #[test]
    fn test_new_is_not_visible() {
        let state = ModelsModalState::new();
        assert!(!state.visible);
        assert!(!state.loading);
        assert_eq!(state.pending_providers, 0);
        assert!(state.entries.is_empty());
        assert!(state.filter.is_empty());
        assert_eq!(state.selected, 0);
        assert!(state.current_model.is_none());
    }

    #[test]
    fn test_open_sets_visible_and_loading() {
        let mut state = ModelsModalState::new();
        state.open(Some("gpt-4o".to_owned()), 3);
        assert!(state.visible);
        assert!(state.loading);
        assert_eq!(state.pending_providers, 3);
        assert!(state.entries.is_empty());
        assert_eq!(state.current_model, Some("gpt-4o".to_owned()));
    }

    #[test]
    fn test_close() {
        let mut state = ModelsModalState::new();
        state.open(None, 1);
        assert!(state.visible);
        state.close();
        assert!(!state.visible);
    }

    #[test]
    fn test_add_models_clears_loading() {
        let mut state = ModelsModalState::new();
        state.open(None, 2);
        assert!(state.loading);

        let models = vec![make_model("gpt-4o", Some("GPT-4o"))];
        state.add_models("openai", &models);
        // Still one provider pending.
        assert!(state.loading);
        assert_eq!(state.pending_providers, 1);

        let models2 = vec![make_model("claude-3-5-sonnet", None)];
        state.add_models("anthropic", &models2);
        // All providers done.
        assert!(!state.loading);
        assert_eq!(state.pending_providers, 0);
        assert_eq!(state.entries.len(), 2);
    }

    #[test]
    fn test_filter_narrows_list() {
        let mut state = ModelsModalState::new();
        state.open(None, 1);
        let models = vec![
            make_model("gpt-4o", Some("GPT-4o")),
            make_model("gpt-3.5-turbo", None),
            make_model("claude-3-5-sonnet", None),
        ];
        state.add_models("openai", &models);

        let total = state.filtered_indices.len();
        assert_eq!(total, 3);

        for c in "gpt".chars() {
            state.insert_char(c);
        }
        assert_eq!(state.filtered_indices.len(), 2);

        for c in "-4o".chars() {
            state.insert_char(c);
        }
        assert_eq!(state.filtered_indices.len(), 1);
        let entry = state.selected_entry().unwrap();
        assert_eq!(entry.model_id, "gpt-4o");
    }

    #[test]
    fn test_navigate_up_down() {
        let mut state = ModelsModalState::new();
        state.open(None, 1);
        let models = vec![
            make_model("a", None),
            make_model("b", None),
            make_model("c", None),
        ];
        state.add_models("p", &models);

        assert_eq!(state.selected, 0);
        state.move_up(); // saturates at 0
        assert_eq!(state.selected, 0);

        state.move_down();
        state.move_down();
        assert_eq!(state.selected, 2);

        state.move_up();
        assert_eq!(state.selected, 1);

        // Clamp at last.
        for _ in 0..10 {
            state.move_down();
        }
        assert_eq!(state.selected, 2);
    }

    #[test]
    fn test_selected_entry() {
        let mut state = ModelsModalState::new();
        state.open(None, 1);
        let models = vec![make_model("m1", None), make_model("m2", Some("Model Two"))];
        state.add_models("p", &models);

        assert_eq!(state.selected_entry().unwrap().model_id, "m1");
        state.move_down();
        assert_eq!(state.selected_entry().unwrap().model_id, "m2");
    }

    #[test]
    fn test_model_entry_label() {
        let same = ModelEntry {
            provider: "p".into(),
            model_id: "gpt-4o".into(),
            display_name: Some("gpt-4o".into()),
        };
        assert_eq!(same.label(), "gpt-4o");

        let different = ModelEntry {
            provider: "p".into(),
            model_id: "gpt-4o".into(),
            display_name: Some("GPT-4o".into()),
        };
        assert_eq!(different.label(), "gpt-4o (GPT-4o)");

        let none = ModelEntry {
            provider: "p".into(),
            model_id: "gpt-4o".into(),
            display_name: None,
        };
        assert_eq!(none.label(), "gpt-4o");
    }

    // -- widget render smoke tests ------------------------------------------

    fn make_terminal() -> ratatui::Terminal<ratatui::backend::TestBackend> {
        let backend = ratatui::backend::TestBackend::new(120, 40);
        ratatui::Terminal::new(backend).unwrap()
    }

    #[test]
    fn models_modal_hidden_renders_nothing() {
        let mut terminal = make_terminal();
        let state = ModelsModalState::new();
        let theme = crate::theme::UcodeTheme::default();
        terminal
            .draw(|f| f.render_widget(ModelsModal::new(&state, &theme), f.area()))
            .unwrap();
    }

    #[test]
    fn models_modal_loading_renders() {
        let mut terminal = make_terminal();
        let mut state = ModelsModalState::new();
        state.open(None, 2);
        let theme = crate::theme::UcodeTheme::default();
        terminal
            .draw(|f| f.render_widget(ModelsModal::new(&state, &theme), f.area()))
            .unwrap();
    }

    #[test]
    fn models_modal_with_entries_renders() {
        let mut terminal = make_terminal();
        let mut state = ModelsModalState::new();
        state.open(Some("gpt-4o".to_owned()), 1);
        let models = vec![
            make_model("gpt-4o", Some("GPT-4o")),
            make_model("gpt-3.5-turbo", None),
        ];
        state.add_models("openai", &models);
        let theme = crate::theme::UcodeTheme::default();
        terminal
            .draw(|f| f.render_widget(ModelsModal::new(&state, &theme), f.area()))
            .unwrap();
    }

    #[test]
    fn models_modal_empty_entries_renders() {
        let mut terminal = make_terminal();
        let mut state = ModelsModalState::new();
        state.open(None, 1);
        state.add_models("openai", &[]);
        let theme = crate::theme::UcodeTheme::default();
        terminal
            .draw(|f| f.render_widget(ModelsModal::new(&state, &theme), f.area()))
            .unwrap();
    }
}
