# ucode-themes Crate Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Extract theme definitions into a standalone `ucode-themes` crate with built-in presets, per-theme syntax highlighting colors, and user override support via TOML.

**Architecture:** `ucode-themes` is a pure data crate (no ratatui, no syntect). It defines `ThemeDef` with UI colors + `SyntaxColors`. `ucode-tui` depends on it, converts `ThemeDef` → `UcodeTheme` (ratatui colors) and `SyntaxColors` → `syntect::highlighting::Theme`. Users override via `~/.config/ucode/themes/*.toml` with optional `base` inheritance.

**Tech Stack:** serde, toml, dirs (for config path)

---

## Data Types

### Rgb
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}
```

### SyntaxColors
```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyntaxColors {
    pub keyword: Rgb,
    pub string: Rgb,
    pub comment: Rgb,
    pub type_name: Rgb,
    pub function: Rgb,
    pub number: Rgb,
    pub operator: Rgb,
    pub variable: Rgb,
    pub constant: Rgb,
    pub attribute: Rgb,
    pub tag: Rgb,
    pub punctuation: Rgb,
}
```

### ThemeDef
```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeDef {
    pub name: String,
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
    pub syntax: SyntaxColors,
}
```

### UserThemeOverride (for TOML loading)
All fields optional, with optional `base` to inherit from a built-in:
```rust
#[derive(Debug, Clone, Deserialize)]
pub struct UserThemeOverride {
    pub name: String,
    pub base: Option<String>,
    // All color fields are Option<Rgb>
    pub background: Option<Rgb>,
    // ... etc
    pub syntax: Option<SyntaxColorsOverride>,
}
```

---

## Built-in Themes

6 themes, each with carefully chosen syntax colors:

| Name | Style | Accent | Syntax Base |
|------|-------|--------|-------------|
| `ucode` | Our default teal-on-dark | `#00d4aa` | Custom teal-based |
| `tokyonight` | Blue-purple dark | `#7aa2f7` | Tokyo Night colors |
| `catppuccin-mocha` | Warm pastel dark | `#cba6f7` | Catppuccin palette |
| `gruvbox-dark` | Retro warm dark | `#d79921` | Gruvbox palette |
| `nord` | Arctic cool dark | `#88c0d0` | Nord palette |
| `dracula` | Purple-pink dark | `#bd93f9` | Dracula palette |

---

## Task 1: Create ucode-themes crate scaffold

**Files:**
- Create: `crates/ucode-themes/Cargo.toml`
- Create: `crates/ucode-themes/src/lib.rs`
- Modify: `Cargo.toml` (workspace members + dependencies)

**Step 1:** Create crate directory and Cargo.toml

```toml
[package]
name = "ucode-themes"
version.workspace = true
edition.workspace = true

[dependencies]
serde = { workspace = true }
toml = { workspace = true }
```

**Step 2:** Add to workspace members and default-members in root `Cargo.toml`. Add `ucode-themes = { path = "crates/ucode-themes" }` to workspace dependencies. Add `toml` to workspace dependencies if not present.

**Step 3:** Write `lib.rs` with `Rgb`, `SyntaxColors`, `ThemeDef`, helper methods (`Rgb::new()`, `const fn rgb()`), and `impl ThemeDef` with `is_dark()` (luminance check on background).

**Step 4:** `cargo check -p ucode-themes`

**Step 5:** Commit: `feat(themes): scaffold ucode-themes crate with core types`

---

## Task 2: Add built-in theme definitions

**Files:**
- Create: `crates/ucode-themes/src/builtin.rs`
- Modify: `crates/ucode-themes/src/lib.rs` (add `mod builtin; pub use builtin::*;`)

**Step 1:** Define each theme as a `pub const fn` or `pub fn` returning `ThemeDef`. Use accurate color values sourced from each theme's official palette.

**Step 2:** Add `pub fn builtin_themes() -> Vec<ThemeDef>` and `pub fn builtin_theme(name: &str) -> Option<ThemeDef>`.

**Step 3:** Add `pub fn theme_names() -> Vec<&'static str>`.

**Step 4:** Write tests: each built-in theme exists, names are unique, `builtin_theme("opencode")` returns Some.

**Step 5:** `cargo test -p ucode-themes`

**Step 6:** Commit: `feat(themes): add 6 built-in theme definitions`

---

## Task 3: Add user theme loader

**Files:**
- Create: `crates/ucode-themes/src/loader.rs`
- Modify: `crates/ucode-themes/src/lib.rs`

**Step 1:** Define `UserThemeOverride` and `SyntaxColorsOverride` (all fields `Option`).

**Step 2:** Implement `UserThemeOverride::apply_to(base: &ThemeDef) -> ThemeDef` — merges non-None fields onto base.

**Step 3:** Implement `pub fn load_user_themes(config_dir: &Path) -> Vec<ThemeDef>`:
- Reads `config_dir/themes/*.toml`
- Parses each as `UserThemeOverride`
- If `base` is set, looks up built-in and applies overrides
- If no `base`, requires all fields (or error)

**Step 4:** Implement `pub fn resolve_theme(name: &str, config_dir: &Path) -> ThemeDef`:
- Check user themes first, then built-ins, fallback to "ucode"

**Step 5:** Write tests with tempdir: load a partial override TOML, verify merge.

**Step 6:** `cargo test -p ucode-themes`

**Step 7:** Commit: `feat(themes): add user theme loader with TOML override support`

---

## Task 4: Integrate ucode-themes into ucode-tui

**Files:**
- Modify: `crates/ucode-tui/Cargo.toml` (add ucode-themes dep)
- Modify: `crates/ucode-tui/src/theme.rs` (UcodeTheme wraps ThemeDef)
- Modify: all files that use ThemePreset

**Step 1:** Add `ucode-themes = { workspace = true }` to ucode-tui deps.

**Step 2:** Refactor `UcodeTheme`:
- Store `pub def: ThemeDef` inside UcodeTheme
- `UcodeTheme::from_def(def: ThemeDef) -> Self` converts Rgb → Color::Rgb
- Keep all existing style helper methods (accent_style, text_style, etc.)
- Keep Density, SandboxTier, ModelGroup as-is (they're TUI-specific)

**Step 3:** Replace `ThemePreset` enum:
- `ThemePreset` becomes a wrapper around theme name string
- `from_preset()` calls `ucode_themes::builtin_theme(name)` then `UcodeTheme::from_def()`
- Theme cycling iterates through `ucode_themes::theme_names()`

**Step 4:** Update all call sites. Most just use `UcodeTheme::default()` which stays the same.

**Step 5:** `cargo test --manifest-path crates/ucode-tui/Cargo.toml` — all 621+ tests pass.

**Step 6:** `cargo check` — full workspace clean.

**Step 7:** Commit: `refactor(tui): integrate ucode-themes, UcodeTheme wraps ThemeDef`

---

## Task 5: Build syntect Theme from SyntaxColors

**Files:**
- Modify: `crates/ucode-tui/src/components/markdown.rs`

**Step 1:** Remove the `THEME_SET` LazyLock (no longer needed).

**Step 2:** Add function `fn build_syntect_theme(syntax: &SyntaxColors, bg: &Rgb) -> syntect::highlighting::Theme`:
- Creates `ThemeSettings` with foreground/background from theme
- Creates `Vec<ThemeItem>` with scope selectors:
  - `keyword` → SyntaxColors.keyword
  - `string` → SyntaxColors.string
  - `comment` → SyntaxColors.comment
  - `entity.name.type, support.type` → SyntaxColors.type_name
  - `entity.name.function` → SyntaxColors.function
  - `constant.numeric` → SyntaxColors.number
  - `keyword.operator` → SyntaxColors.operator
  - `variable` → SyntaxColors.variable
  - `constant.language` → SyntaxColors.constant
  - `entity.other.attribute-name` → SyntaxColors.attribute
  - `entity.name.tag` → SyntaxColors.tag
  - `punctuation` → SyntaxColors.punctuation

**Step 3:** In the CodeBlock handler, replace `THEME_SET.themes.get(...)` with `build_syntect_theme(&self.theme.def.syntax, &self.theme.def.surface)`. Cache per-theme with a HashMap<String, syntect::highlighting::Theme> in a LazyLock if needed.

**Step 4:** `cargo test --manifest-path crates/ucode-tui/Cargo.toml`

**Step 5:** `cargo check`

**Step 6:** Commit: `feat(tui): build syntect theme from SyntaxColors, drop built-in theme dependency`

---

## Task 6: Update config template and docs

**Files:**
- Modify: `crates/ucode-agent/src/config.rs` (add theme field to config template)

**Step 1:** Add `theme = "ucode"` to DEFAULT_CONFIG_TEMPLATE with comment showing available themes.

**Step 2:** Commit: `docs: add theme config to default template`
