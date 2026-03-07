//! ucode-auth: keychain, login flows, token refresh

pub mod chain_store;
pub mod copilot;
pub mod credential;
pub mod error;
pub mod file_store;
pub mod flows;
pub mod providers;
pub mod refresh;
pub mod resolve;

pub use chain_store::ChainStore;
pub use copilot::{COPILOT_TOKEN_URL, exchange_copilot_token};
pub use credential::{
    AuthMaterial, CredentialStatus, CredentialStore, InMemoryStore, KeyringStore, ProviderType,
    redact,
};
pub use error::AuthError;
pub use file_store::FileStore;
pub use flows::{
    browser_oauth::{
        BrowserOAuthConfig, OAuthPending, browser_oauth_authorize, complete_browser_oauth,
        start_browser_oauth,
    },
    device_code::{DeviceCodeConfig, DeviceCodePending, poll_for_token, request_device_code},
    wellknown::wellknown_authorize,
};
pub use providers::{
    AuthMethod, ProviderAuthInfo, anthropic_console_oauth_config, anthropic_max_oauth_config,
    anthropic_refresh_config, github_copilot_device_config, openai_refresh_config,
    openai_subscription_oauth_config, provider_auth_info,
};
pub use refresh::{RefreshConfig, refresh_oauth_token, resolve_auth_with_refresh};
pub use resolve::resolve_auth;
