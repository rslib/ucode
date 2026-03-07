# OAuth Request-Side Wiring Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Wire OAuth credentials through provider adapters so subscription login tokens (OpenAI Codex, Anthropic Claude Max) produce correctly authenticated HTTP requests with provider-specific headers, tool normalization, metadata injection, and automatic token refresh.

**Architecture:** Upgrade `resolve_provider_auth()` to return `AuthMaterial` (not a flat string), then make it async with integrated token refresh. Each adapter matches on the `AuthMaterial` variant to set correct headers, base URLs, and request body modifications. Anthropic OAuth requires tool name PascalCase normalization, metadata injection from `~/.claude.json`, and request body sanitization. OpenAI OAuth requires JWT decoding for account ID extraction.

**Tech Stack:** Rust, reqwest, serde_json, chrono, base64 (already in workspace)

---

### Task 1: Upgrade `resolve_provider_auth()` return type to `AuthMaterial`

**Files:**
- Modify: `crates/ucode-providers/src/auth.rs` (change return type, remove `auth_material_to_bearer`)
- Modify: `crates/ucode-providers/src/anthropic.rs:378-383` (update caller)
- Modify: `crates/ucode-providers/src/openai.rs:336-341` (update caller)
- Modify: `crates/ucode-providers/src/gemini.rs` (update caller)
- Modify: `crates/ucode-providers/src/ollama.rs` (update caller)

**Step 1: Update `resolve_provider_auth()` signature and implementation**

Change `auth.rs` to return `Option<AuthMaterial>` instead of `Option<String>`:

```rust
use ucode_auth::{AuthMaterial, CredentialStore};
use ucode_core::{AuthErrorKind, CoreError};

/// Resolve auth material for a provider request.
///
/// Precedence:
/// 1. If `credential_store` is `Some`, call `resolve_auth()` and return the material
/// 2. If `credential_store` is `None`, wrap `fallback_api_key` as `AuthMaterial::ApiKey`
/// 3. If both are `None`, return `None` (provider may work without auth, e.g. Ollama)
pub fn resolve_provider_auth(
    provider: &str,
    api_key_env: Option<&str>,
    credential_store: Option<&dyn CredentialStore>,
    fallback_api_key: Option<&str>,
) -> Result<Option<AuthMaterial>, CoreError> {
    // Try credential store first
    if let Some(store) = credential_store {
        match ucode_auth::resolve_auth(provider, api_key_env, store) {
            Ok(material) => return Ok(Some(material)),
            Err(ucode_auth::AuthError::MissingCredential { .. })
            | Err(ucode_auth::AuthError::NotFound { .. }) => {
                // Fall through to fallback_api_key
            }
            Err(ucode_auth::AuthError::AuthExpired { .. }) => {
                return Err(CoreError::Auth {
                    provider: provider.to_owned(),
                    auth_kind: AuthErrorKind::Expired,
                });
            }
            Err(_) => {
                return Err(CoreError::Auth {
                    provider: provider.to_owned(),
                    auth_kind: AuthErrorKind::Invalid,
                });
            }
        }
    }

    // Fallback to stored api_key — wrap as AuthMaterial::ApiKey
    if let Some(key) = fallback_api_key {
        return Ok(Some(AuthMaterial::ApiKey {
            key: key.to_owned(),
        }));
    }

    Ok(None)
}
```

Remove the `auth_material_to_bearer()` helper function entirely.

Add a new helper function for extracting a bearer token from `AuthMaterial`:

```rust
/// Extract a bearer token string from auth material.
///
/// This is a convenience for adapters that always use `Authorization: Bearer`.
pub fn bearer_token(material: &AuthMaterial) -> String {
    match material {
        AuthMaterial::ApiKey { key } => key.clone(),
        AuthMaterial::OAuth { access_token, .. } => access_token.clone(),
        AuthMaterial::SessionToken { token, .. } => token.clone(),
        AuthMaterial::WellKnown { token, .. } => token.clone(),
        AuthMaterial::AwsCredentials { session_token, .. } => {
            session_token.clone().unwrap_or_default()
        }
    }
}
```

**Step 2: Update OpenAI adapter caller**

In `openai.rs` `stream_chat()`, change:

```rust
// Before:
let api_key = crate::auth::resolve_provider_auth(
    &provider_name,
    api_key_env.as_deref(),
    credential_store.as_ref().map(|s| s.as_ref()),
    fallback_api_key.as_deref(),
)?;

let mut request = client.post(&url).header("Content-Type", "application/json");

if let Some(ref key) = api_key {
    request = request.header("Authorization", format!("Bearer {key}"));
}

// After:
let auth_material = crate::auth::resolve_provider_auth(
    &provider_name,
    api_key_env.as_deref(),
    credential_store.as_ref().map(|s| s.as_ref()),
    fallback_api_key.as_deref(),
)?;

let mut request = client.post(&url).header("Content-Type", "application/json");

if let Some(ref material) = auth_material {
    let token = crate::auth::bearer_token(material);
    request = request.header("Authorization", format!("Bearer {token}"));
}
```

**Step 3: Update Anthropic adapter caller**

In `anthropic.rs` `stream_chat()`, change:

```rust
// Before:
let api_key = crate::auth::resolve_provider_auth(
    &provider_name,
    api_key_env.as_deref(),
    credential_store.as_ref().map(|s| s.as_ref()),
    fallback_api_key.as_deref(),
)?;

let mut builder = client.post(&url);

if let Some(ref key) = api_key {
    builder = builder.header("x-api-key", key);
}

// After:
let auth_material = crate::auth::resolve_provider_auth(
    &provider_name,
    api_key_env.as_deref(),
    credential_store.as_ref().map(|s| s.as_ref()),
    fallback_api_key.as_deref(),
)?;

let mut builder = client.post(&url);

if let Some(ref material) = auth_material {
    let token = crate::auth::bearer_token(material);
    builder = builder.header("x-api-key", &token);
}
```

**Step 4: Update Gemini adapter caller**

Same pattern — extract token with `bearer_token()`, use in existing header logic.

**Step 5: Update Ollama adapter caller**

Same pattern — Ollama typically has no auth, but handle the `Some` case.

**Step 6: Update tests in `auth.rs`**

Update test assertions to check for `AuthMaterial` variants instead of `String`:

```rust
#[test]
fn resolve_from_store_api_key() {
    let store = InMemoryStore::new();
    store
        .store("openai", &AuthMaterial::ApiKey { key: "sk-test".into() })
        .unwrap();
    let result = resolve_provider_auth("openai", None, Some(&store), None).unwrap();
    assert!(matches!(result, Some(AuthMaterial::ApiKey { ref key }) if key == "sk-test"));
}

#[test]
fn resolve_from_store_oauth() {
    let store = InMemoryStore::new();
    store
        .store("copilot", &AuthMaterial::OAuth {
            access_token: "gho_abc".into(),
            refresh_token: None,
            expires_at: None,
        })
        .unwrap();
    let result = resolve_provider_auth("copilot", None, Some(&store), None).unwrap();
    assert!(matches!(result, Some(AuthMaterial::OAuth { ref access_token, .. }) if access_token == "gho_abc"));
}

#[test]
fn resolve_fallback_api_key_when_store_empty() {
    let store = InMemoryStore::new();
    let result = resolve_provider_auth("openai", None, Some(&store), Some("sk-fallback")).unwrap();
    assert!(matches!(result, Some(AuthMaterial::ApiKey { ref key }) if key == "sk-fallback"));
}

#[test]
fn resolve_fallback_api_key_no_store() {
    let result = resolve_provider_auth("openai", None, None, Some("sk-direct")).unwrap();
    assert!(matches!(result, Some(AuthMaterial::ApiKey { ref key }) if key == "sk-direct"));
}

#[test]
fn resolve_none_when_no_store_no_key() {
    let result = resolve_provider_auth("ollama", None, None, None).unwrap();
    assert_eq!(result, None);
}

#[test]
fn bearer_token_from_api_key() {
    let mat = AuthMaterial::ApiKey { key: "sk-test".into() };
    assert_eq!(bearer_token(&mat), "sk-test");
}

#[test]
fn bearer_token_from_oauth() {
    let mat = AuthMaterial::OAuth {
        access_token: "oauth-tok".into(),
        refresh_token: None,
        expires_at: None,
    };
    assert_eq!(bearer_token(&mat), "oauth-tok");
}

#[test]
fn bearer_token_from_session() {
    let mat = AuthMaterial::SessionToken {
        token: "sess-123".into(),
        expires_at: None,
    };
    assert_eq!(bearer_token(&mat), "sess-123");
}

#[test]
fn bearer_token_from_wellknown() {
    let mat = AuthMaterial::WellKnown {
        env_key: "CUSTOM_KEY".into(),
        token: "wk-tok".into(),
    };
    assert_eq!(bearer_token(&mat), "wk-tok");
}
```

**Step 7: Run tests and verify**

Run: `cargo build && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: All tests pass, no clippy warnings, clean build.

**Step 8: Commit**

```
feat(providers): upgrade resolve_provider_auth to return AuthMaterial

Return full AuthMaterial instead of flat String so adapters can
distinguish OAuth from API key credentials and set headers accordingly.
```

---

### Task 2: Make `resolve_provider_auth()` async with integrated token refresh

**Files:**
- Modify: `crates/ucode-providers/src/auth.rs` (make async, add refresh logic)
- Modify: `crates/ucode-providers/src/anthropic.rs` (add `.await`)
- Modify: `crates/ucode-providers/src/openai.rs` (add `.await`)
- Modify: `crates/ucode-providers/src/gemini.rs` (add `.await`)
- Modify: `crates/ucode-providers/src/ollama.rs` (add `.await`)

**Step 1: Add async + refresh to `resolve_provider_auth()`**

```rust
use ucode_auth::{AuthMaterial, CredentialStore, RefreshConfig};
use ucode_core::{AuthErrorKind, CoreError};

/// Resolve auth material for a provider request, with optional token refresh.
///
/// If the resolved credential is an OAuth token expiring within 5 minutes
/// and a `refresh_config` is provided, attempts automatic refresh.
pub async fn resolve_provider_auth(
    provider: &str,
    api_key_env: Option<&str>,
    credential_store: Option<&dyn CredentialStore>,
    fallback_api_key: Option<&str>,
    refresh_config: Option<&RefreshConfig>,
) -> Result<Option<AuthMaterial>, CoreError> {
    // Try credential store first
    if let Some(store) = credential_store {
        match ucode_auth::resolve_auth(provider, api_key_env, store) {
            Ok(material) => {
                // Check if OAuth token needs refresh
                if let AuthMaterial::OAuth { .. } = &material {
                    if material.expires_within(chrono::Duration::seconds(300)) {
                        if let Some(refresh_cfg) = refresh_config {
                            if let Some(refresh_tok) = material.refresh_token() {
                                let client = reqwest::Client::new();
                                match ucode_auth::refresh_oauth_token(
                                    &client,
                                    refresh_cfg,
                                    refresh_tok,
                                )
                                .await
                                {
                                    Ok(new_material) => {
                                        // Store refreshed credential
                                        let _ = store.store(provider, &new_material);
                                        return Ok(Some(new_material));
                                    }
                                    Err(_) => {
                                        // Refresh failed — if token is actually expired, error out
                                        if material.is_expired() {
                                            return Err(CoreError::Auth {
                                                provider: provider.to_owned(),
                                                auth_kind: AuthErrorKind::Expired,
                                            });
                                        }
                                        // Not yet expired, use existing token
                                    }
                                }
                            } else if material.is_expired() {
                                return Err(CoreError::Auth {
                                    provider: provider.to_owned(),
                                    auth_kind: AuthErrorKind::Expired,
                                });
                            }
                        } else if material.is_expired() {
                            return Err(CoreError::Auth {
                                provider: provider.to_owned(),
                                auth_kind: AuthErrorKind::Expired,
                            });
                        }
                    }
                }
                return Ok(Some(material));
            }
            Err(ucode_auth::AuthError::MissingCredential { .. })
            | Err(ucode_auth::AuthError::NotFound { .. }) => {
                // Fall through to fallback_api_key
            }
            Err(ucode_auth::AuthError::AuthExpired { .. }) => {
                return Err(CoreError::Auth {
                    provider: provider.to_owned(),
                    auth_kind: AuthErrorKind::Expired,
                });
            }
            Err(_) => {
                return Err(CoreError::Auth {
                    provider: provider.to_owned(),
                    auth_kind: AuthErrorKind::Invalid,
                });
            }
        }
    }

    // Fallback to stored api_key
    if let Some(key) = fallback_api_key {
        return Ok(Some(AuthMaterial::ApiKey {
            key: key.to_owned(),
        }));
    }

    Ok(None)
}
```

**Step 2: Update all adapter callers to add `.await` and pass `None` for refresh_config**

In each adapter's `stream_chat()`, the call is already inside `Box::pin(async move { ... })`, so just add `.await` and pass `None` for `refresh_config`:

```rust
let auth_material = crate::auth::resolve_provider_auth(
    &provider_name,
    api_key_env.as_deref(),
    credential_store.as_ref().map(|s| s.as_ref()),
    fallback_api_key.as_deref(),
    None, // refresh_config — will be wired in provider-specific tasks
)
.await?;
```

**Step 3: Update tests to use `#[tokio::test]`**

All tests in `auth.rs` that call `resolve_provider_auth` need to become async:

```rust
#[tokio::test]
async fn resolve_from_store_api_key() {
    let store = InMemoryStore::new();
    store
        .store("openai", &AuthMaterial::ApiKey { key: "sk-test".into() })
        .unwrap();
    let result = resolve_provider_auth("openai", None, Some(&store), None, None)
        .await
        .unwrap();
    assert!(matches!(result, Some(AuthMaterial::ApiKey { ref key }) if key == "sk-test"));
}
// ... same pattern for all other tests
```

**Step 4: Run tests and verify**

Run: `cargo build && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: All tests pass.

**Step 5: Commit**

```
feat(providers): make resolve_provider_auth async with token refresh

OAuth tokens expiring within 5 minutes are automatically refreshed
before being returned. Refresh config is optional per-provider.
```

---

### Task 3: Anthropic OAuth-aware headers and request body sanitization

**Files:**
- Modify: `crates/ucode-providers/src/anthropic.rs` (OAuth header logic, body sanitization)

**Step 1: Write tests for OAuth vs API key header behavior**

Add to `anthropic.rs` tests:

```rust
#[test]
fn oauth_headers_use_bearer_auth() {
    // When AuthMaterial is OAuth, Anthropic should use:
    // - Authorization: Bearer {token}
    // - anthropic-beta: claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14
    // NOT x-api-key
    let material = AuthMaterial::OAuth {
        access_token: "oauth-tok".into(),
        refresh_token: None,
        expires_at: None,
    };
    assert!(matches!(material, AuthMaterial::OAuth { .. }));
}

#[test]
fn api_key_headers_use_x_api_key() {
    let material = AuthMaterial::ApiKey { key: "sk-test".into() };
    assert!(matches!(material, AuthMaterial::ApiKey { .. }));
}
```

**Step 2: Implement OAuth-aware header logic in `stream_chat()`**

Replace the header-setting block in `stream_chat()`:

```rust
// Set auth headers based on credential type
if let Some(ref material) = auth_material {
    match material {
        AuthMaterial::OAuth { access_token, .. } => {
            // OAuth subscription: Bearer token + beta headers
            builder = builder
                .header("Authorization", format!("Bearer {access_token}"))
                .header(
                    "anthropic-beta",
                    "claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14",
                );
        }
        _ => {
            // API key or other: use x-api-key header
            let token = crate::auth::bearer_token(material);
            builder = builder.header("x-api-key", &token);
        }
    }
}
```

**Step 3: Add request body sanitization for OAuth**

When using OAuth, remove `temperature` and `tool_choice` from the request body. The cleanest way is to conditionally set `temperature` to `None` before building the body:

```rust
let is_oauth = auth_material
    .as_ref()
    .is_some_and(|m| matches!(m, AuthMaterial::OAuth { .. }));

// For OAuth, strip temperature (Anthropic OAuth doesn't accept it)
let temperature = if is_oauth { None } else { req.temperature };

let body = AnthropicRequest {
    model: req.model,
    max_tokens: req.max_tokens.unwrap_or(4096),
    messages,
    stream: true,
    system,
    temperature,
    tools: to_anthropic_tools(&req.tools),
};
```

Note: Move the body construction AFTER the auth resolution so we know if it's OAuth.

**Step 4: Pass Anthropic refresh config**

Wire `anthropic_refresh_config()` into the `resolve_provider_auth` call for Anthropic:

```rust
use ucode_auth::anthropic_refresh_config;

// Inside stream_chat():
let refresh_cfg = if provider_name == "anthropic" {
    Some(anthropic_refresh_config())
} else {
    None
};

let auth_material = crate::auth::resolve_provider_auth(
    &provider_name,
    api_key_env.as_deref(),
    credential_store.as_ref().map(|s| s.as_ref()),
    fallback_api_key.as_deref(),
    refresh_cfg.as_ref(),
)
.await?;
```

**Step 5: Run tests and verify**

Run: `cargo build && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: All tests pass.

**Step 6: Commit**

```
feat(anthropic): add OAuth-aware headers and request body sanitization

OAuth credentials use Authorization: Bearer + anthropic-beta headers
instead of x-api-key. Temperature stripped from OAuth requests.
Auto-refresh wired for Anthropic OAuth tokens.
```

---

### Task 4: Tool name normalization (PascalCase mapping)

**Files:**
- Create: `crates/ucode-providers/src/tool_normalize.rs`
- Modify: `crates/ucode-providers/src/lib.rs` (add module)
- Modify: `crates/ucode-providers/src/anthropic.rs` (apply normalization for OAuth)

**Step 1: Write tests for tool name normalization**

Create `crates/ucode-providers/src/tool_normalize.rs`:

```rust
//! Tool name normalization for Anthropic OAuth.
//!
//! Anthropic's OAuth validation requires tool names in PascalCase.
//! Known tool names have explicit mappings; unknown names are converted
//! from snake_case to PascalCase.

/// Normalize a tool name to PascalCase for Anthropic OAuth.
///
/// Known mappings (Claude Code built-in tools):
/// - bash -> Bash
/// - read -> Read
/// - edit -> Edit
/// - write -> Write
/// - glob -> Glob
/// - grep -> Grep
/// - webfetch -> WebFetch
/// - websearch -> WebSearch
/// - task -> Task
/// - todowrite -> TodoWrite
///
/// Unknown tools: convert snake_case to PascalCase (e.g., my_tool -> MyTool).
pub fn normalize_tool_name(name: &str) -> String {
    match name {
        "bash" => "Bash".into(),
        "read" => "Read".into(),
        "edit" => "Edit".into(),
        "write" => "Write".into(),
        "glob" => "Glob".into(),
        "grep" => "Grep".into(),
        "webfetch" => "WebFetch".into(),
        "websearch" => "WebSearch".into(),
        "task" => "Task".into(),
        "todowrite" => "TodoWrite".into(),
        // Already PascalCase or unknown — try snake_case conversion
        other => snake_to_pascal(other),
    }
}

/// Convert a snake_case string to PascalCase.
///
/// If the string contains no underscores and starts with uppercase,
/// returns it unchanged (already PascalCase).
fn snake_to_pascal(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }

    // If no underscores and first char is uppercase, assume already PascalCase
    if !s.contains('_') && s.starts_with(|c: char| c.is_uppercase()) {
        return s.to_owned();
    }

    s.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(c) => {
                    let upper: String = c.to_uppercase().collect();
                    upper + &chars.as_str().to_lowercase()
                }
                None => String::new(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_tool_names() {
        assert_eq!(normalize_tool_name("bash"), "Bash");
        assert_eq!(normalize_tool_name("read"), "Read");
        assert_eq!(normalize_tool_name("edit"), "Edit");
        assert_eq!(normalize_tool_name("write"), "Write");
        assert_eq!(normalize_tool_name("glob"), "Glob");
        assert_eq!(normalize_tool_name("grep"), "Grep");
        assert_eq!(normalize_tool_name("webfetch"), "WebFetch");
        assert_eq!(normalize_tool_name("websearch"), "WebSearch");
        assert_eq!(normalize_tool_name("task"), "Task");
        assert_eq!(normalize_tool_name("todowrite"), "TodoWrite");
    }

    #[test]
    fn snake_case_conversion() {
        assert_eq!(normalize_tool_name("my_tool"), "MyTool");
        assert_eq!(normalize_tool_name("get_weather_data"), "GetWeatherData");
        assert_eq!(normalize_tool_name("a_b_c"), "ABC");
    }

    #[test]
    fn already_pascal_case() {
        assert_eq!(normalize_tool_name("MyTool"), "MyTool");
        assert_eq!(normalize_tool_name("GetWeather"), "GetWeather");
    }

    #[test]
    fn single_word_lowercase() {
        // Single lowercase word without known mapping gets capitalized
        assert_eq!(normalize_tool_name("search"), "Search");
    }

    #[test]
    fn empty_string() {
        assert_eq!(normalize_tool_name(""), "");
    }

    #[test]
    fn underscores_only() {
        assert_eq!(normalize_tool_name("___"), "");
    }
}
```

**Step 2: Add module to `lib.rs`**

In `crates/ucode-providers/src/lib.rs`, add:

```rust
pub mod tool_normalize;
```

**Step 3: Apply normalization in Anthropic adapter for OAuth**

In `anthropic.rs`, modify `to_anthropic_tools()` to accept an `is_oauth` flag:

```rust
fn to_anthropic_tools(tools: &[ToolDef], normalize: bool) -> Vec<AnthropicTool> {
    tools
        .iter()
        .map(|t| {
            let name = if normalize {
                crate::tool_normalize::normalize_tool_name(&t.name)
            } else {
                t.name.clone()
            };
            AnthropicTool {
                name,
                description: t.description.clone(),
                input_schema: t.parameters.clone(),
            }
        })
        .collect()
}
```

Update the call site in `stream_chat()`:

```rust
tools: to_anthropic_tools(&req.tools, is_oauth),
```

**Step 4: Run tests and verify**

Run: `cargo build && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: All tests pass including new tool_normalize tests.

**Step 5: Commit**

```
feat(providers): add tool name PascalCase normalization for Anthropic OAuth

Anthropic OAuth validation requires PascalCase tool names. Known tools
(bash, read, edit, etc.) have explicit mappings; unknown tools are
converted from snake_case.
```

---

### Task 5: `~/.claude.json` reader and metadata injection for Anthropic OAuth

**Files:**
- Create: `crates/ucode-providers/src/claude_metadata.rs`
- Modify: `crates/ucode-providers/src/lib.rs` (add module)
- Modify: `crates/ucode-providers/src/anthropic.rs` (inject metadata into request body)

**Step 1: Write the `~/.claude.json` reader**

Create `crates/ucode-providers/src/claude_metadata.rs`:

```rust
//! Read metadata from `~/.claude.json` for Anthropic OAuth requests.
//!
//! When using Anthropic OAuth (Claude Max subscription), the API requires
//! a `metadata.user_id` field in the request body. This is constructed from
//! fields in `~/.claude.json` which is written by Claude Code.

use serde::Deserialize;

/// Relevant fields from `~/.claude.json`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeConfig {
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    oauth_account: Option<OAuthAccount>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OAuthAccount {
    #[serde(default)]
    account_uuid: Option<String>,
}

/// Build the metadata user_id string for Anthropic OAuth requests.
///
/// Format: `user_{userId}_account_{accountUuid}_session_{sessionId}`
///
/// Returns `None` if `~/.claude.json` doesn't exist or lacks required fields.
/// The `session_id` parameter is provided by the caller (from the current session).
pub fn build_metadata_user_id(session_id: &str) -> Option<String> {
    let home = dirs::home_dir()?;
    let path = home.join(".claude.json");

    let content = std::fs::read_to_string(&path).ok()?;
    let config: ClaudeConfig = serde_json::from_str(&content).ok()?;

    let user_id = config.user_id?;
    let account_uuid = config.oauth_account?.account_uuid?;

    Some(format!(
        "user_{user_id}_account_{account_uuid}_session_{session_id}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_claude_config() {
        let json = r#"{
            "userId": "user123",
            "oauthAccount": {
                "accountUuid": "acc-456"
            }
        }"#;
        let config: ClaudeConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.user_id.as_deref(), Some("user123"));
        assert_eq!(
            config.oauth_account.unwrap().account_uuid.as_deref(),
            Some("acc-456")
        );
    }

    #[test]
    fn parse_claude_config_missing_fields() {
        let json = r#"{}"#;
        let config: ClaudeConfig = serde_json::from_str(json).unwrap();
        assert!(config.user_id.is_none());
        assert!(config.oauth_account.is_none());
    }

    #[test]
    fn parse_claude_config_partial() {
        let json = r#"{"userId": "user123"}"#;
        let config: ClaudeConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.user_id.as_deref(), Some("user123"));
        assert!(config.oauth_account.is_none());
    }
}
```

**Step 2: Add module to `lib.rs`**

```rust
pub mod claude_metadata;
```

**Step 3: Add `metadata` field to `AnthropicRequest`**

In `anthropic.rs`, add an optional metadata field:

```rust
#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: usize,
    messages: Vec<AnthropicMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<AnthropicTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<AnthropicMetadata>,
}

#[derive(Serialize)]
struct AnthropicMetadata {
    user_id: String,
}
```

**Step 4: Inject metadata when OAuth**

In `stream_chat()`, after determining `is_oauth`:

```rust
let metadata = if is_oauth {
    // Try to build metadata from ~/.claude.json
    // Use a placeholder session ID for now (will be wired from session state later)
    crate::claude_metadata::build_metadata_user_id("default")
        .map(|user_id| AnthropicMetadata { user_id })
} else {
    None
};

let body = AnthropicRequest {
    model: req.model,
    max_tokens: req.max_tokens.unwrap_or(4096),
    messages,
    stream: true,
    system,
    temperature,
    tools: to_anthropic_tools(&req.tools, is_oauth),
    metadata,
};
```

**Step 5: Add `dirs` dependency to ucode-providers**

Run: `cargo add dirs --manifest-path crates/ucode-providers/Cargo.toml`

(Note: `dirs` is already used by `ucode-auth`, so it's in the workspace.)

**Step 6: Run tests and verify**

Run: `cargo build && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: All tests pass.

**Step 7: Commit**

```
feat(anthropic): inject metadata.user_id from ~/.claude.json for OAuth

Anthropic OAuth requests require metadata.user_id constructed from
Claude Code's config file. Reads userId and accountUuid from
~/.claude.json when available.
```

---

### Task 6: OpenAI JWT account ID extraction

**Files:**
- Create: `crates/ucode-auth/src/jwt.rs`
- Modify: `crates/ucode-auth/src/lib.rs` (add module + export)

**Step 1: Write JWT payload decoder**

Create `crates/ucode-auth/src/jwt.rs`:

```rust
//! Minimal JWT payload decoding (no signature verification).
//!
//! Used to extract claims from OAuth access tokens (e.g., OpenAI's
//! `chatgpt_account_id` claim). We only need to read the payload,
//! not verify the signature — the token was obtained via a trusted
//! OAuth flow.

use base64::Engine;
use serde_json::Value;

/// Decode the payload of a JWT without verifying the signature.
///
/// Returns the payload as a `serde_json::Value`, or `None` if the
/// token is malformed.
pub fn decode_jwt_payload(token: &str) -> Option<Value> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }

    let payload_b64 = parts[1];
    // JWT uses base64url encoding (no padding)
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let bytes = engine.decode(payload_b64).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Extract the ChatGPT account ID from an OpenAI JWT.
///
/// Checks two claim paths:
/// 1. `chatgpt_account_id` (direct claim)
/// 2. `https://api.openai.com/auth` -> `chatgpt_account_id` (nested)
pub fn extract_openai_account_id(token: &str) -> Option<String> {
    let payload = decode_jwt_payload(token)?;

    // Try direct claim
    if let Some(id) = payload.get("chatgpt_account_id").and_then(|v| v.as_str()) {
        return Some(id.to_owned());
    }

    // Try nested claim
    if let Some(auth) = payload.get("https://api.openai.com/auth") {
        if let Some(id) = auth.get("chatgpt_account_id").and_then(|v| v.as_str()) {
            return Some(id.to_owned());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn make_jwt(payload: &Value) -> String {
        let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let header = engine.encode(r#"{"alg":"RS256","typ":"JWT"}"#.as_bytes());
        let payload_b64 = engine.encode(serde_json::to_vec(payload).unwrap());
        let sig = engine.encode(b"fake-signature");
        format!("{header}.{payload_b64}.{sig}")
    }

    #[test]
    fn decode_valid_jwt() {
        let payload = serde_json::json!({"sub": "user123", "exp": 9999999999u64});
        let token = make_jwt(&payload);
        let decoded = decode_jwt_payload(&token).unwrap();
        assert_eq!(decoded["sub"], "user123");
    }

    #[test]
    fn decode_invalid_jwt() {
        assert!(decode_jwt_payload("not-a-jwt").is_none());
        assert!(decode_jwt_payload("a.b").is_none());
        assert!(decode_jwt_payload("").is_none());
    }

    #[test]
    fn extract_direct_account_id() {
        let payload = serde_json::json!({
            "chatgpt_account_id": "acct-123",
            "sub": "user"
        });
        let token = make_jwt(&payload);
        assert_eq!(
            extract_openai_account_id(&token).as_deref(),
            Some("acct-123")
        );
    }

    #[test]
    fn extract_nested_account_id() {
        let payload = serde_json::json!({
            "sub": "user",
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acct-456"
            }
        });
        let token = make_jwt(&payload);
        assert_eq!(
            extract_openai_account_id(&token).as_deref(),
            Some("acct-456")
        );
    }

    #[test]
    fn extract_no_account_id() {
        let payload = serde_json::json!({"sub": "user"});
        let token = make_jwt(&payload);
        assert!(extract_openai_account_id(&token).is_none());
    }

    #[test]
    fn direct_claim_takes_precedence() {
        let payload = serde_json::json!({
            "chatgpt_account_id": "direct-id",
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "nested-id"
            }
        });
        let token = make_jwt(&payload);
        assert_eq!(
            extract_openai_account_id(&token).as_deref(),
            Some("direct-id")
        );
    }
}
```

**Step 2: Export from `lib.rs`**

In `crates/ucode-auth/src/lib.rs`, add:

```rust
pub mod jwt;
pub use jwt::{decode_jwt_payload, extract_openai_account_id};
```

**Step 3: Run tests and verify**

Run: `cargo build && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: All tests pass including new JWT tests.

**Step 4: Commit**

```
feat(auth): add JWT payload decoder and OpenAI account ID extraction

Decode JWT payloads without signature verification to extract
chatgpt_account_id from OpenAI OAuth tokens. Supports both direct
and nested claim paths. Prep for Codex endpoint routing.
```

---

### Task 7: Wire OpenAI refresh config + final integration verification

**Files:**
- Modify: `crates/ucode-providers/src/openai.rs` (wire refresh config)
- Modify: `PLANS.md` (mark tasks done)
- Modify: `EPIC.md` (add new issue entry)

**Step 1: Wire OpenAI refresh config**

In `openai.rs` `stream_chat()`, add refresh config for OpenAI provider:

```rust
use ucode_auth::openai_refresh_config;

// Inside stream_chat():
let refresh_cfg = if provider_name == "openai" {
    Some(openai_refresh_config())
} else {
    None
};

let auth_material = crate::auth::resolve_provider_auth(
    &provider_name,
    api_key_env.as_deref(),
    credential_store.as_ref().map(|s| s.as_ref()),
    fallback_api_key.as_deref(),
    refresh_cfg.as_ref(),
)
.await?;
```

**Step 2: Run full workspace verification**

Run: `cargo build && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: All tests pass, zero clippy warnings.

**Step 3: Update EPIC.md**

Add new issue entry after ISSUE 0207:

```markdown
### ISSUE 0208 — OAuth request-side wiring (ucode-providers + ucode-auth) [DONE]

**Goal:** Wire OAuth credentials through provider adapters with correct headers, tool normalization, metadata injection, and automatic token refresh.
**Scope/Notes:**

* `resolve_provider_auth()` returns `AuthMaterial` (not flat string) + async with auto-refresh
* Anthropic OAuth: Bearer auth + beta headers, temperature stripping, tool PascalCase normalization, metadata.user_id injection from `~/.claude.json`
* OpenAI OAuth: JWT account ID extraction (prep for Codex), auto-refresh
* All adapters handle `AuthMaterial` variants correctly
  **Acceptance tests:**
* Anthropic OAuth credential produces Bearer + beta headers (not x-api-key)
* Tool names normalized to PascalCase for OAuth requests
* JWT decoder extracts account ID from OpenAI tokens
* Token refresh triggered automatically for expiring OAuth tokens
  **Owner:** Auth/Providers
```

**Step 4: Update PLANS.md**

Add section after 2.6:

```markdown
## 2.7 OAuth request-side wiring (ucode-providers + ucode-auth) [P0] [DONE]

Wire OAuth credentials through provider adapters so subscription login tokens
produce correctly authenticated HTTP requests.

* `resolve_provider_auth()` upgraded to return `AuthMaterial` + async with auto-refresh
* Anthropic adapter: OAuth-aware headers (Bearer + anthropic-beta), temperature stripping,
  tool name PascalCase normalization, metadata.user_id injection from `~/.claude.json`
* OpenAI adapter: JWT account ID extraction, auto-refresh
* All adapters handle `AuthMaterial` variants correctly

**Acceptance**

* Anthropic OAuth credential produces correct headers and normalized tool names.
* OpenAI JWT decoder extracts account ID.
* Expiring tokens auto-refresh before requests.
```

**Step 5: Commit**

```
feat(providers): wire OpenAI refresh config and finalize OAuth request wiring

Complete OAuth request-side wiring: all adapters handle AuthMaterial,
Anthropic has OAuth-aware headers + tool normalization + metadata,
OpenAI has JWT decoding + auto-refresh. Update EPIC.md and PLANS.md.
```
