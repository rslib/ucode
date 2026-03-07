use serde::Deserialize;

use crate::credential::AuthMaterial;
use crate::error::AuthError;

// ── Internal deserialization types ────────────────────────────────────────────

#[derive(Deserialize)]
struct WellKnownResponse {
    auth: WellKnownAuth,
}

#[derive(Deserialize)]
struct WellKnownAuth {
    command: String,
    env: String,
}

// ── Public functions ───────────────────────────────────────────────────────────

/// Authorize via a well-known endpoint that provides an auth command.
pub async fn wellknown_authorize(base_url: &str) -> Result<AuthMaterial, AuthError> {
    let client = reqwest::Client::new();
    let url = format!("{base_url}/.well-known/opencode");

    let resp = client.get(&url).send().await.map_err(|e| AuthError::Http {
        message: e.to_string(),
    })?;

    if !resp.status().is_success() {
        let status = resp.status();
        return Err(AuthError::AuthFlow {
            message: format!("well-known request failed: HTTP {status}"),
        });
    }

    let wk: WellKnownResponse = resp.json().await.map_err(|e| AuthError::AuthFlow {
        message: format!("failed to parse well-known response: {e}"),
    })?;

    let output = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(&wk.auth.command)
        .output()
        .await
        .map_err(|e| AuthError::AuthFlow {
            message: format!("failed to spawn auth command: {e}"),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        return Err(AuthError::AuthFlow {
            message: format!("auth command failed: {stderr}"),
        });
    }

    let token = String::from_utf8_lossy(&output.stdout).trim().to_owned();

    if token.is_empty() {
        return Err(AuthError::AuthFlow {
            message: "auth command produced no output".into(),
        });
    }

    Ok(AuthMaterial::WellKnown {
        env_key: wk.auth.env,
        token,
    })
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wellknown_response_valid() {
        let json = r#"{"auth": {"command": "echo secret", "env": "MY_API_KEY"}}"#;
        let r: WellKnownResponse = serde_json::from_str(json).unwrap();
        assert_eq!(r.auth.command, "echo secret");
        assert_eq!(r.auth.env, "MY_API_KEY");
    }

    #[test]
    fn wellknown_response_missing_auth_field() {
        let json = r#"{"other": "value"}"#;
        let result: Result<WellKnownResponse, _> = serde_json::from_str(json);
        assert!(result.is_err(), "missing 'auth' field should fail");
    }

    #[test]
    fn wellknown_response_missing_command_field() {
        let json = r#"{"auth": {"env": "MY_API_KEY"}}"#;
        let result: Result<WellKnownResponse, _> = serde_json::from_str(json);
        assert!(result.is_err(), "missing 'command' field should fail");
    }

    #[test]
    fn wellknown_response_missing_env_field() {
        let json = r#"{"auth": {"command": "echo secret"}}"#;
        let result: Result<WellKnownResponse, _> = serde_json::from_str(json);
        assert!(result.is_err(), "missing 'env' field should fail");
    }

    #[test]
    fn wellknown_auth_extra_fields_ignored() {
        let json = r#"{"command": "echo tok", "env": "TOKEN_VAR", "extra": "ignored"}"#;
        let r: WellKnownAuth = serde_json::from_str(json).unwrap();
        assert_eq!(r.command, "echo tok");
        assert_eq!(r.env, "TOKEN_VAR");
    }
}
