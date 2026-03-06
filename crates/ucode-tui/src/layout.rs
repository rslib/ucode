use ratatui::layout::{Constraint, Layout, Rect};

pub const MIN_WIDTH: u16 = 80;
pub const MIN_HEIGHT: u16 = 24;
pub const SIDEBAR_DEFAULT_WIDTH: u16 = 34;
pub const SIDEBAR_MIN_WIDTH: u16 = 28;
pub const SIDEBAR_MAX_WIDTH: u16 = 48;
pub const SIDEBAR_ICON_STRIP_WIDTH: u16 = 6;
pub const SIDEBAR_FULL_THRESHOLD: u16 = 120;
pub const INPUT_MAX_LINES: u16 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalSize {
    pub width: u16,
    pub height: u16,
}

impl TerminalSize {
    pub fn is_minimum(&self) -> bool {
        self.width >= MIN_WIDTH && self.height >= MIN_HEIGHT
    }

    pub fn sidebar_mode(&self) -> SidebarMode {
        if self.width >= SIDEBAR_FULL_THRESHOLD {
            SidebarMode::Full
        } else if self.width >= MIN_WIDTH {
            SidebarMode::IconStrip
        } else {
            SidebarMode::Hidden
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarMode {
    Full,
    IconStrip,
    Hidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SidebarState {
    pub width: u16,
    pub mode: SidebarMode,
}

impl SidebarState {
    pub fn new(mode: SidebarMode) -> Self {
        Self {
            width: SIDEBAR_DEFAULT_WIDTH,
            mode,
        }
    }

    pub fn grow(&mut self) {
        self.width = (self.width + 2).min(SIDEBAR_MAX_WIDTH);
    }

    pub fn shrink(&mut self) {
        self.width = self.width.saturating_sub(2).max(SIDEBAR_MIN_WIDTH);
    }

    pub fn icon_strip_width() -> u16 {
        SIDEBAR_ICON_STRIP_WIDTH
    }

    fn effective_width(&self) -> u16 {
        match self.mode {
            SidebarMode::Full => self.width,
            SidebarMode::IconStrip => SIDEBAR_ICON_STRIP_WIDTH,
            SidebarMode::Hidden => 0,
        }
    }
}

impl Default for SidebarState {
    fn default() -> Self {
        Self::new(SidebarMode::Full)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputState {
    /// Number of visible input lines, clamped to 1..=INPUT_MAX_LINES.
    pub line_count: u16,
}

impl InputState {
    pub fn new(line_count: u16) -> Self {
        Self {
            line_count: line_count.clamp(1, INPUT_MAX_LINES),
        }
    }

    /// Height including top and bottom border.
    pub fn height(&self) -> u16 {
        self.line_count + 2
    }
}

impl Default for InputState {
    fn default() -> Self {
        Self::new(1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutAreas {
    pub title_bar: Rect,
    pub transcript: Rect,
    pub sidebar: Rect,
    pub input: Rect,
    pub status_bar: Rect,
}

/// Compute the five layout areas from the full terminal `area`.
///
/// Vertical split (top to bottom):
///   title_bar  — Length(1)
///   middle     — Fill(1)  → split horizontally into transcript | sidebar
///   input      — Length(input.height())
///   status_bar — Length(1)
pub fn compute_layout(area: Rect, sidebar: &SidebarState, input: &InputState) -> LayoutAreas {
    let [title_bar, middle, input_area, status_bar] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(input.height()),
        Constraint::Length(1),
    ])
    .areas(area);

    let sidebar_width = sidebar.effective_width();

    let [transcript, sidebar_area] = if sidebar_width > 0 {
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(sidebar_width)]).areas(middle)
    } else {
        // No sidebar: give everything to transcript, sidebar gets a zero-size rect.
        let [transcript] = Layout::horizontal([Constraint::Fill(1)]).areas(middle);
        [transcript, Rect::default()]
    };

    LayoutAreas {
        title_bar,
        transcript,
        sidebar: sidebar_area,
        input: input_area,
        status_bar,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_area(w: u16, h: u16) -> Rect {
        Rect::new(0, 0, w, h)
    }

    #[test]
    fn terminal_size_minimum() {
        assert!(
            TerminalSize {
                width: 80,
                height: 24
            }
            .is_minimum()
        );
        assert!(
            TerminalSize {
                width: 200,
                height: 50
            }
            .is_minimum()
        );
        assert!(
            !TerminalSize {
                width: 79,
                height: 24
            }
            .is_minimum()
        );
        assert!(
            !TerminalSize {
                width: 80,
                height: 23
            }
            .is_minimum()
        );
    }

    #[test]
    fn sidebar_mode_full() {
        assert_eq!(
            TerminalSize {
                width: 120,
                height: 24
            }
            .sidebar_mode(),
            SidebarMode::Full
        );
        assert_eq!(
            TerminalSize {
                width: 200,
                height: 24
            }
            .sidebar_mode(),
            SidebarMode::Full
        );
    }

    #[test]
    fn sidebar_mode_icon_strip() {
        assert_eq!(
            TerminalSize {
                width: 80,
                height: 24
            }
            .sidebar_mode(),
            SidebarMode::IconStrip
        );
        assert_eq!(
            TerminalSize {
                width: 119,
                height: 24
            }
            .sidebar_mode(),
            SidebarMode::IconStrip
        );
    }

    #[test]
    fn sidebar_mode_hidden() {
        assert_eq!(
            TerminalSize {
                width: 79,
                height: 24
            }
            .sidebar_mode(),
            SidebarMode::Hidden
        );
        assert_eq!(
            TerminalSize {
                width: 0,
                height: 24
            }
            .sidebar_mode(),
            SidebarMode::Hidden
        );
    }

    #[test]
    fn sidebar_grow_clamp() {
        let mut s = SidebarState::new(SidebarMode::Full);
        s.width = SIDEBAR_MAX_WIDTH;
        s.grow();
        assert_eq!(s.width, SIDEBAR_MAX_WIDTH);

        s.width = SIDEBAR_MAX_WIDTH - 1;
        s.grow();
        assert_eq!(s.width, SIDEBAR_MAX_WIDTH);
    }

    #[test]
    fn sidebar_shrink_clamp() {
        let mut s = SidebarState::new(SidebarMode::Full);
        s.width = SIDEBAR_MIN_WIDTH;
        s.shrink();
        assert_eq!(s.width, SIDEBAR_MIN_WIDTH);

        s.width = SIDEBAR_MIN_WIDTH + 1;
        s.shrink();
        assert_eq!(s.width, SIDEBAR_MIN_WIDTH);
    }

    #[test]
    fn compute_layout_full_sidebar() {
        let area = make_area(200, 50);
        let sidebar = SidebarState::new(SidebarMode::Full);
        let input = InputState::default();
        let layout = compute_layout(area, &sidebar, &input);

        // title bar: row 0, height 1
        assert_eq!(layout.title_bar.y, 0);
        assert_eq!(layout.title_bar.height, 1);
        assert_eq!(layout.title_bar.width, 200);

        // status bar: last row, height 1
        assert_eq!(layout.status_bar.y, 49);
        assert_eq!(layout.status_bar.height, 1);
        assert_eq!(layout.status_bar.width, 200);

        // input: above status bar, height = 1 line + 2 borders = 3
        assert_eq!(layout.input.height, 3);
        assert_eq!(layout.input.y, 46);
        assert_eq!(layout.input.width, 200);

        // sidebar: right portion, width = SIDEBAR_DEFAULT_WIDTH
        assert_eq!(layout.sidebar.width, SIDEBAR_DEFAULT_WIDTH);
        assert_eq!(layout.sidebar.x, 200 - SIDEBAR_DEFAULT_WIDTH);

        // transcript: left portion
        assert_eq!(layout.transcript.width, 200 - SIDEBAR_DEFAULT_WIDTH);
        assert_eq!(layout.transcript.x, 0);
    }

    #[test]
    fn compute_layout_icon_strip() {
        let area = make_area(100, 30);
        let sidebar = SidebarState::new(SidebarMode::IconStrip);
        let input = InputState::default();
        let layout = compute_layout(area, &sidebar, &input);

        assert_eq!(layout.sidebar.width, SIDEBAR_ICON_STRIP_WIDTH);
        assert_eq!(layout.transcript.width, 100 - SIDEBAR_ICON_STRIP_WIDTH);
    }

    #[test]
    fn compute_layout_multiline_input() {
        let area = make_area(120, 40);
        let sidebar = SidebarState::new(SidebarMode::Full);
        let input = InputState::new(3);
        let layout = compute_layout(area, &sidebar, &input);

        // 3 lines + 2 borders = 5
        assert_eq!(layout.input.height, 5);
    }

    #[test]
    fn layout_areas_no_overlap() {
        let area = make_area(160, 40);
        let sidebar = SidebarState::new(SidebarMode::Full);
        let input = InputState::new(2);
        let layout = compute_layout(area, &sidebar, &input);

        let rects = [
            ("title_bar", layout.title_bar),
            ("transcript", layout.transcript),
            ("sidebar", layout.sidebar),
            ("input", layout.input),
            ("status_bar", layout.status_bar),
        ];

        // No two non-empty rects should overlap.
        for i in 0..rects.len() {
            for j in (i + 1)..rects.len() {
                let (na, a) = rects[i];
                let (nb, b) = rects[j];
                if a.area() == 0 || b.area() == 0 {
                    continue;
                }
                let overlap = a.x < b.x + b.width
                    && b.x < a.x + a.width
                    && a.y < b.y + b.height
                    && b.y < a.y + a.height;
                assert!(!overlap, "{na} and {nb} overlap: {a:?} vs {b:?}");
            }
        }

        // The union of all rects should cover the full area.
        // We verify by checking total cell count equals area cells.
        // (This works because the layout is a perfect partition.)
        let total_cells: u32 = rects.iter().map(|(_, r)| r.area()).sum();
        assert_eq!(total_cells, area.area());
    }
}
