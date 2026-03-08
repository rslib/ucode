use crate::{SyntaxColors, ThemeDef, rgb};

// ---------------------------------------------------------------------------
// Theme catalog
// ---------------------------------------------------------------------------

/// All built-in theme names, in display order.
const THEME_NAMES: &[&str] = &[
    "ucode",
    "tokyonight",
    "catppuccin-mocha",
    "gruvbox-dark",
    "nord",
    "dracula",
];

/// Returns the list of built-in theme names.
pub fn theme_names() -> &'static [&'static str] {
    THEME_NAMES
}

/// Look up a built-in theme by name (case-sensitive).
pub fn builtin_theme(name: &str) -> Option<ThemeDef> {
    match name {
        "ucode" => Some(ucode()),
        "tokyonight" => Some(tokyonight()),
        "catppuccin-mocha" => Some(catppuccin_mocha()),
        "gruvbox-dark" => Some(gruvbox_dark()),
        "nord" => Some(nord()),
        "dracula" => Some(dracula()),
        _ => None,
    }
}

/// Returns all built-in themes.
pub fn builtin_themes() -> Vec<ThemeDef> {
    THEME_NAMES
        .iter()
        .filter_map(|name| builtin_theme(name))
        .collect()
}

// ---------------------------------------------------------------------------
// ucode — our default teal-on-dark theme
// ---------------------------------------------------------------------------

fn ucode() -> ThemeDef {
    ThemeDef {
        name: "ucode".into(),
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
            keyword: rgb(0x00, 0xd4, 0xaa),     // teal accent
            string: rgb(0xa8, 0xdb, 0x8a),      // soft green
            comment: rgb(0x6b, 0x72, 0x80),     // muted gray
            type_name: rgb(0x7a, 0xc8, 0xf0),   // light blue
            function: rgb(0xe5, 0xc0, 0x7b),    // warm yellow
            number: rgb(0xf5, 0x9e, 0x0b),      // orange
            operator: rgb(0x9c, 0xa3, 0xaf),    // dim text
            variable: rgb(0xe5, 0xe7, 0xeb),    // normal text
            constant: rgb(0xf0, 0x80, 0x80),    // soft red
            attribute: rgb(0x00, 0xd4, 0xaa),   // teal accent
            tag: rgb(0x7a, 0xc8, 0xf0),         // light blue
            punctuation: rgb(0x9c, 0xa3, 0xaf), // dim text
        },
    }
}

// ---------------------------------------------------------------------------
// tokyonight
// ---------------------------------------------------------------------------

fn tokyonight() -> ThemeDef {
    ThemeDef {
        name: "tokyonight".into(),
        background: rgb(0x1a, 0x1b, 0x26),
        surface: rgb(0x24, 0x28, 0x3b),
        border: rgb(0x3b, 0x40, 0x61),
        border_focus: rgb(0x54, 0x5c, 0x7e),
        accent: rgb(0x7a, 0xa2, 0xf7),
        safe: rgb(0x9e, 0xce, 0x6a),
        warning: rgb(0xe0, 0xaf, 0x68),
        danger: rgb(0xf7, 0x76, 0x8e),
        muted: rgb(0x56, 0x5f, 0x89),
        text: rgb(0xc0, 0xca, 0xf5),
        text_dim: rgb(0xa9, 0xb1, 0xd6),
        select_cursor: rgb(0x28, 0x2e, 0x44),
        select: rgb(0x33, 0x3a, 0x55),
        syntax: SyntaxColors {
            keyword: rgb(0xbb, 0x9a, 0xf7),     // purple
            string: rgb(0x9e, 0xce, 0x6a),      // green
            comment: rgb(0x56, 0x5f, 0x89),     // slate gray
            type_name: rgb(0x2a, 0xc3, 0xde),   // cyan
            function: rgb(0x7a, 0xa2, 0xf7),    // blue
            number: rgb(0xff, 0x9e, 0x64),      // orange
            operator: rgb(0x89, 0xdd, 0xff),    // light cyan
            variable: rgb(0xc0, 0xca, 0xf5),    // text
            constant: rgb(0xff, 0x9e, 0x64),    // orange
            attribute: rgb(0xbb, 0x9a, 0xf7),   // purple
            tag: rgb(0xf7, 0x76, 0x8e),         // red/pink
            punctuation: rgb(0xa9, 0xb1, 0xd6), // dim text
        },
    }
}

// ---------------------------------------------------------------------------
// catppuccin-mocha
// ---------------------------------------------------------------------------

fn catppuccin_mocha() -> ThemeDef {
    ThemeDef {
        name: "catppuccin-mocha".into(),
        background: rgb(0x1e, 0x1e, 0x2e),
        surface: rgb(0x31, 0x32, 0x44),
        border: rgb(0x45, 0x47, 0x5a),
        border_focus: rgb(0x58, 0x5b, 0x70),
        accent: rgb(0xcb, 0xa6, 0xf7),
        safe: rgb(0xa6, 0xe3, 0xa1),
        warning: rgb(0xf9, 0xe2, 0xaf),
        danger: rgb(0xf3, 0x8b, 0xa8),
        muted: rgb(0x6c, 0x70, 0x86),
        text: rgb(0xcd, 0xd6, 0xf4),
        text_dim: rgb(0xba, 0xc2, 0xde),
        select_cursor: rgb(0x2a, 0x2b, 0x3d),
        select: rgb(0x36, 0x37, 0x4a),
        syntax: SyntaxColors {
            keyword: rgb(0xcb, 0xa6, 0xf7),     // mauve
            string: rgb(0xa6, 0xe3, 0xa1),      // green
            comment: rgb(0x6c, 0x70, 0x86),     // overlay0
            type_name: rgb(0x89, 0xb4, 0xfa),   // blue
            function: rgb(0x89, 0xdc, 0xeb),    // sky
            number: rgb(0xfa, 0xb3, 0x87),      // peach
            operator: rgb(0x94, 0xe2, 0xd5),    // teal
            variable: rgb(0xcd, 0xd6, 0xf4),    // text
            constant: rgb(0xfa, 0xb3, 0x87),    // peach
            attribute: rgb(0xf9, 0xe2, 0xaf),   // yellow
            tag: rgb(0xf3, 0x8b, 0xa8),         // maroon/pink
            punctuation: rgb(0xba, 0xc2, 0xde), // subtext1
        },
    }
}

// ---------------------------------------------------------------------------
// gruvbox-dark
// ---------------------------------------------------------------------------

fn gruvbox_dark() -> ThemeDef {
    ThemeDef {
        name: "gruvbox-dark".into(),
        background: rgb(0x28, 0x28, 0x28),
        surface: rgb(0x3c, 0x38, 0x36),
        border: rgb(0x50, 0x49, 0x45),
        border_focus: rgb(0x66, 0x5c, 0x54),
        accent: rgb(0xd7, 0x99, 0x21),
        safe: rgb(0xb8, 0xbb, 0x26),
        warning: rgb(0xfa, 0xbd, 0x2f),
        danger: rgb(0xfb, 0x49, 0x34),
        muted: rgb(0x92, 0x83, 0x74),
        text: rgb(0xeb, 0xdb, 0xb2),
        text_dim: rgb(0xbd, 0xae, 0x93),
        select_cursor: rgb(0x3c, 0x38, 0x36),
        select: rgb(0x50, 0x49, 0x45),
        syntax: SyntaxColors {
            keyword: rgb(0xfb, 0x49, 0x34),     // red
            string: rgb(0xb8, 0xbb, 0x26),      // green
            comment: rgb(0x92, 0x83, 0x74),     // gray
            type_name: rgb(0xd7, 0x99, 0x21),   // yellow
            function: rgb(0x83, 0xa5, 0x98),    // aqua
            number: rgb(0xd3, 0x86, 0x9b),      // purple
            operator: rgb(0xfe, 0x80, 0x19),    // orange
            variable: rgb(0xeb, 0xdb, 0xb2),    // fg
            constant: rgb(0xd3, 0x86, 0x9b),    // purple
            attribute: rgb(0xfa, 0xbd, 0x2f),   // bright yellow
            tag: rgb(0x83, 0xa5, 0x98),         // aqua
            punctuation: rgb(0xbd, 0xae, 0x93), // fg dim
        },
    }
}

// ---------------------------------------------------------------------------
// nord
// ---------------------------------------------------------------------------

fn nord() -> ThemeDef {
    ThemeDef {
        name: "nord".into(),
        background: rgb(0x2e, 0x34, 0x40),
        surface: rgb(0x3b, 0x42, 0x52),
        border: rgb(0x43, 0x4c, 0x5e),
        border_focus: rgb(0x4c, 0x56, 0x6a),
        accent: rgb(0x88, 0xc0, 0xd0),
        safe: rgb(0xa3, 0xbe, 0x8c),
        warning: rgb(0xeb, 0xcb, 0x8b),
        danger: rgb(0xbf, 0x61, 0x6a),
        muted: rgb(0x61, 0x6e, 0x88),
        text: rgb(0xec, 0xef, 0xf4),
        text_dim: rgb(0xd8, 0xde, 0xe9),
        select_cursor: rgb(0x3b, 0x42, 0x52),
        select: rgb(0x43, 0x4c, 0x5e),
        syntax: SyntaxColors {
            keyword: rgb(0x81, 0xa1, 0xc1),     // nord9 blue
            string: rgb(0xa3, 0xbe, 0x8c),      // nord14 green
            comment: rgb(0x61, 0x6e, 0x88),     // nord3 muted
            type_name: rgb(0x8f, 0xbc, 0xbb),   // nord7 teal
            function: rgb(0x88, 0xc0, 0xd0),    // nord8 cyan
            number: rgb(0xb4, 0x8e, 0xad),      // nord15 purple
            operator: rgb(0x81, 0xa1, 0xc1),    // nord9 blue
            variable: rgb(0xec, 0xef, 0xf4),    // nord6 text
            constant: rgb(0xb4, 0x8e, 0xad),    // nord15 purple
            attribute: rgb(0xeb, 0xcb, 0x8b),   // nord13 yellow
            tag: rgb(0x81, 0xa1, 0xc1),         // nord9 blue
            punctuation: rgb(0xd8, 0xde, 0xe9), // nord4 dim
        },
    }
}

// ---------------------------------------------------------------------------
// dracula
// ---------------------------------------------------------------------------

fn dracula() -> ThemeDef {
    ThemeDef {
        name: "dracula".into(),
        background: rgb(0x28, 0x2a, 0x36),
        surface: rgb(0x44, 0x47, 0x5a),
        border: rgb(0x62, 0x72, 0xa4),
        border_focus: rgb(0x6c, 0x71, 0x86),
        accent: rgb(0xbd, 0x93, 0xf9),
        safe: rgb(0x50, 0xfa, 0x7b),
        warning: rgb(0xf1, 0xfa, 0x8c),
        danger: rgb(0xff, 0x55, 0x55),
        muted: rgb(0x62, 0x72, 0xa4),
        text: rgb(0xf8, 0xf8, 0xf2),
        text_dim: rgb(0xbd, 0xbf, 0xc2),
        select_cursor: rgb(0x36, 0x38, 0x48),
        select: rgb(0x44, 0x47, 0x5a),
        syntax: SyntaxColors {
            keyword: rgb(0xff, 0x79, 0xc6),     // pink
            string: rgb(0xf1, 0xfa, 0x8c),      // yellow
            comment: rgb(0x62, 0x72, 0xa4),     // muted blue
            type_name: rgb(0x8b, 0xe9, 0xfd),   // cyan
            function: rgb(0x50, 0xfa, 0x7b),    // green
            number: rgb(0xbd, 0x93, 0xf9),      // purple
            operator: rgb(0xff, 0x79, 0xc6),    // pink
            variable: rgb(0xf8, 0xf8, 0xf2),    // fg
            constant: rgb(0xbd, 0x93, 0xf9),    // purple
            attribute: rgb(0x50, 0xfa, 0x7b),   // green
            tag: rgb(0xff, 0x79, 0xc6),         // pink
            punctuation: rgb(0xf8, 0xf8, 0xf2), // fg
        },
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_themes_exist() {
        for name in theme_names() {
            assert!(
                builtin_theme(name).is_some(),
                "builtin_theme({name:?}) returned None"
            );
        }
    }

    #[test]
    fn names_are_unique() {
        let names = theme_names();
        let mut seen = std::collections::HashSet::new();
        for name in names {
            assert!(seen.insert(name), "duplicate theme name: {name}");
        }
    }

    #[test]
    fn builtin_themes_count() {
        assert_eq!(builtin_themes().len(), theme_names().len());
    }

    #[test]
    fn ucode_is_default() {
        let t = builtin_theme("ucode").unwrap();
        assert_eq!(t.name, "ucode");
        assert!(t.is_dark());
    }

    #[test]
    fn unknown_returns_none() {
        assert!(builtin_theme("nonexistent").is_none());
    }

    #[test]
    fn all_dark_themes_report_dark() {
        for t in builtin_themes() {
            assert!(t.is_dark(), "theme {:?} should be dark", t.name);
        }
    }
}
