use std::collections::HashMap;

use ucode_auth::{AuthMethod, CredentialStatus, provider_auth_info};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const QUICK_CONNECT_IDS: &[&str] = &["anthropic", "openai", "github-copilot", "gemini"];
pub const API_KEY_IDS: &[&str] = &[
    "groq",
    "deepseek",
    "openrouter",
    "together",
    "fireworks",
    "mistral",
    "azure-openai",
    "aws-bedrock",
    "vertex-ai",
];

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// Which section of the connect modal a provider belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectSection {
    QuickConnect,
    ApiKey,
}

/// Display status for a provider row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderStatus {
    Connected { kind: String },
    NotConfigured,
}

/// A single provider entry in the connect modal list.
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

/// Which phase of the connect flow the modal is in.
#[derive(Debug, Clone)]
pub enum ConnectPhase {
    /// Showing the provider list.
    ProviderList,
    /// Showing auth method picker for a provider.
    MethodPicker {
        provider_id: String,
        methods: Vec<AuthMethod>,
        selected: usize,
    },
    /// User is typing an API key.
    ApiKeyEntry {
        provider_id: String,
        display_name: String,
        input: String,
        cursor: usize,
        env_hint: String,
    },
    /// Waiting for user to complete browser OAuth.
    BrowserOAuth {
        provider_id: String,
        display_name: String,
        url: String,
    },
    /// Showing device code for the user to enter on another device.
    DeviceCode {
        provider_id: String,
        display_name: String,
        user_code: String,
        verification_uri: String,
    },
    /// Polling / verifying credentials after the user completes a flow.
    Verifying {
        provider_id: String,
        display_name: String,
    },
}

/// Full state for the connect modal overlay.
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

// ---------------------------------------------------------------------------
// ConnectModalState impl
// ---------------------------------------------------------------------------

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

    /// Open the modal, building the provider list from the given credential statuses.
    ///
    /// `statuses` maps provider id → `CredentialStatus`.
    pub fn open(&mut self, statuses: &HashMap<String, CredentialStatus>) {
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

    // -- filter text editing -------------------------------------------------

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

    // -- navigation ----------------------------------------------------------

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

    // -- queries -------------------------------------------------------------

    pub fn selected_provider(&self) -> Option<&ConnectProvider> {
        let idx = *self.filtered_indices.get(self.selected)?;
        self.providers.get(idx)
    }

    // -- internal ------------------------------------------------------------

    pub fn update_filter(&mut self) {
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

impl Default for ConnectModalState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// build_provider_list
// ---------------------------------------------------------------------------

fn build_provider_list(statuses: &HashMap<String, CredentialStatus>) -> Vec<ConnectProvider> {
    let mut providers = Vec::with_capacity(QUICK_CONNECT_IDS.len() + API_KEY_IDS.len());

    for (section, ids) in [
        (ConnectSection::QuickConnect, QUICK_CONNECT_IDS),
        (ConnectSection::ApiKey, API_KEY_IDS),
    ] {
        for &id in ids {
            let Some(info) = provider_auth_info(id) else {
                continue;
            };

            let status = match statuses.get(id) {
                Some(CredentialStatus::Configured { kind, .. }) => {
                    ProviderStatus::Connected { kind: kind.clone() }
                }
                _ => ProviderStatus::NotConfigured,
            };

            let has_browser_oauth = info.auth_methods.contains(&AuthMethod::BrowserOAuth);
            let has_device_code = info.auth_methods.contains(&AuthMethod::DeviceCode);
            let has_api_key = info.auth_methods.contains(&AuthMethod::ApiKey);

            providers.push(ConnectProvider {
                id: id.to_owned(),
                display_name: info.display_name.to_owned(),
                section,
                status,
                env_vars: info.env_vars.iter().map(|s| s.to_string()).collect(),
                has_browser_oauth,
                has_device_code,
                has_api_key,
            });
        }
    }

    providers
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_statuses() -> HashMap<String, CredentialStatus> {
        HashMap::new()
    }

    fn statuses_with(provider: &str, kind: &str) -> HashMap<String, CredentialStatus> {
        let mut m = HashMap::new();
        m.insert(
            provider.to_owned(),
            CredentialStatus::Configured {
                provider: provider.to_owned(),
                kind: kind.to_owned(),
            },
        );
        m
    }

    #[test]
    fn test_new_is_not_visible() {
        let state = ConnectModalState::new();
        assert!(!state.visible);
        assert!(state.providers.is_empty());
        assert!(state.filter.is_empty());
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn test_open_sets_visible_and_populates_providers() {
        let mut state = ConnectModalState::new();
        state.open(&empty_statuses());
        assert!(state.visible);
        assert!(!state.providers.is_empty());
        assert_eq!(state.filter, "");
        assert_eq!(state.selected, 0);
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

        assert_eq!(quick.len(), 4, "expected 4 quick-connect providers");
        assert_eq!(api_key.len(), 9, "expected 9 api-key providers");
    }

    #[test]
    fn test_status_badge_connected() {
        let statuses = statuses_with("anthropic", "api_key");
        let mut state = ConnectModalState::new();
        state.open(&statuses);

        let anthropic = state
            .providers
            .iter()
            .find(|p| p.id == "anthropic")
            .expect("anthropic must be present");

        assert_eq!(
            anthropic.status,
            ProviderStatus::Connected {
                kind: "api_key".to_owned()
            }
        );
    }

    #[test]
    fn test_status_badge_not_configured() {
        let mut state = ConnectModalState::new();
        state.open(&empty_statuses());

        let openai = state
            .providers
            .iter()
            .find(|p| p.id == "openai")
            .expect("openai must be present");

        assert_eq!(openai.status, ProviderStatus::NotConfigured);
    }

    #[test]
    fn test_filter_narrows_list() {
        let mut state = ConnectModalState::new();
        state.open(&empty_statuses());

        let total = state.filtered_indices.len();
        for c in "anthropic".chars() {
            state.insert_char(c);
        }
        assert!(state.filtered_indices.len() < total);
        assert_eq!(state.filtered_indices.len(), 1);
        let p = state.selected_provider().unwrap();
        assert_eq!(p.id, "anthropic");
    }

    #[test]
    fn test_navigate_up_down() {
        let mut state = ConnectModalState::new();
        state.open(&empty_statuses());

        assert_eq!(state.selected, 0);
        state.move_up(); // saturates at 0
        assert_eq!(state.selected, 0);

        state.move_down();
        state.move_down();
        assert_eq!(state.selected, 2);

        state.move_up();
        assert_eq!(state.selected, 1);

        // clamp at last
        let last = state.filtered_indices.len() - 1;
        for _ in 0..last + 10 {
            state.move_down();
        }
        assert_eq!(state.selected, last);
    }

    #[test]
    fn test_selected_provider_first_is_anthropic() {
        let mut state = ConnectModalState::new();
        state.open(&empty_statuses());

        let p = state.selected_provider().expect("should have a selection");
        assert_eq!(p.id, "anthropic");
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

        let anthropic = state
            .providers
            .iter()
            .find(|p| p.id == "anthropic")
            .unwrap();
        assert!(anthropic.has_browser_oauth, "anthropic should have OAuth");
        assert!(anthropic.has_api_key, "anthropic should have api key");

        let copilot = state
            .providers
            .iter()
            .find(|p| p.id == "github-copilot")
            .unwrap();
        assert!(copilot.has_device_code, "copilot should have device code");
        assert!(!copilot.has_browser_oauth, "copilot should not have OAuth");
        assert!(!copilot.has_api_key, "copilot should not have api key");
    }
}
