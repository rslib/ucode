# `/connect` UI Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** In-TUI provider connect flow for API keys, browser OAuth, and device code login.

**Architecture:** New `connect_modal.rs` overlay following the existing `PaletteState`/`ApprovalModalState` patterns. Multi-phase state machine (ProviderList → MethodPicker → AuthFlow → Verifying). Async auth flows spawned via `tokio::spawn`, results sent back through `TuiEvent` channel. Cancellation via `JoinHandle::abort()`.

**Tech Stack:** ratatui, crossterm, tokio, ucode-auth, ucode-core

**Design doc:** `docs/plans/2026-03-07-connect-ui-design.md`

---

### Task 1: Add ucode-auth dependency + ConnectModalState data model

**Files:**
- Modify: `crates/ucode-tui/Cargo.toml`
- Create: `crates/ucode-tui/src/overlays/connect_modal.rs`
- Modify: `crates/ucode-tui/src/overlays/mod.rs`

**Step 1: Add ucode-auth dependency**

Run: `cargo add ucode-auth --path ../ucode-auth -p ucode-tui`

**Step 2: Create connect_modal.rs with data types and state**

```rust
use ucode_auth::{AuthMethod, CredentialStatus, provider_auth_info};

/// Which section a provider belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectSection {
    QuickConnect,
    ApiKey,
}

/// Auth status for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderStatus {
    Connected { kind: String },
    NotConfigured,
}

/// A provider entry in the connect list.
#[derive(Debug, Clone)]
pub struct ConnectProvider {
    pub id: String,
    pub display_name: String,
    pub section: ConnectSection,
    pub status: ProviderStatus,
    pub env_vars: Vec<String>,
    pub has_browser_oauth: bool,
    pub has_device_code: bool,
    pub has_api_key: bool,
}

/// Current phase of the connect modal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectPhase {
    ProviderList,
    MethodPicker {
        provider_id: String,
        methods: Vec<String>,
        selected: usize,
    },
    ApiKeyEntry {
        provider_id: String,
        display_name: String,
        input: String,
        cursor: usize,
        env_hint: String,
    },
    BrowserOAuth {
        provider_id: String,
        display_name: String,
        url: Option<String>,
    },
    DeviceCode {
        provider_id: String,
        display_name: String,
        user_code: String,
        verification_uri: String,
    },
    Verifying {
        provider_id: String,
        display_name: String,
    },
}

/// State for the /connect modal overlay.
#[derive(Debug, Clone)]
pub struct ConnectModalState {
    pub visible: bool,
    pub phase: ConnectPhase,
    pub providers: Vec<ConnectProvider>,
    pub filter: String,
    pub filter_cursor: usize,
    pub filtered_indices: Vec<usize>,
    pub selected: usize,
}

/// Known providers in display order. Ollama excluded (no auth).
const QUICK_CONNECT_IDS: &[&str] = &["anthropic", "openai", "github-copilot", "gemini"];
const API_KEY_IDS: &[&str] = &[
    "groq", "deepseek", "openrouter", "together", "fireworks",
    "mistral", "azure-openai", "aws-bedrock", "vertex-ai",
];

impl ConnectModalState {
    pub fn new() -> Self {
        Self {
            visible: false,
            phase: ConnectPhase::ProviderList,
            providers: Vec::new(),
            filter: String::new(),
            filter_cursor: 0,
            filtered_indices: Vec::new(),
            selected: 0,
        }
    }

    /// Open the modal, refreshing provider status from the credential store.
    pub fn open(&mut self, statuses: &[(String, CredentialStatus)]) {
        self.visible = true;
        self.phase = ConnectPhase::ProviderList;
        self.filter.clear();
        self.filter_cursor = 0;
        self.selected = 0;
        self.providers = build_provider_list(statuses);
        self.update_filter();
    }

    pub fn close(&mut self) {
        self.visible = false;
    }

    pub fn insert_char(&mut self, c: char) {
        self.filter.insert(self.filter_cursor, c);
        self.filter_cursor += c.len_utf8();
        self.update_filter();
    }

    pub fn delete_char(&mut self) {
        if self.filter_cursor == 0 {
            return;
        }
        let before = &self.filter[..self.filter_cursor];
        let char_start = before
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.filter.drain(char_start..self.filter_cursor);
        self.filter_cursor = char_start;
        self.update_filter();
    }

    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn move_down(&mut self) {
        if self.filtered_indices.is_empty() {
            return;
        }
        let max = self.filtered_indices.len() - 1;
        if self.selected < max {
            self.selected += 1;
        }
    }

    pub fn selected_provider(&self) -> Option<&ConnectProvider> {
        let idx = *self.filtered_indices.get(self.selected)?;
        self.providers.get(idx)
    }

    fn update_filter(&mut self) {
        if self.filter.is_empty() {
            self.filtered_indices = (0..self.providers.len()).collect();
        } else {
            let needle = self.filter.to_lowercase();
            self.filtered_indices = self
                .providers
                .iter()
                .enumerate()
                .filter(|(_, p)| {
                    p.display_name.to_lowercase().contains(&needle)
                        || p.id.to_lowercase().contains(&needle)
                })
                .map(|(i, _)| i)
                .collect();
        }
        if self.filtered_indices.is_empty() {
            self.selected = 0;
        } else {
            let max = self.filtered_indices.len() - 1;
            if self.selected > max {
                self.selected = max;
            }
        }
    }
}

fn build_provider_list(statuses: &[(String, CredentialStatus)]) -> Vec<ConnectProvider> {
    let mut providers = Vec::new();

    let all_ids: Vec<(&str, ConnectSection)> = QUICK_CONNECT_IDS
        .iter()
        .map(|id| (*id, ConnectSection::QuickConnect))
        .chain(API_KEY_IDS.iter().map(|id| (*id, ConnectSection::ApiKey)))
        .collect();

    for (id, section) in all_ids {
        let Some(info) = provider_auth_info(id) else {
            continue;
        };

        let status = statuses
            .iter()
            .find(|(pid, _)| pid == id)
            .map(|(_, s)| match s {
                CredentialStatus::Configured { kind, .. } => ProviderStatus::Connected {
                    kind: kind.clone(),
                },
                CredentialStatus::NotConfigured { .. } => ProviderStatus::NotConfigured,
            })
            .unwrap_or(ProviderStatus::NotConfigured);

        let has_browser_oauth = info
            .auth_methods
            .iter()
            .any(|m| matches!(m, AuthMethod::BrowserOAuth));
        let has_device_code = info
            .auth_methods
            .iter()
            .any(|m| matches!(m, AuthMethod::DeviceCode));
        let has_api_key = info
            .auth_methods
            .iter()
            .any(|m| matches!(m, AuthMethod::ApiKey));

        providers.push(ConnectProvider {
            id: id.to_owned(),
            display_name: info.display_name.to_owned(),
            section,
            status,
            env_vars: info.env_vars.iter().map(|s| (*s).to_owned()).collect(),
            has_browser_oauth,
            has_device_code,
            has_api_key,
        });
    }

    providers
}
```

**Step 3: Register module in overlays/mod.rs**

Add `pub mod connect_modal;` to `crates/ucode-tui/src/overlays/mod.rs`.

**Step 4: Add tests**

Add to the bottom of `connect_modal.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ucode_auth::CredentialStatus;

    fn empty_statuses() -> Vec<(String, CredentialStatus)> {
        Vec::new()
    }

    fn with_anthropic_configured() -> Vec<(String, CredentialStatus)> {
        vec![(
            "anthropic".to_owned(),
            CredentialStatus::Configured {
                provider: "anthropic".to_owned(),
                kind: "oauth".to_owned(),
            },
        )]
    }

    #[test]
    fn test_new_is_not_visible() {
        let state = ConnectModalState::new();
        assert!(!state.visible);
    }

    #[test]
    fn test_open_sets_visible_and_populates_providers() {
        let mut state = ConnectModalState::new();
        state.open(&empty_statuses());
        assert!(state.visible);
        assert!(!state.providers.is_empty());
        assert_eq!(state.phase, ConnectPhase::ProviderList);
    }

    #[test]
    fn test_provider_sections() {
        let mut state = ConnectModalState::new();
        state.open(&empty_statuses());
        let quick: Vec<_> = state
            .providers
            .iter()
            .filter(|p| p.section == ConnectSection::QuickConnect)
            .collect();
        let api_key: Vec<_> = state
            .providers
            .iter()
            .filter(|p| p.section == ConnectSection::ApiKey)
            .collect();
        assert_eq!(quick.len(), 4); // anthropic, openai, github-copilot, gemini
        assert_eq!(api_key.len(), 9);
    }

    #[test]
    fn test_status_badge_connected() {
        let mut state = ConnectModalState::new();
        state.open(&with_anthropic_configured());
        let anthropic = state.providers.iter().find(|p| p.id == "anthropic").unwrap();
        assert!(matches!(anthropic.status, ProviderStatus::Connected { .. }));
    }

    #[test]
    fn test_status_badge_not_configured() {
        let mut state = ConnectModalState::new();
        state.open(&empty_statuses());
        let openai = state.providers.iter().find(|p| p.id == "openai").unwrap();
        assert_eq!(openai.status, ProviderStatus::NotConfigured);
    }

    #[test]
    fn test_filter_narrows_list() {
        let mut state = ConnectModalState::new();
        state.open(&empty_statuses());
        let total = state.filtered_indices.len();
        state.insert_char('a');
        state.insert_char('n');
        state.insert_char('t');
        // "ant" should match "Anthropic"
        assert!(state.filtered_indices.len() < total);
        assert!(state.filtered_indices.len() >= 1);
    }

    #[test]
    fn test_navigate_up_down() {
        let mut state = ConnectModalState::new();
        state.open(&empty_statuses());
        assert_eq!(state.selected, 0);
        state.move_down();
        assert_eq!(state.selected, 1);
        state.move_up();
        assert_eq!(state.selected, 0);
        state.move_up(); // should not go below 0
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn test_selected_provider() {
        let mut state = ConnectModalState::new();
        state.open(&empty_statuses());
        let first = state.selected_provider().unwrap();
        assert_eq!(first.id, "anthropic"); // first quick connect
    }

    #[test]
    fn test_close() {
        let mut state = ConnectModalState::new();
        state.open(&empty_statuses());
        assert!(state.visible);
        state.close();
        assert!(!state.visible);
    }

    #[test]
    fn test_auth_methods_populated() {
        let mut state = ConnectModalState::new();
        state.open(&empty_statuses());
        let anthropic = state.providers.iter().find(|p| p.id == "anthropic").unwrap();
        assert!(anthropic.has_browser_oauth);
        assert!(anthropic.has_api_key);
        assert!(!anthropic.has_device_code);

        let copilot = state
            .providers
            .iter()
            .find(|p| p.id == "github-copilot")
            .unwrap();
        assert!(copilot.has_device_code);
        assert!(!copilot.has_api_key);
    }
}
```

**Step 5: Verify**

Run: `cargo build -p ucode-tui && cargo test -p ucode-tui`
Expected: Build succeeds, all tests pass.

**Step 6: Commit**

```
feat(tui): add ConnectModalState data model for /connect UI

Provider list with two sections (Quick Connect + API Key), status
badges, filter logic, and navigation. 10 unit tests.
```

---

### Task 2: Wire Action::OpenConnect + command registry + AppState

**Files:**
- Modify: `crates/ucode-tui/src/keybinds.rs` — add `Action::OpenConnect`
- Modify: `crates/ucode-tui/src/command_registry.rs` — wire action on `/connect`
- Modify: `crates/ucode-tui/src/app.rs` — add `connect_modal` field, handle action

**Step 1: Add Action::OpenConnect to keybinds.rs**

Add `OpenConnect,` to the `Action` enum (after `ToggleDensity`).

**Step 2: Wire action in command_registry.rs**

Change the `/connect` entry from:
```rust
("/connect", "Connect provider or auth method"),
```

The `/connect` command is registered in a loop that sets `action: None`. Move `/connect` out of that loop and register it separately with `action: Some(Action::OpenConnect)`:

Before the `tools` loop, add:
```rust
reg.commands.push(CommandDef {
    name: "/connect".to_owned(),
    description: "Connect provider or auth method".to_owned(),
    category: CommandCategory::Tools,
    source: CommandSource::Builtin,
    args_hint: None,
    action: Some(Action::OpenConnect),
});
```

Remove `("/connect", "Connect provider or auth method"),` from the `tools` array.

**Step 3: Add connect_modal to AppState**

In `app.rs`, add to the `AppState` struct:
```rust
pub connect_modal: ConnectModalState,
```

In `AppState::new()`, add:
```rust
connect_modal: ConnectModalState::new(),
```

Add the import:
```rust
use crate::overlays::connect_modal::ConnectModalState;
```

**Step 4: Handle Action::OpenConnect in execute_command**

In `app.rs`, modify `execute_command` to handle the action. Currently the method checks `has_action` but doesn't actually dispatch. Find where `has_action` is checked and add dispatch logic.

Actually, looking at the code more carefully, `execute_command` just prints "Executed: {bare}" when `has_action` is true. The actual action dispatch happens in `dispatch_action` in `event_loop.rs`. So we need to handle it there.

In `event_loop.rs`, in the `dispatch_action` function, add:
```rust
Action::OpenConnect => {
    app.connect_modal.open(&[]); // empty statuses for now
    app.focus = FocusTarget::Overlay;
    app.mark_dirty();
}
```

**Step 5: Update the test for execute_command**

The existing test `test_execute_command_known` expects "not yet implemented" message. Now that `/connect` has an action, it should print "Executed: connect" instead. Update the test assertion if needed.

**Step 6: Verify**

Run: `cargo build -p ucode-tui && cargo test -p ucode-tui`
Expected: Build succeeds, all tests pass.

**Step 7: Commit**

```
feat(tui): wire Action::OpenConnect and /connect command dispatch

Add OpenConnect action variant, register on /connect command, add
connect_modal field to AppState, dispatch in event loop.
```

---

### Task 3: Event loop routing for connect modal key events

**Files:**
- Modify: `crates/ucode-tui/src/event_loop.rs`

**Step 1: Add connect modal key routing**

In `handle_terminal_event`, add a block for the connect modal BEFORE the palette block (since connect modal should capture keys when visible). Follow the exact pattern of the palette block:

```rust
// Connect modal routing
if app.connect_modal.visible {
    match app.connect_modal.phase {
        ConnectPhase::ProviderList => match key.code {
            crossterm::event::KeyCode::Esc => {
                app.connect_modal.close();
                app.focus = FocusTarget::Input;
                app.mark_dirty();
            }
            crossterm::event::KeyCode::Enter => {
                if let Some(provider) = app.connect_modal.selected_provider().cloned() {
                    app.connect_modal.select_provider(&provider);
                    app.mark_dirty();
                }
            }
            crossterm::event::KeyCode::Up => {
                app.connect_modal.move_up();
                app.mark_dirty();
            }
            crossterm::event::KeyCode::Down => {
                app.connect_modal.move_down();
                app.mark_dirty();
            }
            crossterm::event::KeyCode::Backspace => {
                app.connect_modal.delete_char();
                app.mark_dirty();
            }
            crossterm::event::KeyCode::Char(c) => {
                app.connect_modal.insert_char(c);
                app.mark_dirty();
            }
            _ => {}
        },
        ConnectPhase::MethodPicker { .. } => match key.code {
            crossterm::event::KeyCode::Esc => {
                app.connect_modal.phase = ConnectPhase::ProviderList;
                app.mark_dirty();
            }
            crossterm::event::KeyCode::Enter => {
                app.connect_modal.select_method();
                app.mark_dirty();
            }
            crossterm::event::KeyCode::Up => {
                app.connect_modal.method_up();
                app.mark_dirty();
            }
            crossterm::event::KeyCode::Down => {
                app.connect_modal.method_down();
                app.mark_dirty();
            }
            _ => {}
        },
        ConnectPhase::ApiKeyEntry { .. } => match key.code {
            crossterm::event::KeyCode::Esc => {
                app.connect_modal.phase = ConnectPhase::ProviderList;
                app.mark_dirty();
            }
            crossterm::event::KeyCode::Enter => {
                // Will be handled in Task 6
            }
            crossterm::event::KeyCode::Backspace => {
                app.connect_modal.api_key_delete_char();
                app.mark_dirty();
            }
            crossterm::event::KeyCode::Char(c) => {
                app.connect_modal.api_key_insert_char(c);
                app.mark_dirty();
            }
            _ => {}
        },
        ConnectPhase::BrowserOAuth { .. } | ConnectPhase::DeviceCode { .. } => {
            if key.code == crossterm::event::KeyCode::Esc {
                // Cancel async auth — will be handled in Task 7/8
                app.connect_modal.phase = ConnectPhase::ProviderList;
                app.mark_dirty();
            }
        }
        ConnectPhase::Verifying { .. } => {
            // No key handling during verification
        }
    }
    return false;
}
```

Add the necessary import at the top of event_loop.rs:
```rust
use crate::overlays::connect_modal::ConnectPhase;
```

**Step 2: Add the phase transition methods to ConnectModalState**

In `connect_modal.rs`, add:

```rust
/// Called when user presses Enter on a provider in the list.
pub fn select_provider(&mut self, provider: &ConnectProvider) {
    let mut methods = Vec::new();
    if provider.has_browser_oauth {
        // Anthropic has two OAuth configs (Max + Console)
        if provider.id == "anthropic" {
            methods.push("Browser login (Max)".to_owned());
            methods.push("Browser login (Console)".to_owned());
        } else {
            methods.push("Browser login".to_owned());
        }
    }
    if provider.has_device_code {
        methods.push("Device code".to_owned());
    }
    if provider.has_api_key {
        methods.push("API key".to_owned());
    }

    if methods.len() == 1 {
        // Skip method picker, go directly to the flow
        self.start_auth_flow(&provider.id, &provider.display_name, &methods[0], &provider.env_vars);
    } else {
        self.phase = ConnectPhase::MethodPicker {
            provider_id: provider.id.clone(),
            methods,
            selected: 0,
        };
    }
}

/// Called when user presses Enter in the method picker.
pub fn select_method(&mut self) {
    let (provider_id, method_label) = match &self.phase {
        ConnectPhase::MethodPicker {
            provider_id,
            methods,
            selected,
        } => {
            let method = methods.get(*selected).cloned().unwrap_or_default();
            (provider_id.clone(), method)
        }
        _ => return,
    };

    let provider = self.providers.iter().find(|p| p.id == provider_id).cloned();
    if let Some(provider) = provider {
        self.start_auth_flow(&provider.id, &provider.display_name, &method_label, &provider.env_vars);
    }
}

fn start_auth_flow(
    &mut self,
    provider_id: &str,
    display_name: &str,
    method_label: &str,
    env_vars: &[String],
) {
    if method_label.starts_with("Browser login") || method_label == "Device code" {
        // These will be started by the event loop (Tasks 7-8)
        // For now, transition to the waiting phase
        if method_label == "Device code" {
            self.phase = ConnectPhase::DeviceCode {
                provider_id: provider_id.to_owned(),
                display_name: display_name.to_owned(),
                user_code: String::new(),
                verification_uri: String::new(),
            };
        } else {
            self.phase = ConnectPhase::BrowserOAuth {
                provider_id: provider_id.to_owned(),
                display_name: display_name.to_owned(),
                url: None,
            };
        }
    } else {
        // API key entry
        let env_hint = env_vars.first().cloned().unwrap_or_default();
        self.phase = ConnectPhase::ApiKeyEntry {
            provider_id: provider_id.to_owned(),
            display_name: display_name.to_owned(),
            input: String::new(),
            cursor: 0,
            env_hint,
        };
    }
}

pub fn method_up(&mut self) {
    if let ConnectPhase::MethodPicker { selected, .. } = &mut self.phase {
        *selected = selected.saturating_sub(1);
    }
}

pub fn method_down(&mut self) {
    if let ConnectPhase::MethodPicker {
        selected, methods, ..
    } = &mut self.phase
    {
        let max = methods.len().saturating_sub(1);
        if *selected < max {
            *selected += 1;
        }
    }
}

pub fn api_key_insert_char(&mut self, c: char) {
    if let ConnectPhase::ApiKeyEntry { input, cursor, .. } = &mut self.phase {
        input.insert(*cursor, c);
        *cursor += c.len_utf8();
    }
}

pub fn api_key_delete_char(&mut self) {
    if let ConnectPhase::ApiKeyEntry { input, cursor, .. } = &mut self.phase {
        if *cursor == 0 {
            return;
        }
        let before = &input[..*cursor];
        let char_start = before
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        input.drain(char_start..*cursor);
        *cursor = char_start;
    }
}
```

**Step 3: Add tests for phase transitions**

```rust
#[test]
fn test_select_provider_single_method_skips_picker() {
    let mut state = ConnectModalState::new();
    state.open(&empty_statuses());
    // GitHub Copilot has only DeviceCode
    let copilot = state
        .providers
        .iter()
        .find(|p| p.id == "github-copilot")
        .unwrap()
        .clone();
    state.select_provider(&copilot);
    assert!(matches!(state.phase, ConnectPhase::DeviceCode { .. }));
}

#[test]
fn test_select_provider_multiple_methods_shows_picker() {
    let mut state = ConnectModalState::new();
    state.open(&empty_statuses());
    let anthropic = state
        .providers
        .iter()
        .find(|p| p.id == "anthropic")
        .unwrap()
        .clone();
    state.select_provider(&anthropic);
    assert!(matches!(state.phase, ConnectPhase::MethodPicker { .. }));
}

#[test]
fn test_select_provider_api_key_only() {
    let mut state = ConnectModalState::new();
    state.open(&empty_statuses());
    let groq = state
        .providers
        .iter()
        .find(|p| p.id == "groq")
        .unwrap()
        .clone();
    state.select_provider(&groq);
    assert!(matches!(state.phase, ConnectPhase::ApiKeyEntry { .. }));
}

#[test]
fn test_method_picker_navigation() {
    let mut state = ConnectModalState::new();
    state.open(&empty_statuses());
    let anthropic = state
        .providers
        .iter()
        .find(|p| p.id == "anthropic")
        .unwrap()
        .clone();
    state.select_provider(&anthropic);
    if let ConnectPhase::MethodPicker { selected, .. } = &state.phase {
        assert_eq!(*selected, 0);
    }
    state.method_down();
    if let ConnectPhase::MethodPicker { selected, .. } = &state.phase {
        assert_eq!(*selected, 1);
    }
    state.method_up();
    if let ConnectPhase::MethodPicker { selected, .. } = &state.phase {
        assert_eq!(*selected, 0);
    }
}

#[test]
fn test_api_key_input() {
    let mut state = ConnectModalState::new();
    state.open(&empty_statuses());
    let groq = state
        .providers
        .iter()
        .find(|p| p.id == "groq")
        .unwrap()
        .clone();
    state.select_provider(&groq);
    state.api_key_insert_char('s');
    state.api_key_insert_char('k');
    if let ConnectPhase::ApiKeyEntry { input, cursor, .. } = &state.phase {
        assert_eq!(input, "sk");
        assert_eq!(*cursor, 2);
    }
    state.api_key_delete_char();
    if let ConnectPhase::ApiKeyEntry { input, .. } = &state.phase {
        assert_eq!(input, "s");
    }
}
```

**Step 4: Verify**

Run: `cargo build -p ucode-tui && cargo test -p ucode-tui`
Expected: Build succeeds, all tests pass.

**Step 5: Commit**

```
feat(tui): wire /connect event loop routing and phase transitions

Handle key events for all connect modal phases (provider list, method
picker, API key entry, OAuth/device code waiting). 5 new tests for
phase transitions.
```

---

### Task 4: ConnectModal widget rendering

**Files:**
- Modify: `crates/ucode-tui/src/overlays/connect_modal.rs`

**Step 1: Add the ConnectModal widget**

Add the rendering widget at the bottom of `connect_modal.rs` (before `#[cfg(test)]`):

```rust
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};

/// Render widget for the connect modal.
pub struct ConnectModal<'a> {
    state: &'a ConnectModalState,
    theme: &'a crate::theme::UcodeTheme,
}

impl<'a> ConnectModal<'a> {
    pub fn new(state: &'a ConnectModalState, theme: &'a crate::theme::UcodeTheme) -> Self {
        Self { state, theme }
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_width = area.width * percent_x / 100;
    let popup_height = area.height * percent_y / 100;
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    Rect::new(x, y, popup_width, popup_height)
}

impl Widget for ConnectModal<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if !self.state.visible {
            return;
        }

        let popup = centered_rect(60, 70, area);
        Clear.render(popup, buf);

        match &self.state.phase {
            ConnectPhase::ProviderList => self.render_provider_list(popup, buf),
            ConnectPhase::MethodPicker {
                provider_id: _,
                methods,
                selected,
            } => self.render_method_picker(methods, *selected, popup, buf),
            ConnectPhase::ApiKeyEntry {
                display_name,
                input,
                cursor: _,
                env_hint,
                ..
            } => self.render_api_key_entry(display_name, input, env_hint, popup, buf),
            ConnectPhase::BrowserOAuth {
                display_name, url, ..
            } => self.render_browser_oauth(display_name, url.as_deref(), popup, buf),
            ConnectPhase::DeviceCode {
                display_name,
                user_code,
                verification_uri,
                ..
            } => self.render_device_code(display_name, user_code, verification_uri, popup, buf),
            ConnectPhase::Verifying { display_name, .. } => {
                self.render_verifying(display_name, popup, buf);
            }
        }
    }
}

impl ConnectModal<'_> {
    fn render_provider_list(&self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .title(" Connect ")
            .borders(Borders::ALL)
            .border_style(self.theme.border_style());
        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height < 4 {
            return;
        }

        // Layout: filter (1 line) + gap (1) + list (remaining - 4 for footer) + separator (1) + footer (3)
        let filter_area = Rect::new(inner.x, inner.y, inner.width, 1);
        let footer_height = 3u16;
        let separator_height = 1u16;
        let list_height = inner
            .height
            .saturating_sub(2 + footer_height + separator_height);
        let list_area = Rect::new(inner.x, inner.y + 2, inner.width, list_height);
        let separator_area = Rect::new(
            inner.x,
            inner.y + 2 + list_height,
            inner.width,
            separator_height,
        );
        let footer_area = Rect::new(
            inner.x,
            inner.y + 2 + list_height + separator_height,
            inner.width,
            footer_height,
        );

        // Filter line
        let filter_text = format!("Filter: {}", self.state.filter);
        Paragraph::new(filter_text).render(filter_area, buf);

        // Provider list with section headers
        let mut items: Vec<ListItem> = Vec::new();
        let mut list_idx = 0;
        let mut current_section = None;

        for &fi in &self.state.filtered_indices {
            let provider = &self.state.providers[fi];

            // Section header
            if current_section != Some(provider.section) {
                current_section = Some(provider.section);
                let header = match provider.section {
                    ConnectSection::QuickConnect => "Quick Connect",
                    ConnectSection::ApiKey => "API Key",
                };
                items.push(ListItem::new(Line::from(Span::styled(
                    header,
                    Style::default().add_modifier(Modifier::BOLD),
                ))));
            }

            // Provider entry
            let badge = match &provider.status {
                ProviderStatus::Connected { .. } => " [connected]",
                ProviderStatus::NotConfigured => "",
            };
            let prefix = if list_idx == self.state.selected {
                "> "
            } else {
                "  "
            };
            let style = if list_idx == self.state.selected {
                self.theme.selected_style()
            } else {
                Style::default()
            };
            let line = format!("{prefix}{}{badge}", provider.display_name);
            items.push(ListItem::new(Line::from(Span::styled(line, style))));
            list_idx += 1;
        }

        List::new(items).render(list_area, buf);

        // Separator
        let sep = "─".repeat(inner.width as usize);
        Paragraph::new(sep).render(separator_area, buf);

        // Footer: detail for selected provider
        if let Some(provider) = self.state.selected_provider() {
            let badge = match &provider.status {
                ProviderStatus::Connected { kind } => format!("[connected] via {kind}"),
                ProviderStatus::NotConfigured => "[not configured]".to_owned(),
            };
            let env_status = if provider.env_vars.is_empty() {
                String::new()
            } else {
                let var = &provider.env_vars[0];
                let set = std::env::var(var).is_ok();
                format!("Env: {} ({})", var, if set { "set" } else { "unset" })
            };
            let methods: Vec<&str> = [
                provider.has_browser_oauth.then_some("OAuth"),
                provider.has_device_code.then_some("Device"),
                provider.has_api_key.then_some("Key"),
            ]
            .into_iter()
            .flatten()
            .collect();
            let line1 = format!("{} {}", provider.display_name, badge);
            let line2 = format!("Methods: {}", methods.join(", "));
            let footer_text = if env_status.is_empty() {
                format!("{line1}\n{line2}")
            } else {
                format!("{line1}\n{line2}\n{env_status}")
            };
            Paragraph::new(footer_text).render(footer_area, buf);
        }
    }

    fn render_method_picker(
        &self,
        methods: &[String],
        selected: usize,
        area: Rect,
        buf: &mut Buffer,
    ) {
        let popup = centered_rect(50, 30, area);
        Clear.render(popup, buf);
        let block = Block::default()
            .title(" Auth Method ")
            .borders(Borders::ALL)
            .border_style(self.theme.border_style());
        let inner = block.inner(popup);
        block.render(popup, buf);

        let items: Vec<ListItem> = methods
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let prefix = if i == selected { "> " } else { "  " };
                let style = if i == selected {
                    self.theme.selected_style()
                } else {
                    Style::default()
                };
                ListItem::new(Line::from(Span::styled(format!("{prefix}{m}"), style)))
            })
            .collect();

        List::new(items).render(inner, buf);
    }

    fn render_api_key_entry(
        &self,
        display_name: &str,
        input: &str,
        env_hint: &str,
        area: Rect,
        buf: &mut Buffer,
    ) {
        let popup = centered_rect(50, 30, area);
        Clear.render(popup, buf);
        let title = format!(" {display_name}: API Key ");
        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(self.theme.border_style());
        let inner = block.inner(popup);
        block.render(popup, buf);

        let masked: String = if input.len() > 4 {
            format!("{}****", &input[..4])
        } else {
            "*".repeat(input.len())
        };
        let lines = vec![
            Line::from("Paste API key:"),
            Line::from(format!("> {masked}_")),
            Line::from(""),
            Line::from(format!("Env: {env_hint}")),
            Line::from(""),
            Line::from("[Enter: Save] [Esc: Back]"),
        ];
        Paragraph::new(lines).render(inner, buf);
    }

    fn render_browser_oauth(
        &self,
        display_name: &str,
        url: Option<&str>,
        area: Rect,
        buf: &mut Buffer,
    ) {
        let popup = centered_rect(50, 30, area);
        Clear.render(popup, buf);
        let title = format!(" {display_name} ");
        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(self.theme.border_style());
        let inner = block.inner(popup);
        block.render(popup, buf);

        let mut lines = vec![
            Line::from("Opening browser..."),
            Line::from(""),
        ];
        if let Some(url) = url {
            lines.push(Line::from("If browser didn't open:"));
            // Truncate URL for display
            let display_url = if url.len() > 50 {
                format!("{}...", &url[..50])
            } else {
                url.to_owned()
            };
            lines.push(Line::from(display_url));
        }
        lines.push(Line::from(""));
        lines.push(Line::from("Waiting for redirect..."));
        lines.push(Line::from(""));
        lines.push(Line::from("[Esc: Cancel]"));

        Paragraph::new(lines).render(inner, buf);
    }

    fn render_device_code(
        &self,
        display_name: &str,
        user_code: &str,
        verification_uri: &str,
        area: Rect,
        buf: &mut Buffer,
    ) {
        let popup = centered_rect(50, 30, area);
        Clear.render(popup, buf);
        let title = format!(" {display_name} ");
        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(self.theme.border_style());
        let inner = block.inner(popup);
        block.render(popup, buf);

        let lines = vec![
            Line::from(format!("Open: {verification_uri}")),
            Line::from(format!("Code: {user_code}")),
            Line::from(""),
            Line::from("Waiting for authorization..."),
            Line::from(""),
            Line::from("[Esc: Cancel]"),
        ];
        Paragraph::new(lines).render(inner, buf);
    }

    fn render_verifying(&self, display_name: &str, area: Rect, buf: &mut Buffer) {
        let popup = centered_rect(40, 20, area);
        Clear.render(popup, buf);
        let title = format!(" {display_name} ");
        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(self.theme.border_style());
        let inner = block.inner(popup);
        block.render(popup, buf);

        let lines = vec![
            Line::from("Verifying credentials..."),
        ];
        Paragraph::new(lines).render(inner, buf);
    }
}
```

Note: `self.theme.border_style()` and `self.theme.selected_style()` — check if these methods exist on `UcodeTheme`. If not, use `Style::default()` as placeholder and adapt to whatever the theme API actually provides. The subagent implementing this should check `theme.rs` for the actual API.

**Step 2: Verify**

Run: `cargo build -p ucode-tui && cargo test -p ucode-tui`
Expected: Build succeeds, all tests pass.

**Step 3: Commit**

```
feat(tui): add ConnectModal widget rendering for all phases

Renders provider list with sections, status badges, detail footer,
method picker, API key entry, browser OAuth waiting, device code
waiting, and verification phases.
```

---

### Task 5: TuiEvent auth variants + credential store integration

**Files:**
- Modify: `crates/ucode-tui/src/event_loop.rs` — add TuiEvent variants + handlers
- Modify: `crates/ucode-tui/Cargo.toml` — may need `reqwest` if not already available

**Step 1: Add TuiEvent variants**

In `event_loop.rs`, add to the `TuiEvent` enum:

```rust
AuthCompleted {
    provider: String,
},
AuthFailed {
    provider: String,
    error: String,
},
VerifyResult {
    provider: String,
    success: bool,
    message: Option<String>,
},
```

**Step 2: Handle new TuiEvent variants**

In `handle_tui_event`, add:

```rust
TuiEvent::AuthCompleted { provider } => {
    app.connect_modal.phase = ConnectPhase::Verifying {
        provider_id: provider.clone(),
        display_name: provider.clone(),
    };
    app.mark_dirty();
    // Verification will be spawned by the caller that has event_tx
}
TuiEvent::AuthFailed { provider, error } => {
    app.connect_modal.close();
    app.focus = FocusTarget::Input;
    app.toast(
        crate::components::toast::ToastLevel::Error,
        format!("{provider} auth failed: {error}"),
    );
}
TuiEvent::VerifyResult {
    provider,
    success,
    message,
} => {
    app.connect_modal.close();
    app.focus = FocusTarget::Input;
    if success {
        app.toast(
            crate::components::toast::ToastLevel::Info,
            format!("{provider} connected"),
        );
    } else {
        let msg = message.unwrap_or_default();
        app.toast(
            crate::components::toast::ToastLevel::Warn,
            format!("{provider} connected (verification failed: {msg})"),
        );
    }
}
```

**Step 3: Verify**

Run: `cargo build -p ucode-tui && cargo test -p ucode-tui`
Expected: Build succeeds, all tests pass.

**Step 4: Commit**

```
feat(tui): add TuiEvent auth variants and handlers

AuthCompleted transitions to verification phase, AuthFailed shows
error toast, VerifyResult shows success/warning toast and closes modal.
```

---

### Task 6: API key save flow

**Files:**
- Modify: `crates/ucode-tui/src/event_loop.rs` — handle Enter in ApiKeyEntry phase
- Modify: `crates/ucode-tui/src/overlays/connect_modal.rs` — add `take_api_key` method

**Step 1: Add take_api_key method**

In `connect_modal.rs`:

```rust
/// Extract the API key input and provider info, resetting the phase.
/// Returns (provider_id, api_key) if in ApiKeyEntry phase.
pub fn take_api_key(&mut self) -> Option<(String, String)> {
    if let ConnectPhase::ApiKeyEntry {
        provider_id,
        input,
        ..
    } = &self.phase
    {
        if input.is_empty() {
            return None;
        }
        let result = Some((provider_id.clone(), input.clone()));
        result
    } else {
        None
    }
}
```

**Step 2: Wire Enter key in ApiKeyEntry phase**

In `event_loop.rs`, in the `ConnectPhase::ApiKeyEntry` match arm, replace the Enter handler:

```rust
crossterm::event::KeyCode::Enter => {
    if let Some((provider_id, api_key)) = app.connect_modal.take_api_key() {
        // Store credential
        let store = ucode_auth::KeyringStore::new();
        let material = ucode_auth::AuthMaterial::ApiKey { key: api_key };
        match store.store(&provider_id, &material) {
            Ok(()) => {
                let display = app
                    .connect_modal
                    .selected_provider()
                    .map(|p| p.display_name.clone())
                    .unwrap_or(provider_id.clone());
                app.connect_modal.phase = ConnectPhase::Verifying {
                    provider_id: provider_id.clone(),
                    display_name: display,
                };
                // TODO: spawn verification task (Task 9)
                // For now, just show success toast
                app.connect_modal.close();
                app.focus = FocusTarget::Input;
                app.toast(
                    crate::components::toast::ToastLevel::Info,
                    format!("{provider_id} connected"),
                );
            }
            Err(e) => {
                app.connect_modal.close();
                app.focus = FocusTarget::Input;
                app.toast(
                    crate::components::toast::ToastLevel::Error,
                    format!("Failed to store key: {e}"),
                );
            }
        }
        app.mark_dirty();
    }
}
```

Note: `KeyringStore::new()` — check the actual constructor. The subagent should verify the `CredentialStore` trait import and `KeyringStore` constructor.

**Step 3: Verify**

Run: `cargo build -p ucode-tui && cargo test -p ucode-tui`
Expected: Build succeeds, all tests pass.

**Step 4: Commit**

```
feat(tui): wire API key save flow in /connect modal

Enter in API key entry stores credential via KeyringStore, shows
success/error toast.
```

---

### Task 7: Browser OAuth async flow

**Files:**
- Modify: `crates/ucode-tui/src/event_loop.rs` — spawn OAuth task on phase transition
- Modify: `crates/ucode-tui/src/overlays/connect_modal.rs` — track JoinHandle for cancellation
- Modify: `crates/ucode-tui/src/app.rs` — add auth_task field

**Step 1: Add auth task handle to AppState**

In `app.rs`, add:
```rust
/// Handle for in-flight auth task (for cancellation).
#[allow(dead_code)]
pub auth_task: Option<tokio::task::JoinHandle<()>>,
```

Initialize as `None` in `new()`. Note: `JoinHandle` is not `Clone` or `Debug`, so we may need to adjust. Use `Option<tokio::task::JoinHandle<()>>` and skip Debug derive or use a wrapper.

Actually, since `AppState` derives `Debug, Clone`, and `JoinHandle` is neither, we need a different approach. Store the handle separately in the event loop, not in AppState. Pass it as a local variable in `handle_terminal_event` or store it in a separate struct.

Better approach: store it as a `&mut Option<JoinHandle<()>>` parameter to `handle_terminal_event`, or keep it as a local in `run_event_loop`.

**Step 2: In event_loop.rs, add auth_task local**

In `run_event_loop`, add:
```rust
let mut auth_task: Option<tokio::task::JoinHandle<()>> = None;
```

Pass `&mut auth_task` to `handle_terminal_event`.

**Step 3: Spawn browser OAuth on phase transition**

When the connect modal transitions to `BrowserOAuth` phase (in the Enter handler for ProviderList or MethodPicker), spawn the async task:

```rust
// In the Enter handler, after state.select_provider() or state.select_method():
if let ConnectPhase::BrowserOAuth { provider_id, .. } = &app.connect_modal.phase {
    let provider_id = provider_id.clone();
    let event_tx = event_tx.clone(); // need event_tx available
    let config = match provider_id.as_str() {
        "openai" => Some(ucode_auth::openai_subscription_oauth_config()),
        "anthropic" => Some(ucode_auth::anthropic_max_oauth_config()),
        // Add more as needed
        _ => None,
    };
    if let Some(config) = config {
        // Update URL in phase for display
        app.connect_modal.phase = ConnectPhase::BrowserOAuth {
            provider_id: provider_id.clone(),
            display_name: app.connect_modal.phase_display_name().unwrap_or_default(),
            url: Some(config.auth_url.clone()),
        };
        let handle = tokio::spawn(async move {
            match ucode_auth::browser_oauth_authorize(&config).await {
                Ok(material) => {
                    let store = ucode_auth::KeyringStore::new();
                    let _ = store.store(&provider_id, &material);
                    let _ = event_tx.send(TuiEvent::AuthCompleted {
                        provider: provider_id,
                    });
                }
                Err(e) => {
                    let _ = event_tx.send(TuiEvent::AuthFailed {
                        provider: provider_id,
                        error: e.to_string(),
                    });
                }
            }
        });
        *auth_task = Some(handle);
    }
}
```

**Step 4: Cancel on Esc**

In the `BrowserOAuth` Esc handler:
```rust
if let Some(handle) = auth_task.take() {
    handle.abort();
}
app.connect_modal.phase = ConnectPhase::ProviderList;
app.mark_dirty();
```

**Step 5: Add helper method**

In `connect_modal.rs`:
```rust
pub fn phase_display_name(&self) -> Option<String> {
    match &self.phase {
        ConnectPhase::BrowserOAuth { display_name, .. }
        | ConnectPhase::DeviceCode { display_name, .. }
        | ConnectPhase::ApiKeyEntry { display_name, .. }
        | ConnectPhase::Verifying { display_name, .. } => Some(display_name.clone()),
        _ => None,
    }
}
```

**Step 6: Verify**

Run: `cargo build -p ucode-tui && cargo test -p ucode-tui`
Expected: Build succeeds, all tests pass.

**Step 7: Commit**

```
feat(tui): wire browser OAuth async flow in /connect modal

Spawn browser_oauth_authorize in tokio task, send AuthCompleted/
AuthFailed via TuiEvent channel. Cancel via JoinHandle::abort on Esc.
```

---

### Task 8: Device code async flow

**Files:**
- Modify: `crates/ucode-tui/src/event_loop.rs`

**Step 1: Spawn device code flow on phase transition**

Similar to Task 7, when transitioning to `DeviceCode` phase:

```rust
if let ConnectPhase::DeviceCode { provider_id, .. } = &app.connect_modal.phase {
    let provider_id = provider_id.clone();
    let event_tx = event_tx.clone();
    let config = match provider_id.as_str() {
        "github-copilot" => Some(ucode_auth::github_copilot_device_config(None)),
        _ => None,
    };
    if let Some(config) = config {
        let handle = tokio::spawn(async move {
            let client = reqwest::Client::new();
            match ucode_auth::request_device_code(&client, &config).await {
                Ok(pending) => {
                    // Send device code info back for display
                    let _ = event_tx.send(TuiEvent::DeviceCodeReady {
                        provider: provider_id.clone(),
                        user_code: pending.user_code.clone(),
                        verification_uri: pending.verification_uri.clone(),
                    });
                    // Poll for token
                    match ucode_auth::poll_for_token(&client, &config, &pending).await {
                        Ok(material) => {
                            let store = ucode_auth::KeyringStore::new();
                            let _ = store.store(&provider_id, &material);
                            let _ = event_tx.send(TuiEvent::AuthCompleted {
                                provider: provider_id,
                            });
                        }
                        Err(e) => {
                            let _ = event_tx.send(TuiEvent::AuthFailed {
                                provider: provider_id,
                                error: e.to_string(),
                            });
                        }
                    }
                }
                Err(e) => {
                    let _ = event_tx.send(TuiEvent::AuthFailed {
                        provider: provider_id,
                        error: e.to_string(),
                    });
                }
            }
        });
        *auth_task = Some(handle);
    }
}
```

**Step 2: Add DeviceCodeReady TuiEvent variant**

```rust
DeviceCodeReady {
    provider: String,
    user_code: String,
    verification_uri: String,
},
```

**Step 3: Handle DeviceCodeReady**

```rust
TuiEvent::DeviceCodeReady {
    provider,
    user_code,
    verification_uri,
} => {
    if let ConnectPhase::DeviceCode {
        provider_id,
        display_name,
        ..
    } = &app.connect_modal.phase
    {
        if *provider_id == provider {
            app.connect_modal.phase = ConnectPhase::DeviceCode {
                provider_id: provider_id.clone(),
                display_name: display_name.clone(),
                user_code,
                verification_uri,
            };
            app.mark_dirty();
        }
    }
}
```

**Step 4: Verify**

Run: `cargo build -p ucode-tui && cargo test -p ucode-tui`
Expected: Build succeeds, all tests pass.

**Step 5: Commit**

```
feat(tui): wire device code async flow in /connect modal

Spawn request_device_code + poll_for_token in tokio task, display
user code and verification URI, cancel via abort on Esc.
```

---

### Task 9: Verification ping

**Files:**
- Modify: `crates/ucode-tui/src/event_loop.rs`

**Step 1: Spawn verification after AuthCompleted**

In the `TuiEvent::AuthCompleted` handler, spawn a verification task:

```rust
TuiEvent::AuthCompleted { provider } => {
    let display = provider.clone();
    app.connect_modal.phase = ConnectPhase::Verifying {
        provider_id: provider.clone(),
        display_name: display.clone(),
    };
    app.mark_dirty();

    // Spawn verification ping
    let event_tx = event_tx.clone();
    let provider_clone = provider.clone();
    tokio::spawn(async move {
        let result = verify_provider(&provider_clone).await;
        let _ = event_tx.send(TuiEvent::VerifyResult {
            provider: provider_clone,
            success: result.is_ok(),
            message: result.err(),
        });
    });
}
```

Note: `event_tx` needs to be accessible in `handle_tui_event`. This may require passing it as a parameter or restructuring slightly. The subagent should handle this.

**Step 2: Add verify_provider function**

```rust
async fn verify_provider(provider: &str) -> Result<(), String> {
    let store = ucode_auth::KeyringStore::new();
    let material = store.load(provider).map_err(|e| e.to_string())?;

    let client = reqwest::Client::new();
    let (url, auth_header) = match (provider, &material) {
        ("openai", ucode_auth::AuthMaterial::ApiKey { key }) => (
            "https://api.openai.com/v1/models".to_owned(),
            format!("Bearer {key}"),
        ),
        ("openai", ucode_auth::AuthMaterial::OAuth { access_token, .. }) => (
            "https://api.openai.com/v1/models".to_owned(),
            format!("Bearer {access_token}"),
        ),
        ("anthropic", ucode_auth::AuthMaterial::ApiKey { key }) => (
            "https://api.anthropic.com/v1/models".to_owned(),
            key.clone(), // x-api-key header
        ),
        ("anthropic", ucode_auth::AuthMaterial::OAuth { access_token, .. }) => (
            "https://api.anthropic.com/v1/models".to_owned(),
            format!("Bearer {access_token}"),
        ),
        _ => return Ok(()), // Skip verification for unknown providers
    };

    let mut req = client.get(&url).timeout(std::time::Duration::from_secs(10));

    // Set appropriate auth header
    if provider == "anthropic" && matches!(material, ucode_auth::AuthMaterial::ApiKey { .. }) {
        req = req.header("x-api-key", &auth_header);
        req = req.header("anthropic-version", "2023-06-01");
    } else {
        req = req.header("Authorization", &auth_header);
    }

    let resp = req.send().await.map_err(|e| e.to_string())?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("HTTP {}", resp.status()))
    }
}
```

Note: `reqwest` needs to be a dependency. Check if it's already in the workspace. If not, add it. The subagent should handle this.

**Step 3: Verify**

Run: `cargo build -p ucode-tui && cargo test -p ucode-tui`
Expected: Build succeeds, all tests pass.

**Step 4: Commit**

```
feat(tui): add verification ping after /connect auth

Lightweight API call to verify credentials work after auth completes.
Shows success or warning toast based on result.
```

---

### Task 10: Update EPIC.md + PLANS.md

**Files:**
- Modify: `EPIC.md` — mark ISSUE 0705 as DONE
- Modify: `PLANS.md` — mark corresponding task as DONE (if exists)

**Step 1: Mark ISSUE 0705 as DONE in EPIC.md**

Change:
```markdown
### ISSUE 0705 — /connect UI (providers + auth method picker) (ucode-tui + auth)
```
To:
```markdown
### ISSUE 0705 — /connect UI (providers + auth method picker) (ucode-tui + auth) [DONE]
```

**Step 2: Update PLANS.md if applicable**

Search for a corresponding task entry and mark it DONE.

**Step 3: Full verification**

Run: `cargo build --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: All pass, zero warnings.

**Step 4: Commit**

```
docs: mark ISSUE 0705 (/connect UI) as done

All phases implemented: provider list with sections and status badges,
method picker, API key entry, browser OAuth, device code, verification
ping. Toast notifications for success/failure.
```
