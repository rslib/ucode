use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Widget};

use crate::theme::UcodeTheme;

// ---------------------------------------------------------------------------
// ToastLevel
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastLevel {
    Info,
    Success,
    Warning,
    Error,
}

impl ToastLevel {
    /// Icon prefix for the toast.
    pub fn icon(self) -> &'static str {
        match self {
            Self::Info => "ℹ",
            Self::Success => "✓",
            Self::Warning => "⚠",
            Self::Error => "✗",
        }
    }

    /// Auto-dismiss duration in milliseconds. `None` = persistent (manual dismiss).
    pub fn auto_dismiss_ms(self) -> Option<u64> {
        match self {
            Self::Info | Self::Success => Some(4000),
            Self::Warning => Some(8000),
            Self::Error => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Toast
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Toast {
    pub id: u64,
    pub level: ToastLevel,
    pub title: String,
    pub body: Option<String>,
    pub created_at: std::time::Instant,
    pub dismiss_after_ms: Option<u64>,
}

impl Toast {
    pub fn new(id: u64, level: ToastLevel, title: impl Into<String>) -> Self {
        Self {
            id,
            level,
            title: title.into(),
            body: None,
            created_at: std::time::Instant::now(),
            dismiss_after_ms: level.auto_dismiss_ms(),
        }
    }

    pub fn with_body(mut self, body: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self
    }

    /// True if this toast has exceeded its auto-dismiss timer.
    pub fn is_expired(&self) -> bool {
        if let Some(ms) = self.dismiss_after_ms {
            self.created_at.elapsed().as_millis() as u64 >= ms
        } else {
            false
        }
    }
}

// ---------------------------------------------------------------------------
// ToastState
// ---------------------------------------------------------------------------

/// Maximum number of toasts visible at once.
const MAX_VISIBLE_TOASTS: usize = 3;

#[derive(Debug, Clone)]
pub struct ToastState {
    toasts: Vec<Toast>,
    next_id: u64,
}

impl ToastState {
    pub fn new() -> Self {
        Self {
            toasts: Vec::new(),
            next_id: 0,
        }
    }

    /// Add a toast and return its id.
    pub fn push(&mut self, level: ToastLevel, title: impl Into<String>) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.toasts.push(Toast::new(id, level, title));
        id
    }

    /// Add a toast with body text and return its id.
    pub fn push_with_body(
        &mut self,
        level: ToastLevel,
        title: impl Into<String>,
        body: impl Into<String>,
    ) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.toasts
            .push(Toast::new(id, level, title).with_body(body));
        id
    }

    /// Dismiss a toast by id.
    pub fn dismiss(&mut self, id: u64) {
        self.toasts.retain(|t| t.id != id);
    }

    /// Dismiss the most recent (topmost visible) toast.
    pub fn dismiss_top(&mut self) {
        if let Some(id) = self.visible().last().map(|t| t.id) {
            self.dismiss(id);
        }
    }

    /// Remove expired toasts. Returns `true` if any were removed.
    pub fn tick(&mut self) -> bool {
        let before = self.toasts.len();
        self.toasts.retain(|t| !t.is_expired());
        self.toasts.len() != before
    }

    /// Visible toasts (up to `MAX_VISIBLE_TOASTS`, most recent last).
    pub fn visible(&self) -> &[Toast] {
        let start = self.toasts.len().saturating_sub(MAX_VISIBLE_TOASTS);
        &self.toasts[start..]
    }

    pub fn is_empty(&self) -> bool {
        self.toasts.is_empty()
    }

    pub fn len(&self) -> usize {
        self.toasts.len()
    }

    pub fn clear(&mut self) {
        self.toasts.clear();
    }
}

impl Default for ToastState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// ToastStack widget
// ---------------------------------------------------------------------------

pub struct ToastStack<'a> {
    pub state: &'a ToastState,
    pub theme: &'a UcodeTheme,
}

impl Widget for ToastStack<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let toasts = self.state.visible();
        if toasts.is_empty() || area.width < 20 || area.height < 3 {
            return;
        }

        // Each toast is 3 rows tall (border + title + border), or 4 with body.
        // Stack from top-right, going downward.
        let toast_width = area.width.min(40);
        let x = area.x + area.width.saturating_sub(toast_width + 1);

        let mut y = area.y + 1; // 1 row below top for title bar

        for toast in toasts {
            let height: u16 = if toast.body.is_some() { 4 } else { 3 };
            if y + height > area.y + area.height {
                break;
            }

            let toast_area = Rect::new(x, y, toast_width, height);

            let (border_color, icon_color) = match toast.level {
                ToastLevel::Info => (self.theme.accent, self.theme.accent),
                ToastLevel::Success => (self.theme.safe, self.theme.safe),
                ToastLevel::Warning => (self.theme.warning, self.theme.warning),
                ToastLevel::Error => (self.theme.danger, self.theme.danger),
            };

            Clear.render(toast_area, buf);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .style(Style::default().bg(self.theme.surface));
            let inner = block.inner(toast_area);
            block.render(toast_area, buf);

            if inner.height == 0 || inner.width == 0 {
                y += height + 1;
                continue;
            }

            let icon = toast.level.icon();
            let title_line = Line::from(vec![
                Span::styled(format!("{icon} "), Style::default().fg(icon_color)),
                Span::styled(
                    &toast.title,
                    Style::default()
                        .fg(self.theme.text)
                        .add_modifier(Modifier::BOLD),
                ),
            ]);
            let title_area = Rect {
                y: inner.y,
                height: 1,
                ..inner
            };
            title_line.render(title_area, buf);

            if let Some(body) = &toast.body
                && inner.height > 1
            {
                let body_line = Line::from(Span::styled(
                    body.as_str(),
                    Style::default().fg(self.theme.muted),
                ));
                let body_area = Rect {
                    y: inner.y + 1,
                    height: 1,
                    ..inner
                };
                body_line.render(body_area, buf);
            }

            y += height + 1;
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_toast_level_icons() {
        assert_eq!(ToastLevel::Info.icon(), "ℹ");
        assert_eq!(ToastLevel::Success.icon(), "✓");
        assert_eq!(ToastLevel::Warning.icon(), "⚠");
        assert_eq!(ToastLevel::Error.icon(), "✗");
    }

    #[test]
    fn test_toast_level_auto_dismiss() {
        assert_eq!(ToastLevel::Info.auto_dismiss_ms(), Some(4000));
        assert_eq!(ToastLevel::Success.auto_dismiss_ms(), Some(4000));
        assert_eq!(ToastLevel::Warning.auto_dismiss_ms(), Some(8000));
        assert_eq!(ToastLevel::Error.auto_dismiss_ms(), None);
    }

    #[test]
    fn test_toast_new() {
        let t = Toast::new(42, ToastLevel::Info, "hello");
        assert_eq!(t.id, 42);
        assert_eq!(t.level, ToastLevel::Info);
        assert_eq!(t.title, "hello");
        assert!(t.body.is_none());
        assert_eq!(t.dismiss_after_ms, Some(4000));
    }

    #[test]
    fn test_toast_with_body() {
        let t = Toast::new(0, ToastLevel::Warning, "title").with_body("details");
        assert_eq!(t.body.as_deref(), Some("details"));
    }

    #[test]
    fn test_toast_state_push() {
        let mut state = ToastState::new();
        state.push(ToastLevel::Info, "a");
        state.push(ToastLevel::Success, "b");
        assert_eq!(state.len(), 2);
    }

    #[test]
    fn test_toast_state_dismiss() {
        let mut state = ToastState::new();
        let id = state.push(ToastLevel::Info, "hello");
        state.push(ToastLevel::Error, "world");
        state.dismiss(id);
        assert_eq!(state.len(), 1);
        assert_eq!(state.visible()[0].title, "world");
    }

    #[test]
    fn test_toast_state_dismiss_top() {
        let mut state = ToastState::new();
        state.push(ToastLevel::Info, "first");
        state.push(ToastLevel::Info, "second");
        state.push(ToastLevel::Info, "third");
        state.dismiss_top();
        assert_eq!(state.len(), 2);
        // "third" (last pushed) should be gone
        assert!(state.visible().iter().all(|t| t.title != "third"));
    }

    #[test]
    fn test_toast_state_visible_max_3() {
        let mut state = ToastState::new();
        for i in 0..5 {
            state.push(ToastLevel::Info, format!("toast {i}"));
        }
        assert_eq!(state.len(), 5);
        let visible = state.visible();
        assert_eq!(visible.len(), 3);
        // Should be the last 3 pushed
        assert_eq!(visible[0].title, "toast 2");
        assert_eq!(visible[1].title, "toast 3");
        assert_eq!(visible[2].title, "toast 4");
    }

    #[test]
    fn test_toast_state_tick_removes_expired() {
        let mut state = ToastState::new();
        let id = state.next_id;
        state.next_id += 1;
        // Create a toast that is already expired (dismiss_after_ms = 0).
        let mut toast = Toast::new(id, ToastLevel::Info, "expiring");
        toast.dismiss_after_ms = Some(0);
        state.toasts.push(toast);

        // Give it a tiny moment so elapsed >= 0ms is guaranteed.
        std::thread::sleep(std::time::Duration::from_millis(1));

        let removed = state.tick();
        assert!(removed);
        assert!(state.is_empty());
    }

    #[test]
    fn test_toast_state_error_persistent() {
        let mut state = ToastState::new();
        state.push(ToastLevel::Error, "critical");
        let removed = state.tick();
        assert!(!removed);
        assert_eq!(state.len(), 1);
    }

    #[test]
    fn test_toast_state_clear() {
        let mut state = ToastState::new();
        state.push(ToastLevel::Info, "a");
        state.push(ToastLevel::Error, "b");
        state.clear();
        assert!(state.is_empty());
    }
}
