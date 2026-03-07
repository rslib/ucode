use std::io::{self, BufRead, Write};

use anyhow::Result;
use ucode_auth::{AuthError, AuthMaterial, CredentialStatus, CredentialStore};

pub fn handle_status(store: &dyn CredentialStore) -> Result<()> {
    for status in store.list_configured() {
        match status {
            CredentialStatus::Configured { provider, kind } => {
                println!("{provider}: configured ({kind})");
            }
            CredentialStatus::NotConfigured { provider } => {
                println!("{provider}: not configured");
            }
        }
    }
    Ok(())
}

pub fn handle_set_key(store: &dyn CredentialStore, provider: &str) -> Result<()> {
    print!("Enter API key for {provider}: ");
    io::stdout().flush()?;

    let mut key = String::new();
    io::stdin().lock().read_line(&mut key)?;
    let key = key.trim().to_owned();

    store.store(provider, &AuthMaterial::ApiKey { key })?;
    println!("API key for {provider} stored successfully.");
    Ok(())
}

pub fn handle_logout(store: &dyn CredentialStore, provider: &str) -> Result<()> {
    match store.delete(provider) {
        Ok(()) => println!("Logged out from {provider}."),
        Err(AuthError::NotFound { .. }) => {
            println!("No credentials found for {provider}.");
        }
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

pub async fn handle_login(
    store: &dyn CredentialStore,
    provider: &str,
    device: bool,
    subscription: bool,
    url: Option<&str>,
) -> Result<()> {
    // 1. Well-known URL login: provider argument is itself a URL.
    if provider.starts_with("http://") || provider.starts_with("https://") {
        println!("Authenticating via well-known endpoint: {provider}");
        let material = ucode_auth::wellknown_authorize(provider).await?;
        store.store(provider, &material)?;
        println!("Authenticated successfully via well-known endpoint.");
        return Ok(());
    }

    let info = ucode_auth::provider_auth_info(provider);

    // 2. Device code flow: explicit --device flag or provider defaults to it.
    if device
        || matches!(&info, Some(i) if i.auth_methods.contains(&ucode_auth::AuthMethod::DeviceCode))
    {
        let display = info.as_ref().map_or(provider, |i| i.display_name);
        let enterprise_domain = url.map(|u| u.trim_start_matches("https://").trim_end_matches('/'));
        let config = ucode_auth::github_copilot_device_config(enterprise_domain);

        println!("Starting device code flow for {display}...");

        let client = reqwest::Client::new();
        let pending = ucode_auth::request_device_code(&client, &config).await?;

        println!();
        println!("  Open:  {}", pending.verification_uri);
        println!("  Code:  {}", pending.user_code);
        println!();
        println!("Waiting for authorization...");

        let material = ucode_auth::poll_for_token(&client, &config, &pending).await?;
        store.store(provider, &material)?;
        println!("Authenticated successfully as {display}.");
        return Ok(());
    }

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

                let material = ucode_auth::complete_browser_oauth(&config, &pending, code).await?;
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

    // 4. No auth needed (e.g., Ollama).
    if matches!(&info, Some(i) if i.auth_methods.contains(&ucode_auth::AuthMethod::None)) {
        let display = info.as_ref().unwrap().display_name;
        println!("{display} does not require authentication.");
        return Ok(());
    }

    // 5. Default: prompt for API key (known provider or unknown).
    handle_set_key(store, provider)
}

#[cfg(test)]
mod tests {
    use ucode_auth::InMemoryStore;

    use super::*;

    #[test]
    fn status_shows_all_providers() {
        let store = InMemoryStore::new();
        handle_status(&store).unwrap();
    }

    #[test]
    fn logout_not_found_is_not_an_error() {
        let store = InMemoryStore::new();
        handle_logout(&store, "anthropic").unwrap();
    }

    #[test]
    fn logout_removes_existing_credential() {
        let store = InMemoryStore::new();
        store
            .store(
                "openai",
                &AuthMaterial::ApiKey {
                    key: "sk-test".into(),
                },
            )
            .unwrap();
        handle_logout(&store, "openai").unwrap();
        assert!(store.load("openai").is_err());
    }

    #[tokio::test]
    async fn login_no_auth_provider() {
        let store = InMemoryStore::new();
        handle_login(&store, "ollama", false, false, None)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn login_url_provider_attempts_wellknown() {
        let store = InMemoryStore::new();
        // Fails with an HTTP error since the URL doesn't exist,
        // but proves the well-known path is taken (not the API key path).
        let result = handle_login(
            &store,
            "https://nonexistent.example.com",
            false,
            false,
            None,
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn login_subscription_unknown_provider() {
        let store = InMemoryStore::new();
        // Unknown provider with --subscription should print a message, not error
        handle_login(&store, "groq", false, true, None)
            .await
            .unwrap();
    }

    #[test]
    fn set_key_then_status_shows_configured() {
        let store = InMemoryStore::new();
        store
            .store(
                "anthropic",
                &AuthMaterial::ApiKey {
                    key: "sk-ant-test".into(),
                },
            )
            .unwrap();
        let statuses = store.list_configured();
        let anthropic = statuses
            .iter()
            .find(|s| {
                matches!(s, CredentialStatus::Configured { provider, .. } if provider == "anthropic")
            });
        assert!(anthropic.is_some());
    }
}
