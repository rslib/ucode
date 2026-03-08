use std::collections::HashSet;

use crate::credential::{AuthMaterial, CredentialStatus, CredentialStore, material_kind};
use crate::error::AuthError;

/// Credential store that tries a primary store first, falling back to a secondary.
///
/// Writes go to both stores so the fallback acts as a persistent backup.
/// If the primary fails but the fallback succeeds, the write is considered
/// successful (with a warning). Reads try primary first, then fallback.
/// Deletes attempt both stores (best-effort on fallback).
pub struct ChainStore {
    primary: Box<dyn CredentialStore>,
    fallback: Box<dyn CredentialStore>,
}

impl ChainStore {
    pub fn new(primary: Box<dyn CredentialStore>, fallback: Box<dyn CredentialStore>) -> Self {
        Self { primary, fallback }
    }
}

impl CredentialStore for ChainStore {
    fn store(&self, provider: &str, material: &AuthMaterial) -> Result<(), AuthError> {
        // Always persist to fallback (file) as a backup copy.
        let fallback_ok = self.fallback.store(provider, material).is_ok();
        match self.primary.store(provider, material) {
            Ok(()) => Ok(()),
            Err(e) if fallback_ok => {
                // Primary (keyring) failed but fallback (file) succeeded — credential is saved.
                tracing::warn!("keyring store failed ({e}), credential saved to file store");
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    fn load(&self, provider: &str) -> Result<AuthMaterial, AuthError> {
        match self.primary.load(provider) {
            Ok(mat) => Ok(mat),
            Err(AuthError::NotFound { .. }) => self.fallback.load(provider),
            Err(e) => {
                // Primary had a real error (keyring failure, etc.) — try fallback
                match self.fallback.load(provider) {
                    Ok(mat) => Ok(mat),
                    Err(AuthError::NotFound { .. }) => Err(e),
                    Err(fallback_err) => Err(fallback_err),
                }
            }
        }
    }

    fn delete(&self, provider: &str) -> Result<(), AuthError> {
        let primary_result = self.primary.delete(provider);
        // Best-effort delete from fallback
        let _ = self.fallback.delete(provider);
        primary_result
    }

    fn status(&self, provider: &str) -> CredentialStatus {
        match self.load(provider) {
            Ok(mat) => CredentialStatus::Configured {
                provider: provider.to_owned(),
                kind: material_kind(&mat).into(),
            },
            Err(_) => CredentialStatus::NotConfigured {
                provider: provider.to_owned(),
            },
        }
    }

    fn list_configured(&self) -> Vec<CredentialStatus> {
        let mut seen = HashSet::new();
        let mut result = Vec::new();

        for status in self.primary.list_configured() {
            if let CredentialStatus::Configured { ref provider, .. } = status {
                seen.insert(provider.clone());
                result.push(status);
            }
        }

        for status in self.fallback.list_configured() {
            if let CredentialStatus::Configured { ref provider, .. } = status
                && !seen.contains(provider)
            {
                result.push(status);
            }
        }

        result
    }
}
