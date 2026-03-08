use std::path::Path;

use serde::Deserialize;

use crate::{Rgb, SyntaxColorsOverride, ThemeDef, builtin_theme};

// ---------------------------------------------------------------------------
// UserThemeOverride
// ---------------------------------------------------------------------------

/// A user-authored theme file. All color fields are optional.
/// If `base` is set, unspecified fields inherit from that built-in theme.
/// If `base` is not set, all fields must be provided.
#[derive(Debug, Clone, Deserialize)]
pub struct UserThemeOverride {
    pub name: String,
    /// Name of a built-in theme to inherit from.
    pub base: Option<String>,

    // -- UI colors (all optional) --
    pub background: Option<Rgb>,
    pub surface: Option<Rgb>,
    pub border: Option<Rgb>,
    pub border_focus: Option<Rgb>,
    pub accent: Option<Rgb>,
    pub safe: Option<Rgb>,
    pub warning: Option<Rgb>,
    pub danger: Option<Rgb>,
    pub muted: Option<Rgb>,
    pub text: Option<Rgb>,
    pub text_dim: Option<Rgb>,
    pub select_cursor: Option<Rgb>,
    pub select: Option<Rgb>,

    // -- Syntax colors (optional block) --
    pub syntax: Option<SyntaxColorsOverride>,
}

impl UserThemeOverride {
    /// Merge this override onto a base `ThemeDef`, returning a new theme.
    pub fn apply_to(&self, base: &ThemeDef) -> ThemeDef {
        ThemeDef {
            name: self.name.clone(),
            background: self.background.unwrap_or(base.background),
            surface: self.surface.unwrap_or(base.surface),
            border: self.border.unwrap_or(base.border),
            border_focus: self.border_focus.unwrap_or(base.border_focus),
            accent: self.accent.unwrap_or(base.accent),
            safe: self.safe.unwrap_or(base.safe),
            warning: self.warning.unwrap_or(base.warning),
            danger: self.danger.unwrap_or(base.danger),
            muted: self.muted.unwrap_or(base.muted),
            text: self.text.unwrap_or(base.text),
            text_dim: self.text_dim.unwrap_or(base.text_dim),
            select_cursor: self.select_cursor.unwrap_or(base.select_cursor),
            select: self.select.unwrap_or(base.select),
            syntax: match &self.syntax {
                Some(over) => over.apply_to(&base.syntax),
                None => base.syntax.clone(),
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

/// Load all user theme files from `config_dir/themes/*.toml`.
///
/// Each file is parsed as a `UserThemeOverride`. If `base` is specified,
/// the override is merged onto that built-in theme. Files that fail to
/// parse are silently skipped (logged at debug level in the future).
pub fn load_user_themes(config_dir: &Path) -> Vec<ThemeDef> {
    let themes_dir = config_dir.join("themes");
    let Ok(entries) = std::fs::read_dir(&themes_dir) else {
        return Vec::new();
    };

    let mut themes = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "toml")
            && let Ok(contents) = std::fs::read_to_string(&path)
            && let Ok(over) = toml::from_str::<UserThemeOverride>(&contents)
        {
            let base_name = over.base.as_deref().unwrap_or("ucode");
            let base = builtin_theme(base_name)
                .unwrap_or_else(|| builtin_theme("ucode").expect("ucode theme must exist"));
            themes.push(over.apply_to(&base));
        }
    }
    themes
}

/// Resolve a theme by name: check user themes first, then built-ins.
/// Falls back to the `ucode` built-in if not found.
pub fn resolve_theme(name: &str, config_dir: &Path) -> ThemeDef {
    // Check user themes.
    let user_themes = load_user_themes(config_dir);
    if let Some(t) = user_themes.into_iter().find(|t| t.name == name) {
        return t;
    }

    // Check built-ins.
    if let Some(t) = builtin_theme(name) {
        return t;
    }

    // Fallback.
    builtin_theme("ucode").expect("ucode theme must exist")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rgb;

    #[test]
    fn resolve_builtin() {
        let dir = std::path::PathBuf::from("/tmp/ucode-test-nonexistent");
        let t = resolve_theme("tokyonight", &dir);
        assert_eq!(t.name, "tokyonight");
    }

    #[test]
    fn resolve_unknown_falls_back() {
        let dir = std::path::PathBuf::from("/tmp/ucode-test-nonexistent");
        let t = resolve_theme("nonexistent-theme", &dir);
        assert_eq!(t.name, "ucode");
    }

    #[test]
    fn user_override_partial_merge() {
        let over = UserThemeOverride {
            name: "my-theme".into(),
            base: Some("ucode".into()),
            accent: Some(rgb(0xff, 0x00, 0x80)),
            background: None,
            surface: None,
            border: None,
            border_focus: None,
            safe: None,
            warning: None,
            danger: None,
            muted: None,
            text: None,
            text_dim: None,
            select_cursor: None,
            select: None,
            syntax: None,
        };

        let base = builtin_theme("ucode").unwrap();
        let merged = over.apply_to(&base);

        assert_eq!(merged.name, "my-theme");
        assert_eq!(merged.accent, rgb(0xff, 0x00, 0x80));
        assert_eq!(merged.background, base.background); // inherited
        assert_eq!(merged.syntax, base.syntax); // inherited
    }

    #[test]
    fn load_user_themes_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let themes = load_user_themes(dir.path());
        assert!(themes.is_empty());
    }

    #[test]
    fn load_user_themes_with_file() {
        let dir = tempfile::tempdir().unwrap();
        let themes_dir = dir.path().join("themes");
        std::fs::create_dir_all(&themes_dir).unwrap();

        let toml_content = r#"
name = "custom"
base = "ucode"

[syntax]
keyword = { r = 255, g = 0, b = 128 }
"#;
        std::fs::write(themes_dir.join("custom.toml"), toml_content).unwrap();

        let themes = load_user_themes(dir.path());
        assert_eq!(themes.len(), 1);
        assert_eq!(themes[0].name, "custom");
        assert_eq!(themes[0].syntax.keyword, rgb(0xff, 0x00, 0x80));
        // Other syntax colors inherited from ucode
        let base = builtin_theme("ucode").unwrap();
        assert_eq!(themes[0].syntax.string, base.syntax.string);
    }
}
