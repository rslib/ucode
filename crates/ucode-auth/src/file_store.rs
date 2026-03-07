use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use crate::credential::{AuthMaterial, CredentialStatus, CredentialStore, material_kind};
use crate::error::AuthError;

/// Credential store backed by a JSON file.
///
/// File format: `{ "provider-id": { "type": "api_key", "key": "..." }, ... }`
///
/// File permissions are set to 0o600 on Unix.
pub struct FileStore {
    path: PathBuf,
}

impl FileStore {
    /// Create a FileStore at the default path (`~/.local/share/ucode/auth.json`).
    pub fn new() -> Result<Self, AuthError> {
        let dir = dirs::data_local_dir()
            .ok_or_else(|| AuthError::FileStore {
                message: "cannot determine local data directory".into(),
            })?
            .join("ucode");
        Ok(Self {
            path: dir.join("auth.json"),
        })
    }

    /// Create a FileStore at a specific path (for testing).
    pub fn with_path(path: PathBuf) -> Self {
        Self { path }
    }

    fn read_all(&self) -> Result<HashMap<String, AuthMaterial>, AuthError> {
        if !self.path.exists() {
            return Ok(HashMap::new());
        }
        let data = fs::read_to_string(&self.path).map_err(|e| AuthError::FileStore {
            message: format!("read {}: {e}", self.path.display()),
        })?;
        serde_json::from_str(&data).map_err(|e| AuthError::Serialization {
            message: format!("parse {}: {e}", self.path.display()),
        })
    }

    fn write_all(&self, data: &HashMap<String, AuthMaterial>) -> Result<(), AuthError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| AuthError::FileStore {
                message: format!("create dir {}: {e}", parent.display()),
            })?;
        }
        let json = serde_json::to_string_pretty(data).map_err(|e| AuthError::Serialization {
            message: e.to_string(),
        })?;
        fs::write(&self.path, &json).map_err(|e| AuthError::FileStore {
            message: format!("write {}: {e}", self.path.display()),
        })?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::Permissions::from_mode(0o600);
            fs::set_permissions(&self.path, perms).map_err(|e| AuthError::FileStore {
                message: format!("chmod {}: {e}", self.path.display()),
            })?;
        }

        Ok(())
    }
}

impl CredentialStore for FileStore {
    fn store(&self, provider: &str, material: &AuthMaterial) -> Result<(), AuthError> {
        let mut data = self.read_all()?;
        data.insert(provider.to_owned(), material.clone());
        self.write_all(&data)
    }

    fn load(&self, provider: &str) -> Result<AuthMaterial, AuthError> {
        let data = self.read_all()?;
        data.get(provider)
            .cloned()
            .ok_or_else(|| AuthError::NotFound {
                provider: provider.to_owned(),
            })
    }

    fn delete(&self, provider: &str) -> Result<(), AuthError> {
        let mut data = self.read_all()?;
        if data.remove(provider).is_none() {
            return Err(AuthError::NotFound {
                provider: provider.to_owned(),
            });
        }
        self.write_all(&data)
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
        match self.read_all() {
            Ok(data) => data
                .iter()
                .map(|(id, mat)| CredentialStatus::Configured {
                    provider: id.clone(),
                    kind: material_kind(mat).into(),
                })
                .collect(),
            Err(_) => Vec::new(),
        }
    }
}
