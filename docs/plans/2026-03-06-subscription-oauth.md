# Subscription OAuth (OpenAI + Anthropic) Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add browser-based OAuth login for OpenAI (ChatGPT Plus/Pro via Codex) and Anthropic (Claude Max) subscriptions, reusing existing PKCE infrastructure.

**Architecture:** Extend `BrowserOAuthConfig` with optional `extra_params` and `redirect_uri` fields. Add a manual-code flow variant for providers (like Anthropic) whose redirect URI is not localhost. Add provider-specific config constructors and wire them into the CLI login handler.

**Tech Stack:** Rust, tokio, reqwest, url, sha2, base64, chrono

---

### Task 1: Extend BrowserOAuthConfig with extra_params and redirect_uri

**Files:**
- Modify: `crates/ucode-auth/src/flows/browser_oauth.rs`

**Step 1: Add fields to BrowserOAuthConfig**

Add two new optional fields:

```rust
pub struct BrowserOAuthConfig {
    pub client_id: String,
    pub auth_url: String,
    pub token_url: String,
    pub scope: String,
    pub redirect_port: u16,
    /// Override the redirect URI (default: http://127.0.0.1:{redirect_port}).
    /// Use for providers whose redirect goes to their own server (e.g., Anthropic).
    pub redirect_uri: Option<String>,
    /// Extra query parameters appended to the authorization URL.
    pub extra_params: Vec<(String, String)>,
}
```

**Step 2: Update build_auth_url to use new fields**

```rust
fn build_auth_url(
    config: &BrowserOAuthConfig,
    code_challenge: &str,
    state: &str,
) -> Result<String, AuthError> {
    let mut url = url::Url::parse(&config.auth_url).map_err(|e| AuthError::AuthFlow {
        message: format!("invalid auth_url: {e}"),
    })?;

    let redirect_uri = config
        .redirect_uri
        .clone()
        .unwrap_or_else(|| format!("http://127.0.0.1:{}", config.redirect_port));

    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &config.client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("scope", &config.scope)
        .append_pair("code_challenge", code_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", state);

    for (key, value) in &config.extra_params {
        url.query_pairs_mut().append_pair(key, value);
    }

    Ok(url.to_string())
}
```

**Step 3: Update exchange_code to use redirect_uri and compute expires_at**

```rust
async fn exchange_code(
    config: &BrowserOAuthConfig,
    code: &str,
    code_verifier: &str,
) -> Result<AuthMaterial, AuthError> {
    let redirect_uri = config
        .redirect_uri
        .clone()
        .unwrap_or_else(|| format!("http://127.0.0.1:{}", config.redirect_port));

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

    let expires_at = token.expires_in.map(|secs| {
        (chrono::Utc::now() + chrono::Duration::seconds(secs as i64)).to_rfc3339()
    });

    Ok(AuthMaterial::OAuth {
        access_token: token.access_token,
        refresh_token: token.refresh_token,
        expires_at,
    })
}
```

**Step 4: Fix existing tests to include new fields**

Update all test `BrowserOAuthConfig` constructions to include the new fields:

```rust
let config = BrowserOAuthConfig {
    client_id: "my-client".into(),
    auth_url: "https://auth.example.com/authorize".into(),
    token_url: "https://auth.example.com/token".into(),
    scope: "openid profile".into(),
    redirect_port: 8080,
    redirect_uri: None,
    extra_params: vec![],
};
```

**Step 5: Add test for extra_params in auth URL**

```rust
#[test]
fn build_auth_url_includes_extra_params() {
    let config = BrowserOAuthConfig {
        client_id: "c".into(),
        auth_url: "https://auth.example.com/authorize".into(),
        token_url: "https://auth.example.com/token".into(),
        scope: "openid".into(),
        redirect_port: 8080,
        redirect_uri: None,
        extra_params: vec![
            ("originator".into(), "codex_cli_rs".into()),
            ("custom_flag".into(), "true".into()),
        ],
    };
    let url = build_auth_url(&config, "ch", "st").unwrap();
    let parsed = url::Url::parse(&url).unwrap();
    let params: std::collections::HashMap<_, _> = parsed.query_pairs().collect();
    assert_eq!(params["originator"], "codex_cli_rs");
    assert_eq!(params["custom_flag"], "true");
}
```

**Step 6: Add test for custom redirect_uri**

```rust
#[test]
fn build_auth_url_custom_redirect_uri() {
    let config = BrowserOAuthConfig {
        client_id: "c".into(),
        auth_url: "https://auth.example.com/authorize".into(),
        token_url: "https://auth.example.com/token".into(),
        scope: "openid".into(),
        redirect_port: 8080,
        redirect_uri: Some("https://console.example.com/oauth/callback".into()),
        extra_params: vec![],
    };
    let url = build_auth_url(&config, "ch", "st").unwrap();
    let parsed = url::Url::parse(&url).unwrap();
    let params: std::collections::HashMap<_, _> = parsed.query_pairs().collect();
    assert_eq!(params["redirect_uri"], "https://console.example.com/oauth/callback");
}
```

**Step 7: Run tests**

Run: `cargo test -p ucode-auth`
Expected: All existing + new tests pass.

**Step 8: Commit**

```
git add crates/ucode-auth/src/flows/browser_oauth.rs
git commit -m "feat(auth): extend BrowserOAuthConfig with extra_params and redirect_uri"
```

---

### Task 2: Add manual-code OAuth flow for non-localhost redirects

**Files:**
- Modify: `crates/ucode-auth/src/flows/browser_oauth.rs`

For providers like Anthropic whose redirect URI goes to their own server (not localhost), we need a flow where the user pastes the authorization code manually.

**Step 1: Add browser_oauth_authorize_manual function**

```rust
/// Perform browser-based OAuth with PKCE, prompting the user to paste the code.
///
/// Use this for providers whose redirect URI is not localhost (e.g., Anthropic).
/// The browser opens the auth URL, the provider's server shows the code,
/// and the user pastes it back.
pub async fn browser_oauth_authorize_manual(
    config: &BrowserOAuthConfig,
    authorization_code: &str,
) -> Result<AuthMaterial, AuthError> {
    // We still need the code_verifier that was used to build the auth URL.
    // This function is the second half — it only does the token exchange.
    // The caller is responsible for:
    //   1. Calling build_auth_url_for_manual() to get the URL + verifier
    //   2. Opening the browser
    //   3. Getting the code from the user
    //   4. Calling this with the code
    // So this is just exchange_code.
    // Let's instead expose a two-phase API.
    exchange_code(config, authorization_code, "").await
}
```

Actually, the better approach is a two-phase API:

**Step 1: Add OAuthPending struct and start function**

```rust
/// Pending OAuth authorization — holds the PKCE verifier needed for token exchange.
pub struct OAuthPending {
    /// The authorization URL to open in the browser.
    pub auth_url: String,
    /// The PKCE code verifier (needed for token exchange).
    code_verifier: String,
}

/// Start a browser OAuth flow: generate PKCE, build the auth URL.
///
/// Returns an `OAuthPending` with the URL to open and the verifier
/// needed for the subsequent `complete_browser_oauth` call.
pub fn start_browser_oauth(config: &BrowserOAuthConfig) -> Result<OAuthPending, AuthError> {
    let code_verifier = generate_code_verifier();
    let code_challenge = generate_code_challenge(&code_verifier);

    let mut rng = rand::rng();
    let state_bytes: [u8; 16] = std::array::from_fn(|_| rand::Rng::random::<u8>(&mut rng));
    let state = hex_encode(&state_bytes);

    let auth_url = build_auth_url(config, &code_challenge, &state)?;

    Ok(OAuthPending {
        auth_url,
        code_verifier,
    })
}

/// Complete a browser OAuth flow by exchanging the authorization code for tokens.
pub async fn complete_browser_oauth(
    config: &BrowserOAuthConfig,
    pending: &OAuthPending,
    authorization_code: &str,
) -> Result<AuthMaterial, AuthError> {
    exchange_code(config, authorization_code, &pending.code_verifier).await
}
```

**Step 2: Add tests**

```rust
#[test]
fn start_browser_oauth_returns_url_with_pkce() {
    let config = BrowserOAuthConfig {
        client_id: "test-client".into(),
        auth_url: "https://auth.example.com/authorize".into(),
        token_url: "https://auth.example.com/token".into(),
        scope: "openid".into(),
        redirect_port: 8080,
        redirect_uri: None,
        extra_params: vec![],
    };
    let pending = start_browser_oauth(&config).unwrap();
    assert!(pending.auth_url.contains("code_challenge="));
    assert!(pending.auth_url.contains("code_challenge_method=S256"));
    assert!(!pending.code_verifier.is_empty());
}
```

**Step 3: Run tests**

Run: `cargo test -p ucode-auth`
Expected: All tests pass.

**Step 4: Commit**

```
git add crates/ucode-auth/src/flows/browser_oauth.rs
git commit -m "feat(auth): add two-phase OAuth API (start/complete) for manual code entry"
```

---

### Task 3: Add OpenAI subscription OAuth config

**Files:**
- Modify: `crates/ucode-auth/src/providers.rs`

**Step 1: Add openai_subscription_oauth_config function**

```rust
/// Build a browser OAuth config for OpenAI ChatGPT subscription (Codex).
///
/// Uses the same client ID and endpoints as the official OpenAI Codex CLI.
/// Requires a ChatGPT Plus/Pro subscription.
pub fn openai_subscription_oauth_config() -> BrowserOAuthConfig {
    BrowserOAuthConfig {
        client_id: "app_EMoamEEZ73f0CkXaXp7hrann".into(),
        auth_url: "https://auth.openai.com/oauth/authorize".into(),
        token_url: "https://auth.openai.com/oauth/token".into(),
        scope: "openid profile email offline_access".into(),
        redirect_port: 1455,
        redirect_uri: None,
        extra_params: vec![
            ("id_token_add_organizations".into(), "true".into()),
            ("codex_cli_simplified_flow".into(), "true".into()),
            ("originator".into(), "codex_cli_rs".into()),
        ],
    }
}

/// Build a refresh config for OpenAI OAuth tokens.
pub fn openai_refresh_config() -> RefreshConfig {
    RefreshConfig {
        token_url: "https://auth.openai.com/oauth/token".into(),
        client_id: "app_EMoamEEZ73f0CkXaXp7hrann".into(),
    }
}
```

**Step 2: Update provider_auth_info for openai to include BrowserOAuth**

```rust
"openai" => Some(ProviderAuthInfo {
    display_name: "OpenAI",
    env_vars: &["OPENAI_API_KEY"],
    auth_methods: &[AuthMethod::ApiKey, AuthMethod::BrowserOAuth],
}),
```

**Step 3: Add import for BrowserOAuthConfig and RefreshConfig**

```rust
use crate::flows::browser_oauth::BrowserOAuthConfig;
use crate::refresh::RefreshConfig;
```

**Step 4: Add tests**

```rust
#[test]
fn openai_subscription_config() {
    let cfg = openai_subscription_oauth_config();
    assert_eq!(cfg.client_id, "app_EMoamEEZ73f0CkXaXp7hrann");
    assert!(cfg.auth_url.contains("auth.openai.com"));
    assert!(cfg.token_url.contains("auth.openai.com"));
    assert!(cfg.scope.contains("offline_access"));
    assert_eq!(cfg.redirect_port, 1455);
    assert!(cfg.redirect_uri.is_none());
    assert!(!cfg.extra_params.is_empty());
    assert!(cfg.extra_params.iter().any(|(k, _)| k == "originator"));
}

#[test]
fn openai_refresh_config_values() {
    let cfg = openai_refresh_config();
    assert!(cfg.token_url.contains("auth.openai.com"));
    assert_eq!(cfg.client_id, "app_EMoamEEZ73f0CkXaXp7hrann");
}

#[test]
fn openai_now_supports_browser_oauth() {
    let info = provider_auth_info("openai").unwrap();
    assert!(info.auth_methods.contains(&AuthMethod::BrowserOAuth));
}
```

**Step 5: Run tests**

Run: `cargo test -p ucode-auth`
Expected: All tests pass.

**Step 6: Commit**

```
git add crates/ucode-auth/src/providers.rs
git commit -m "feat(auth): add OpenAI subscription OAuth config (Codex CLI compatible)"
```

---

### Task 4: Add Anthropic subscription OAuth configs

**Files:**
- Modify: `crates/ucode-auth/src/providers.rs`

**Step 1: Add anthropic_max_oauth_config function**

```rust
/// Build a browser OAuth config for Anthropic Claude Max subscription.
///
/// Uses the same client ID as Claude Code. The redirect goes to Anthropic's
/// console, so the user must paste the authorization code manually.
pub fn anthropic_max_oauth_config() -> BrowserOAuthConfig {
    BrowserOAuthConfig {
        client_id: "9d1c250a-e61b-44d9-88ed-5944d1962f5e".into(),
        auth_url: "https://claude.ai/oauth/authorize".into(),
        token_url: "https://console.anthropic.com/v1/oauth/token".into(),
        scope: "org:create_api_key user:profile user:inference".into(),
        redirect_port: 0, // not used — redirect_uri overrides
        redirect_uri: Some("https://console.anthropic.com/oauth/code/callback".into()),
        extra_params: vec![],
    }
}

/// Build a browser OAuth config for Anthropic Console (creates an API key).
///
/// Same as max config but auth URL points to console.anthropic.com.
pub fn anthropic_console_oauth_config() -> BrowserOAuthConfig {
    BrowserOAuthConfig {
        client_id: "9d1c250a-e61b-44d9-88ed-5944d1962f5e".into(),
        auth_url: "https://console.anthropic.com/oauth/authorize".into(),
        token_url: "https://console.anthropic.com/v1/oauth/token".into(),
        scope: "org:create_api_key user:profile user:inference".into(),
        redirect_port: 0,
        redirect_uri: Some("https://console.anthropic.com/oauth/code/callback".into()),
        extra_params: vec![],
    }
}

/// Build a refresh config for Anthropic OAuth tokens.
pub fn anthropic_refresh_config() -> RefreshConfig {
    RefreshConfig {
        token_url: "https://console.anthropic.com/v1/oauth/token".into(),
        client_id: "9d1c250a-e61b-44d9-88ed-5944d1962f5e".into(),
    }
}
```

**Step 2: Add tests**

```rust
#[test]
fn anthropic_max_config() {
    let cfg = anthropic_max_oauth_config();
    assert_eq!(cfg.client_id, "9d1c250a-e61b-44d9-88ed-5944d1962f5e");
    assert!(cfg.auth_url.contains("claude.ai"));
    assert!(cfg.token_url.contains("console.anthropic.com"));
    assert!(cfg.redirect_uri.is_some());
    assert!(cfg.redirect_uri.unwrap().contains("console.anthropic.com"));
}

#[test]
fn anthropic_console_config() {
    let cfg = anthropic_console_oauth_config();
    assert!(cfg.auth_url.contains("console.anthropic.com"));
    assert_eq!(cfg.client_id, "9d1c250a-e61b-44d9-88ed-5944d1962f5e");
}

#[test]
fn anthropic_refresh_config_values() {
    let cfg = anthropic_refresh_config();
    assert!(cfg.token_url.contains("console.anthropic.com"));
}
```

**Step 3: Run tests**

Run: `cargo test -p ucode-auth`
Expected: All tests pass.

**Step 4: Commit**

```
git add crates/ucode-auth/src/providers.rs
git commit -m "feat(auth): add Anthropic Max and Console OAuth configs"
```

---

### Task 5: Export new functions from lib.rs

**Files:**
- Modify: `crates/ucode-auth/src/lib.rs`

**Step 1: Add exports**

Add to the `pub use flows::browser_oauth` line:

```rust
pub use flows::browser_oauth::{
    BrowserOAuthConfig, OAuthPending, browser_oauth_authorize, complete_browser_oauth,
    start_browser_oauth,
};
```

Add to the `pub use providers` line:

```rust
pub use providers::{
    AuthMethod, ProviderAuthInfo, anthropic_console_oauth_config, anthropic_max_oauth_config,
    anthropic_refresh_config, github_copilot_device_config, openai_refresh_config,
    openai_subscription_oauth_config, provider_auth_info,
};
```

**Step 2: Run build**

Run: `cargo build -p ucode-auth`
Expected: Clean build.

**Step 3: Commit**

```
git add crates/ucode-auth/src/lib.rs
git commit -m "feat(auth): export subscription OAuth config functions"
```

---

### Task 6: Wire subscription OAuth into CLI login handler

**Files:**
- Modify: `crates/ucode-cli/src/auth_handler.rs`

**Step 1: Replace the subscription stub with real OAuth flows**

Replace the `if subscription` block (lines 87-92) with:

```rust
// 3. Browser OAuth (subscription login).
if subscription {
    let info = ucode_auth::provider_auth_info(provider);
    let display = info.as_ref().map_or(provider, |i| i.display_name);

    match provider.to_lowercase().as_str() {
        "openai" => {
            let config = ucode_auth::openai_subscription_oauth_config();
            println!("Starting ChatGPT subscription login for {display}...");
            let material = ucode_auth::browser_oauth_authorize(&config).await?;
            store.store(provider, &material)?;
            println!("Authenticated successfully as {display} (ChatGPT subscription).");
        }
        "anthropic" => {
            let config = ucode_auth::anthropic_max_oauth_config();
            let pending = ucode_auth::start_browser_oauth(&config)?;

            println!("Opening browser for {display} Claude Max login...");
            let _ = open::that(&pending.auth_url);

            println!();
            println!("After authorizing, paste the authorization code below.");
            print!("Authorization code: ");
            io::stdout().flush()?;

            let mut code = String::new();
            io::stdin().lock().read_line(&mut code)?;
            let code = code.trim();

            let material =
                ucode_auth::complete_browser_oauth(&config, &pending, code).await?;
            store.store(provider, &material)?;
            println!("Authenticated successfully as {display} (Claude Max).");
        }
        _ => {
            println!("Subscription OAuth is not available for {provider}.");
            println!("Use 'ucode auth set-key {provider}' to enter an API key instead.");
        }
    }
    return Ok(());
}
```

**Step 2: Update the test for subscription login**

Replace the `login_subscription_not_configured` test:

```rust
#[tokio::test]
async fn login_subscription_unknown_provider() {
    let store = InMemoryStore::new();
    // Unknown provider with --subscription should print a message, not error
    handle_login(&store, "groq", false, true, None)
        .await
        .unwrap();
}
```

**Step 3: Run tests**

Run: `cargo test -p ucode-cli`
Expected: All tests pass.

**Step 4: Run full workspace build and test**

Run: `cargo build && cargo test && cargo clippy`
Expected: Clean build, all tests pass, no clippy warnings.

**Step 5: Commit**

```
git add crates/ucode-cli/src/auth_handler.rs
git commit -m "feat(cli): wire OpenAI and Anthropic subscription OAuth into login handler"
```

---

### Task 7: Verify and add open crate dependency if needed

**Files:**
- Check: `crates/ucode-cli/Cargo.toml`

The `auth_handler.rs` uses `open::that()` to open the browser for Anthropic manual flow. Check if the `open` crate is already a dependency of `ucode-cli`. If not, add it.

**Step 1: Check dependency**

Run: `grep -q 'open' crates/ucode-cli/Cargo.toml && echo "exists" || echo "missing"`

**Step 2: Add if missing**

Run: `cargo add open -p ucode-cli` (only if missing)

**Step 3: Run full build**

Run: `cargo build && cargo test && cargo clippy`
Expected: Clean.

**Step 4: Commit (if changes)**

```
git add crates/ucode-cli/Cargo.toml
git commit -m "build(cli): add open crate for browser launching"
```

---

### Task 8: Final integration verification

**Step 1: Full workspace build**

Run: `cargo build`
Expected: Clean build.

**Step 2: Full test suite**

Run: `cargo test`
Expected: All tests pass (should be ~1490+ tests).

**Step 3: Clippy**

Run: `cargo clippy`
Expected: No warnings.

**Step 4: Verify new functions are accessible**

Run: `cargo doc -p ucode-auth --no-deps`
Expected: Docs build cleanly, new functions visible.

---

## Notes

### Fragility Warning

Both OAuth flows reuse official CLI client IDs:
- **OpenAI**: `app_EMoamEEZ73f0CkXaXp7hrann` (from Codex CLI)
- **Anthropic**: `9d1c250a-e61b-44d9-88ed-5944d1962f5e` (from Claude Code)

These may break if the providers tighten validation. The configs are centralized in `providers.rs` for easy updates.

### Anthropic Request Requirements

When using Anthropic OAuth tokens (not API keys), requests must include:
- Header: `Authorization: Bearer {access_token}` (instead of `x-api-key`)
- Header: `anthropic-beta: oauth-2025-04-20`
- The `x-api-key` header must NOT be present

This request-side adaptation belongs in the Anthropic provider adapter (`crates/ucode-providers/src/anthropic.rs`), not in the auth crate. It should be handled when `resolve_provider_auth()` returns an OAuth token. This is a follow-up task, not part of this plan.

### OpenAI Codex Request Requirements

When using OpenAI OAuth tokens, requests go to `https://chatgpt.com/backend-api` (not `api.openai.com`) with:
- Header: `Authorization: Bearer {access_token}`
- Header: `originator: codex_cli_rs`
- Header: `chatgpt-account-id: {extracted from JWT}`

This request-side adaptation also belongs in the provider adapter layer. Follow-up task.
