# Character/Line/Block Visual Selection Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace line-only copy mode with three vim-style visual selection modes: character (`v`), line (`V`), and block (`Ctrl+V`).

**Architecture:** Extend `CopyModeState` to track `Position { line, col }` instead of bare `usize` for anchor/cursor, add a `VisualMode` enum. The highlighting in `transcript.rs` splits spans at column boundaries for partial-line selection. Mouse handlers compute `(line, col)` from click coordinates. Text extraction handles all three modes.

**Tech Stack:** Rust, ratatui (Line/Span manipulation), crossterm (mouse events)

---

### Task 1: Rewrite CopyModeState data model

**Files:**
- Modify: `crates/ucode-tui/src/overlays/copy_mode.rs`

**Step 1: Replace the data model**

Replace the entire `CopyModeState` struct and its methods. Key changes:
- Add `Position { line: usize, col: usize }` struct
- Add `VisualMode { Char, Line, Block }` enum
- Change `anchor` and `cursor` from `usize` to `Position`
- Add `mode: VisualMode` field
- Update `enter()` to accept `(line, col)`
- Add `start_selecting_with_mode(mode)` replacing `start_selecting()`
- Add `move_left()` / `move_right()` for column movement
- Add `line_col_range(line)` → returns `Option<(usize, usize)>` column range for a given line based on mode
- Update `selection_range()` to return line range `(start_line, end_line)`
- Add `is_cursor_pos(line, col)` for phase 1 cursor column indicator
- Keep `is_cursor_line(line)` for phase 1 line highlight
- Keep `swap_anchor()` — now swaps Position structs
- Update `move_up()` / `move_down()` to preserve col

**Step 2: Rewrite text extraction helpers**

- `collect_selected_text(lines, state)` — new function that handles all 3 modes:
  - **Char**: first line from anchor.col, middle lines full, last line to cursor.col
  - **Line**: full lines (current behavior)
  - **Block**: for each line, extract columns in rectangle
- Keep `line_to_text()` and `collect_lines_text()` for backward compat
- Add `line_text_range(line, start_col, end_col)` — extract substring by column from a Line

**Step 3: Rewrite all tests**

Update all existing tests to use `Position` instead of bare `usize`. Add new tests for:
- `VisualMode::Char` selection range and text extraction
- `VisualMode::Line` selection (same as before)
- `VisualMode::Block` selection range and text extraction
- `move_left()` / `move_right()` bounds
- `line_col_range()` for each mode
- `collect_selected_text()` for each mode

**Step 4: Verify**

Run: `cargo check -p ucode-tui 2>&1`
Expected: compilation errors in event_loop.rs and transcript.rs (they still use old API)

---

### Task 2: Update transcript.rs highlighting for partial-line selection

**Files:**
- Modify: `crates/ucode-tui/src/components/transcript.rs:58-88`

**Step 1: Replace the highlighting loop**

The current loop at lines 58-88 applies bg to entire lines. Replace with column-aware highlighting:

```rust
if self.copy_mode.active {
    use ratatui::style::{Modifier, Style};
    for (line_idx, line) in all_lines.iter_mut().enumerate() {
        if self.copy_mode.selecting {
            // Get column range for this line based on visual mode
            if let Some((start_col, end_col)) = self.copy_mode.line_col_range(line_idx) {
                let hl = Style::default()
                    .bg(self.theme.select)
                    .add_modifier(Modifier::BOLD);
                *line = highlight_line_range(line, start_col, end_col, hl);
            }
        }
        // Phase 1 cursor: highlight just the cursor position
        if self.copy_mode.is_cursor_line(line_idx) {
            let hl = Style::default().bg(self.theme.select_cursor);
            // In phase 1 (not selecting), highlight entire cursor line
            // In phase 2 (selecting), the cursor line is already handled above
            if !self.copy_mode.selecting {
                *line = highlight_line_full(line, hl);
            }
        }
    }
}
```

**Step 2: Add `highlight_line_range()` helper**

New function that takes a `Line`, a column range `(start_col, end_col)`, and a style. It walks the spans, tracking cumulative column position, and splits spans at the boundaries to apply the highlight only to the selected columns.

```rust
fn highlight_line_range(line: &Line<'_>, start_col: usize, end_col: usize, hl: Style) -> Line<'_> {
    // Walk spans, track cumulative column offset
    // For each span: compute overlap with [start_col, end_col]
    // Split span into pre/selected/post segments
    // Apply hl style only to selected segment
}
```

**Step 3: Add `highlight_line_full()` helper**

Simple version that applies style to all spans (current behavior, extracted to a function).

**Step 4: Verify**

Run: `cargo check -p ucode-tui 2>&1`
Expected: compilation errors in event_loop.rs (still uses old API)

---

### Task 3: Update event_loop.rs key handlers

**Files:**
- Modify: `crates/ucode-tui/src/event_loop.rs:613-684` (copy mode key handler block)

**Step 1: Update navigation keys**

- `j/k/Up/Down`: call `move_up()` / `move_down()` (now preserves col). Update scroll tracking to use `cursor.line`.
- Add `h/l/Left/Right`: call `move_left()` / `move_right()` for column movement.

**Step 2: Update selection mode keys**

- `v`: start/toggle `VisualMode::Char` selection. If already in Char mode, exit selecting. If in another mode, switch to Char.
- `V` (Shift+v): start/toggle `VisualMode::Line` selection. Same toggle logic.
- `Ctrl+V`: start/toggle `VisualMode::Block` selection. Same toggle logic.

**Step 3: Update yank handler**

- `y`: call `collect_selected_text()` instead of `collect_lines_text()`. Pass the copy mode state so it knows the mode.

**Step 4: Update all `app.copy_mode.cursor` references**

Change bare `app.copy_mode.cursor` to `app.copy_mode.cursor.line` for scroll offset comparisons.

**Step 5: Verify**

Run: `cargo check -p ucode-tui 2>&1`
Expected: compilation errors in mouse handlers (still uses old API)

---

### Task 4: Update event_loop.rs mouse handlers

**Files:**
- Modify: `crates/ucode-tui/src/event_loop.rs:1295-1375` (mouse handler block)

**Step 1: Update MouseDown handler**

Compute both line and column from click position:
```rust
let relative_row = (me.row - app.transcript_area.y) as usize;
let col = (me.column - app.transcript_area.x) as usize;
let visual_line = (effective_offset + relative_row).min(total - 1);
app.copy_mode.enter(visual_line, col);
```

**Step 2: Update MouseDrag handler**

Compute line and column, start Char selection on drag, update cursor position:
```rust
let col = (me.column - app.transcript_area.x) as usize;
if !app.copy_mode.selecting {
    app.copy_mode.start_selecting_with_mode(VisualMode::Char);
}
app.copy_mode.cursor = Position { line: visual_line, col };
```

**Step 3: Update MouseUp handler**

Use `collect_selected_text()` for text extraction.

**Step 4: Verify**

Run: `cargo check -p ucode-tui 2>&1`
Expected: compilation errors in EnterSelectionMode/SetMark action handlers

---

### Task 5: Update dispatch_action handlers and AppState references

**Files:**
- Modify: `crates/ucode-tui/src/event_loop.rs:1680-1720` (EnterSelectionMode, SetMark, YankSelection handlers)
- Modify: `crates/ucode-tui/src/app.rs:241` (if `copy_mode` field type changed — it shouldn't, just the inner types)

**Step 1: Update EnterSelectionMode / SetMark**

Change `app.copy_mode.enter(cursor_line)` to `app.copy_mode.enter(cursor_line, 0)`.

**Step 2: Update YankSelection / CopySelection**

Use `collect_selected_text()` instead of `collect_lines_text()`.

**Step 3: Verify**

Run: `cargo check -p ucode-tui 2>&1`
Expected: PASS (all compilation errors resolved)

---

### Task 6: Update keybind overlay descriptions

**Files:**
- Modify: `crates/ucode-tui/src/overlays/keybind_overlay.rs`

**Step 1: Update action descriptions**

The keybind overlay shows help text. Update descriptions to mention the three modes:
- `EnterSelectionMode` → "Enter selection mode (v=char, V=line, ^V=block)"

**Step 2: Verify**

Run: `cargo check -p ucode-tui 2>&1`
Expected: PASS

---

### Task 7: Full test suite verification

**Step 1: Run all tests**

Run: `cargo test --workspace 2>&1`
Expected: 1592+ tests pass, 0 failures

**Step 2: Fix any test failures**

If tests fail due to API changes in copy_mode.rs, update the test call sites.

---

### Task 8: Demo help text update

**Files:**
- Modify: `crates/ucode-tui/examples/demo.rs`

**Step 1: Update help text**

Update the keyboard shortcuts table in the echo response to mention the three visual modes.

**Step 2: Verify**

Run: `cargo check -p ucode-tui --example demo 2>&1`
Expected: PASS
