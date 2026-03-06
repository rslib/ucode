use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::theme::{ModelGroup, SandboxTier, UcodeTheme};

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct StatusBarState {
    pub keybind_hints: Vec<String>,
    /// Transient hint message shown temporarily (e.g. "Press Ctrl+C again to exit").
    pub hint_message: Option<String>,
    pub log_level: String,
    pub branch: String,
    pub is_forked: bool,
    pub sandbox_tier: SandboxTier,
    pub model_group: Option<ModelGroup>,
    pub model_name: String,
    pub agent_status: Option<String>,
    pub additions: u32,
    pub deletions: u32,
    pub cost: String,
    pub tokens_used: String,
    pub tokens_max: String,
    pub streaming: bool,
    pub stream_tok_per_sec: Option<f64>,
}

impl Default for StatusBarState {
    fn default() -> Self {
        Self {
            keybind_hints: vec!["^P".into(), "^O".into(), "^E".into()],
            hint_message: None,
            log_level: "INFO".into(),
            branch: "main".into(),
            is_forked: false,
            sandbox_tier: SandboxTier::Workspace,
            model_group: Some(ModelGroup::Strong),
            model_name: "claude-3-5-sonnet".into(),
            agent_status: None,
            additions: 0,
            deletions: 0,
            cost: "$0.00".into(),
            tokens_used: "0".into(),
            tokens_max: "200k".into(),
            streaming: false,
            stream_tok_per_sec: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Widget
// ---------------------------------------------------------------------------

pub struct StatusBar<'a> {
    pub state: &'a StatusBarState,
    pub theme: &'a UcodeTheme,
}

impl<'a> StatusBar<'a> {
    pub fn new(state: &'a StatusBarState, theme: &'a UcodeTheme) -> Self {
        Self { state, theme }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Map a log level string to the appropriate theme style.
pub fn log_level_style(level: &str, theme: &UcodeTheme) -> Style {
    match level {
        "DEBUG" => theme.accent_style(),
        "INFO" => theme.safe_style(),
        "WARN" => theme.warning_style(),
        "ERROR" | "FATAL" => theme.danger_style(),
        _ => theme.muted_style(),
    }
}

fn sep(theme: &UcodeTheme) -> Span<'static> {
    Span::styled(" │ ", Style::new().fg(theme.border))
}

/// Build the ordered list of segments.  Each segment is a small `Vec<Span>`
/// so that multi-span segments (e.g. model badge + name) stay together when
/// the truncation logic drops rightmost segments.
fn build_segments<'a>(state: &'a StatusBarState, theme: &'a UcodeTheme) -> Vec<Vec<Span<'a>>> {
    let mut segments: Vec<Vec<Span<'a>>> = Vec::new();

    // 1. Keybind hints
    if !state.keybind_hints.is_empty() {
        let hints = state.keybind_hints.join(" ");
        segments.push(vec![Span::styled(hints, theme.muted_style())]);
    }

    // 1b. Transient hint (high-priority: inserted right after keybind hints so
    //     it remains visible even on narrow terminals).
    if let Some(hint) = &state.hint_message {
        segments.push(vec![Span::styled(hint.clone(), theme.warning_style())]);
    }

    // 2. Log level
    segments.push(vec![Span::styled(
        state.log_level.clone(),
        log_level_style(&state.log_level, theme),
    )]);

    // 3. Branch  (append ⎇ when forked)
    let branch_text = if state.is_forked {
        format!("{} ⎇", state.branch)
    } else {
        state.branch.clone()
    };
    segments.push(vec![Span::styled(branch_text, theme.text_style())]);

    // 4. Sandbox tier
    segments.push(vec![Span::styled(
        state.sandbox_tier.symbol(),
        Style::new().fg(state.sandbox_tier.color(theme)),
    )]);

    // 5. Model  (badge in accent, name in text)
    {
        let badge_style = state
            .model_group
            .map(|g| g.style(theme, true))
            .unwrap_or_else(|| theme.muted_style());
        let badge_text = state.model_group.map(|g| g.badge()).unwrap_or("[unknown]");
        segments.push(vec![
            Span::styled(badge_text, badge_style),
            Span::styled(" ", Style::default()),
            Span::styled(state.model_name.clone(), theme.text_style()),
        ]);
    }

    if state.streaming {
        // Segments 6+7 replaced by a single streaming indicator.
        let label = match state.stream_tok_per_sec {
            Some(tps) => format!("streaming... ● {:.0} tok/s", tps),
            None => "streaming... ●".into(),
        };
        segments.push(vec![Span::styled(label, theme.warning_style())]);
    } else {
        // 6. Agent status (only when an agent is running)
        if let Some(agent) = &state.agent_status {
            segments.push(vec![Span::styled(
                format!("⟳ {}", agent),
                theme.warning_style(),
            )]);
        }

        // 7. Diff stats (only when non-zero)
        if state.additions > 0 || state.deletions > 0 {
            segments.push(vec![
                Span::styled(format!("+{}", state.additions), theme.safe_style()),
                Span::styled(" ", Style::default()),
                Span::styled(format!("-{}", state.deletions), theme.danger_style()),
            ]);
        }
    }

    // 8. Cost
    segments.push(vec![Span::styled(state.cost.clone(), theme.text_style())]);

    // 9. Token usage
    segments.push(vec![Span::styled(
        format!("{}/{}", state.tokens_used, state.tokens_max),
        theme.text_style(),
    )]);

    segments
}

impl Widget for StatusBar<'_> {
    fn render(self, area: ratatui::layout::Rect, buf: &mut ratatui::buffer::Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        let segments = build_segments(self.state, self.theme);
        let sep_width = 3u16; // " │ "

        // Measure each segment's display width (sum of content byte lengths;
        // segments are ASCII-dominant so this is a safe approximation).
        let seg_widths: Vec<u16> = segments
            .iter()
            .map(|spans| spans.iter().map(|s| s.content.len() as u16).sum())
            .collect();

        // Greedily include segments from left to right until the area is full.
        let mut used_width: u16 = 0;
        let mut visible_count = 0usize;
        for (i, &w) in seg_widths.iter().enumerate() {
            let needed = if i == 0 { w } else { sep_width + w };
            if used_width + needed > area.width {
                break;
            }
            used_width += needed;
            visible_count += 1;
        }
        // Always show at least one segment so the bar is never blank.
        visible_count = visible_count.max(1);

        // Flatten visible segments with separators into a single Line.
        let mut spans: Vec<Span<'_>> = Vec::new();
        for (i, seg) in segments.into_iter().take(visible_count).enumerate() {
            if i > 0 {
                spans.push(sep(self.theme));
            }
            spans.extend(seg);
        }

        let line = Line::from(spans);
        let row_area = ratatui::layout::Rect { height: 1, ..area };
        line.render(row_area, buf);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    fn render_to_buf(state: &StatusBarState, width: u16) -> Buffer {
        let theme = UcodeTheme::default();
        let widget = StatusBar::new(state, &theme);
        let area = Rect::new(0, 0, width, 1);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);
        buf
    }

    fn buf_text(buf: &Buffer) -> String {
        let area = buf.area;
        (0..area.width)
            .map(|x| buf[(x, 0)].symbol().to_owned())
            .collect()
    }

    #[test]
    fn status_bar_state_default() {
        let state = StatusBarState::default();
        assert_eq!(state.keybind_hints, vec!["^P", "^O", "^E"]);
        assert_eq!(state.log_level, "INFO");
        assert_eq!(state.branch, "main");
        assert!(!state.is_forked);
        assert_eq!(state.sandbox_tier, SandboxTier::Workspace);
        assert_eq!(state.model_group, Some(ModelGroup::Strong));
        assert_eq!(state.model_name, "claude-3-5-sonnet");
        assert!(state.agent_status.is_none());
        assert_eq!(state.additions, 0);
        assert_eq!(state.deletions, 0);
        assert!(!state.streaming);
        assert!(state.stream_tok_per_sec.is_none());
    }

    #[test]
    fn status_bar_renders_basic() {
        let state = StatusBarState::default();
        let buf = render_to_buf(&state, 120);
        let text = buf_text(&buf);
        assert!(text.contains("^P"), "expected ^P in: {text:?}");
        assert!(text.contains("^O"), "expected ^O in: {text:?}");
        assert!(text.contains("^E"), "expected ^E in: {text:?}");
    }

    #[test]
    fn status_bar_streaming_mode() {
        let state = StatusBarState {
            streaming: true,
            stream_tok_per_sec: Some(42.0),
            ..StatusBarState::default()
        };
        let buf = render_to_buf(&state, 120);
        let text = buf_text(&buf);
        assert!(
            text.contains("streaming"),
            "expected 'streaming' in: {text:?}"
        );
        assert!(text.contains("42"), "expected tok/s rate in: {text:?}");
    }

    #[test]
    fn status_bar_streaming_no_rate() {
        let state = StatusBarState {
            streaming: true,
            stream_tok_per_sec: None,
            ..StatusBarState::default()
        };
        let buf = render_to_buf(&state, 120);
        let text = buf_text(&buf);
        assert!(
            text.contains("streaming"),
            "expected 'streaming' in: {text:?}"
        );
    }

    #[test]
    fn status_bar_sandbox_tier_symbol() {
        for (tier, expected) in [
            (SandboxTier::Off, "●off"),
            (SandboxTier::Workspace, "●ws"),
            (SandboxTier::Networked, "●net"),
            (SandboxTier::Strict, "●strict"),
        ] {
            let state = StatusBarState {
                sandbox_tier: tier,
                ..StatusBarState::default()
            };
            let buf = render_to_buf(&state, 120);
            let text = buf_text(&buf);
            assert!(
                text.contains(expected),
                "tier {tier:?}: expected {expected:?} in: {text:?}"
            );
        }
    }

    #[test]
    fn status_bar_diff_stats() {
        let theme = UcodeTheme::default();
        let state = StatusBarState {
            additions: 29,
            deletions: 10,
            ..StatusBarState::default()
        };
        let widget = StatusBar::new(&state, &theme);
        let area = Rect::new(0, 0, 120, 1);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);

        let text = buf_text(&buf);
        assert!(text.contains("+29"), "expected +29 in: {text:?}");
        assert!(text.contains("-10"), "expected -10 in: {text:?}");

        // Verify styling: the '+' cell should carry the safe color.
        let plus_x = (0..120u16)
            .find(|&x| buf[(x, 0)].symbol() == "+")
            .expect("'+' cell not found");
        assert_eq!(
            buf[(plus_x, 0)].style().fg,
            Some(theme.safe),
            "additions should use safe color"
        );

        // The '-' cell of "-10" should carry the danger color.
        // Search for the '-' immediately followed by '1' (to skip the '-' chars
        // inside "claude-3-5-sonnet" which use text color).
        let minus_x = (0..119u16)
            .find(|&x| buf[(x, 0)].symbol() == "-" && buf[(x + 1, 0)].symbol() == "1")
            .expect("'-1' sequence not found in buffer");
        assert_eq!(
            buf[(minus_x, 0)].style().fg,
            Some(theme.danger),
            "deletions should use danger color"
        );
    }

    #[test]
    fn log_level_color_mapping() {
        let theme = UcodeTheme::default();
        assert_eq!(log_level_style("INFO", &theme), theme.safe_style());
        assert_eq!(log_level_style("DEBUG", &theme), theme.accent_style());
        assert_eq!(log_level_style("WARN", &theme), theme.warning_style());
        assert_eq!(log_level_style("ERROR", &theme), theme.danger_style());
        assert_eq!(log_level_style("FATAL", &theme), theme.danger_style());
        // Unknown level falls back to muted.
        assert_eq!(log_level_style("TRACE", &theme), theme.muted_style());
    }

    #[test]
    fn status_bar_agent_status_shown() {
        let state = StatusBarState {
            agent_status: Some("agent-b".into()),
            ..StatusBarState::default()
        };
        let buf = render_to_buf(&state, 120);
        let text = buf_text(&buf);
        assert!(text.contains("agent-b"), "expected agent-b in: {text:?}");
        assert!(text.contains('⟳'), "expected ⟳ prefix in: {text:?}");
    }

    #[test]
    fn status_bar_forked_branch_symbol() {
        let state = StatusBarState {
            branch: "feature".into(),
            is_forked: true,
            ..StatusBarState::default()
        };
        let buf = render_to_buf(&state, 120);
        let text = buf_text(&buf);
        assert!(
            text.contains("feature"),
            "expected branch name in: {text:?}"
        );
        assert!(text.contains('⎇'), "expected ⎇ symbol in: {text:?}");
    }

    #[test]
    fn status_bar_narrow_does_not_panic() {
        let state = StatusBarState::default();
        let _ = render_to_buf(&state, 10);
    }

    #[test]
    fn status_bar_zero_width_does_not_panic() {
        let state = StatusBarState::default();
        let _ = render_to_buf(&state, 0);
    }

    #[test]
    fn status_bar_no_hint_by_default() {
        let state = StatusBarState::default();
        assert!(state.hint_message.is_none());
    }

    #[test]
    fn status_bar_hint_message_rendered() {
        let state = StatusBarState {
            hint_message: Some("Press Ctrl+C again to exit".into()),
            ..StatusBarState::default()
        };
        let buf = render_to_buf(&state, 120);
        let text = buf_text(&buf);
        assert!(
            text.contains("Press Ctrl+C again to exit"),
            "expected hint in: {text:?}"
        );
    }
}
