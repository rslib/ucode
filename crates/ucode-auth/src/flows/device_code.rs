use std::time::{Duration, Instant};

use serde::Deserialize;

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

// ── Internal deserialization types ────────────────────────────────────────────

#[derive(Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    #[serde(default = "default_interval")]
    interval: u64,
    expires_in: u64,
}

fn default_interval() -> u64 {
    5
}

#[derive(Deserialize)]
#[serde(untagged)]
#[allow(dead_code)] // fields kept for spec completeness; used in tests
enum TokenResponse {
    Success {
        access_token: String,
        refresh_token: Option<String>,
        expires_in: Option<u64>,
        token_type: Option<String>,
    },
    Error {
        error: String,
        error_description: Option<String>,
    },
}

// ── Public functions ───────────────────────────────────────────────────────────

/// Request a device code from the authorization server.
pub async fn request_device_code(
    client: &reqwest::Client,
    config: &DeviceCodeConfig,
) -> Result<DeviceCodePending, AuthError> {
    let resp = client
        .post(&config.device_code_url)
        .header("Accept", "application/json")
        .form(&[("client_id", &config.client_id), ("scope", &config.scope)])
        .send()
        .await
        .map_err(|e| AuthError::Http {
            message: e.to_string(),
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(AuthError::AuthFlow {
            message: format!("device code request failed: HTTP {status}: {body}"),
        });
    }

    let parsed: DeviceCodeResponse = resp.json().await.map_err(|e| AuthError::AuthFlow {
        message: format!("failed to parse device code response: {e}"),
    })?;

    Ok(DeviceCodePending {
        user_code: parsed.user_code,
        verification_uri: parsed.verification_uri,
        device_code: parsed.device_code,
        interval: parsed.interval,
        expires_in: parsed.expires_in,
    })
}

/// Poll the token endpoint until authorization completes or times out.
pub async fn poll_for_token(
    client: &reqwest::Client,
    config: &DeviceCodeConfig,
    pending: &DeviceCodePending,
) -> Result<AuthMaterial, AuthError> {
    // Safety margin: start slightly above the server-specified interval.
    let mut interval = Duration::from_secs(pending.interval + 3);
    let deadline = Instant::now() + Duration::from_secs(pending.expires_in);

    loop {
        tokio::time::sleep(interval).await;

        if Instant::now() >= deadline {
            return Err(AuthError::DeviceCodeTimeout);
        }

        let resp = client
            .post(&config.token_url)
            .header("Accept", "application/json")
            .form(&[
                ("client_id", config.client_id.as_str()),
                ("device_code", pending.device_code.as_str()),
                ("grant_type", config.grant_type.as_str()),
            ])
            .send()
            .await
            .map_err(|e| AuthError::Http {
                message: e.to_string(),
            })?;

        let body = resp.text().await.map_err(|e| AuthError::Http {
            message: e.to_string(),
        })?;

        let token_resp: TokenResponse =
            serde_json::from_str(&body).map_err(|e| AuthError::AuthFlow {
                message: format!("failed to parse token response: {e}"),
            })?;

        match token_resp {
            TokenResponse::Success {
                access_token,
                refresh_token,
                ..
            } => {
                return Ok(AuthMaterial::OAuth {
                    access_token,
                    refresh_token,
                    // Task 2.5 handles expiry tracking.
                    expires_at: None,
                });
            }
            TokenResponse::Error { error, .. } => match error.as_str() {
                "authorization_pending" => continue,
                "slow_down" => {
                    interval += Duration::from_secs(5);
                    continue;
                }
                "access_denied" | "expired_token" => return Err(AuthError::AuthDenied),
                other => {
                    return Err(AuthError::AuthFlow {
                        message: format!("token endpoint error: {other}"),
                    });
                }
            },
        }
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_code_response_full() {
        let json = r#"{
            "device_code": "dev-abc",
            "user_code": "ABCD-1234",
            "verification_uri": "https://example.com/activate",
            "interval": 7,
            "expires_in": 1800
        }"#;
        let r: DeviceCodeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(r.device_code, "dev-abc");
        assert_eq!(r.user_code, "ABCD-1234");
        assert_eq!(r.verification_uri, "https://example.com/activate");
        assert_eq!(r.interval, 7);
        assert_eq!(r.expires_in, 1800);
    }

    #[test]
    fn device_code_response_default_interval() {
        let json = r#"{
            "device_code": "dev-xyz",
            "user_code": "WXYZ-5678",
            "verification_uri": "https://example.com/activate",
            "expires_in": 900
        }"#;
        let r: DeviceCodeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(r.interval, 5, "missing interval should default to 5");
        assert_eq!(r.expires_in, 900);
    }

    #[test]
    fn token_response_success() {
        let json = r#"{
            "access_token": "tok-abc",
            "refresh_token": "ref-xyz",
            "expires_in": 3600,
            "token_type": "Bearer"
        }"#;
        let r: TokenResponse = serde_json::from_str(json).unwrap();
        match r {
            TokenResponse::Success {
                access_token,
                refresh_token,
                expires_in,
                token_type,
            } => {
                assert_eq!(access_token, "tok-abc");
                assert_eq!(refresh_token.as_deref(), Some("ref-xyz"));
                assert_eq!(expires_in, Some(3600));
                assert_eq!(token_type.as_deref(), Some("Bearer"));
            }
            TokenResponse::Error { .. } => panic!("expected Success variant"),
        }
    }

    #[test]
    fn token_response_success_minimal() {
        let json = r#"{"access_token": "tok-min"}"#;
        let r: TokenResponse = serde_json::from_str(json).unwrap();
        match r {
            TokenResponse::Success {
                access_token,
                refresh_token,
                expires_in,
                token_type,
            } => {
                assert_eq!(access_token, "tok-min");
                assert!(refresh_token.is_none());
                assert!(expires_in.is_none());
                assert!(token_type.is_none());
            }
            TokenResponse::Error { .. } => panic!("expected Success variant"),
        }
    }

    #[test]
    fn token_response_error_authorization_pending() {
        let json = r#"{"error": "authorization_pending", "error_description": "Still waiting"}"#;
        let r: TokenResponse = serde_json::from_str(json).unwrap();
        match r {
            TokenResponse::Error {
                error,
                error_description,
            } => {
                assert_eq!(error, "authorization_pending");
                assert_eq!(error_description.as_deref(), Some("Still waiting"));
            }
            TokenResponse::Success { .. } => panic!("expected Error variant"),
        }
    }

    #[test]
    fn token_response_error_slow_down() {
        let json = r#"{"error": "slow_down"}"#;
        let r: TokenResponse = serde_json::from_str(json).unwrap();
        match r {
            TokenResponse::Error { error, .. } => assert_eq!(error, "slow_down"),
            TokenResponse::Success { .. } => panic!("expected Error variant"),
        }
    }

    #[test]
    fn token_response_error_access_denied() {
        let json = r#"{"error": "access_denied", "error_description": "User denied"}"#;
        let r: TokenResponse = serde_json::from_str(json).unwrap();
        match r {
            TokenResponse::Error { error, .. } => assert_eq!(error, "access_denied"),
            TokenResponse::Success { .. } => panic!("expected Error variant"),
        }
    }

    #[test]
    fn device_code_config_construction() {
        let cfg = DeviceCodeConfig {
            client_id: "my-client".into(),
            device_code_url: "https://auth.example.com/device".into(),
            token_url: "https://auth.example.com/token".into(),
            scope: "read write".into(),
            grant_type: "urn:ietf:params:oauth:grant-type:device_code".into(),
        };
        assert_eq!(cfg.client_id, "my-client");
        assert_eq!(
            cfg.grant_type,
            "urn:ietf:params:oauth:grant-type:device_code"
        );
    }

    #[test]
    fn device_code_pending_construction() {
        let pending = DeviceCodePending {
            user_code: "ABCD-1234".into(),
            verification_uri: "https://example.com/activate".into(),
            device_code: "dev-secret".into(),
            interval: 5,
            expires_in: 1800,
        };
        assert_eq!(pending.user_code, "ABCD-1234");
        assert_eq!(pending.interval, 5);
        assert_eq!(pending.expires_in, 1800);
    }
}
