# Preset-Aware Copy Mode Keybindings

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make copy mode navigation and selection keys match the active keybind preset (vim/emacs/vscode) so they're consistent with textbox editing.

**Architecture:** Add new movement methods to `CopyModeState` (word, line-start/end, first/last line). Extract `is_word_char` to a shared location. Branch the copy mode key handler in `event_loop.rs` on `app.keybinds.preset` — shared keys first, then preset-specific blocks. Word movement needs current line text, computed on demand via `compute_all_lines` (already used for yank).

**Tech Stack:** Rust, crossterm key events, ratatui Line/Span, unicode-width

---

### Task 1: Add movement methods to CopyModeState

**Files:**
- Modify: `crates/ucode-tui/src/overlays/copy_mode.rs`

Add these methods after `move_right()`:

```rust
/// Move cursor to column 0.
pub fn move_to_line_start(&mut self) {
    self.cursor.col = 0;
}

/// Move cursor to end of line (line_width - 1, or 0 if empty).
pub fn move_to_line_end(&mut self, line_width: usize) {
    self.cursor.col = line_width.saturating_sub(1);
}

/// Move cursor to first line, preserving column.
pub fn move_to_first_line(&mut self) {
    self.cursor.line = 0;
}

/// Move cursor to last line, preserving column.
pub fn move_to_last_line(&mut self) {
    self.cursor.line = self.total_lines.saturating_sub(1);
}

/// Move cursor to a specific column.
pub fn move_to_col(&mut self, col: usize) {
    self.cursor.col = col;
}
```

Add tests for each method.

**Verify:** `cargo test -p ucode-tui -- copy_mode`

### Task 2: Extract `is_word_char` and add word-boundary helpers

**Files:**
- Modify: `crates/ucode-tui/src/overlays/copy_mode.rs`
- Modify: `crates/ucode-tui/src/components/input.rs` (reuse helper)

Add to `copy_mode.rs`:

```rust
/// Word character predicate (same as input.rs).
pub fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Find the column of the next word start from `col` in `text`.
/// Uses display-width columns (unicode-aware).
pub fn next_word_start(text: &str, col: usize) -> usize { ... }

/// Find the column of the previous word start from `col` in `text`.
pub fn prev_word_start(text: &str, col: usize) -> usize { ... }

/// Find the column of the next word end from `col` in `text`.
pub fn next_word_end(text: &str, col: usize) -> usize { ... }
```

Make `input.rs` use `crate::overlays::copy_mode::is_word_char` instead of its private copy.

Add tests for word boundary functions.

**Verify:** `cargo test -p ucode-tui`

### Task 3: Helper to get current line text in copy mode

**Files:**
- Modify: `crates/ucode-tui/src/event_loop.rs`

Add a helper function near the copy mode handler:

```rust
/// Get the text and display width of a specific visual line.
fn copy_mode_line_info(app: &AppState, line_idx: usize) -> (String, usize) {
    let lines = crate::components::transcript::compute_all_lines(
        &app.transcript, &app.theme, app.transcript_area.width, false,
    );
    if line_idx < lines.len() {
        let text = crate::overlays::copy_mode::line_to_text(&lines[line_idx]);
        let width: usize = lines[line_idx].spans.iter()
            .map(|s| unicode_width::UnicodeWidthStr::width(s.content.as_ref()))
            .sum();
        (text, width)
    } else {
        (String::new(), 0)
    }
}
```

**Verify:** `cargo check -p ucode-tui`

### Task 4: Refactor copy mode handler — shared keys + preset branches

**Files:**
- Modify: `crates/ucode-tui/src/event_loop.rs` (lines ~614-722)

Restructure the copy mode handler:

```
if app.copy_mode.active {
    let preset = app.keybinds.preset;

    // === Shared keys (all presets) ===
    match key.code {
        Esc => { ... }  // exit (all presets)
        Up => { move_up + scroll }
        Down => { move_down + scroll }
        Left => { move_left }
        Right => { move_right }
        _ => { /* fall through to preset-specific */ }
    }

    // === Preset-specific keys ===
    match preset {
        Vim => { handle_vim_copy_keys(key, app) }
        Emacs => { handle_emacs_copy_keys(key, app) }
        Vscode => { handle_vscode_copy_keys(key, app) }
    }

    return false;
}
```

**Vim keys:**
- h/l → move left/right
- j/k → move up/down
- w → next word start
- b → prev word start
- e → next word end
- 0 → line start
- $ → line end
- G → last line
- g → first line (simplified from gg)
- v/V/Ctrl+V → selection modes
- y → yank
- o → swap anchor

**Emacs keys:**
- Ctrl+B/F → move left/right
- Ctrl+N/P → move up/down
- Alt+B/F → word left/right
- Ctrl+A → line start
- Ctrl+E → line end
- Alt+< → first line
- Alt+> → last line
- Ctrl+Space → start/toggle char selection
- Alt+W → yank/copy
- Ctrl+G → exit (same as Esc)

**VSCode keys:**
- Home → line start
- End → line end
- Ctrl+Left/Right → word left/right
- Ctrl+Home → first line
- Ctrl+End → last line
- v/V/Ctrl+V → selection modes (no better VSCode equivalent)
- y → yank

**Verify:** `cargo check --workspace && cargo test --workspace`

### Task 5: Update keybind overlay descriptions

**Files:**
- Modify: `crates/ucode-tui/src/overlays/keybind_overlay.rs`

Update the copy mode section to show preset-appropriate keys.

**Verify:** `cargo check -p ucode-tui`

### Task 6: Update demo help text

**Files:**
- Modify: `crates/ucode-tui/examples/demo.rs`

Update the help text to reflect preset-aware keys.

**Verify:** `cargo check -p ucode-tui --examples`

### Task 7: Final verification

**Verify:** `cargo check --workspace && cargo test --workspace` — all 1610+ tests pass.
