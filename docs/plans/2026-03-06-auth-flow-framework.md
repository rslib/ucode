# Auth Flow Framework (Task 2.3) Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Implement generic auth flow functions (device code, browser OAuth, well-known) that providers can use for interactive login.

**Architecture:** Each flow is a standalone async function in `crates/ucode-auth/src/flows/`. No traits — one implementation per flow type. Flows return `AuthMaterial` on success. The caller (CLI `handle_login`) is responsible for storing the result via `CredentialStore`.

**Tech Stack:** Rust, reqwest (HTTP client), tokio (async runtime, TcpListener for OAuth callback), sha2 + base64 (PKCE), open (browser), rand (code_verifier)

---

## Task 1: Add dependencies, error variants, and module structure

**Files:**
- Modify: `crates/ucode-auth/Cargo.toml`
- Modify: `crates/ucode-auth/src/error.rs`
- Create: `crates/ucode-auth/src/flows/mod.rs`
- Modify: `crates/ucode-auth/src/lib.rs`

**Step 1: Add dependencies**

```bash
cargo add reqwest --features json -p ucode-auth
cargo add tokio --features "net time" -p ucode-auth
cargo add rand -p ucode-auth
cargo add sha2 -p ucode-auth
cargo add base64 -p ucode-auth
cargo add open -p ucode-auth
```

Use workspace versions where available (reqwest, tokio, rand).

**Step 2: Add flow-specific error variants to error.rs**

Add to `AuthError` enum:

```rust
#[error("auth flow error: {message}")]
AuthFlow { message: String },

#[error("device code flow timed out")]
DeviceCodeTimeout,

#[error("authorization denied by user")]
AuthDenied,

#[error("HTTP request failed: {message}")]
Http { message: String },
```

**Step 3: Create flows module**

Create `crates/ucode-auth/src/flows/mod.rs`:

```rust
//! Auth flow implementations for interactive login.

pub mod device_code;
pub mod browser_oauth;
pub mod wellknown;

pub use device_code::{DeviceCodeConfig, DeviceCodePending, device_code_authorize};
pub use browser_oauth::{BrowserOAuthConfig, browser_oauth_authorize};
pub use wellknown::wellknown_authorize;
```

**Step 4: Update lib.rs**

Add `pub mod flows;` and re-export the flow types.

**Step 5: Build (will fail — flow submodules don't exist yet)**

Create empty placeholder files so the module structure compiles:
- `crates/ucode-auth/src/flows/device_code.rs` (empty structs + stub function)
- `crates/ucode-auth/src/flows/browser_oauth.rs` (empty structs + stub function)
- `crates/ucode-auth/src/flows/wellknown.rs` (stub function)

**Step 6: Verify build**

Run: `cargo build -p ucode-auth`

**Step 7: Commit**

```
feat(auth): add flows module structure and auth flow error variants
```

---

## Task 2: Device code flow (RFC 8628)

**Files:**
- Modify: `crates/ucode-auth/src/flows/device_code.rs`
- Create: `crates/ucode-auth/tests/device_code_tests.rs`

**Implementation:**

```rust
use reqwest::Client;
use serde::Deserialize;
use tokio::time::{Duration, sleep};

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

#[derive(Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    #[serde(default = "default_interval")]
    interval: u64,
    expires_in: u64,
}

fn default_interval() -> u64 { 5 }

#[derive(Deserialize)]
#[serde(untagged)]
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

/// Request a device code from the authorization server.
pub async fn request_device_code(
    client: &Client,
    config: &DeviceCodeConfig,
) -> Result<DeviceCodePending, AuthError> {
    let resp = client
        .post(&config.device_code_url)
        .header("Accept", "application/json")
        .form(&[
            ("client_id", &config.client_id),
            ("scope", &config.scope),
        ])
        .send()
        .await
        .map_err(|e| AuthError::Http { message: e.to_string() })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(AuthError::AuthFlow {
            message: format!("device code request failed ({status}): {body}"),
        });
    }

    let dc: DeviceCodeResponse = resp
        .json()
        .await
        .map_err(|e| AuthError::AuthFlow { message: e.to_string() })?;

    Ok(DeviceCodePending {
        user_code: dc.user_code,
        verification_uri: dc.verification_uri,
        device_code: dc.device_code,
        interval: dc.interval,
        expires_in: dc.expires_in,
    })
}

/// Poll the token endpoint until authorization completes or times out.
pub async fn poll_for_token(
    client: &Client,
    config: &DeviceCodeConfig,
    pending: &DeviceCodePending,
) -> Result<AuthMaterial, AuthError> {
    let mut interval = Duration::from_secs(pending.interval + 3); // safety margin
    let deadline = tokio::time::Instant::now() + Duration::from_secs(pending.expires_in);

    loop {
        sleep(interval).await;

        if tokio::time::Instant::now() >= deadline {
            return Err(AuthError::DeviceCodeTimeout);
        }

        let resp = client
            .post(&config.token_url)
            .header("Accept", "application/json")
            .form(&[
                ("client_id", &config.client_id),
                ("device_code", &pending.device_code),
                ("grant_type", &config.grant_type),
            ])
            .send()
            .await
            .map_err(|e| AuthError::Http { message: e.to_string() })?;

        let body = resp
            .text()
            .await
            .map_err(|e| AuthError::Http { message: e.to_string() })?;

        let token_resp: TokenResponse = serde_json::from_str(&body)
            .map_err(|e| AuthError::AuthFlow {
                message: format!("parse token response: {e}"),
            })?;

        match token_resp {
            TokenResponse::Success {
                access_token,
                refresh_token,
                expires_in,
                ..
            } => {
                let expires_at = expires_in.map(|secs| {
                    let dt = std::time::SystemTime::now()
                        + Duration::from_secs(secs);
                    humantime::format_rfc3339(dt).to_string()
                });
                return Ok(AuthMaterial::OAuth {
                    access_token,
                    refresh_token,
                    expires_at,
                });
            }
            TokenResponse::Error { error, .. } => match error.as_str() {
                "authorization_pending" => continue,
                "slow_down" => {
                    interval += Duration::from_secs(5);
                    continue;
                }
                "access_denied" | "expired_token" => {
                    return Err(AuthError::AuthDenied);
                }
                _ => {
                    return Err(AuthError::AuthFlow {
                        message: format!("token error: {error}"),
                    });
                }
            },
        }
    }
}

/// Full device code authorization flow.
///
/// 1. Request device code
/// 2. Return pending info (caller displays to user)
/// 3. Poll for token
pub async fn device_code_authorize(
    config: &DeviceCodeConfig,
) -> Result<(DeviceCodePending, AuthMaterial), AuthError> {
    let client = Client::new();
    let pending = request_device_code(&client, config).await?;
    let material = poll_for_token(&client, config, &pending).await?;
    Ok((pending, material))
}
```

Wait — the function signature in the plan returns `Result<AuthMaterial>` but the caller needs the `DeviceCodePending` to display the user_code. Let me split this into two steps:
1. `request_device_code()` → returns `DeviceCodePending` (caller displays)
2. `poll_for_token()` → returns `AuthMaterial` (caller stores)

The combined `device_code_authorize()` can exist as a convenience but the CLI will use the two-step version.

**Note on humantime:** We need a way to format timestamps. Options:
- `humantime` crate for RFC 3339
- Manual formatting with chrono
- Just store seconds-since-epoch as string

Simplest: compute the expiry as seconds from epoch and format as ISO 8601 manually, or just use `chrono`. But we don't have chrono. Let's avoid adding it — store `expires_in` as the raw seconds value and let the caller compute the timestamp, or just store None for now and handle expiry in Task 2.5.

Actually, simpler: just store the `expires_in` as a relative value in the `expires_at` field for now. The token refresh task (2.5) will handle proper expiry tracking.

**Tests:**

```rust
// Test DeviceCodeConfig construction
// Test DeviceCodeResponse deserialization
// Test TokenResponse deserialization (success + error variants)
// Test slow_down increases interval
```

We can test the deserialization logic without a real HTTP server.

**Step 1: Commit**

```
feat(auth): implement device code flow (RFC 8628)
```

---

## Task 3: Browser OAuth flow (PKCE)

**Files:**
- Modify: `crates/ucode-auth/src/flows/browser_oauth.rs`
- Create: `crates/ucode-auth/tests/browser_oauth_tests.rs`

**Implementation:**

PKCE helpers:
```rust
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::Rng;
use sha2::{Sha256, Digest};

fn generate_code_verifier() -> String {
    let mut rng = rand::rng();
    let bytes: Vec<u8> = (0..32).map(|_| rng.random::<u8>()).collect();
    URL_SAFE_NO_PAD.encode(&bytes)
}

fn generate_code_challenge(verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let hash = hasher.finalize();
    URL_SAFE_NO_PAD.encode(&hash)
}
```

OAuth callback server (minimal):
```rust
async fn wait_for_callback(port: u16) -> Result<String, AuthError> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|e| AuthError::AuthFlow {
            message: format!("bind callback server: {e}"),
        })?;

    let (mut stream, _) = listener.accept().await...;
    // Read HTTP request, extract ?code= parameter
    // Send back a simple HTML response
    // Return the code
}
```

**Tests:**
- Test PKCE code_verifier length and charset
- Test code_challenge is correct SHA-256 of verifier
- Test URL construction with all parameters

**Step 1: Commit**

```
feat(auth): implement browser OAuth flow with PKCE
```

---

## Task 4: Well-known auth flow

**Files:**
- Modify: `crates/ucode-auth/src/flows/wellknown.rs`
- Create: `crates/ucode-auth/tests/wellknown_tests.rs`

**Implementation:**

```rust
use reqwest::Client;
use serde::Deserialize;
use tokio::process::Command;

use crate::credential::AuthMaterial;
use crate::error::AuthError;

#[derive(Deserialize)]
struct WellKnownResponse {
    auth: WellKnownAuth,
}

#[derive(Deserialize)]
struct WellKnownAuth {
    command: String,
    env: String,
}

pub async fn wellknown_authorize(base_url: &str) -> Result<AuthMaterial, AuthError> {
    let client = Client::new();
    let url = format!("{base_url}/.well-known/opencode");

    let resp = client.get(&url).send().await
        .map_err(|e| AuthError::Http { message: e.to_string() })?;

    if !resp.status().is_success() {
        return Err(AuthError::AuthFlow {
            message: format!("well-known endpoint returned {}", resp.status()),
        });
    }

    let wk: WellKnownResponse = resp.json().await
        .map_err(|e| AuthError::AuthFlow { message: e.to_string() })?;

    // Run the auth command
    let output = Command::new("sh")
        .arg("-c")
        .arg(&wk.auth.command)
        .output()
        .await
        .map_err(|e| AuthError::AuthFlow {
            message: format!("run auth command: {e}"),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AuthError::AuthFlow {
            message: format!("auth command failed: {stderr}"),
        });
    }

    let token = String::from_utf8_lossy(&output.stdout).trim().to_owned();

    Ok(AuthMaterial::WellKnown {
        env_key: wk.auth.env,
        token,
    })
}
```

**Tests:**
- Test WellKnownResponse deserialization
- Test with various JSON formats

**Step 1: Commit**

```
feat(auth): implement well-known auth flow
```

---

## Task 5: Workspace verification

Run: `cargo build && cargo test && cargo clippy`

Verify all flows compile and existing tests still pass.

---

## Summary

| Task | What | New deps | Tests |
|------|------|----------|-------|
| 1 | Module structure + error variants | reqwest, tokio, rand, sha2, base64, open | build check |
| 2 | Device code flow (RFC 8628) | — | response parsing, error handling |
| 3 | Browser OAuth (PKCE) | — | PKCE generation, URL construction |
| 4 | Well-known auth | — | JSON parsing |
| 5 | Verification | — | full suite |
