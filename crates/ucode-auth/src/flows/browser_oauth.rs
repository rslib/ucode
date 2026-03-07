use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt as _;
use tokio::io::AsyncWriteExt as _;
use tokio::net::TcpListener;

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

// ── PKCE helpers ──────────────────────────────────────────────────────────────

fn generate_code_verifier() -> String {
    let mut rng = rand::rng();
    let bytes: Vec<u8> = (0..32).map(|_| rand::Rng::random::<u8>(&mut rng)).collect();
    URL_SAFE_NO_PAD.encode(&bytes)
}

fn generate_code_challenge(verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(hasher.finalize())
}

// ── Authorization URL builder ─────────────────────────────────────────────────

fn build_auth_url(
    config: &BrowserOAuthConfig,
    code_challenge: &str,
    state: &str,
) -> Result<String, AuthError> {
    let mut url = url::Url::parse(&config.auth_url).map_err(|e| AuthError::AuthFlow {
        message: format!("invalid auth_url: {e}"),
    })?;

    let redirect_uri = format!("http://127.0.0.1:{}", config.redirect_port);

    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &config.client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("scope", &config.scope)
        .append_pair("code_challenge", code_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", state);

    Ok(url.to_string())
}

// ── Callback server ───────────────────────────────────────────────────────────

async fn wait_for_callback(
    listener: TcpListener,
    expected_state: &str,
) -> Result<String, AuthError> {
    let (mut stream, _) = listener.accept().await.map_err(|e| AuthError::AuthFlow {
        message: format!("failed to accept callback connection: {e}"),
    })?;

    let mut buf = [0u8; 4096];
    let n = stream
        .read(&mut buf)
        .await
        .map_err(|e| AuthError::AuthFlow {
            message: format!("failed to read callback request: {e}"),
        })?;

    let request = std::str::from_utf8(&buf[..n]).unwrap_or("");

    // Extract the request path from "GET /path?query HTTP/1.1"
    let path_and_query = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");

    // Parse as a URL to extract query parameters (prepend a dummy base).
    let full_url = format!("http://localhost{path_and_query}");
    let parsed = url::Url::parse(&full_url).map_err(|e| AuthError::AuthFlow {
        message: format!("failed to parse callback URL: {e}"),
    })?;

    let mut code: Option<String> = None;
    let mut state: Option<String> = None;
    let mut error: Option<String> = None;

    for (key, value) in parsed.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            "error" => error = Some(value.into_owned()),
            _ => {}
        }
    }

    // Send the browser response before checking errors so the user sees feedback.
    let html = "<html><body><h1>Authorization successful!</h1>\
                <p>You can close this tab.</p></body></html>";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    );
    // Best-effort write; ignore errors — the auth result is what matters.
    let _ = stream.write_all(response.as_bytes()).await;

    if error.is_some() {
        return Err(AuthError::AuthDenied);
    }

    match state {
        Some(s) if s == expected_state => {}
        _ => return Err(AuthError::AuthDenied),
    }

    code.ok_or(AuthError::AuthDenied)
}

// ── Token exchange ────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    #[allow(dead_code)]
    expires_in: Option<u64>,
}

async fn exchange_code(
    config: &BrowserOAuthConfig,
    code: &str,
    code_verifier: &str,
) -> Result<AuthMaterial, AuthError> {
    let redirect_uri = format!("http://127.0.0.1:{}", config.redirect_port);

    let client = reqwest::Client::new();
    let resp = client
        .post(&config.token_url)
        .header("Accept", "application/json")
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", &config.client_id),
            ("code", code),
            ("redirect_uri", &redirect_uri),
            ("code_verifier", code_verifier),
        ])
        .send()
        .await
        .map_err(|e| AuthError::Http {
            message: e.to_string(),
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(AuthError::Http {
            message: format!("token exchange failed: HTTP {status}: {body}"),
        });
    }

    let token: TokenResponse = resp.json().await.map_err(|e| AuthError::AuthFlow {
        message: format!("failed to parse token response: {e}"),
    })?;

    Ok(AuthMaterial::OAuth {
        access_token: token.access_token,
        refresh_token: token.refresh_token,
        expires_at: None,
    })
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Perform browser-based OAuth authorization with PKCE.
pub async fn browser_oauth_authorize(
    config: &BrowserOAuthConfig,
) -> Result<AuthMaterial, AuthError> {
    let code_verifier = generate_code_verifier();
    let code_challenge = generate_code_challenge(&code_verifier);

    // 16 random bytes → 32-char hex state
    let mut rng = rand::rng();
    let state_bytes: [u8; 16] = std::array::from_fn(|_| rand::Rng::random::<u8>(&mut rng));
    let state = hex_encode(&state_bytes);

    let auth_url = build_auth_url(config, &code_challenge, &state)?;

    // Bind BEFORE opening the browser so the port is ready.
    let addr = format!("127.0.0.1:{}", config.redirect_port);
    let listener = TcpListener::bind(&addr)
        .await
        .map_err(|e| AuthError::AuthFlow {
            message: format!("failed to bind callback listener on {addr}: {e}"),
        })?;

    open::that(&auth_url).map_err(|e| AuthError::AuthFlow {
        message: format!("failed to open browser: {e}"),
    })?;

    let code = wait_for_callback(listener, &state).await?;
    exchange_code(config, &code, &code_verifier).await
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut s, b| {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
        s
    })
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_verifier_length() {
        // 32 bytes base64url-encoded without padding = ceil(32 * 4/3) = 43 chars
        let v = generate_code_verifier();
        assert_eq!(
            v.len(),
            43,
            "expected 43 base64url chars for 32 bytes, got {}",
            v.len()
        );
    }

    #[test]
    fn code_verifier_is_base64url() {
        let v = generate_code_verifier();
        assert!(
            v.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "verifier contains non-base64url chars: {v}"
        );
        assert!(!v.contains('='), "verifier must not have padding");
    }

    #[test]
    fn code_verifier_unique() {
        let a = generate_code_verifier();
        let b = generate_code_verifier();
        assert_ne!(a, b, "two consecutive verifiers should differ");
    }

    #[test]
    fn code_challenge_known_vector() {
        // RFC 7636 Appendix B test vector:
        //   verifier  = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"
        //   challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = generate_code_challenge(verifier);
        assert_eq!(
            challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
            "PKCE challenge mismatch for RFC 7636 test vector"
        );
    }

    #[test]
    fn build_auth_url_contains_required_params() {
        let config = BrowserOAuthConfig {
            client_id: "my-client".into(),
            auth_url: "https://auth.example.com/authorize".into(),
            token_url: "https://auth.example.com/token".into(),
            scope: "openid profile".into(),
            redirect_port: 8080,
        };
        let url = build_auth_url(&config, "challenge-abc", "state-xyz").unwrap();

        let parsed = url::Url::parse(&url).unwrap();
        let params: std::collections::HashMap<_, _> = parsed.query_pairs().collect();

        assert_eq!(params["response_type"], "code");
        assert_eq!(params["client_id"], "my-client");
        assert_eq!(params["redirect_uri"], "http://127.0.0.1:8080");
        assert_eq!(params["scope"], "openid profile");
        assert_eq!(params["code_challenge"], "challenge-abc");
        assert_eq!(params["code_challenge_method"], "S256");
        assert_eq!(params["state"], "state-xyz");
    }

    #[test]
    fn build_auth_url_encodes_special_chars_in_scope() {
        let config = BrowserOAuthConfig {
            client_id: "c".into(),
            auth_url: "https://auth.example.com/authorize".into(),
            token_url: "https://auth.example.com/token".into(),
            scope: "read:user write:repo".into(),
            redirect_port: 9000,
        };
        let url = build_auth_url(&config, "ch", "st").unwrap();

        // The raw URL string must not contain unencoded spaces or colons in the
        // query value position (url crate percent-encodes them).
        let parsed = url::Url::parse(&url).unwrap();
        let params: std::collections::HashMap<_, _> = parsed.query_pairs().collect();
        assert_eq!(params["scope"], "read:user write:repo");

        // Verify the raw query string has the space percent-encoded.
        let raw_query = parsed.query().unwrap_or("");
        assert!(
            raw_query.contains("read%3Auser") || raw_query.contains("read:user"),
            "scope should be present in query: {raw_query}"
        );
        assert!(
            !raw_query.contains("scope=read:user write:repo"),
            "unencoded space must not appear in raw query: {raw_query}"
        );
    }

    #[test]
    fn build_auth_url_invalid_base_url() {
        let config = BrowserOAuthConfig {
            client_id: "c".into(),
            auth_url: "not a url".into(),
            token_url: "https://t.example.com/token".into(),
            scope: "read".into(),
            redirect_port: 8080,
        };
        assert!(build_auth_url(&config, "ch", "st").is_err());
    }

    #[test]
    fn hex_encode_known() {
        assert_eq!(hex_encode(&[0x00, 0xff, 0x0a]), "00ff0a");
        assert_eq!(hex_encode(&[]), "");
    }
}
