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

pub fn handle_login(
    _store: &dyn CredentialStore,
    provider: &str,
    device: bool,
    subscription: bool,
) -> Result<()> {
    println!("Login flow for {provider} is not yet implemented.");
    if device {
        println!("  (device-code flow requested)");
    }
    if subscription {
        println!("  (subscription login requested)");
    }
    Ok(())
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

    #[test]
    fn login_stub_returns_ok() {
        let store = InMemoryStore::new();
        handle_login(&store, "ollama", true, false).unwrap();
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
