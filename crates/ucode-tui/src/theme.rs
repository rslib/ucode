use ratatui::style::{Color, Style};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Palette helpers
// ---------------------------------------------------------------------------

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThemePreset {
    #[default]
    Hybrid,
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Density {
    Compact,
    #[default]
    Comfortable,
}

impl Density {
    /// Blank lines between sidebar sections.
    pub fn section_spacing(self) -> u16 {
        match self {
            Density::Compact => 0,
            Density::Comfortable => 1,
        }
    }

    /// Lines used to render a single tool-call entry.
    pub fn tool_call_lines(self) -> u16 {
        match self {
            Density::Compact => 1,
            Density::Comfortable => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SandboxTier {
    #[default]
    Off,
    Workspace,
    Networked,
    Strict,
}

impl SandboxTier {
    pub fn symbol(self) -> &'static str {
        match self {
            SandboxTier::Off => "●off",
            SandboxTier::Workspace => "●ws",
            SandboxTier::Networked => "●net",
            SandboxTier::Strict => "●strict",
        }
    }

    pub fn color(self, theme: &UcodeTheme) -> Color {
        match self {
            SandboxTier::Off => theme.muted,
            SandboxTier::Workspace => theme.safe,
            SandboxTier::Networked => theme.warning,
            SandboxTier::Strict => theme.accent,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelGroup {
    Fast,
    Strong,
    LongCtx,
}

impl ModelGroup {
    pub fn badge(self) -> &'static str {
        match self {
            ModelGroup::Fast => "[fast]",
            ModelGroup::Strong => "[strong]",
            ModelGroup::LongCtx => "[longctx]",
        }
    }

    /// Accent when active (primary target), Muted when inactive (fallback target).
    pub fn style(self, theme: &UcodeTheme, active: bool) -> Style {
        if active {
            theme.accent_style()
        } else {
            theme.muted_style()
        }
    }
}

// ---------------------------------------------------------------------------
// Theme
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UcodeTheme {
    pub background: Color,
    pub surface: Color,
    pub border: Color,
    pub border_focus: Color,
    pub accent: Color,
    pub safe: Color,
    pub warning: Color,
    pub danger: Color,
    pub muted: Color,
    pub text: Color,
    pub text_dim: Color,
}

impl Default for UcodeTheme {
    fn default() -> Self {
        Self {
            background: rgb(0x0d, 0x0d, 0x0d),
            surface: rgb(0x14, 0x14, 0x14),
            border: rgb(0x2a, 0x2a, 0x2a),
            border_focus: rgb(0x3a, 0x3a, 0x3a),
            accent: rgb(0x00, 0xd4, 0xaa),
            safe: rgb(0x22, 0xc5, 0x5e),
            warning: rgb(0xf5, 0x9e, 0x0b),
            danger: rgb(0xef, 0x44, 0x44),
            muted: rgb(0x6b, 0x72, 0x80),
            text: rgb(0xe5, 0xe7, 0xeb),
            text_dim: rgb(0x9c, 0xa3, 0xaf),
        }
    }
}

impl UcodeTheme {
    /// Return the default theme with a custom accent color.
    pub fn with_accent(accent: Color) -> Self {
        Self {
            accent,
            ..Self::default()
        }
    }

    // ------------------------------------------------------------------
    // Style helpers
    // ------------------------------------------------------------------

    pub fn border_style(&self, focused: bool) -> Style {
        let color = if focused {
            self.border_focus
        } else {
            self.border
        };
        Style::new().fg(color)
    }

    pub fn text_style(&self) -> Style {
        Style::new().fg(self.text)
    }

    pub fn dim_style(&self) -> Style {
        Style::new().fg(self.text_dim)
    }

    pub fn muted_style(&self) -> Style {
        Style::new().fg(self.muted)
    }

    pub fn accent_style(&self) -> Style {
        Style::new().fg(self.accent)
    }

    pub fn safe_style(&self) -> Style {
        Style::new().fg(self.safe)
    }

    pub fn warning_style(&self) -> Style {
        Style::new().fg(self.warning)
    }

    pub fn danger_style(&self) -> Style {
        Style::new().fg(self.danger)
    }

    /// Background style for panels and sidebar surfaces.
    pub fn surface_style(&self) -> Style {
        Style::new().bg(self.surface)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_theme_has_correct_accent() {
        let theme = UcodeTheme::default();
        assert_eq!(theme.accent, Color::Rgb(0x00, 0xd4, 0xaa));
    }

    #[test]
    fn with_accent_overrides() {
        let custom = Color::Rgb(0xff, 0x00, 0x80);
        let theme = UcodeTheme::with_accent(custom);
        assert_eq!(theme.accent, custom);
        // Other fields remain at defaults.
        assert_eq!(theme.text, UcodeTheme::default().text);
    }

    #[test]
    fn sandbox_tier_symbols() {
        assert_eq!(SandboxTier::Off.symbol(), "●off");
        assert_eq!(SandboxTier::Workspace.symbol(), "●ws");
        assert_eq!(SandboxTier::Networked.symbol(), "●net");
        assert_eq!(SandboxTier::Strict.symbol(), "●strict");
    }

    #[test]
    fn sandbox_tier_colors() {
        let theme = UcodeTheme::default();
        assert_eq!(SandboxTier::Off.color(&theme), theme.muted);
        assert_eq!(SandboxTier::Workspace.color(&theme), theme.safe);
        assert_eq!(SandboxTier::Networked.color(&theme), theme.warning);
        assert_eq!(SandboxTier::Strict.color(&theme), theme.accent);
    }

    #[test]
    fn model_group_badges() {
        assert_eq!(ModelGroup::Fast.badge(), "[fast]");
        assert_eq!(ModelGroup::Strong.badge(), "[strong]");
        assert_eq!(ModelGroup::LongCtx.badge(), "[longctx]");
    }

    #[test]
    fn model_group_active_vs_inactive_style() {
        let theme = UcodeTheme::default();
        let active = ModelGroup::Fast.style(&theme, true);
        let inactive = ModelGroup::Fast.style(&theme, false);
        assert_eq!(active, theme.accent_style());
        assert_eq!(inactive, theme.muted_style());
        assert_ne!(active, inactive);
    }

    #[test]
    fn border_style_focused_vs_unfocused() {
        let theme = UcodeTheme::default();
        let focused = theme.border_style(true);
        let unfocused = theme.border_style(false);
        assert_eq!(focused, Style::new().fg(theme.border_focus));
        assert_eq!(unfocused, Style::new().fg(theme.border));
        assert_ne!(focused, unfocused);
    }

    #[test]
    fn density_spacing() {
        assert_eq!(Density::Compact.section_spacing(), 0);
        assert_eq!(Density::Comfortable.section_spacing(), 1);
    }

    #[test]
    fn density_tool_call_lines() {
        assert_eq!(Density::Compact.tool_call_lines(), 1);
        assert_eq!(Density::Comfortable.tool_call_lines(), 2);
    }
}
