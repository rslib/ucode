use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// KeybindPreset
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum KeybindPreset {
    #[default]
    Vscode,
    Vim,
    Emacs,
}

// ---------------------------------------------------------------------------
// Action
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    OpenPalette,
    OpenSessionSwitcher,
    ToggleSidebar,
    SearchTranscript,
    ClearTranscript,
    CancelGeneration,
    Exit,
    Dismiss,
    AcceptAutocomplete,
    SendMessage,
    NewlineInInput,
    ScrollUp,
    ScrollDown,
    PageUp,
    PageDown,
    ScrollToTop,
    ScrollToBottom,
    HalfPageUp,
    HalfPageDown,
    GrowSidebar,
    ShrinkSidebar,
    EnterCopyMode,
    YankSelection,
    NextSearchMatch,
    PrevSearchMatch,
    ShowKeybindOverlay,
    ApproveAction,
    RejectAction,
    ShowDiff,
    EnterInsertMode,
    EnterNormalMode,
    ReverseSearch,
    SetMark,
    CopySelection,
    ToggleTheme,
    ToggleDensity,
}

// ---------------------------------------------------------------------------
// KeyCombo
// ---------------------------------------------------------------------------

/// A key combination suitable for use as a `HashMap` key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyCombo {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyCombo {
    pub fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        Self { code, modifiers }
    }
}

impl From<&KeyEvent> for KeyCombo {
    fn from(ev: &KeyEvent) -> Self {
        Self {
            code: ev.code,
            modifiers: ev.modifiers,
        }
    }
}

// ---------------------------------------------------------------------------
// KeybindMap
// ---------------------------------------------------------------------------

pub type KeybindMap = HashMap<KeyCombo, Action>;

// ---------------------------------------------------------------------------
// Preset constructors
// ---------------------------------------------------------------------------

pub fn default_vscode_bindings() -> KeybindMap {
    use Action as A;
    use KeyCode as K;
    use KeyModifiers as Mod;

    let mut m = KeybindMap::new();

    // Ctrl combos
    m.insert(KeyCombo::new(K::Char('p'), Mod::CONTROL), A::OpenPalette);
    m.insert(
        KeyCombo::new(K::Char('o'), Mod::CONTROL),
        A::OpenSessionSwitcher,
    );
    m.insert(KeyCombo::new(K::Char('e'), Mod::CONTROL), A::ToggleSidebar);
    m.insert(
        KeyCombo::new(K::Char('f'), Mod::CONTROL),
        A::SearchTranscript,
    );
    m.insert(
        KeyCombo::new(K::Char('l'), Mod::CONTROL),
        A::ClearTranscript,
    );
    m.insert(
        KeyCombo::new(K::Char('c'), Mod::CONTROL),
        A::CancelGeneration,
    );
    m.insert(KeyCombo::new(K::Char('q'), Mod::CONTROL), A::Exit);

    // Bare keys
    m.insert(KeyCombo::new(K::Esc, Mod::NONE), A::Dismiss);
    m.insert(KeyCombo::new(K::Tab, Mod::NONE), A::AcceptAutocomplete);
    m.insert(KeyCombo::new(K::Enter, Mod::NONE), A::SendMessage);
    m.insert(KeyCombo::new(K::Enter, Mod::SHIFT), A::NewlineInInput);

    // Scroll / navigation
    m.insert(KeyCombo::new(K::Up, Mod::NONE), A::ScrollUp);
    m.insert(KeyCombo::new(K::Down, Mod::NONE), A::ScrollDown);
    m.insert(KeyCombo::new(K::PageUp, Mod::NONE), A::PageUp);
    m.insert(KeyCombo::new(K::PageDown, Mod::NONE), A::PageDown);

    // Sidebar resize
    m.insert(KeyCombo::new(K::Char('['), Mod::NONE), A::ShrinkSidebar);
    m.insert(KeyCombo::new(K::Char(']'), Mod::NONE), A::GrowSidebar);

    // Transcript-focused actions (bare letters)
    m.insert(KeyCombo::new(K::Char('v'), Mod::NONE), A::EnterCopyMode);
    m.insert(KeyCombo::new(K::Char('y'), Mod::NONE), A::YankSelection);
    m.insert(KeyCombo::new(K::Char('n'), Mod::NONE), A::NextSearchMatch);
    m.insert(KeyCombo::new(K::Char('N'), Mod::SHIFT), A::PrevSearchMatch);
    m.insert(
        KeyCombo::new(K::Char('?'), Mod::NONE),
        A::ShowKeybindOverlay,
    );

    // Approval-focused actions
    m.insert(KeyCombo::new(K::Char('a'), Mod::NONE), A::ApproveAction);
    m.insert(KeyCombo::new(K::Char('r'), Mod::NONE), A::RejectAction);
    m.insert(KeyCombo::new(K::Char('d'), Mod::NONE), A::ShowDiff);

    // Theme / density toggles — function keys avoid all readline conflicts.
    m.insert(KeyCombo::new(K::F(6), Mod::NONE), A::ToggleTheme);
    m.insert(KeyCombo::new(K::F(7), Mod::NONE), A::ToggleDensity);

    m
}

pub fn default_vim_bindings() -> KeybindMap {
    use Action as A;
    use KeyCode as K;
    use KeyModifiers as Mod;

    let mut m = KeybindMap::new();

    // Mode transitions
    m.insert(KeyCombo::new(K::Esc, Mod::NONE), A::EnterNormalMode);
    m.insert(KeyCombo::new(K::Char('i'), Mod::NONE), A::EnterInsertMode);

    // Command palette — both vim-native and muscle-memory alias
    m.insert(KeyCombo::new(K::Char(':'), Mod::NONE), A::OpenPalette);
    m.insert(KeyCombo::new(K::Char('p'), Mod::CONTROL), A::OpenPalette);

    // Session / sidebar
    m.insert(
        KeyCombo::new(K::Char('o'), Mod::CONTROL),
        A::OpenSessionSwitcher,
    );
    m.insert(KeyCombo::new(K::Char('e'), Mod::CONTROL), A::ToggleSidebar);

    // Search
    m.insert(KeyCombo::new(K::Char('/'), Mod::NONE), A::SearchTranscript);
    m.insert(KeyCombo::new(K::Char('n'), Mod::NONE), A::NextSearchMatch);
    m.insert(KeyCombo::new(K::Char('N'), Mod::SHIFT), A::PrevSearchMatch);

    // Scroll — vim hjkl style
    m.insert(KeyCombo::new(K::Char('j'), Mod::NONE), A::ScrollDown);
    m.insert(KeyCombo::new(K::Char('k'), Mod::NONE), A::ScrollUp);

    // Jump to top/bottom — single-key approximations (gg requires multi-key)
    m.insert(KeyCombo::new(K::Home, Mod::CONTROL), A::ScrollToTop);
    m.insert(KeyCombo::new(K::End, Mod::NONE), A::ScrollToBottom);

    // Half-page scroll
    m.insert(KeyCombo::new(K::Char('u'), Mod::CONTROL), A::HalfPageUp);
    m.insert(KeyCombo::new(K::Char('d'), Mod::CONTROL), A::HalfPageDown);

    // Page scroll (also keep arrow keys)
    m.insert(KeyCombo::new(K::PageUp, Mod::NONE), A::PageUp);
    m.insert(KeyCombo::new(K::PageDown, Mod::NONE), A::PageDown);
    m.insert(KeyCombo::new(K::Up, Mod::NONE), A::ScrollUp);
    m.insert(KeyCombo::new(K::Down, Mod::NONE), A::ScrollDown);

    // Editing
    m.insert(KeyCombo::new(K::Enter, Mod::NONE), A::SendMessage);
    m.insert(KeyCombo::new(K::Enter, Mod::SHIFT), A::NewlineInInput);
    m.insert(KeyCombo::new(K::Tab, Mod::NONE), A::AcceptAutocomplete);
    m.insert(
        KeyCombo::new(K::Char('c'), Mod::CONTROL),
        A::CancelGeneration,
    );
    m.insert(
        KeyCombo::new(K::Char('l'), Mod::CONTROL),
        A::ClearTranscript,
    );

    // Exit — bare q (normal mode) and Ctrl+Q (universal)
    m.insert(KeyCombo::new(K::Char('q'), Mod::NONE), A::Exit);
    m.insert(KeyCombo::new(K::Char('q'), Mod::CONTROL), A::Exit);

    // Copy mode
    m.insert(KeyCombo::new(K::Char('v'), Mod::NONE), A::EnterCopyMode);
    m.insert(KeyCombo::new(K::Char('y'), Mod::NONE), A::YankSelection);

    // Sidebar resize
    m.insert(KeyCombo::new(K::Char('['), Mod::NONE), A::ShrinkSidebar);
    m.insert(KeyCombo::new(K::Char(']'), Mod::NONE), A::GrowSidebar);

    // Approval
    m.insert(KeyCombo::new(K::Char('a'), Mod::NONE), A::ApproveAction);
    m.insert(KeyCombo::new(K::Char('r'), Mod::NONE), A::RejectAction);

    // Help
    m.insert(
        KeyCombo::new(K::Char('?'), Mod::NONE),
        A::ShowKeybindOverlay,
    );

    // Theme / density toggles
    m.insert(KeyCombo::new(K::F(6), Mod::NONE), A::ToggleTheme);
    m.insert(KeyCombo::new(K::F(7), Mod::NONE), A::ToggleDensity);

    m
}

pub fn default_emacs_bindings() -> KeybindMap {
    use Action as A;
    use KeyCode as K;
    use KeyModifiers as Mod;

    let mut m = KeybindMap::new();

    // Command palette — Meta+x (Alt+x) and Ctrl+P alias
    m.insert(KeyCombo::new(K::Char('x'), Mod::ALT), A::OpenPalette);
    m.insert(KeyCombo::new(K::Char('p'), Mod::CONTROL), A::OpenPalette);

    // Session / sidebar
    m.insert(
        KeyCombo::new(K::Char('o'), Mod::CONTROL),
        A::OpenSessionSwitcher,
    );

    // Cancel / dismiss — Ctrl+G (emacs) and Esc alias
    m.insert(KeyCombo::new(K::Char('g'), Mod::CONTROL), A::Dismiss);
    m.insert(KeyCombo::new(K::Esc, Mod::NONE), A::Dismiss);

    // Search
    m.insert(
        KeyCombo::new(K::Char('s'), Mod::CONTROL),
        A::SearchTranscript,
    );
    m.insert(KeyCombo::new(K::Char('r'), Mod::CONTROL), A::ReverseSearch);

    // Transcript navigation — Ctrl+N / Ctrl+P
    m.insert(KeyCombo::new(K::Char('n'), Mod::CONTROL), A::ScrollDown);
    m.insert(KeyCombo::new(K::Char('p'), Mod::CONTROL), A::ScrollUp);

    // Page scroll — Ctrl+V / Meta+V
    m.insert(KeyCombo::new(K::Char('v'), Mod::CONTROL), A::PageDown);
    m.insert(KeyCombo::new(K::Char('v'), Mod::ALT), A::PageUp);

    // Also keep arrow keys and PgUp/PgDn
    m.insert(KeyCombo::new(K::Up, Mod::NONE), A::ScrollUp);
    m.insert(KeyCombo::new(K::Down, Mod::NONE), A::ScrollDown);
    m.insert(KeyCombo::new(K::PageUp, Mod::NONE), A::PageUp);
    m.insert(KeyCombo::new(K::PageDown, Mod::NONE), A::PageDown);

    // Input line movement — Ctrl+A / Ctrl+E
    m.insert(KeyCombo::new(K::Char('a'), Mod::CONTROL), A::ScrollToTop);
    m.insert(KeyCombo::new(K::Char('e'), Mod::CONTROL), A::ScrollToBottom);

    // Editing
    m.insert(KeyCombo::new(K::Enter, Mod::NONE), A::SendMessage);
    m.insert(KeyCombo::new(K::Enter, Mod::SHIFT), A::NewlineInInput);
    m.insert(KeyCombo::new(K::Tab, Mod::NONE), A::AcceptAutocomplete);
    m.insert(
        KeyCombo::new(K::Char('c'), Mod::CONTROL),
        A::CancelGeneration,
    );
    m.insert(
        KeyCombo::new(K::Char('l'), Mod::CONTROL),
        A::ClearTranscript,
    );

    // Exit
    m.insert(KeyCombo::new(K::Char('q'), Mod::CONTROL), A::Exit);

    // Selection / copy — emacs mark-and-copy
    m.insert(KeyCombo::new(K::Char(' '), Mod::CONTROL), A::SetMark);
    m.insert(KeyCombo::new(K::Char('w'), Mod::ALT), A::CopySelection);

    // Sidebar resize
    m.insert(KeyCombo::new(K::Char('['), Mod::NONE), A::ShrinkSidebar);
    m.insert(KeyCombo::new(K::Char(']'), Mod::NONE), A::GrowSidebar);

    // Approval
    m.insert(KeyCombo::new(K::Char('a'), Mod::NONE), A::ApproveAction);
    m.insert(KeyCombo::new(K::Char('r'), Mod::NONE), A::RejectAction);

    // Help
    m.insert(
        KeyCombo::new(K::Char('?'), Mod::NONE),
        A::ShowKeybindOverlay,
    );

    // Theme / density toggles
    m.insert(KeyCombo::new(K::F(6), Mod::NONE), A::ToggleTheme);
    m.insert(KeyCombo::new(K::F(7), Mod::NONE), A::ToggleDensity);

    m
}

pub fn bindings_for_preset(preset: KeybindPreset) -> KeybindMap {
    match preset {
        KeybindPreset::Vscode => default_vscode_bindings(),
        KeybindPreset::Vim => default_vim_bindings(),
        KeybindPreset::Emacs => default_emacs_bindings(),
    }
}

// ---------------------------------------------------------------------------
// InputMode
// ---------------------------------------------------------------------------

/// Vim modal editing state. Only meaningful when `preset == Vim`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InputMode {
    #[default]
    Normal,
    Insert,
}

// ---------------------------------------------------------------------------
// KeybindResolver
// ---------------------------------------------------------------------------

/// Resolves raw crossterm key events to [`Action`]s for the active preset.
#[derive(Debug, Clone)]
pub struct KeybindResolver {
    bindings: KeybindMap,
    pub preset: KeybindPreset,
    pub mode: InputMode,
}

impl KeybindResolver {
    pub fn new(preset: KeybindPreset) -> Self {
        Self {
            bindings: bindings_for_preset(preset),
            preset,
            mode: InputMode::default(),
        }
    }

    /// Resolve a key event to an action.
    ///
    /// Returns `None` for non-press events or unbound keys.
    /// For the vim preset, keys that are only meaningful in Normal mode
    /// (navigation, command entry) are suppressed while in Insert mode,
    /// and `EnterInsertMode` is suppressed while already in Insert mode.
    pub fn resolve(&self, key: &KeyEvent) -> Option<Action> {
        // Only act on key presses (not releases or repeats).
        if key.kind != KeyEventKind::Press {
            return None;
        }

        let action = self.bindings.get(&KeyCombo::from(key)).copied()?;

        if self.preset == KeybindPreset::Vim {
            match self.mode {
                InputMode::Insert => {
                    // In insert mode, suppress normal-mode-only navigation actions.
                    // The user must press Esc (→ EnterNormalMode) to leave insert mode.
                    match action {
                        Action::ScrollDown
                        | Action::ScrollUp
                        | Action::ScrollToTop
                        | Action::ScrollToBottom
                        | Action::HalfPageUp
                        | Action::HalfPageDown
                        | Action::OpenPalette
                        | Action::SearchTranscript
                        | Action::EnterCopyMode
                        | Action::EnterInsertMode => return None,
                        _ => {}
                    }
                }
                InputMode::Normal => {
                    // In normal mode, suppress insert-mode-only actions.
                    // SendMessage and NewlineInInput require insert mode.
                    match action {
                        Action::SendMessage | Action::NewlineInInput => return None,
                        _ => {}
                    }
                }
            }
        }

        Some(action)
    }

    pub fn set_mode(&mut self, mode: InputMode) {
        self.mode = mode;
    }

    /// Override a single keybinding. The new binding replaces any existing
    /// binding for the same key combo. This allows individual customization
    /// on top of the active preset.
    ///
    /// To remove a binding, use `remove_binding`.
    pub fn override_binding(&mut self, combo: KeyCombo, action: Action) {
        self.bindings.insert(combo, action);
    }

    /// Remove a keybinding. Returns the previously bound action, if any.
    pub fn remove_binding(&mut self, combo: &KeyCombo) -> Option<Action> {
        self.bindings.remove(combo)
    }

    /// Iterate over all (KeyCombo, Action) pairs in the active binding map.
    pub fn bindings(&self) -> impl Iterator<Item = (&KeyCombo, &Action)> {
        self.bindings.iter()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn press(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn release(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Release,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn vscode_palette_binding() {
        let bindings = default_vscode_bindings();
        let combo = KeyCombo::new(KeyCode::Char('p'), KeyModifiers::CONTROL);
        assert_eq!(bindings.get(&combo).copied(), Some(Action::OpenPalette));
    }

    #[test]
    fn vim_palette_binding() {
        // `:` maps to OpenPalette in vim preset (normal mode)
        let resolver = KeybindResolver::new(KeybindPreset::Vim);
        assert_eq!(resolver.mode, InputMode::Normal);
        let ev = press(KeyCode::Char(':'), KeyModifiers::NONE);
        assert_eq!(resolver.resolve(&ev), Some(Action::OpenPalette));
    }

    #[test]
    fn emacs_palette_binding() {
        // Alt+x maps to OpenPalette in emacs preset
        let bindings = default_emacs_bindings();
        let combo = KeyCombo::new(KeyCode::Char('x'), KeyModifiers::ALT);
        assert_eq!(bindings.get(&combo).copied(), Some(Action::OpenPalette));
    }

    #[test]
    fn vscode_is_default_preset() {
        assert_eq!(KeybindPreset::default(), KeybindPreset::Vscode);
    }

    #[test]
    fn resolver_filters_key_release() {
        let resolver = KeybindResolver::new(KeybindPreset::Vscode);
        let ev = release(KeyCode::Char('p'), KeyModifiers::CONTROL);
        assert_eq!(resolver.resolve(&ev), None);
    }

    #[test]
    fn vim_j_scrolls_down() {
        let resolver = KeybindResolver::new(KeybindPreset::Vim);
        // Default mode is Normal — j should scroll down.
        let ev = press(KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(resolver.resolve(&ev), Some(Action::ScrollDown));
    }

    #[test]
    fn emacs_ctrl_n_scrolls_down() {
        let resolver = KeybindResolver::new(KeybindPreset::Emacs);
        let ev = press(KeyCode::Char('n'), KeyModifiers::CONTROL);
        assert_eq!(resolver.resolve(&ev), Some(Action::ScrollDown));
    }

    #[test]
    fn all_presets_have_exit() {
        for preset in [
            KeybindPreset::Vscode,
            KeybindPreset::Vim,
            KeybindPreset::Emacs,
        ] {
            let bindings = bindings_for_preset(preset);
            let has_exit = bindings.values().any(|&a| a == Action::Exit);
            assert!(has_exit, "{preset:?} has no Exit binding");
        }
    }

    #[test]
    fn all_presets_have_ctrl_q_exit() {
        // Ctrl+Q is the universal exit binding across all presets.
        let ctrl_q = KeyCombo::new(KeyCode::Char('q'), KeyModifiers::CONTROL);
        for preset in [
            KeybindPreset::Vscode,
            KeybindPreset::Vim,
            KeybindPreset::Emacs,
        ] {
            let bindings = bindings_for_preset(preset);
            assert_eq!(
                bindings.get(&ctrl_q).copied(),
                Some(Action::Exit),
                "{preset:?} does not map Ctrl+Q to Exit"
            );
        }
    }

    #[test]
    fn vim_has_both_q_and_ctrl_q_exit() {
        let bindings = default_vim_bindings();
        let bare_q = KeyCombo::new(KeyCode::Char('q'), KeyModifiers::NONE);
        let ctrl_q = KeyCombo::new(KeyCode::Char('q'), KeyModifiers::CONTROL);
        assert_eq!(bindings.get(&bare_q).copied(), Some(Action::Exit));
        assert_eq!(bindings.get(&ctrl_q).copied(), Some(Action::Exit));
    }

    #[test]
    fn vscode_ctrl_d_no_longer_exits() {
        // Ctrl+D was removed from vscode; it now falls through to readline delete-forward.
        let bindings = default_vscode_bindings();
        let ctrl_d = KeyCombo::new(KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert_ne!(bindings.get(&ctrl_d).copied(), Some(Action::Exit));
    }

    #[test]
    fn emacs_ctrl_d_no_longer_exits() {
        // Ctrl+D was removed from emacs; it now falls through to readline delete-forward.
        let bindings = default_emacs_bindings();
        let ctrl_d = KeyCombo::new(KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert_ne!(bindings.get(&ctrl_d).copied(), Some(Action::Exit));
    }

    #[test]
    fn all_presets_have_send_message() {
        // Every preset must bind Enter (no modifiers) to SendMessage.
        // For vim, SendMessage is suppressed in Normal mode but the binding
        // still exists in the map — we test the map directly here.
        for preset in [
            KeybindPreset::Vscode,
            KeybindPreset::Vim,
            KeybindPreset::Emacs,
        ] {
            let bindings = bindings_for_preset(preset);
            let combo = KeyCombo::new(KeyCode::Enter, KeyModifiers::NONE);
            assert_eq!(
                bindings.get(&combo).copied(),
                Some(Action::SendMessage),
                "{preset:?} does not map Enter to SendMessage"
            );
        }
    }

    #[test]
    fn all_presets_have_toggle_theme() {
        for preset in [
            KeybindPreset::Vscode,
            KeybindPreset::Vim,
            KeybindPreset::Emacs,
        ] {
            let map = bindings_for_preset(preset);
            assert!(
                map.values().any(|a| *a == Action::ToggleTheme),
                "{preset:?} missing ToggleTheme"
            );
        }
    }

    #[test]
    fn all_presets_have_toggle_density() {
        for preset in [
            KeybindPreset::Vscode,
            KeybindPreset::Vim,
            KeybindPreset::Emacs,
        ] {
            let map = bindings_for_preset(preset);
            assert!(
                map.values().any(|a| *a == Action::ToggleDensity),
                "{preset:?} missing ToggleDensity"
            );
        }
    }

    #[test]
    fn override_binding_replaces_existing() {
        let mut resolver = KeybindResolver::new(KeybindPreset::Vscode);
        // Ctrl+P is OpenPalette by default in vscode preset
        let combo = KeyCombo::new(KeyCode::Char('p'), KeyModifiers::CONTROL);
        let key = KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL);
        assert_eq!(resolver.resolve(&key), Some(Action::OpenPalette));

        resolver.override_binding(combo, Action::SearchTranscript);
        assert_eq!(resolver.resolve(&key), Some(Action::SearchTranscript));
    }

    #[test]
    fn override_binding_adds_new() {
        let mut resolver = KeybindResolver::new(KeybindPreset::Vscode);
        // Ctrl+T is not bound by default
        let combo = KeyCombo::new(KeyCode::Char('t'), KeyModifiers::CONTROL);
        let key = KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL);
        assert_eq!(resolver.resolve(&key), None);

        resolver.override_binding(combo, Action::ToggleTheme);
        assert_eq!(resolver.resolve(&key), Some(Action::ToggleTheme));
    }

    #[test]
    fn remove_binding_removes_existing() {
        let mut resolver = KeybindResolver::new(KeybindPreset::Vscode);
        let combo = KeyCombo::new(KeyCode::Char('p'), KeyModifiers::CONTROL);
        let key = KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL);
        assert_eq!(resolver.resolve(&key), Some(Action::OpenPalette));

        let removed = resolver.remove_binding(&combo);
        assert_eq!(removed, Some(Action::OpenPalette));
        assert_eq!(resolver.resolve(&key), None);
    }

    #[test]
    fn remove_binding_returns_none_for_unbound() {
        let mut resolver = KeybindResolver::new(KeybindPreset::Vscode);
        let combo = KeyCombo::new(KeyCode::Char('t'), KeyModifiers::CONTROL);
        assert_eq!(resolver.remove_binding(&combo), None);
    }

    #[test]
    fn preset_serde_roundtrip() {
        for (preset, expected_str) in [
            (KeybindPreset::Vscode, "\"vscode\""),
            (KeybindPreset::Vim, "\"vim\""),
            (KeybindPreset::Emacs, "\"emacs\""),
        ] {
            let serialized = serde_json::to_string(&preset).expect("serialize");
            assert_eq!(serialized, expected_str);
            let deserialized: KeybindPreset =
                serde_json::from_str(&serialized).expect("deserialize");
            assert_eq!(deserialized, preset);
        }
    }
}
