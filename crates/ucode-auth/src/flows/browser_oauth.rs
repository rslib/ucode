use crate::credential::AuthMaterial;
use crate::error::AuthError;

/// Configuration for browser-based OAuth with PKCE.
pub struct BrowserOAuthConfig {
    pub client_id: String,
    pub auth_url: String,
    pub token_url: String,
    pub scope: String,
    pub redirect_port: u16,
}

/// Perform browser-based OAuth authorization with PKCE.
pub async fn browser_oauth_authorize(
    _config: &BrowserOAuthConfig,
) -> Result<AuthMaterial, AuthError> {
    todo!("implement in Task 2.3c")
}
