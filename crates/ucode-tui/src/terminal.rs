use std::io::Write;

/// Set the terminal title via OSC 0 escape sequence.
///
/// Format: `ESC ] 0 ; <title> BEL`
///
/// Works in most terminals including xterm, iTerm2, and through tmux
/// (tmux forwards the title to the outer terminal).
pub fn set_terminal_title(title: &str, writer: &mut impl Write) -> std::io::Result<()> {
    write!(writer, "\x1b]0;{title}\x07")?;
    writer.flush()
}

/// Restore the terminal title to the default (empty).
pub fn restore_terminal_title(writer: &mut impl Write) -> std::io::Result<()> {
    set_terminal_title("", writer)
}

// ---------------------------------------------------------------------------
// Color support detection
// ---------------------------------------------------------------------------

/// Terminal color capability level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorSupport {
    /// 24-bit true color (16 million colors).
    TrueColor,
    /// 256-color palette.
    Color256,
    /// Basic 16 colors.
    Basic,
}

/// Detect the terminal's color support level from environment variables.
///
/// Detection order:
/// 1. `$COLORTERM` = "truecolor" or "24bit" → TrueColor
/// 2. `$TERM` contains "256color" → Color256
/// 3. Inside tmux: check `$TERM` for "256color" or assume Color256
///    (tmux typically supports at least 256 colors)
/// 4. Otherwise → Basic
pub fn detect_color_support() -> ColorSupport {
    // COLORTERM is the most reliable indicator of true color support.
    if let Ok(ct) = std::env::var("COLORTERM") {
        let ct_lower = ct.to_lowercase();
        if ct_lower == "truecolor" || ct_lower == "24bit" {
            return ColorSupport::TrueColor;
        }
    }

    // Check TERM for 256color suffix.
    if let Ok(term) = std::env::var("TERM")
        && term.contains("256color")
    {
        return ColorSupport::Color256;
    }

    // Inside tmux, assume at least 256 colors.
    if std::env::var("TMUX").is_ok() {
        return ColorSupport::Color256;
    }

    ColorSupport::Basic
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_title_writes_osc_escape() {
        let mut buf: Vec<u8> = Vec::new();
        set_terminal_title("ucode - main", &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert_eq!(s, "\x1b]0;ucode - main\x07");
    }

    #[test]
    fn restore_title_writes_empty() {
        let mut buf: Vec<u8> = Vec::new();
        restore_terminal_title(&mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert_eq!(s, "\x1b]0;\x07");
    }

    #[test]
    fn detect_color_support_does_not_panic() {
        // Should work regardless of environment.
        let _ = detect_color_support();
    }

    #[test]
    fn color_support_variants_are_distinct() {
        assert_ne!(ColorSupport::TrueColor, ColorSupport::Color256);
        assert_ne!(ColorSupport::Color256, ColorSupport::Basic);
        assert_ne!(ColorSupport::TrueColor, ColorSupport::Basic);
    }
}
