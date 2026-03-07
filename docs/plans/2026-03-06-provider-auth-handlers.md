# Provider-Specific Auth Handlers (Task 2.4) Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add provider auth metadata (env var names, supported auth methods, device code configs) and wire the CLI `handle_login` to dispatch to the correct auth flow from Task 2.3.

**Architecture:** A single `providers.rs` module in ucode-auth with a `ProviderAuthInfo` struct and a lookup function. No traits, no registry pattern — just a match on provider name. The CLI `handle_login` becomes async and dispatches to device_code/browser_oauth/wellknown flows based on provider info. Simple API-key-only providers just prompt for a key.

**Tech Stack:** Rust, ucode-auth (flows module), clap (CLI), tokio (async)

---

## Task 1: Add provider auth metadata module

**Files:**
- Create: `crates/ucode-auth/src/providers.rs`
- Modify: `crates/ucode-auth/src/lib.rs`

**Implementation:**

Create `crates/ucode-auth/src/providers.rs`:

```rust
//! Provider-specific auth metadata.
//!
//! Each known provider has a [`ProviderAuthInfo`] describing which env vars
//! to check and which auth flows are available. Unknown providers return `None`
//! from [`provider_auth_info`] — the caller can still use well-known auth or
//! manual API key entry.

use crate::flows::device_code::DeviceCodeConfig;

/// Auth methods a provider supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMethod {
    /// Direct API key entry.
    ApiKey,
    /// Device code flow (RFC 8628).
    DeviceCode,
    /// Browser-based OAuth with PKCE.
    BrowserOAuth,
    /// Well-known endpoint discovery.
    WellKnown,
    /// No auth required (e.g., local Ollama).
    None,
}

/// Auth metadata for a known provider.
#[derive(Debug, Clone)]
pub struct ProviderAuthInfo {
    /// Human-readable display name.
    pub display_name: &'static str,
    /// Environment variable(s) to check for API key. First match wins.
    pub env_vars: &'static [&'static str],
    /// Supported auth methods, in preference order.
    pub auth_methods: &'static [AuthMethod],
}

/// Look up auth metadata for a known provider.
///
/// Returns `None` for unknown providers — the caller should offer
/// well-known auth or manual API key entry as fallback.
pub fn provider_auth_info(provider: &str) -> Option<ProviderAuthInfo> {
    match provider.to_lowercase().as_str() {
        "openai" => Some(ProviderAuthInfo {
            display_name: "OpenAI",
            env_vars: &["OPENAI_API_KEY"],
            auth_methods: &[AuthMethod::ApiKey],
        }),
        "anthropic" => Some(ProviderAuthInfo {
            display_name: "Anthropic",
            env_vars: &["ANTHROPIC_API_KEY"],
            auth_methods: &[AuthMethod::ApiKey, AuthMethod::BrowserOAuth],
        }),
        "github-copilot" => Some(ProviderAuthInfo {
            display_name: "GitHub Copilot",
            env_vars: &[],
            auth_methods: &[AuthMethod::DeviceCode],
        }),
        "gemini" | "google-gemini" => Some(ProviderAuthInfo {
            display_name: "Google Gemini",
            env_vars: &["GEMINI_API_KEY", "GOOGLE_API_KEY"],
            auth_methods: &[AuthMethod::ApiKey, AuthMethod::BrowserOAuth],
        }),
        "vertex-ai" | "google-vertex" => Some(ProviderAuthInfo {
            display_name: "Google Vertex AI",
            env_vars: &["GOOGLE_APPLICATION_CREDENTIALS"],
            auth_methods: &[AuthMethod::ApiKey],
        }),
        "aws-bedrock" | "bedrock" => Some(ProviderAuthInfo {
            display_name: "AWS Bedrock",
            env_vars: &["AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY", "AWS_REGION"],
            auth_methods: &[AuthMethod::ApiKey],
        }),
        "azure-openai" | "azure" => Some(ProviderAuthInfo {
            display_name: "Azure OpenAI",
            env_vars: &["AZURE_OPENAI_API_KEY"],
            auth_methods: &[AuthMethod::ApiKey],
        }),
        "groq" => Some(ProviderAuthInfo {
            display_name: "Groq",
            env_vars: &["GROQ_API_KEY"],
            auth_methods: &[AuthMethod::ApiKey],
        }),
        "deepseek" => Some(ProviderAuthInfo {
            display_name: "DeepSeek",
            env_vars: &["DEEPSEEK_API_KEY"],
            auth_methods: &[AuthMethod::ApiKey],
        }),
        "openrouter" => Some(ProviderAuthInfo {
            display_name: "OpenRouter",
            env_vars: &["OPENROUTER_API_KEY"],
            auth_methods: &[AuthMethod::ApiKey],
        }),
        "together" => Some(ProviderAuthInfo {
            display_name: "Together",
            env_vars: &["TOGETHER_API_KEY"],
            auth_methods: &[AuthMethod::ApiKey],
        }),
        "fireworks" => Some(ProviderAuthInfo {
            display_name: "Fireworks",
            env_vars: &["FIREWORKS_API_KEY"],
            auth_methods: &[AuthMethod::ApiKey],
        }),
        "mistral" => Some(ProviderAuthInfo {
            display_name: "Mistral",
            env_vars: &["MISTRAL_API_KEY"],
            auth_methods: &[AuthMethod::ApiKey],
        }),
        "ollama" => Some(ProviderAuthInfo {
            display_name: "Ollama",
            env_vars: &[],
            auth_methods: &[AuthMethod::None],
        }),
        _ => None,
    }
}

/// Build a device code config for GitHub Copilot.
///
/// Pass `Some("github.example.com")` for enterprise instances.
pub fn github_copilot_device_config(enterprise_domain: Option<&str>) -> DeviceCodeConfig {
    let domain = enterprise_domain.unwrap_or("github.com");
    DeviceCodeConfig {
        client_id: "Ov23ligRrf17Z8tfE3oY".into(),
        device_code_url: format!("https://{domain}/login/device/code"),
        token_url: format!("https://{domain}/login/oauth/access_token"),
        scope: "read:user".into(),
        grant_type: "urn:ietf:params:oauth:grant-type:device_code".into(),
    }
}
```

Update `crates/ucode-auth/src/lib.rs` — add `pub mod providers;` and re-export:
```rust
pub use providers::{AuthMethod, ProviderAuthInfo, provider_auth_info, github_copilot_device_config};
```

**Verify:** `cargo build -p ucode-auth`

**Commit:**
```
feat(auth): add provider auth metadata with env vars and auth methods
```

---

## Task 2: Add provider metadata tests

**Files:**
- Modify: `crates/ucode-auth/src/providers.rs` (add `#[cfg(test)] mod tests`)

**Tests:**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_provider_openai() {
        let info = provider_auth_info("openai").unwrap();
        assert_eq!(info.display_name, "OpenAI");
        assert_eq!(info.env_vars, &["OPENAI_API_KEY"]);
        assert!(info.auth_methods.contains(&AuthMethod::ApiKey));
    }

    #[test]
    fn known_provider_case_insensitive() {
        assert!(provider_auth_info("OpenAI").is_some());
        assert!(provider_auth_info("ANTHROPIC").is_some());
        assert!(provider_auth_info("GitHub-Copilot").is_some());
    }

    #[test]
    fn unknown_provider_returns_none() {
        assert!(provider_auth_info("my-custom-proxy").is_none());
        assert!(provider_auth_info("").is_none());
    }

    #[test]
    fn github_copilot_has_device_code() {
        let info = provider_auth_info("github-copilot").unwrap();
        assert!(info.auth_methods.contains(&AuthMethod::DeviceCode));
        assert!(info.env_vars.is_empty());
    }

    #[test]
    fn ollama_has_no_auth() {
        let info = provider_auth_info("ollama").unwrap();
        assert!(info.auth_methods.contains(&AuthMethod::None));
    }

    #[test]
    fn gemini_alias_works() {
        let g1 = provider_auth_info("gemini").unwrap();
        let g2 = provider_auth_info("google-gemini").unwrap();
        assert_eq!(g1.display_name, g2.display_name);
    }

    #[test]
    fn copilot_device_config_default() {
        let cfg = github_copilot_device_config(None);
        assert_eq!(cfg.client_id, "Ov23ligRrf17Z8tfE3oY");
        assert!(cfg.device_code_url.contains("github.com"));
        assert!(cfg.token_url.contains("github.com"));
        assert_eq!(cfg.scope, "read:user");
    }

    #[test]
    fn copilot_device_config_enterprise() {
        let cfg = github_copilot_device_config(Some("github.example.com"));
        assert!(cfg.device_code_url.contains("github.example.com"));
        assert!(cfg.token_url.contains("github.example.com"));
    }

    #[test]
    fn all_api_key_providers_have_env_vars() {
        for name in ["openai", "anthropic", "groq", "deepseek", "openrouter",
                      "together", "fireworks", "mistral", "azure-openai"] {
            let info = provider_auth_info(name).unwrap();
            assert!(info.auth_methods.contains(&AuthMethod::ApiKey),
                    "{name} should support ApiKey");
            assert!(!info.env_vars.is_empty(),
                    "{name} should have at least one env var");
        }
    }
}
```

**Verify:** `cargo test -p ucode-auth`

**Commit:**
```
test(auth): add provider metadata tests
```

---

## Task 3: Wire CLI handle_login to auth flows

**Files:**
- Modify: `crates/ucode-cli/src/cmd_auth.rs` — add `--url` flag
- Modify: `crates/ucode-cli/src/auth_handler.rs` — make `handle_login` async, dispatch to flows
- Modify: `crates/ucode-cli/src/main.rs` — call `handle_login` with `.await`

**Step 1: Update cmd_auth.rs**

Add `--url` flag to Login variant:
```rust
Login {
    /// Provider name or URL for well-known auth.
    provider: String,

    /// Use device-code flow.
    #[arg(long)]
    device: bool,

    /// Use subscription-based login (browser OAuth).
    #[arg(long)]
    subscription: bool,

    /// Enterprise URL (for GitHub Copilot Enterprise).
    #[arg(long)]
    url: Option<String>,
}
```

**Step 2: Update auth_handler.rs**

Make `handle_login` async. The logic:

1. If `provider` starts with `http://` or `https://` — treat as well-known URL, run `wellknown_authorize`.
2. Look up `provider_auth_info(provider)`.
3. If provider is "github-copilot" or `--device` flag: run device code flow.
4. If `--subscription` flag: run browser OAuth flow (not yet configured per-provider, print message).
5. If provider supports ApiKey: prompt for API key (reuse `handle_set_key` logic).
6. If provider is Ollama (AuthMethod::None): print "no auth needed".
7. If unknown provider and no flags: suggest `--url` for well-known or manual key entry.

On success, store the `AuthMaterial` via the credential store.

```rust
pub async fn handle_login(
    store: &dyn CredentialStore,
    provider: &str,
    device: bool,
    subscription: bool,
    url: Option<&str>,
) -> Result<()> {
    // Well-known URL login
    if provider.starts_with("http://") || provider.starts_with("https://") {
        println!("Authenticating via well-known endpoint: {provider}");
        let material = ucode_auth::wellknown_authorize(provider).await?;
        store.store(provider, &material)?;
        println!("Authenticated successfully via well-known endpoint.");
        return Ok(());
    }

    let info = ucode_auth::provider_auth_info(provider);

    // Device code flow
    if device || matches!(&info, Some(i) if i.auth_methods.contains(&ucode_auth::AuthMethod::DeviceCode)) {
        let enterprise_url = url.map(|u| u.trim_start_matches("https://").trim_end_matches('/'));
        let config = ucode_auth::github_copilot_device_config(enterprise_url);

        println!("Starting device code flow for {}...", info.as_ref().map_or(provider, |i| i.display_name));

        let client = reqwest::Client::new();
        let pending = ucode_auth::request_device_code(&client, &config).await?;

        println!();
        println!("  Open:  {}", pending.verification_uri);
        println!("  Code:  {}", pending.user_code);
        println!();
        println!("Waiting for authorization...");

        let material = ucode_auth::poll_for_token(&client, &config, &pending).await?;
        store.store(provider, &material)?;
        println!("Authenticated successfully as {}.", info.as_ref().map_or(provider, |i| i.display_name));
        return Ok(());
    }

    // Browser OAuth (subscription)
    if subscription {
        println!("Browser OAuth login is not yet configured for {provider}.");
        println!("Use 'ucode auth set-key {provider}' to enter an API key instead.");
        return Ok(());
    }

    // Check if provider needs no auth
    if matches!(&info, Some(i) if i.auth_methods.contains(&ucode_auth::AuthMethod::None)) {
        println!("{} does not require authentication.", info.as_ref().unwrap().display_name);
        return Ok(());
    }

    // Default: prompt for API key
    if info.is_some() || !provider.starts_with("http") {
        return handle_set_key(store, provider);
    }

    println!("Unknown provider '{provider}'.");
    println!("Try: ucode auth login https://your-server.com  (well-known auth)");
    println!("  or: ucode auth set-key {provider}  (manual API key)");
    Ok(())
}
```

**Step 3: Update main.rs**

Change the `handle_login` call to pass `url` and `.await`:
```rust
AuthCommand::Login {
    provider,
    device,
    subscription,
    url,
} => auth_handler::handle_login(&store, &provider, device, subscription, url.as_deref()).await?,
```

**Step 4: Update auth_handler tests**

Update the `login_stub_returns_ok` test to be async and pass the new `url` parameter.

**Verify:** `cargo build && cargo test`

**Commit:**
```
feat(auth): wire CLI login to device code and well-known auth flows
```

---

## Task 4: Workspace verification

Run: `cargo build && cargo test && cargo clippy`

Verify all flows compile and existing tests still pass.

---

## Summary

| Task | What | Tests |
|------|------|-------|
| 1 | Provider auth metadata module | build check |
| 2 | Provider metadata tests | 9 unit tests |
| 3 | Wire CLI handle_login to flows | updated existing tests |
| 4 | Verification | full suite |
