use std::collections::HashMap;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};
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
        methods: Vec<String>,
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
        url: Option<String>,
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

    // -- phase transitions ---------------------------------------------------

    /// Called when the user presses Enter on a provider in the list.
    ///
    /// If the provider supports only one auth method, skips the picker and
    /// transitions directly to the appropriate flow phase. Otherwise shows
    /// the method picker.
    pub fn select_provider(&mut self, provider: &ConnectProvider) {
        let mut methods: Vec<String> = Vec::new();

        if provider.has_browser_oauth {
            if provider.id == "anthropic" {
                methods.push("Browser login (Max)".into());
                methods.push("Browser login (Console)".into());
            } else {
                methods.push("Browser login".into());
            }
        }
        if provider.has_device_code {
            methods.push("Device code".into());
        }
        if provider.has_api_key {
            methods.push("API key".into());
        }

        if methods.len() == 1 {
            let label = methods.remove(0);
            self.start_auth_flow(
                provider.id.clone(),
                provider.display_name.clone(),
                &label,
                &provider.env_vars,
            );
        } else {
            self.phase = ConnectPhase::MethodPicker {
                provider_id: provider.id.clone(),
                methods,
                selected: 0,
            };
        }
    }

    /// Called when the user presses Enter in the method picker.
    pub fn select_method(&mut self) {
        let (provider_id, label) = match &self.phase {
            ConnectPhase::MethodPicker {
                provider_id,
                methods,
                selected,
            } => {
                let label = methods.get(*selected).cloned().unwrap_or_default();
                (provider_id.clone(), label)
            }
            _ => return,
        };

        let provider = self.providers.iter().find(|p| p.id == provider_id).cloned();
        if let Some(p) = provider {
            self.start_auth_flow(p.id.clone(), p.display_name.clone(), &label, &p.env_vars);
        }
    }

    fn start_auth_flow(
        &mut self,
        provider_id: String,
        display_name: String,
        method_label: &str,
        env_vars: &[String],
    ) {
        if method_label == "Device code" {
            self.phase = ConnectPhase::DeviceCode {
                provider_id,
                display_name,
                user_code: String::new(),
                verification_uri: String::new(),
            };
        } else if method_label.starts_with("Browser login") {
            self.phase = ConnectPhase::BrowserOAuth {
                provider_id,
                display_name,
                url: None,
            };
        } else {
            // API key
            let env_hint = env_vars.first().cloned().unwrap_or_default();
            self.phase = ConnectPhase::ApiKeyEntry {
                provider_id,
                display_name,
                input: String::new(),
                cursor: 0,
                env_hint,
            };
        }
    }

    /// Navigate up in the method picker.
    pub fn method_up(&mut self) {
        if let ConnectPhase::MethodPicker { selected, .. } = &mut self.phase {
            *selected = selected.saturating_sub(1);
        }
    }

    /// Navigate down in the method picker.
    pub fn method_down(&mut self) {
        if let ConnectPhase::MethodPicker {
            methods, selected, ..
        } = &mut self.phase
        {
            let max = methods.len().saturating_sub(1);
            if *selected < max {
                *selected += 1;
            }
        }
    }

    /// Insert a character into the API key input.
    pub fn api_key_insert_char(&mut self, c: char) {
        if let ConnectPhase::ApiKeyEntry { input, cursor, .. } = &mut self.phase {
            input.insert(*cursor, c);
            *cursor += c.len_utf8();
        }
    }

    /// Delete the character before the cursor in the API key input.
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

    /// Extract the display name from phases that have one.
    pub fn phase_display_name(&self) -> Option<String> {
        match &self.phase {
            ConnectPhase::BrowserOAuth { display_name, .. }
            | ConnectPhase::DeviceCode { display_name, .. }
            | ConnectPhase::ApiKeyEntry { display_name, .. }
            | ConnectPhase::Verifying { display_name, .. } => Some(display_name.clone()),
            _ => None,
        }
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
// ConnectModal widget
// ---------------------------------------------------------------------------

pub struct ConnectModal<'a> {
    state: &'a ConnectModalState,
    theme: &'a crate::theme::UcodeTheme,
}

impl<'a> ConnectModal<'a> {
    pub fn new(state: &'a ConnectModalState, theme: &'a crate::theme::UcodeTheme) -> Self {
        Self { state, theme }
    }
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
                methods, selected, ..
            } => self.render_method_picker(methods, *selected, area, buf),
            ConnectPhase::ApiKeyEntry {
                display_name,
                input,
                env_hint,
                ..
            } => self.render_api_key_entry(display_name, input, env_hint, area, buf),
            ConnectPhase::BrowserOAuth {
                display_name, url, ..
            } => self.render_browser_oauth(display_name, url.as_deref(), area, buf),
            ConnectPhase::DeviceCode {
                display_name,
                user_code,
                verification_uri,
                ..
            } => self.render_device_code(display_name, user_code, verification_uri, area, buf),
            ConnectPhase::Verifying { display_name, .. } => {
                self.render_verifying(display_name, area, buf)
            }
        }
    }
}

impl ConnectModal<'_> {
    fn render_provider_list(&self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border_style(true))
            .title(" Connect ");
        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height < 4 || inner.width < 10 {
            return;
        }

        // Layout: filter line, separator, list body, separator, footer (3 lines)
        let footer_height = 3u16;
        let list_height = inner
            .height
            .saturating_sub(1 + 1 + 1 + footer_height) // filter + sep + sep + footer
            .max(1);

        let filter_rect = Rect::new(inner.x, inner.y, inner.width, 1);
        let sep1_rect = Rect::new(inner.x, inner.y + 1, inner.width, 1);
        let list_rect = Rect::new(inner.x, inner.y + 2, inner.width, list_height);
        let sep2_y = inner.y + 2 + list_height;
        let sep2_rect = Rect::new(inner.x, sep2_y, inner.width, 1);
        let footer_rect = Rect::new(inner.x, sep2_y + 1, inner.width, footer_height);

        // Filter line
        let filter_line = Line::from(vec![
            Span::styled("Filter: ", self.theme.dim_style()),
            Span::styled(&self.state.filter, self.theme.text_style()),
        ]);
        Paragraph::new(filter_line).render(filter_rect, buf);

        // Separator
        let sep = "─".repeat(inner.width as usize);
        Paragraph::new(sep.as_str())
            .style(self.theme.dim_style())
            .render(sep1_rect, buf);

        // Provider list with section headers
        let visible_height = list_rect.height as usize;
        let scroll_offset = if self.state.selected >= visible_height {
            self.state.selected - visible_height + 1
        } else {
            0
        };

        let mut current_section: Option<ConnectSection> = None;
        let mut row = 0usize;

        for (display_idx, &provider_idx) in self.state.filtered_indices.iter().enumerate() {
            if row >= scroll_offset + visible_height {
                break;
            }

            let provider = &self.state.providers[provider_idx];

            // Section header
            if current_section != Some(provider.section) {
                current_section = Some(provider.section);
                if row >= scroll_offset {
                    let y = list_rect.y + (row - scroll_offset) as u16;
                    if y < list_rect.y + list_rect.height {
                        let label = match provider.section {
                            ConnectSection::QuickConnect => "Quick Connect",
                            ConnectSection::ApiKey => "API Key",
                        };
                        let header = Line::from(Span::styled(
                            label,
                            self.theme.dim_style().add_modifier(Modifier::BOLD),
                        ));
                        Paragraph::new(header)
                            .render(Rect::new(list_rect.x, y, list_rect.width, 1), buf);
                    }
                }
                row += 1;
                if row >= scroll_offset + visible_height {
                    break;
                }
            }

            // Provider row
            if row >= scroll_offset {
                let y = list_rect.y + (row - scroll_offset) as u16;
                if y < list_rect.y + list_rect.height {
                    let is_selected = display_idx == self.state.selected;
                    let prefix = if is_selected { "> " } else { "  " };
                    let badge = match &provider.status {
                        ProviderStatus::Connected { .. } => " [connected]",
                        ProviderStatus::NotConfigured => "",
                    };
                    let name_style = if is_selected {
                        Style::default().add_modifier(Modifier::REVERSED)
                    } else {
                        self.theme.text_style()
                    };
                    let badge_style = self.theme.safe_style();

                    let line = Line::from(vec![
                        Span::styled(format!("{prefix}{}", provider.display_name), name_style),
                        Span::styled(badge, badge_style),
                    ]);
                    let row_rect = Rect::new(list_rect.x, y, list_rect.width, 1);
                    Paragraph::new(line).render(row_rect, buf);
                }
            }
            row += 1;
        }

        // Bottom separator
        if sep2_rect.y < inner.y + inner.height {
            Paragraph::new(sep.as_str())
                .style(self.theme.dim_style())
                .render(sep2_rect, buf);
        }

        // Detail footer for selected provider
        if footer_rect.y < inner.y + inner.height
            && let Some(provider) = self.state.selected_provider()
        {
            self.render_provider_footer(provider, footer_rect, buf);
        }
    }

    fn render_provider_footer(&self, provider: &ConnectProvider, area: Rect, buf: &mut Buffer) {
        // Line 0: name + badge
        let badge = match &provider.status {
            ProviderStatus::Connected { kind } => format!(" [connected: {kind}]"),
            ProviderStatus::NotConfigured => String::new(),
        };
        let name_line = Line::from(vec![
            Span::styled(
                &provider.display_name,
                self.theme.text_style().add_modifier(Modifier::BOLD),
            ),
            Span::styled(badge, self.theme.safe_style()),
        ]);
        Paragraph::new(name_line).render(Rect::new(area.x, area.y, area.width, 1), buf);

        // Line 1: auth methods
        let mut methods = Vec::new();
        if provider.has_browser_oauth {
            methods.push("browser");
        }
        if provider.has_device_code {
            methods.push("device-code");
        }
        if provider.has_api_key {
            methods.push("api-key");
        }
        let methods_str = format!("methods: {}", methods.join(", "));
        if area.height > 1 {
            Paragraph::new(methods_str)
                .style(self.theme.dim_style())
                .render(Rect::new(area.x, area.y + 1, area.width, 1), buf);
        }

        // Line 2: env var status
        if area.height > 2 {
            let env_str = if provider.env_vars.is_empty() {
                "env: —".to_owned()
            } else {
                format!("env: {}", provider.env_vars.join(", "))
            };
            Paragraph::new(env_str)
                .style(self.theme.dim_style())
                .render(Rect::new(area.x, area.y + 2, area.width, 1), buf);
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
            .borders(Borders::ALL)
            .border_style(self.theme.border_style(true))
            .title(" Auth Method ");
        let inner = block.inner(popup);
        block.render(popup, buf);

        for (i, method) in methods.iter().enumerate() {
            if i as u16 >= inner.height {
                break;
            }
            let prefix = if i == selected { "> " } else { "  " };
            let style = if i == selected {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                self.theme.text_style()
            };
            let line = Line::from(Span::styled(format!("{prefix}{method}"), style));
            Paragraph::new(line)
                .render(Rect::new(inner.x, inner.y + i as u16, inner.width, 1), buf);
        }
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
            .borders(Borders::ALL)
            .border_style(self.theme.border_style(true))
            .title(title);
        let inner = block.inner(popup);
        block.render(popup, buf);

        if inner.height < 1 {
            return;
        }

        // Label
        Paragraph::new("Paste API key:")
            .style(self.theme.dim_style())
            .render(Rect::new(inner.x, inner.y, inner.width, 1), buf);

        // Masked input
        if inner.height > 1 {
            let masked = if input.len() > 4 {
                format!("{}****", &input[..4])
            } else {
                "*".repeat(input.len())
            };
            Paragraph::new(masked)
                .style(self.theme.text_style())
                .render(Rect::new(inner.x, inner.y + 1, inner.width, 1), buf);
        }

        // Env var hint
        if inner.height > 2 && !env_hint.is_empty() {
            let hint = format!("hint: ${env_hint}");
            Paragraph::new(hint)
                .style(self.theme.dim_style())
                .render(Rect::new(inner.x, inner.y + 2, inner.width, 1), buf);
        }

        // Footer
        if inner.height > 3 {
            let footer = Line::from(vec![
                Span::styled("[Enter: Save]", self.theme.accent_style()),
                Span::raw("  "),
                Span::styled("[Esc: Back]", self.theme.muted_style()),
            ]);
            Paragraph::new(footer).render(Rect::new(inner.x, inner.y + 3, inner.width, 1), buf);
        }
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
            .borders(Borders::ALL)
            .border_style(self.theme.border_style(true))
            .title(title);
        let inner = block.inner(popup);
        block.render(popup, buf);

        if inner.height < 1 {
            return;
        }

        Paragraph::new("Opening browser...")
            .style(self.theme.text_style())
            .render(Rect::new(inner.x, inner.y, inner.width, 1), buf);

        if inner.height > 1 {
            let url_display = match url {
                Some(u) if u.len() > 50 => format!("{}…", &u[..50]),
                Some(u) => u.to_owned(),
                None => String::new(),
            };
            if !url_display.is_empty() {
                Paragraph::new("If browser didn't open:")
                    .style(self.theme.dim_style())
                    .render(Rect::new(inner.x, inner.y + 1, inner.width, 1), buf);
                if inner.height > 2 {
                    Paragraph::new(url_display)
                        .style(self.theme.accent_style())
                        .render(Rect::new(inner.x, inner.y + 2, inner.width, 1), buf);
                }
            }
        }

        if inner.height > 3 {
            Paragraph::new("Waiting for redirect...")
                .style(self.theme.dim_style())
                .render(Rect::new(inner.x, inner.y + 3, inner.width, 1), buf);
        }

        if inner.height > 4 {
            Paragraph::new("[Esc: Cancel]")
                .style(self.theme.muted_style())
                .render(Rect::new(inner.x, inner.y + 4, inner.width, 1), buf);
        }
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
            .borders(Borders::ALL)
            .border_style(self.theme.border_style(true))
            .title(title);
        let inner = block.inner(popup);
        block.render(popup, buf);

        if inner.height < 1 {
            return;
        }

        let open_line = format!("Open: {verification_uri}");
        Paragraph::new(open_line)
            .style(self.theme.text_style())
            .render(Rect::new(inner.x, inner.y, inner.width, 1), buf);

        if inner.height > 1 {
            let code_line = format!("Code: {user_code}");
            Paragraph::new(code_line)
                .style(self.theme.accent_style().add_modifier(Modifier::BOLD))
                .render(Rect::new(inner.x, inner.y + 1, inner.width, 1), buf);
        }

        if inner.height > 2 {
            Paragraph::new("Waiting for authorization...")
                .style(self.theme.dim_style())
                .render(Rect::new(inner.x, inner.y + 2, inner.width, 1), buf);
        }

        if inner.height > 3 {
            Paragraph::new("[Esc: Cancel]")
                .style(self.theme.muted_style())
                .render(Rect::new(inner.x, inner.y + 3, inner.width, 1), buf);
        }
    }

    fn render_verifying(&self, display_name: &str, area: Rect, buf: &mut Buffer) {
        let popup = centered_rect(40, 20, area);
        Clear.render(popup, buf);

        let title = format!(" {display_name} ");
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border_style(true))
            .title(title);
        let inner = block.inner(popup);
        block.render(popup, buf);

        if inner.height < 1 {
            return;
        }

        Paragraph::new("Verifying credentials...")
            .style(self.theme.dim_style())
            .render(Rect::new(inner.x, inner.y, inner.width, 1), buf);
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_width = area.width * percent_x / 100;
    let popup_height = area.height * percent_y / 100;
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    Rect::new(x, y, popup_width, popup_height)
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

    #[test]
    fn test_select_provider_single_method_skips_picker() {
        // GitHub Copilot only has DeviceCode — should skip picker.
        let mut state = ConnectModalState::new();
        state.open(&empty_statuses());

        let copilot = state
            .providers
            .iter()
            .find(|p| p.id == "github-copilot")
            .unwrap()
            .clone();

        state.select_provider(&copilot);

        assert!(
            matches!(
                &state.phase,
                ConnectPhase::DeviceCode { provider_id, .. } if provider_id == "github-copilot"
            ),
            "expected DeviceCode phase, got {:?}",
            state.phase
        );
    }

    #[test]
    fn test_select_provider_multiple_methods_shows_picker() {
        // Anthropic has BrowserOAuth (×2) + ApiKey → 3 methods → show picker.
        let mut state = ConnectModalState::new();
        state.open(&empty_statuses());

        let anthropic = state
            .providers
            .iter()
            .find(|p| p.id == "anthropic")
            .unwrap()
            .clone();

        state.select_provider(&anthropic);

        match &state.phase {
            ConnectPhase::MethodPicker {
                provider_id,
                methods,
                selected,
            } => {
                assert_eq!(provider_id, "anthropic");
                assert_eq!(*selected, 0);
                assert!(
                    methods.contains(&"Browser login (Max)".to_owned()),
                    "missing Max option"
                );
                assert!(
                    methods.contains(&"Browser login (Console)".to_owned()),
                    "missing Console option"
                );
                assert!(methods.contains(&"API key".to_owned()), "missing API key");
            }
            other => panic!("expected MethodPicker, got {other:?}"),
        }
    }

    #[test]
    fn test_select_provider_api_key_only() {
        // Groq only has ApiKey → skip picker, go straight to ApiKeyEntry.
        let mut state = ConnectModalState::new();
        state.open(&empty_statuses());

        let groq = state
            .providers
            .iter()
            .find(|p| p.id == "groq")
            .unwrap()
            .clone();

        state.select_provider(&groq);

        assert!(
            matches!(
                &state.phase,
                ConnectPhase::ApiKeyEntry { provider_id, env_hint, .. }
                    if provider_id == "groq" && env_hint == "GROQ_API_KEY"
            ),
            "expected ApiKeyEntry phase, got {:?}",
            state.phase
        );
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

        // Should be in MethodPicker with 3 methods.
        let method_count = match &state.phase {
            ConnectPhase::MethodPicker { methods, .. } => methods.len(),
            _ => panic!("expected MethodPicker"),
        };
        assert_eq!(method_count, 3);

        // Start at 0.
        assert!(matches!(
            &state.phase,
            ConnectPhase::MethodPicker { selected: 0, .. }
        ));

        // Up at 0 saturates.
        state.method_up();
        assert!(matches!(
            &state.phase,
            ConnectPhase::MethodPicker { selected: 0, .. }
        ));

        // Down twice.
        state.method_down();
        state.method_down();
        assert!(matches!(
            &state.phase,
            ConnectPhase::MethodPicker { selected: 2, .. }
        ));

        // Down again clamps at last.
        state.method_down();
        assert!(matches!(
            &state.phase,
            ConnectPhase::MethodPicker { selected: 2, .. }
        ));

        // Up once.
        state.method_up();
        assert!(matches!(
            &state.phase,
            ConnectPhase::MethodPicker { selected: 1, .. }
        ));
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

        // Should be in ApiKeyEntry.
        assert!(matches!(&state.phase, ConnectPhase::ApiKeyEntry { .. }));

        // Insert characters.
        for c in "abc".chars() {
            state.api_key_insert_char(c);
        }
        match &state.phase {
            ConnectPhase::ApiKeyEntry { input, cursor, .. } => {
                assert_eq!(input, "abc");
                assert_eq!(*cursor, 3);
            }
            _ => panic!("expected ApiKeyEntry"),
        }

        // Delete one character.
        state.api_key_delete_char();
        match &state.phase {
            ConnectPhase::ApiKeyEntry { input, cursor, .. } => {
                assert_eq!(input, "ab");
                assert_eq!(*cursor, 2);
            }
            _ => panic!("expected ApiKeyEntry"),
        }

        // Delete at start is a no-op.
        state.api_key_delete_char();
        state.api_key_delete_char();
        state.api_key_delete_char(); // cursor now at 0, this is a no-op
        match &state.phase {
            ConnectPhase::ApiKeyEntry { input, cursor, .. } => {
                assert_eq!(input, "");
                assert_eq!(*cursor, 0);
            }
            _ => panic!("expected ApiKeyEntry"),
        }
    }

    // -- widget render smoke tests ------------------------------------------

    fn make_terminal() -> ratatui::Terminal<ratatui::backend::TestBackend> {
        let backend = ratatui::backend::TestBackend::new(120, 40);
        ratatui::Terminal::new(backend).unwrap()
    }

    #[test]
    fn connect_modal_hidden_renders_nothing() {
        let mut terminal = make_terminal();
        let state = ConnectModalState::new(); // visible = false
        let theme = crate::theme::UcodeTheme::default();
        terminal
            .draw(|f| f.render_widget(ConnectModal::new(&state, &theme), f.area()))
            .unwrap();
    }

    #[test]
    fn connect_modal_provider_list_renders() {
        let mut terminal = make_terminal();
        let mut state = ConnectModalState::new();
        state.open(&empty_statuses());
        let theme = crate::theme::UcodeTheme::default();
        terminal
            .draw(|f| f.render_widget(ConnectModal::new(&state, &theme), f.area()))
            .unwrap();
    }

    #[test]
    fn connect_modal_provider_list_with_connected_renders() {
        let mut terminal = make_terminal();
        let statuses = statuses_with("anthropic", "api_key");
        let mut state = ConnectModalState::new();
        state.open(&statuses);
        let theme = crate::theme::UcodeTheme::default();
        terminal
            .draw(|f| f.render_widget(ConnectModal::new(&state, &theme), f.area()))
            .unwrap();
    }

    #[test]
    fn connect_modal_method_picker_renders() {
        let mut terminal = make_terminal();
        let mut state = ConnectModalState::new();
        state.open(&empty_statuses());
        let anthropic = state
            .providers
            .iter()
            .find(|p| p.id == "anthropic")
            .unwrap()
            .clone();
        state.select_provider(&anthropic);
        let theme = crate::theme::UcodeTheme::default();
        terminal
            .draw(|f| f.render_widget(ConnectModal::new(&state, &theme), f.area()))
            .unwrap();
    }

    #[test]
    fn connect_modal_api_key_entry_renders() {
        let mut terminal = make_terminal();
        let mut state = ConnectModalState::new();
        state.open(&empty_statuses());
        let groq = state
            .providers
            .iter()
            .find(|p| p.id == "groq")
            .unwrap()
            .clone();
        state.select_provider(&groq);
        for c in "sk-test1234".chars() {
            state.api_key_insert_char(c);
        }
        let theme = crate::theme::UcodeTheme::default();
        terminal
            .draw(|f| f.render_widget(ConnectModal::new(&state, &theme), f.area()))
            .unwrap();
    }

    #[test]
    fn connect_modal_browser_oauth_renders() {
        let mut terminal = make_terminal();
        let mut state = ConnectModalState::new();
        state.open(&empty_statuses());
        state.phase = ConnectPhase::BrowserOAuth {
            provider_id: "anthropic".into(),
            display_name: "Anthropic".into(),
            url: Some("https://console.anthropic.com/oauth/authorize?code=abc123".into()),
        };
        state.visible = true;
        let theme = crate::theme::UcodeTheme::default();
        terminal
            .draw(|f| f.render_widget(ConnectModal::new(&state, &theme), f.area()))
            .unwrap();
    }

    #[test]
    fn connect_modal_browser_oauth_no_url_renders() {
        let mut terminal = make_terminal();
        let mut state = ConnectModalState::new();
        state.open(&empty_statuses());
        state.phase = ConnectPhase::BrowserOAuth {
            provider_id: "anthropic".into(),
            display_name: "Anthropic".into(),
            url: None,
        };
        state.visible = true;
        let theme = crate::theme::UcodeTheme::default();
        terminal
            .draw(|f| f.render_widget(ConnectModal::new(&state, &theme), f.area()))
            .unwrap();
    }

    #[test]
    fn connect_modal_device_code_renders() {
        let mut terminal = make_terminal();
        let mut state = ConnectModalState::new();
        state.open(&empty_statuses());
        state.phase = ConnectPhase::DeviceCode {
            provider_id: "github-copilot".into(),
            display_name: "GitHub Copilot".into(),
            user_code: "ABCD-1234".into(),
            verification_uri: "https://github.com/login/device".into(),
        };
        state.visible = true;
        let theme = crate::theme::UcodeTheme::default();
        terminal
            .draw(|f| f.render_widget(ConnectModal::new(&state, &theme), f.area()))
            .unwrap();
    }

    #[test]
    fn connect_modal_verifying_renders() {
        let mut terminal = make_terminal();
        let mut state = ConnectModalState::new();
        state.open(&empty_statuses());
        state.phase = ConnectPhase::Verifying {
            provider_id: "groq".into(),
            display_name: "Groq".into(),
        };
        state.visible = true;
        let theme = crate::theme::UcodeTheme::default();
        terminal
            .draw(|f| f.render_widget(ConnectModal::new(&state, &theme), f.area()))
            .unwrap();
    }
}
