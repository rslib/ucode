use ratatui::style::{Color, Style};
use serde::{Deserialize, Serialize};
use ucode_themes::{ThemeDef, builtin_theme, theme_names};

// ---------------------------------------------------------------------------
// Rgb → Color conversion
// ---------------------------------------------------------------------------

fn to_color(rgb: ucode_themes::Rgb) -> Color {
    Color::Rgb(rgb.r, rgb.g, rgb.b)
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// Legacy preset enum kept for backward compatibility.
///
/// New code should prefer `UcodeTheme::from_def` / `next_theme` directly.
/// The three variants map to the first three built-in themes: "ucode",
/// "tokyonight", and the light-background "ucode" variant (synthesised).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThemePreset {
    #[default]
    Hybrid,
    Dark,
    Light,
}

impl ThemePreset {
    /// Cycle to the next preset: Hybrid → Dark → Light → Hybrid.
    pub fn next(self) -> Self {
        match self {
            Self::Hybrid => Self::Dark,
            Self::Dark => Self::Light,
            Self::Light => Self::Hybrid,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Density {
    Compact,
    #[default]
    Comfortable,
}

impl Density {
    /// Cycle to the next density: Compact → Comfortable → Compact.
    pub fn next(self) -> Self {
        match self {
            Self::Compact => Self::Comfortable,
            Self::Comfortable => Self::Compact,
        }
    }

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
    /// Theme name (matches `ThemeDef::name`).
    pub name: String,
    /// Original definition — gives the markdown renderer access to `SyntaxColors`
    /// and `is_dark()` without re-deriving them from ratatui `Color` values.
    pub def: ThemeDef,

    // -- ratatui Color fields (converted from `def` at construction time) --
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
    /// Background for the cursor line in selection mode (phase 1).
    pub select_cursor: Color,
    /// Background for selected lines in visual selection (phase 2).
    pub select: Color,

    // -- Legacy field kept for backward compatibility --
    pub preset: ThemePreset,
}

impl Default for UcodeTheme {
    fn default() -> Self {
        let def = builtin_theme("ucode").expect("ucode built-in theme must exist");
        Self::from_def(def)
    }
}

impl UcodeTheme {
    /// Build a `UcodeTheme` from a `ThemeDef`, converting `Rgb` → `Color::Rgb`.
    pub fn from_def(def: ThemeDef) -> Self {
        let preset = preset_for_name(&def.name);
        Self {
            name: def.name.clone(),
            background: to_color(def.background),
            surface: to_color(def.surface),
            border: to_color(def.border),
            border_focus: to_color(def.border_focus),
            accent: to_color(def.accent),
            safe: to_color(def.safe),
            warning: to_color(def.warning),
            danger: to_color(def.danger),
            muted: to_color(def.muted),
            text: to_color(def.text),
            text_dim: to_color(def.text_dim),
            select_cursor: to_color(def.select_cursor),
            select: to_color(def.select),
            preset,
            def,
        }
    }

    /// Cycle to the next built-in theme, wrapping around.
    pub fn next_theme(&self) -> Self {
        let names = theme_names();
        let current = names.iter().position(|&n| n == self.name).unwrap_or(0);
        let next_idx = (current + 1) % names.len();
        let next_name = names[next_idx];
        let def = builtin_theme(next_name)
            .unwrap_or_else(|| builtin_theme("ucode").expect("ucode built-in theme must exist"));
        Self::from_def(def)
    }

    /// Create a theme from a legacy preset (backward compat).
    pub fn from_preset(preset: ThemePreset) -> Self {
        let name = match preset {
            ThemePreset::Hybrid => "ucode",
            ThemePreset::Dark => "tokyonight",
            ThemePreset::Light => "nord",
        };
        let def = builtin_theme(name)
            .unwrap_or_else(|| builtin_theme("ucode").expect("ucode built-in theme must exist"));
        let mut theme = Self::from_def(def);
        // Preserve the caller's preset value so legacy code that reads
        // `theme.preset` still sees what it set.
        theme.preset = preset;
        theme
    }

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
// Helpers
// ---------------------------------------------------------------------------

/// Map a theme name to the closest legacy `ThemePreset` for backward compat.
fn preset_for_name(name: &str) -> ThemePreset {
    match name {
        "ucode" => ThemePreset::Hybrid,
        "tokyonight" => ThemePreset::Dark,
        _ => ThemePreset::Dark,
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

    #[test]
    fn theme_from_preset_hybrid() {
        let theme = UcodeTheme::from_preset(ThemePreset::Hybrid);
        assert_eq!(theme.preset, ThemePreset::Hybrid);
        assert_eq!(theme.accent, Color::Rgb(0x00, 0xd4, 0xaa));
    }

    #[test]
    fn theme_from_preset_dark() {
        let theme = UcodeTheme::from_preset(ThemePreset::Dark);
        assert_eq!(theme.preset, ThemePreset::Dark);
        // tokyonight background
        assert_eq!(theme.background, Color::Rgb(0x1a, 0x1b, 0x26));
    }

    #[test]
    fn theme_from_preset_light() {
        let theme = UcodeTheme::from_preset(ThemePreset::Light);
        assert_eq!(theme.preset, ThemePreset::Light);
        // nord background — dark but distinct from ucode
        assert_eq!(theme.background, Color::Rgb(0x2e, 0x34, 0x40));
    }

    #[test]
    fn theme_preset_next_cycles() {
        assert_eq!(ThemePreset::Hybrid.next(), ThemePreset::Dark);
        assert_eq!(ThemePreset::Dark.next(), ThemePreset::Light);
        assert_eq!(ThemePreset::Light.next(), ThemePreset::Hybrid);
    }

    #[test]
    fn theme_with_accent_preserves_preset() {
        let theme = UcodeTheme::with_accent(Color::Rgb(0xff, 0x00, 0x80));
        assert_eq!(theme.preset, ThemePreset::Hybrid);
    }

    #[test]
    fn density_next_cycles() {
        assert_eq!(Density::Compact.next(), Density::Comfortable);
        assert_eq!(Density::Comfortable.next(), Density::Compact);
    }

    #[test]
    fn from_def_populates_all_fields() {
        let def = builtin_theme("ucode").unwrap();
        let theme = UcodeTheme::from_def(def.clone());
        assert_eq!(theme.name, "ucode");
        assert_eq!(theme.def, def);
        assert_eq!(
            theme.accent,
            Color::Rgb(def.accent.r, def.accent.g, def.accent.b)
        );
    }

    #[test]
    fn next_theme_cycles_through_builtins() {
        let theme = UcodeTheme::default();
        assert_eq!(theme.name, "ucode");
        let t2 = theme.next_theme();
        assert_eq!(t2.name, "tokyonight");
        // Cycle all the way back.
        let names = theme_names();
        let mut t = UcodeTheme::default();
        for _ in 0..names.len() {
            t = t.next_theme();
        }
        assert_eq!(t.name, "ucode");
    }

    #[test]
    fn def_is_dark_matches_theme() {
        let theme = UcodeTheme::default();
        assert!(theme.def.is_dark());
    }
}
