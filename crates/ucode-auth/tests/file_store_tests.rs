use ucode_auth::{AuthError, AuthMaterial, CredentialStatus, CredentialStore, FileStore};

fn api_key(key: &str) -> AuthMaterial {
    AuthMaterial::ApiKey { key: key.into() }
}

fn oauth(access: &str) -> AuthMaterial {
    AuthMaterial::OAuth {
        access_token: access.into(),
        refresh_token: Some("refresh-xyz".into()),
        expires_at: Some("2026-01-01T00:00:00Z".into()),
    }
}

fn temp_store() -> (FileStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("auth.json");
    let store = FileStore::with_path(path);
    (store, dir)
}

#[test]
fn store_and_load() {
    let (store, _dir) = temp_store();
    let mat = api_key("sk-test");
    store.store("openai", &mat).unwrap();
    assert_eq!(store.load("openai").unwrap(), mat);
}

#[test]
fn load_not_found() {
    let (store, _dir) = temp_store();
    let err = store.load("openai").unwrap_err();
    assert!(matches!(err, AuthError::NotFound { .. }));
}

#[test]
fn delete_credential() {
    let (store, _dir) = temp_store();
    store.store("openai", &api_key("key")).unwrap();
    store.delete("openai").unwrap();
    assert!(store.load("openai").is_err());
}

#[test]
fn delete_not_found() {
    let (store, _dir) = temp_store();
    let err = store.delete("openai").unwrap_err();
    assert!(matches!(err, AuthError::NotFound { .. }));
}

#[test]
fn multiple_providers() {
    let (store, _dir) = temp_store();
    store.store("openai", &api_key("k1")).unwrap();
    store.store("anthropic", &oauth("tok")).unwrap();
    store.store("my-proxy", &api_key("k3")).unwrap();

    assert_eq!(store.load("openai").unwrap(), api_key("k1"));
    assert_eq!(store.load("my-proxy").unwrap(), api_key("k3"));

    let statuses = store.list_configured();
    assert_eq!(statuses.len(), 3);
}

#[test]
fn overwrite_credential() {
    let (store, _dir) = temp_store();
    store.store("openai", &api_key("old")).unwrap();
    store.store("openai", &api_key("new")).unwrap();
    assert_eq!(store.load("openai").unwrap(), api_key("new"));
}

#[test]
fn persists_across_instances() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("auth.json");

    let store1 = FileStore::with_path(path.clone());
    store1.store("openai", &api_key("persistent")).unwrap();

    let store2 = FileStore::with_path(path);
    assert_eq!(store2.load("openai").unwrap(), api_key("persistent"));
}

#[test]
fn status_configured() {
    let (store, _dir) = temp_store();
    store.store("openai", &api_key("key")).unwrap();
    let s = store.status("openai");
    assert!(matches!(
        s,
        CredentialStatus::Configured { ref provider, ref kind }
        if provider == "openai" && kind == "api_key"
    ));
}

#[test]
fn status_not_configured() {
    let (store, _dir) = temp_store();
    assert_eq!(
        store.status("openai"),
        CredentialStatus::NotConfigured {
            provider: "openai".into()
        }
    );
}

#[cfg(unix)]
#[test]
fn file_permissions_are_0600() {
    use std::os::unix::fs::PermissionsExt;

    let (store, _dir) = temp_store();
    store.store("openai", &api_key("key")).unwrap();

    let path = _dir.path().join("auth.json");
    let perms = std::fs::metadata(&path).unwrap().permissions();
    assert_eq!(perms.mode() & 0o777, 0o600);
}
