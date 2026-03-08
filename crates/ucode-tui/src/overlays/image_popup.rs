use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, Clear, StatefulWidget, Widget};

use ratatui_image::StatefulImage;
use ratatui_image::protocol::StatefulProtocol;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// State for the image popup overlay.
pub struct ImagePopupState {
    pub visible: bool,
    /// Human-readable label (file path or "clipboard").
    pub label: String,
    /// The ratatui-image protocol state for rendering.
    pub protocol: Option<StatefulProtocol>,
}

impl ImagePopupState {
    pub fn new() -> Self {
        Self {
            visible: false,
            label: String::new(),
            protocol: None,
        }
    }

    /// Open the popup with a decoded image.
    ///
    /// `picker` is the `ratatui_image::picker::Picker` from `AppState`.
    /// `label` is a human-readable name (e.g. file path).
    /// `data` is the raw image bytes (PNG, JPEG, etc.).
    pub fn open(
        &mut self,
        picker: &mut ratatui_image::picker::Picker,
        label: String,
        data: &[u8],
    ) -> Result<(), String> {
        let img = image::load_from_memory(data).map_err(|e| format!("image decode: {e}"))?;
        let protocol = picker.new_resize_protocol(img);
        self.protocol = Some(protocol);
        self.label = label;
        self.visible = true;
        Ok(())
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.protocol = None;
        self.label.clear();
    }
}

impl Default for ImagePopupState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_width = area.width * percent_x / 100;
    let popup_height = area.height * percent_y / 100;
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    Rect::new(x, y, popup_width, popup_height)
}

// ---------------------------------------------------------------------------
// Widget
// ---------------------------------------------------------------------------

pub struct ImagePopup<'a> {
    state: &'a mut ImagePopupState,
    theme: &'a crate::theme::UcodeTheme,
}

impl<'a> ImagePopup<'a> {
    pub fn new(state: &'a mut ImagePopupState, theme: &'a crate::theme::UcodeTheme) -> Self {
        Self { state, theme }
    }

    /// Render the image popup. Called directly (not via `f.render_widget`) because
    /// it needs mutable access to the protocol state.
    pub fn render(self, area: Rect, buf: &mut Buffer) {
        if !self.state.visible {
            return;
        }

        let popup = centered_rect(80, 80, area);
        Clear.render(popup, buf);

        let title = if self.state.label.is_empty() {
            " Image ".to_owned()
        } else {
            let max_len = popup.width.saturating_sub(6) as usize;
            let truncated = if self.state.label.len() > max_len {
                format!(
                    "...{}",
                    &self.state.label[self.state.label.len() - max_len + 3..]
                )
            } else {
                self.state.label.clone()
            };
            format!(" {} ", truncated)
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border_style(true))
            .title(title);
        let inner = block.inner(popup);
        block.render(popup, buf);

        if inner.width < 4 || inner.height < 2 {
            return;
        }

        if let Some(ref mut protocol) = self.state.protocol {
            let image_widget = StatefulImage::default();
            image_widget.render(inner, buf, protocol);
        }

        // Footer hint.
        let footer_y = popup.y + popup.height.saturating_sub(1);
        if footer_y < area.y + area.height {
            let footer = ratatui::text::Line::from(vec![
                ratatui::text::Span::styled("Esc", self.theme.accent_style()),
                ratatui::text::Span::styled(" close", self.theme.dim_style()),
            ]);
            let footer_area = Rect::new(inner.x + 1, footer_y, inner.width.saturating_sub(2), 1);
            ratatui::widgets::Paragraph::new(footer).render(footer_area, buf);
        }
    }
}
