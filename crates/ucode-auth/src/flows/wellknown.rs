use crate::credential::AuthMaterial;
use crate::error::AuthError;

/// Authorize via a well-known endpoint that provides an auth command.
pub async fn wellknown_authorize(_base_url: &str) -> Result<AuthMaterial, AuthError> {
    todo!("implement in Task 2.3d")
}
