mod builtin;
mod loader;

pub use builtin::{builtin_theme, builtin_themes, theme_names};
pub use loader::{UserThemeOverride, load_user_themes, resolve_theme};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Rgb
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub fn to_hex(&self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

/// Shorthand for `Rgb::new(r, g, b)`.
pub const fn rgb(r: u8, g: u8, b: u8) -> Rgb {
    Rgb::new(r, g, b)
}

// ---------------------------------------------------------------------------
// SyntaxColors
// ---------------------------------------------------------------------------

/// Colors for syntax highlighting in fenced code blocks.
///
/// Each field maps to a class of syntax tokens. The TUI layer converts
/// these to `syntect::highlighting::Theme` scope selectors at runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyntaxColors {
    /// `if`, `else`, `fn`, `pub`, `let`, `return`, `use`, etc.
    pub keyword: Rgb,
    /// String literals: `"hello"`, `'c'`
    pub string: Rgb,
    /// Comments: `// ...`, `/* ... */`, `# ...`
    pub comment: Rgb,
    /// Type names: `String`, `Vec`, `Result`, `i32`
    pub type_name: Rgb,
    /// Function/method names at call or definition site
    pub function: Rgb,
    /// Numeric literals: `42`, `3.14`, `0xff`
    pub number: Rgb,
    /// Operators: `+`, `-`, `=`, `->`, `=>`, `::`, `&&`
    pub operator: Rgb,
    /// Variable and parameter names
    pub variable: Rgb,
    /// Language constants: `true`, `false`, `None`, `nil`, `null`
    pub constant: Rgb,
    /// Attributes/decorators: `#[derive]`, `@decorator`
    pub attribute: Rgb,
    /// HTML/XML/JSX tags
    pub tag: Rgb,
    /// Brackets, braces, parens, semicolons
    pub punctuation: Rgb,
}

// ---------------------------------------------------------------------------
// SyntaxColorsOverride (for partial user overrides)
// ---------------------------------------------------------------------------

/// Partial override — every field is optional. Used when loading user TOML.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SyntaxColorsOverride {
    pub keyword: Option<Rgb>,
    pub string: Option<Rgb>,
    pub comment: Option<Rgb>,
    pub type_name: Option<Rgb>,
    pub function: Option<Rgb>,
    pub number: Option<Rgb>,
    pub operator: Option<Rgb>,
    pub variable: Option<Rgb>,
    pub constant: Option<Rgb>,
    pub attribute: Option<Rgb>,
    pub tag: Option<Rgb>,
    pub punctuation: Option<Rgb>,
}

impl SyntaxColorsOverride {
    /// Merge non-None fields onto `base`, returning a new `SyntaxColors`.
    pub fn apply_to(&self, base: &SyntaxColors) -> SyntaxColors {
        SyntaxColors {
            keyword: self.keyword.unwrap_or(base.keyword),
            string: self.string.unwrap_or(base.string),
            comment: self.comment.unwrap_or(base.comment),
            type_name: self.type_name.unwrap_or(base.type_name),
            function: self.function.unwrap_or(base.function),
            number: self.number.unwrap_or(base.number),
            operator: self.operator.unwrap_or(base.operator),
            variable: self.variable.unwrap_or(base.variable),
            constant: self.constant.unwrap_or(base.constant),
            attribute: self.attribute.unwrap_or(base.attribute),
            tag: self.tag.unwrap_or(base.tag),
            punctuation: self.punctuation.unwrap_or(base.punctuation),
        }
    }
}

// ---------------------------------------------------------------------------
// ThemeDef
// ---------------------------------------------------------------------------

/// Complete theme definition — UI colors + syntax highlighting colors.
///
/// This is the canonical theme representation. The TUI layer converts it
/// to ratatui styles; the markdown renderer converts `syntax` to a
/// `syntect::highlighting::Theme`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeDef {
    pub name: String,

    // -- UI colors --
    pub background: Rgb,
    pub surface: Rgb,
    pub border: Rgb,
    pub border_focus: Rgb,
    pub accent: Rgb,
    pub safe: Rgb,
    pub warning: Rgb,
    pub danger: Rgb,
    pub muted: Rgb,
    pub text: Rgb,
    pub text_dim: Rgb,
    pub select_cursor: Rgb,
    pub select: Rgb,

    // -- Syntax highlighting --
    pub syntax: SyntaxColors,
}

impl ThemeDef {
    /// Returns `true` if the background is dark (luminance < 0.5).
    pub fn is_dark(&self) -> bool {
        let lum = 0.299 * self.background.r as f64
            + 0.587 * self.background.g as f64
            + 0.114 * self.background.b as f64;
        lum < 128.0
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_to_hex() {
        assert_eq!(Rgb::new(0x4f, 0xc1, 0xff).to_hex(), "#4fc1ff");
        assert_eq!(Rgb::new(0, 0, 0).to_hex(), "#000000");
        assert_eq!(Rgb::new(255, 255, 255).to_hex(), "#ffffff");
    }

    #[test]
    fn rgb_new() {
        let c = Rgb::new(0x0d, 0x0d, 0x0d);
        assert_eq!(c.r, 0x0d);
        assert_eq!(c.g, 0x0d);
        assert_eq!(c.b, 0x0d);
    }

    #[test]
    fn rgb_shorthand() {
        assert_eq!(rgb(1, 2, 3), Rgb::new(1, 2, 3));
    }

    #[test]
    fn is_dark_detects_dark_bg() {
        let theme = ThemeDef {
            name: "test".into(),
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
            select_cursor: rgb(0x1a, 0x2a, 0x25),
            select: rgb(0x1a, 0x3a, 0x30),
            syntax: SyntaxColors {
                keyword: rgb(0xff, 0x00, 0x00),
                string: rgb(0x00, 0xff, 0x00),
                comment: rgb(0x80, 0x80, 0x80),
                type_name: rgb(0x00, 0x00, 0xff),
                function: rgb(0xff, 0xff, 0x00),
                number: rgb(0xff, 0x80, 0x00),
                operator: rgb(0xff, 0xff, 0xff),
                variable: rgb(0xe5, 0xe7, 0xeb),
                constant: rgb(0xff, 0x80, 0x80),
                attribute: rgb(0x80, 0xff, 0x80),
                tag: rgb(0x80, 0x80, 0xff),
                punctuation: rgb(0xc0, 0xc0, 0xc0),
            },
        };
        assert!(theme.is_dark());
    }

    #[test]
    fn is_dark_detects_light_bg() {
        let theme = ThemeDef {
            name: "light".into(),
            background: rgb(0xfa, 0xfa, 0xfa),
            surface: rgb(0xf0, 0xf0, 0xf0),
            border: rgb(0xd0, 0xd0, 0xd0),
            border_focus: rgb(0xa0, 0xa0, 0xa0),
            accent: rgb(0x00, 0x96, 0x7a),
            safe: rgb(0x16, 0xa3, 0x4a),
            warning: rgb(0xd9, 0x7a, 0x06),
            danger: rgb(0xdc, 0x26, 0x26),
            muted: rgb(0x9c, 0xa3, 0xaf),
            text: rgb(0x1f, 0x1f, 0x1f),
            text_dim: rgb(0x6b, 0x72, 0x80),
            select_cursor: rgb(0xd0, 0xee, 0xe4),
            select: rgb(0xb0, 0xe0, 0xd0),
            syntax: SyntaxColors {
                keyword: rgb(0xff, 0x00, 0x00),
                string: rgb(0x00, 0xff, 0x00),
                comment: rgb(0x80, 0x80, 0x80),
                type_name: rgb(0x00, 0x00, 0xff),
                function: rgb(0xff, 0xff, 0x00),
                number: rgb(0xff, 0x80, 0x00),
                operator: rgb(0xff, 0xff, 0xff),
                variable: rgb(0xe5, 0xe7, 0xeb),
                constant: rgb(0xff, 0x80, 0x80),
                attribute: rgb(0x80, 0xff, 0x80),
                tag: rgb(0x80, 0x80, 0xff),
                punctuation: rgb(0xc0, 0xc0, 0xc0),
            },
        };
        assert!(!theme.is_dark());
    }

    #[test]
    fn syntax_override_partial_merge() {
        let base = SyntaxColors {
            keyword: rgb(0xff, 0x00, 0x00),
            string: rgb(0x00, 0xff, 0x00),
            comment: rgb(0x80, 0x80, 0x80),
            type_name: rgb(0x00, 0x00, 0xff),
            function: rgb(0xff, 0xff, 0x00),
            number: rgb(0xff, 0x80, 0x00),
            operator: rgb(0xff, 0xff, 0xff),
            variable: rgb(0xe5, 0xe7, 0xeb),
            constant: rgb(0xff, 0x80, 0x80),
            attribute: rgb(0x80, 0xff, 0x80),
            tag: rgb(0x80, 0x80, 0xff),
            punctuation: rgb(0xc0, 0xc0, 0xc0),
        };

        let over = SyntaxColorsOverride {
            keyword: Some(rgb(0xaa, 0xbb, 0xcc)),
            ..Default::default()
        };

        let merged = over.apply_to(&base);
        assert_eq!(merged.keyword, rgb(0xaa, 0xbb, 0xcc));
        assert_eq!(merged.string, base.string); // unchanged
    }
}
