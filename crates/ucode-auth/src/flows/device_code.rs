use crate::credential::AuthMaterial;
use crate::error::AuthError;

/// Configuration for a device code authorization flow.
pub struct DeviceCodeConfig {
    pub client_id: String,
    pub device_code_url: String,
    pub token_url: String,
    pub scope: String,
    pub grant_type: String,
}

/// Pending device code authorization — display to user.
pub struct DeviceCodePending {
    pub user_code: String,
    pub verification_uri: String,
    pub device_code: String,
    pub interval: u64,
    pub expires_in: u64,
}

/// Request a device code from the authorization server.
pub async fn request_device_code(
    _client: &reqwest::Client,
    _config: &DeviceCodeConfig,
) -> Result<DeviceCodePending, AuthError> {
    todo!("implement in Task 2.3b")
}

/// Poll the token endpoint until authorization completes or times out.
pub async fn poll_for_token(
    _client: &reqwest::Client,
    _config: &DeviceCodeConfig,
    _pending: &DeviceCodePending,
) -> Result<AuthMaterial, AuthError> {
    todo!("implement in Task 2.3b")
}
