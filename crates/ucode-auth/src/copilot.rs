//! GitHub Copilot token exchange.
//!
//! Exchanges a GitHub OAuth token (`gho_xxx`) for a short-lived
//! Copilot API bearer token via the internal Copilot token endpoint.

use reqwest::Client;
use serde::Deserialize;

use crate::credential::AuthMaterial;
use crate::error::AuthError;

/// Default Copilot token exchange endpoint.
pub const COPILOT_TOKEN_URL: &str = "https://api.github.com/copilot_internal/v2/token";

/// Response from the Copilot token exchange endpoint.
#[derive(Debug, Deserialize)]
struct CopilotTokenResponse {
    /// The short-lived bearer token for Copilot API requests.
    token: String,
    /// Unix timestamp when the token expires.
    expires_at: i64,
}

/// Exchange a GitHub OAuth token for a Copilot API bearer token.
///
/// The returned `AuthMaterial::SessionToken` contains the short-lived
/// Copilot bearer token with its expiry time.
pub async fn exchange_copilot_token(
    client: &Client,
    github_token: &str,
) -> Result<AuthMaterial, AuthError> {
    let resp = client
        .get(COPILOT_TOKEN_URL)
        .header("Authorization", format!("token {github_token}"))
        .header("Accept", "application/json")
        .header("User-Agent", "ucode/0.1")
        .send()
        .await
        .map_err(|e| AuthError::Http {
            message: e.to_string(),
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(AuthError::AuthFlow {
            message: format!("Copilot token exchange failed ({status}): {body}"),
        });
    }

    let r: CopilotTokenResponse = resp.json().await.map_err(|e| AuthError::AuthFlow {
        message: format!("failed to parse Copilot token response: {e}"),
    })?;

    // Convert unix timestamp to RFC 3339
    let expires_at = chrono::DateTime::from_timestamp(r.expires_at, 0).map(|dt| dt.to_rfc3339());

    Ok(AuthMaterial::SessionToken {
        token: r.token,
        expires_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copilot_token_url_constant() {
        assert_eq!(
            COPILOT_TOKEN_URL,
            "https://api.github.com/copilot_internal/v2/token"
        );
    }

    #[test]
    fn copilot_token_response_deserialize() {
        let json = r#"{"token":"tid_abc123","expires_at":1709769600}"#;
        let r: CopilotTokenResponse = serde_json::from_str(json).unwrap();
        assert_eq!(r.token, "tid_abc123");
        assert_eq!(r.expires_at, 1709769600);
    }

    #[test]
    fn copilot_token_response_with_extra_fields() {
        let json = r#"{"token":"tid_abc","expires_at":1709769600,"endpoints":{"api":"https://api.githubcopilot.com"},"annotations_enabled":false}"#;
        let r: CopilotTokenResponse = serde_json::from_str(json).unwrap();
        assert_eq!(r.token, "tid_abc");
    }
}
