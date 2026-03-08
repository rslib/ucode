# Line-Level Copy Mode Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace entry-level copy mode with line-level selection, two-phase UX (navigate then select), mouse support, and auto-copy on drag release.

**Architecture:** CopyModeState changes from entry indices to visual-line indices. A new `compute_all_lines()` function is extracted from TranscriptView::render so both rendering and text extraction share the same line computation. Mouse events in the transcript area enter/extend selection mode. Drag-release auto-copies.

**Tech Stack:** Rust, ratatui, crossterm (mouse events)

---

### Task 1: Refactor CopyModeState to visual-line-level

**Files:**
- Modify: `crates/ucode-tui/src/overlays/copy_mode.rs`

**Step 1: Rewrite CopyModeState struct and methods**

Replace the entire CopyModeState with:

```rust
#[derive(Debug, Clone)]
pub struct CopyModeState {
    /// True when selection mode is active (phase 1 or 2).
    pub active: bool,
    /// True when visual selection is active (phase 2, after pressing `v` or mouse drag).
    pub selecting: bool,
    /// Visual line index of the anchor (start of selection). Only meaningful when selecting=true.
    pub anchor: usize,
    /// Visual line index of the cursor (current position).
    pub cursor: usize,
    /// Total rendered lines in the transcript. Updated each render frame.
    pub total_lines: usize,
}
```

Methods to implement:
- `new()` — all false/zero
- `enter(line: usize)` — sets active=true, selecting=false, cursor=line
- `start_selecting()` — sets selecting=true, anchor=cursor
- `exit()` — sets active=false, selecting=false
- `exit_selecting()` — sets selecting=false (stays in phase 1)
- `move_up()` — decrements cursor if > 0
- `move_down()` — increments cursor if < total_lines - 1
- `swap_anchor()` — swaps anchor and cursor (the `o` key)
- `selection_range() -> Option<(usize, usize)>` — returns Some((min, max)) only when selecting=true
- `is_line_selected(line: usize) -> bool` — true if selecting and line is in range
- `is_cursor_line(line: usize) -> bool` — true if line == cursor (for phase 1 indicator)

**Step 2: Rewrite `entry_to_copy_text` and `collect_selection_text`**

Replace `collect_selection_text` with a new function that works on visual lines:

```rust
/// Extract plain text from a single rendered Line (concatenate span contents).
pub fn line_to_text(line: &ratatui::text::Line<'_>) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

/// Collect text from a range of visual lines.
pub fn collect_lines_text(lines: &[ratatui::text::Line<'_>], start: usize, end: usize) -> String {
    lines[start..=end.min(lines.len().saturating_sub(1))]
        .iter()
        .map(|l| line_to_text(l))
        .collect::<Vec<_>>()
        .join("\n")
}
```

Keep `entry_to_copy_text` for backward compat but it can be removed later.

**Step 3: Update tests**

Rewrite existing CopyModeState tests to match new API. Add tests for:
- `enter` sets active=true, selecting=false
- `start_selecting` sets selecting=true, anchor=cursor
- `exit_selecting` keeps active=true
- `swap_anchor` swaps values
- `selection_range` returns None when not selecting
- `is_line_selected` and `is_cursor_line`
- `line_to_text` and `collect_lines_text`

**Step 4: Verify**

Run: `cargo test -p ucode-tui`
Run: `cargo check --workspace`

---

### Task 2: Extract `compute_all_lines()` from TranscriptView

**Files:**
- Modify: `crates/ucode-tui/src/components/transcript.rs`

**Step 1: Create standalone `compute_all_lines` function**

Extract the line-building logic from `TranscriptView::render()` into a public function:

```rust
/// Compute all visual lines for the transcript entries.
/// Returns the flat list of rendered lines (no copy-mode highlighting applied).
pub fn compute_all_lines<'a>(
    entries: &'a [TranscriptEntry],
    theme: &'a UcodeTheme,
    width: u16,
    show_cursor: bool,
) -> Vec<Line<'a>> {
    let last_entry_idx = entries.len().checked_sub(1);
    let cursor_idx = last_entry_idx
        .filter(|&idx| matches!(entries[idx], TranscriptEntry::Streaming(_)));

    entries
        .iter()
        .enumerate()
        .flat_map(|(i, entry)| {
            let cursor = show_cursor && Some(i) == cursor_idx;
            entry_lines(entry, theme, width, cursor)
        })
        .collect()
}
```

**Step 2: Refactor TranscriptView::render to use it**

Replace the inline `all_lines` computation in `render()` with a call to `compute_all_lines()`. Then apply copy-mode highlighting as a separate pass over the computed lines, using line indices instead of entry indices:

```rust
// In render():
let mut all_lines = compute_all_lines(self.entries, self.theme, width, self.show_cursor);

// Apply copy-mode highlighting by line index.
if self.copy_mode.active {
    use ratatui::style::{Modifier, Style};
    for (line_idx, line) in all_lines.iter_mut().enumerate() {
        let highlight = if self.copy_mode.selecting && self.copy_mode.is_line_selected(line_idx) {
            // Phase 2: full selection highlight
            Some(Style::default()
                .bg(self.theme.border_focus)
                .add_modifier(Modifier::BOLD))
        } else if !self.copy_mode.selecting && self.copy_mode.is_cursor_line(line_idx) {
            // Phase 1: cursor line indicator (subtle)
            Some(Style::default().bg(self.theme.surface))
        } else {
            None
        };
        if let Some(hl) = highlight {
            let spans: Vec<_> = line.spans.iter()
                .map(|span| {
                    let mut s = span.clone();
                    s.style = s.style.patch(hl);
                    s
                })
                .collect();
            *line = Line::from(spans);
        }
    }
}
```

**Step 3: Verify**

Run: `cargo test -p ucode-tui`
Run: `cargo check --workspace`

---

### Task 3: Add `transcript_area` and `last_total_lines` to AppState

**Files:**
- Modify: `crates/ucode-tui/src/app.rs`
- Modify: `crates/ucode-tui/src/event_loop.rs` (render_frame)

**Step 1: Add fields to AppState**

```rust
// In AppState struct:
/// Last computed transcript area rect (for mouse hit-testing).
pub transcript_area: ratatui::layout::Rect,
```

Initialize in `AppState::new()`:
```rust
transcript_area: ratatui::layout::Rect::default(),
```

**Step 2: Update render_frame to store transcript_area and total_lines**

In `render_frame()`, after computing layout and before rendering transcript:

```rust
// Store for mouse hit-testing in event loop.
app.transcript_area = areas.transcript;
```

After building the transcript widget (or computing all_lines), update total_lines:

```rust
// Update total_lines for copy mode navigation bounds.
// Use compute_all_lines to get the count.
app.copy_mode.total_lines = crate::components::transcript::compute_all_lines(
    &app.transcript, &app.theme, areas.transcript.width, show_cursor,
).len();
```

Note: This double-computes all_lines. To avoid that, we can compute it once in render_frame, pass it to TranscriptView, and also use it for total_lines. But that requires changing TranscriptView to accept pre-computed lines. For simplicity, just call `entry_height` sum instead:

```rust
use crate::components::transcript::entry_height;
app.copy_mode.total_lines = app.transcript.iter()
    .map(|e| entry_height(e, areas.transcript.width))
    .sum::<usize>();
```

This is cheaper and already exists.

**Step 3: Verify**

Run: `cargo check --workspace`

---

### Task 4: Add `EnterSelectionMode` action and Ctrl+Y binding

**Files:**
- Modify: `crates/ucode-tui/src/keybinds.rs`

**Step 1: Add new action variant**

Add to the `Action` enum:
```rust
EnterSelectionMode,
```

**Step 2: Add Ctrl+Y binding to all three presets**

In `default_vscode_bindings()`:
```rust
m.insert(KeyCombo::new(K::Char('y'), Mod::CONTROL), A::EnterSelectionMode);
```

In `default_vim_bindings()`:
```rust
m.insert(KeyCombo::new(K::Char('y'), Mod::CONTROL), A::EnterSelectionMode);
```

In `default_emacs_bindings()`:
```rust
m.insert(KeyCombo::new(K::Char('y'), Mod::CONTROL), A::EnterSelectionMode);
```

**Step 3: Remove old bare `v` → EnterCopyMode from vscode and vim presets**

Remove these lines:
```rust
// vscode:
m.insert(KeyCombo::new(K::Char('v'), Mod::NONE), A::EnterCopyMode);
// vim:
m.insert(KeyCombo::new(K::Char('v'), Mod::NONE), A::EnterCopyMode);
```

Keep `EnterCopyMode` in the Action enum for now (it will be unused but harmless). Or rename it — but simpler to just leave it.

**Step 4: Verify**

Run: `cargo check --workspace`
Run: `cargo test -p ucode-tui`

---

### Task 5: Rewrite copy mode key handling in event loop

**Files:**
- Modify: `crates/ucode-tui/src/event_loop.rs`

**Step 1: Rewrite the `if app.copy_mode.active` block (~line 613-676)**

Replace the entire copy mode key handler with the two-phase logic:

```rust
if app.copy_mode.active {
    match key.code {
        // Esc: if selecting, drop selection (back to phase 1). If phase 1, exit entirely.
        crossterm::event::KeyCode::Esc => {
            if app.copy_mode.selecting {
                app.copy_mode.exit_selecting();
            } else {
                app.copy_mode.exit();
                app.focus = FocusTarget::Input;
            }
            app.mark_dirty();
        }
        // Navigation: move cursor by one visual line
        crossterm::event::KeyCode::Up | crossterm::event::KeyCode::Char('k') => {
            app.copy_mode.move_up();
            // Scroll viewport to keep cursor visible.
            // Convert cursor (visual line) to scroll offset.
            if app.copy_mode.cursor < app.scroll_offset {
                app.scroll_offset = app.copy_mode.cursor;
            }
            app.mark_dirty();
        }
        crossterm::event::KeyCode::Down | crossterm::event::KeyCode::Char('j') => {
            app.copy_mode.move_down();
            let viewport_height = app.transcript_area.height as usize;
            if viewport_height > 0 && app.copy_mode.cursor >= app.scroll_offset + viewport_height {
                app.scroll_offset = app.copy_mode.cursor.saturating_sub(viewport_height - 1);
            }
            app.mark_dirty();
        }
        // v: enter visual selection (phase 2) — anchor at current cursor
        crossterm::event::KeyCode::Char('v') => {
            if !app.copy_mode.selecting {
                app.copy_mode.start_selecting();
            } else {
                // Already selecting — pressing v again exits visual mode
                app.copy_mode.exit_selecting();
            }
            app.mark_dirty();
        }
        // o: swap anchor and cursor
        crossterm::event::KeyCode::Char('o') => {
            if app.copy_mode.selecting {
                app.copy_mode.swap_anchor();
            }
            app.mark_dirty();
        }
        // y: yank selection (only in phase 2)
        crossterm::event::KeyCode::Char('y') => {
            if app.copy_mode.selecting {
                // Build visual lines and extract text.
                let all_lines = crate::components::transcript::compute_all_lines(
                    &app.transcript,
                    &app.theme,
                    app.transcript_area.width,
                    false, // no streaming cursor in copy text
                );
                if let Some((start, end)) = app.copy_mode.selection_range() {
                    let text = crate::overlays::copy_mode::collect_lines_text(&all_lines, start, end);
                    do_clipboard_copy(&text, app);
                }
                app.copy_mode.exit();
                app.focus = FocusTarget::Input;
                app.mark_dirty();
            }
        }
        _ => {}
    }
    return false;
}
```

**Step 2: Add `do_clipboard_copy` helper**

Extract the clipboard write + toast logic into a helper to avoid duplication (used by keyboard yank and mouse drag release):

```rust
fn do_clipboard_copy(text: &str, app: &mut AppState) {
    let mut writer = std::io::stderr();
    match crate::clipboard::write_clipboard(
        text,
        crate::clipboard::ClipboardMethod::default(),
        &mut writer,
    ) {
        Ok(()) => {
            let line_count = text.lines().count();
            let label = if line_count == 1 { "line" } else { "lines" };
            app.toasts.push(
                crate::components::toast::ToastLevel::Success,
                format!("Copied {line_count} {label}"),
            );
        }
        Err(e) => {
            app.toasts.push(
                crate::components::toast::ToastLevel::Error,
                format!("Copy failed: {e}"),
            );
        }
    }
}
```

**Step 3: Update `dispatch_action` for `EnterSelectionMode`**

Add handler for the new action:

```rust
Action::EnterSelectionMode => {
    if app.copy_mode.total_lines > 0 {
        // Enter selection mode at the first visible line.
        let cursor_line = app.scroll_offset;
        app.copy_mode.enter(cursor_line.min(app.copy_mode.total_lines.saturating_sub(1)));
        app.focus = FocusTarget::Transcript;
        app.mark_dirty();
    }
}
```

**Step 4: Remove old `EnterCopyMode` / `SetMark` handlers or redirect them**

The old `EnterCopyMode` and `SetMark` handlers can redirect to `EnterSelectionMode` logic, or be left as no-ops since the keybinds no longer map to them.

**Step 5: Remove the old bare-char `v` check for empty input**

In the input focus handler (~line 1038), remove the special case that checked keybinds when input was empty. Ctrl+Y goes through normal keybind resolution regardless of input content.

Actually, keep the empty-input keybind check — it's still useful for `?` (ShowKeybindOverlay) and other bare-key bindings. Just remove the `v` → EnterCopyMode binding from the keybind maps (done in Task 4).

**Step 6: Verify**

Run: `cargo check --workspace`
Run: `cargo test -p ucode-tui`

---

### Task 6: Rewrite mouse handling for selection mode

**Files:**
- Modify: `crates/ucode-tui/src/event_loop.rs`

**Step 1: Rewrite the `Event::Mouse` handler (~line 1282-1305)**

Replace with comprehensive mouse handling:

```rust
Event::Mouse(me) if app.mouse_enabled => {
    use crossterm::event::{MouseButton, MouseEventKind};
    match me.kind {
        MouseEventKind::ScrollUp => {
            app.scroll_up(3);
        }
        MouseEventKind::ScrollDown => {
            app.scroll_down(3);
        }
        MouseEventKind::Down(MouseButton::Left) => {
            // Check if click is in transcript area.
            let in_transcript = me.row >= app.transcript_area.y
                && me.row < app.transcript_area.y + app.transcript_area.height
                && me.column >= app.transcript_area.x
                && me.column < app.transcript_area.x + app.transcript_area.width;

            if in_transcript {
                // Compute visual line from click position.
                let relative_row = (me.row - app.transcript_area.y) as usize;
                let visual_line = app.scroll_offset + relative_row;
                let clamped = visual_line.min(app.copy_mode.total_lines.saturating_sub(1));

                // Enter selection mode phase 1 at clicked line.
                app.copy_mode.enter(clamped);
                app.focus = FocusTarget::Transcript;
            } else {
                // Click outside transcript — focus input, exit copy mode.
                if app.copy_mode.active {
                    app.copy_mode.exit();
                }
                let height = app.terminal_size.height;
                if me.row >= height.saturating_sub(3) {
                    app.focus = FocusTarget::Input;
                }
            }
            app.mark_dirty();
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            // Only handle drag in transcript area.
            let in_transcript = me.row >= app.transcript_area.y
                && me.row < app.transcript_area.y + app.transcript_area.height
                && me.column >= app.transcript_area.x
                && me.column < app.transcript_area.x + app.transcript_area.width;

            if in_transcript && app.copy_mode.active {
                let relative_row = (me.row - app.transcript_area.y) as usize;
                let visual_line = app.scroll_offset + relative_row;
                let clamped = visual_line.min(app.copy_mode.total_lines.saturating_sub(1));

                // Start visual selection if not already selecting.
                if !app.copy_mode.selecting {
                    app.copy_mode.start_selecting();
                }
                // Move cursor to drag position.
                app.copy_mode.cursor = clamped;
                app.mark_dirty();
            }
        }
        MouseEventKind::Up(MouseButton::Left) => {
            // On mouse release: if we were selecting via drag, auto-copy and exit.
            if app.copy_mode.active && app.copy_mode.selecting {
                let all_lines = crate::components::transcript::compute_all_lines(
                    &app.transcript,
                    &app.theme,
                    app.transcript_area.width,
                    false,
                );
                if let Some((start, end)) = app.copy_mode.selection_range() {
                    let text = crate::overlays::copy_mode::collect_lines_text(&all_lines, start, end);
                    if !text.trim().is_empty() {
                        do_clipboard_copy(&text, app);
                    }
                }
                app.copy_mode.exit();
                app.focus = FocusTarget::Input;
                app.mark_dirty();
            }
        }
        _ => {}
    }
}
```

**Step 2: Verify**

Run: `cargo check --workspace`
Run: `cargo test -p ucode-tui`

---

### Task 7: Update demo and keybind overlay

**Files:**
- Modify: `crates/ucode-tui/examples/demo.rs` (update help text)
- Modify: `crates/ucode-tui/src/overlays/keybind_overlay.rs` (update displayed bindings)

**Step 1: Update demo help text**

In the demo's title/help string, replace `v copy` with `Ctrl+Y select` or similar.

**Step 2: Update keybind overlay**

If the keybind overlay has hardcoded copy mode references, update them to show:
- `Ctrl+Y` — Enter selection mode
- `v` — Start visual selection (in selection mode)
- `o` — Swap anchor/cursor
- `y` — Yank selection
- `Esc` — Exit selection / back to navigate
- Mouse drag — Select and auto-copy

**Step 3: Verify**

Run: `cargo check --workspace`
Run: `cargo test --workspace`
Expected: All 1578+ tests pass.

---

### Task 8: Final integration test

**Step 1: Run full test suite**

Run: `cargo test --workspace`
Expected: All tests pass.

**Step 2: Run cargo clippy**

Run: `cargo clippy --workspace`
Expected: No errors (warnings acceptable).

---
