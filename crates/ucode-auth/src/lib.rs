//! ucode-auth: keychain, login flows, token refresh

pub mod chain_store;
pub mod credential;
pub mod error;
pub mod file_store;
pub mod flows;
pub mod resolve;

pub use chain_store::ChainStore;
pub use credential::{
    AuthMaterial, CredentialStatus, CredentialStore, InMemoryStore, KeyringStore, ProviderType,
    redact,
};
pub use error::AuthError;
pub use file_store::FileStore;
pub use flows::{
    browser_oauth::{BrowserOAuthConfig, browser_oauth_authorize},
    device_code::{DeviceCodeConfig, DeviceCodePending, poll_for_token, request_device_code},
    wellknown::wellknown_authorize,
};
pub use resolve::resolve_auth;
