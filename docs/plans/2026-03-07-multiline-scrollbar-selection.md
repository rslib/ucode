# Multiline Input, Scrollbars, and Textbox Selection

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix multiline input (bracketed paste, dynamic height, scrolling), add scrollbars to transcript and input box, and add text selection to the input box with the same Shift+arrow keyboard patterns as copy mode.

**Architecture:** Three layers: (1) fix multiline input mechanics (paste, height, scroll viewport), (2) add ratatui Scrollbar widgets to transcript and input, (3) add selection state to InputBoxState with Shift+key handling mirroring copy mode patterns.

**Tech Stack:** Rust, ratatui 0.30 (Scrollbar, ScrollbarState, ScrollbarOrientation), crossterm (EnableBracketedPaste)

---

## Current State

- `InputBoxState` stores a `String` with byte-offset cursor. Has `insert_char`, `insert_newline`, `move_up/down`, `line_count()`.
- `insert_char('\n')` does NOT reset `cursor_col` — only `insert_newline()` does.
- `insert_str` calls `insert_char` per char, so pasting newlines breaks cursor tracking.
- Bracketed paste is NOT enabled — pasted newlines arrive as separate Enter keypresses → `SendMessage`.
- `InputState` in layout has `line_count: u16` defaulting to 1, never updated.
- `INPUT_MAX_LINES = 8` — change to 6.
- No scroll viewport in input box — renders `lines.iter().take(visible_rows)` from index 0.
- No scrollbar on transcript or input.
- No text selection in input box.

## Key Files

- `crates/ucode-tui/src/components/input.rs` — InputBoxState, InputBox widget
- `crates/ucode-tui/src/event_loop.rs` — terminal setup, render_frame, key handling
- `crates/ucode-tui/src/layout.rs` — InputState, INPUT_MAX_LINES, compute_layout
- `crates/ucode-tui/src/components/transcript.rs` — TranscriptView widget
- `crates/ucode-tui/src/app.rs` — AppState (has `input: InputState`, `scroll_offset`)

---

### Task 1: Enable bracketed paste

**Files:**
- Modify: `crates/ucode-tui/src/event_loop.rs` (terminal setup + cleanup)

**What:**
- After `enable_raw_mode()` (line ~180), add `execute!(stderr, crossterm::event::EnableBracketedPaste)?;`
- In the cleanup guard (wherever `LeaveAlternateScreen` is called), add `crossterm::event::DisableBracketedPaste`
- The `Event::Paste(text)` handler at line ~1320 already calls `input_box.insert_str(&text)` — this will work once Task 2 fixes `insert_str`.

**Verify:** `cargo check` — 0 warnings.

---

### Task 2: Fix `insert_char` and `insert_str` for newlines

**Files:**
- Modify: `crates/ucode-tui/src/components/input.rs`

**What:**
- In `insert_char`, handle `'\n'` specially: after inserting, reset `cursor_col = 0` (same as `insert_newline` does).
- Remove `insert_newline` body duplication — just call `insert_char('\n')`.
- This makes `insert_str` work correctly for multiline pastes.

**Verify:** `cargo test -p ucode-tui` — all existing tests pass. Add a test: `insert_str` with embedded newlines, verify `cursor_col` is correct.

---

### Task 3: Dynamic input box height + scroll viewport

**Files:**
- Modify: `crates/ucode-tui/src/layout.rs` — change `INPUT_MAX_LINES` from 8 to 6
- Modify: `crates/ucode-tui/src/components/input.rs` — add `scroll_offset` to `InputBoxState`, update renderer
- Modify: `crates/ucode-tui/src/event_loop.rs` — update `app.input.line_count` after any content change

**What:**

In `InputBoxState`:
- Add field `pub scroll_offset: usize` (line index of first visible line), default 0.
- Add method `pub fn ensure_cursor_visible(&mut self, max_visible: usize)` that adjusts `scroll_offset` so the cursor line is within the visible window.
- Add method `pub fn visible_line_count(&self) -> usize` returning `line_count()`.

In `InputBox::render`:
- Instead of `.take(visible_rows)` from index 0, skip `self.state.scroll_offset` lines first.
- Adjust cursor_y: `cursor_y = inner.y + (cursor_line - scroll_offset) as u16`.

In `event_loop.rs`:
- After every input mutation (insert_char, delete, paste, etc.), call:
  ```rust
  app.input.line_count = (input_box.line_count() as u16).clamp(1, INPUT_MAX_LINES);
  input_box.ensure_cursor_visible(INPUT_MAX_LINES as usize);
  ```
- This makes the input box grow up to 6 lines, then scroll.

**Verify:** `cargo test -p ucode-tui` — all pass. Manual test: paste 10+ lines, verify scrolling.

---

### Task 4: Scrollbar on transcript

**Files:**
- Modify: `crates/ucode-tui/src/event_loop.rs` — render_frame function
- Modify: `crates/ucode-tui/src/components/transcript.rs` — return total_lines from render (or compute externally)

**What:**

In `render_frame`, after rendering the transcript widget, render a vertical scrollbar:
```rust
use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState};

let mut scrollbar_state = ScrollbarState::default()
    .content_length(app.copy_mode.total_lines)
    .viewport_content_length(viewport_h)
    .position(app.scroll_offset);
let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
    .thumb_style(app.theme.accent_style())
    .track_style(app.theme.dim_style());
f.render_stateful_widget(scrollbar, areas.transcript, &mut scrollbar_state);
```

Only render when `total_lines > viewport_h` (content overflows).

**Verify:** `cargo check` — 0 warnings. Manual: scroll transcript, see scrollbar thumb move.

---

### Task 5: Scrollbar on input box

**Files:**
- Modify: `crates/ucode-tui/src/components/input.rs` — InputBox render
- OR modify: `crates/ucode-tui/src/event_loop.rs` — render scrollbar after InputBox

**What:**

After rendering the InputBox widget in `render_frame`, render a vertical scrollbar on the input area when content exceeds visible lines:
```rust
if input_box.line_count() > app.input.line_count as usize {
    let mut sb_state = ScrollbarState::default()
        .content_length(input_box.line_count())
        .viewport_content_length(app.input.line_count as usize)
        .position(input_box.scroll_offset);
    let sb = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .thumb_style(app.theme.accent_style())
        .track_style(app.theme.dim_style());
    f.render_stateful_widget(sb, areas.input, &mut sb_state);
}
```

**Verify:** `cargo check` — 0 warnings. Manual: type/paste 10+ lines in input, see scrollbar.

---

### Task 6: Text selection state in InputBoxState

**Files:**
- Modify: `crates/ucode-tui/src/components/input.rs`

**What:**

Add selection fields to `InputBoxState`:
```rust
/// Byte offset of the selection anchor. When `selecting` is true,
/// the selected range is `min(anchor, cursor_pos)..max(anchor, cursor_pos)`.
pub selection_anchor: usize,
/// True when a selection is active.
pub selecting: bool,
```

Add methods:
- `pub fn start_selecting(&mut self)` — set `selecting = true`, `selection_anchor = cursor_pos`.
- `pub fn cancel_selecting(&mut self)` — set `selecting = false`.
- `pub fn selected_range(&self) -> Option<std::ops::Range<usize>>` — returns `Some(min..max)` if selecting and anchor != cursor, else None.
- `pub fn selected_text(&self) -> Option<&str>` — returns the selected slice.
- `pub fn delete_selected(&mut self)` — removes selected text, places cursor at start of range, cancels selection.
- `pub fn ensure_selecting(&mut self)` — if not selecting, start; if already selecting, no-op. (Mirrors `copy_mode_ensure_char_selecting`.)

**Verify:** Add unit tests for each method. `cargo test -p ucode-tui`.

---

### Task 7: Shift+key selection in textbox key handling

**Files:**
- Modify: `crates/ucode-tui/src/event_loop.rs` — input key handling

**What:**

In the input key handler (where arrow keys, Home, End are processed), add Shift variants:
- `Shift+Left/Right/Up/Down` → `input_box.ensure_selecting()` then move
- `Shift+Home/End` → `input_box.ensure_selecting()` then move to line start/end
- `Shift+Ctrl+Left/Right` (vscode) → `input_box.ensure_selecting()` then word move
- `Shift+Alt+B/F` (emacs) → `input_box.ensure_selecting()` then word move
- Plain movement keys (without Shift) → `input_box.cancel_selecting()` then move
- Typing any character when selecting → `input_box.delete_selected()` then insert
- Backspace/Delete when selecting → `input_box.delete_selected()`
- `Ctrl+C` when selecting → copy selected text to clipboard, cancel selection

**Verify:** `cargo test -p ucode-tui`. Manual: Shift+arrow in input box, see selection, type to replace.

---

### Task 8: Render selection highlight in input box

**Files:**
- Modify: `crates/ucode-tui/src/components/input.rs` — InputBox::render

**What:**

In the InputBox render method, after rendering text lines, if `self.state.selecting`:
- Compute the selected byte range.
- For each visible line, compute which columns overlap the selection.
- Apply `Modifier::REVERSED` style to selected cells (same approach as copy mode's `highlight_line_range`).

**Verify:** `cargo check`. Manual: Shift+arrow, see highlighted selection in input box.

---

### Task 9: Final verification

**Verify:**
- `cargo check` — 0 errors, 0 warnings
- `cargo test --workspace` — 1628+ tests pass, 0 failures
- Manual test checklist:
  - Paste multiline text → input grows up to 6 lines, then scrolls
  - Shift+Enter / Alt+Enter inserts newline
  - Shift+arrow in input → selection highlight appears
  - Type while selected → replaces selection
  - Ctrl+C while selected → copies to clipboard
  - Transcript scrollbar visible when content overflows
  - Input scrollbar visible when content > 6 lines
  - All three presets (vim/emacs/vscode) work
