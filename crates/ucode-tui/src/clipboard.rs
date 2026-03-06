//! Clipboard integration via OSC 52, external tools, or file fallback.
//!
//! Priority order for `ClipboardMethod::Osc52`:
//!   1. OSC 52 escape sequence (works through tmux ≥ 3.3 with `set-clipboard on`)
//!   2. External tool (`wl-copy`, `xclip`, `xsel`, `pbcopy`, `clip.exe`)
//!   3. File at `$XDG_DATA_HOME/ucode/clipboard`

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

// ── Base64 ────────────────────────────────────────────────────────────────────

const B64_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(input: &[u8]) -> String {
    let mut out = Vec::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64_ALPHABET[((n >> 18) & 0x3f) as usize]);
        out.push(B64_ALPHABET[((n >> 12) & 0x3f) as usize]);
        out.push(if chunk.len() > 1 {
            B64_ALPHABET[((n >> 6) & 0x3f) as usize]
        } else {
            b'='
        });
        out.push(if chunk.len() > 2 {
            B64_ALPHABET[(n & 0x3f) as usize]
        } else {
            b'='
        });
    }
    // SAFETY: B64_ALPHABET and '=' are all ASCII.
    unsafe { String::from_utf8_unchecked(out) }
}

// ── Types ─────────────────────────────────────────────────────────────────────

/// How clipboard writes are performed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ClipboardMethod {
    /// OSC 52 escape sequence (default). Falls back to external, then file.
    #[default]
    Osc52,
    /// External tool (`wl-copy`, `xclip`, `xsel`, `pbcopy`, `clip.exe`). Falls back to file.
    External,
    /// Write to `$XDG_DATA_HOME/ucode/clipboard`.
    File,
}

#[derive(Debug, thiserror::Error)]
pub enum ClipboardError {
    #[error("OSC 52 write failed: {0}")]
    Osc52WriteFailed(String),

    #[error("no clipboard tool found (wl-copy, xclip, xsel, pbcopy, clip.exe)")]
    ExternalToolNotFound,

    #[error("clipboard tool failed: {0}")]
    ExternalToolFailed(String),

    #[error("clipboard file write failed: {0}")]
    FileWriteFailed(#[from] std::io::Error),
}

// ── Multiplexer detection ─────────────────────────────────────────────────────

pub fn is_tmux() -> bool {
    std::env::var("TMUX").is_ok()
}

pub fn is_zellij() -> bool {
    std::env::var("ZELLIJ").is_ok()
}

pub fn is_screen() -> bool {
    std::env::var("STY").is_ok()
}

/// Returns the name of the active terminal multiplexer, if any.
pub fn multiplexer_name() -> Option<&'static str> {
    if is_tmux() {
        Some("tmux")
    } else if is_zellij() {
        Some("zellij")
    } else if is_screen() {
        Some("screen")
    } else {
        None
    }
}

// ── OSC 52 ────────────────────────────────────────────────────────────────────

/// Writes an OSC 52 clipboard escape sequence to `writer`.
///
/// Format: `ESC ] 52 ; c ; <base64> BEL`
///
/// `writer` should be the terminal device (typically stderr when stdout is
/// redirected to the TUI alternate screen).
pub fn write_osc52(content: &str, writer: &mut impl Write) -> Result<(), ClipboardError> {
    let encoded = base64_encode(content.as_bytes());
    write!(writer, "\x1b]52;c;{encoded}\x07")
        .map_err(|e| ClipboardError::Osc52WriteFailed(e.to_string()))?;
    writer
        .flush()
        .map_err(|e| ClipboardError::Osc52WriteFailed(e.to_string()))
}

// ── External tool ─────────────────────────────────────────────────────────────

/// Returns the first available clipboard tool for the current platform.
pub fn detect_external_tool() -> Option<&'static str> {
    #[cfg(target_os = "macos")]
    {
        Some("pbcopy")
    }

    #[cfg(target_os = "windows")]
    {
        Some("clip.exe")
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        // WSL: WSLENV is set by the Windows host.
        if std::env::var("WSLENV").is_ok() {
            return Some("clip.exe");
        }
        // Wayland: prefer wl-copy, fall back to X11 tools.
        if std::env::var("WAYLAND_DISPLAY").is_ok() && tool_exists("wl-copy") {
            return Some("wl-copy");
        }
        for tool in ["xclip", "xsel"] {
            if tool_exists(tool) {
                return Some(tool);
            }
        }
        None
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn tool_exists(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Pipes `content` to the stdin of the named clipboard tool.
///
/// Each tool requires specific arguments to target the system clipboard:
/// - `xclip`   → `-selection clipboard`
/// - `xsel`    → `--clipboard --input`
/// - `wl-copy`, `pbcopy`, `clip.exe` → no args needed
pub fn write_external(content: &str, tool: &str) -> Result<(), ClipboardError> {
    let args: &[&str] = match tool {
        "xclip" => &["-selection", "clipboard"],
        "xsel" => &["--clipboard", "--input"],
        _ => &[],
    };

    let mut child = Command::new(tool)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| ClipboardError::ExternalToolFailed(e.to_string()))?;

    {
        let stdin = child.stdin.as_mut().ok_or_else(|| {
            ClipboardError::ExternalToolFailed("could not open stdin".to_string())
        })?;
        stdin
            .write_all(content.as_bytes())
            .map_err(|e| ClipboardError::ExternalToolFailed(e.to_string()))?;
    }

    let status = child
        .wait()
        .map_err(|e| ClipboardError::ExternalToolFailed(e.to_string()))?;

    if status.success() {
        Ok(())
    } else {
        Err(ClipboardError::ExternalToolFailed(format!(
            "{tool} exited with {status}"
        )))
    }
}

// ── File fallback ─────────────────────────────────────────────────────────────

/// Returns the path used for the file-based clipboard fallback.
///
/// Respects `$XDG_DATA_HOME`; defaults to `$HOME/.local/share`.
pub fn clipboard_file_path() -> PathBuf {
    let base = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".local").join("share")
        });
    base.join("ucode").join("clipboard")
}

/// Writes `content` to the file-based clipboard fallback, creating parent
/// directories as needed.
pub fn write_file(content: &str) -> Result<(), ClipboardError> {
    let path = clipboard_file_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, content)?;
    Ok(())
}

// ── Main entry point ──────────────────────────────────────────────────────────

/// Writes `content` to the clipboard using the requested method, with
/// automatic fallback on failure.
///
/// Fallback chain:
/// - `Osc52`    → external tool → file
/// - `External` → file
/// - `File`     → (no fallback)
pub fn write_clipboard(
    content: &str,
    method: ClipboardMethod,
    writer: &mut impl Write,
) -> Result<(), ClipboardError> {
    match method {
        ClipboardMethod::Osc52 => {
            if let Err(_osc_err) = write_osc52(content, writer) {
                if let Some(tool) = detect_external_tool() {
                    if let Err(_ext_err) = write_external(content, tool) {
                        return write_file(content);
                    }
                    return Ok(());
                }
                return write_file(content);
            }
            Ok(())
        }
        ClipboardMethod::External => {
            let tool = detect_external_tool().ok_or(ClipboardError::ExternalToolNotFound)?;
            if let Err(_ext_err) = write_external(content, tool) {
                return write_file(content);
            }
            Ok(())
        }
        ClipboardMethod::File => write_file(content),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    // Env-var mutations are process-global; serialize tests that touch them.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn osc52_escape_format() {
        let mut buf: Vec<u8> = Vec::new();
        write_osc52("hi", &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.starts_with("\x1b]52;c;"), "missing OSC 52 introducer");
        assert!(s.ends_with('\x07'), "missing BEL terminator");
    }

    #[test]
    fn osc52_base64_encoding() {
        // "hello" in standard Base64 is "aGVsbG8="
        let mut buf: Vec<u8> = Vec::new();
        write_osc52("hello", &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert_eq!(s, "\x1b]52;c;aGVsbG8=\x07");
    }

    #[test]
    fn clipboard_file_path_default() {
        // Regardless of XDG_DATA_HOME, the tail must be ucode/clipboard.
        let _guard = ENV_LOCK.lock().unwrap();
        let path = clipboard_file_path();
        let components: Vec<_> = path.components().collect();
        let n = components.len();
        assert!(n >= 2);
        assert_eq!(components[n - 1].as_os_str(), "clipboard");
        assert_eq!(components[n - 2].as_os_str(), "ucode");
    }

    #[test]
    fn multiplexer_detection_none() {
        // multiplexer_name must not panic regardless of env state.
        let _ = multiplexer_name();
    }

    #[test]
    fn clipboard_method_default() {
        assert_eq!(ClipboardMethod::default(), ClipboardMethod::Osc52);
    }

    #[test]
    fn write_file_creates_and_writes() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        // Override XDG_DATA_HOME so clipboard_file_path points inside tempdir.
        // Use a sub-directory that doesn't exist yet to exercise create_dir_all.
        let data_home = dir.path().join("data");

        // SAFETY: single-threaded access guaranteed by ENV_LOCK.
        unsafe { std::env::set_var("XDG_DATA_HOME", &data_home) };
        let result = write_file("test content");
        unsafe { std::env::remove_var("XDG_DATA_HOME") };

        result.unwrap();
        let expected = data_home.join("ucode").join("clipboard");
        let content = std::fs::read_to_string(&expected).unwrap();
        assert_eq!(content, "test content");
    }

    #[test]
    fn write_clipboard_file_method() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let data_home = dir.path().join("xdg");

        // SAFETY: single-threaded access guaranteed by ENV_LOCK.
        unsafe { std::env::set_var("XDG_DATA_HOME", &data_home) };
        let mut sink: Vec<u8> = Vec::new();
        let result = write_clipboard("clipboard text", ClipboardMethod::File, &mut sink);
        unsafe { std::env::remove_var("XDG_DATA_HOME") };

        result.unwrap();
        // OSC 52 writer should not have been touched.
        assert!(sink.is_empty());
        let path = data_home.join("ucode").join("clipboard");
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "clipboard text");
    }
}
