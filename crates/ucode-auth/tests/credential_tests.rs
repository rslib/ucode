use ucode_auth::{
    AuthError, AuthMaterial, CredentialStatus, CredentialStore, InMemoryStore, ProviderId, redact,
};

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

fn session(token: &str) -> AuthMaterial {
    AuthMaterial::SessionToken {
        token: token.into(),
        expires_at: Some("2026-06-01T00:00:00Z".into()),
    }
}

#[test]
fn store_and_load_api_key() {
    let store = InMemoryStore::new();
    let mat = api_key("sk-test-key");
    store.store(ProviderId::OpenAi, &mat).unwrap();
    assert_eq!(store.load(ProviderId::OpenAi).unwrap(), mat);
}

#[test]
fn store_and_load_oauth() {
    let store = InMemoryStore::new();
    let mat = oauth("access-token-abc");
    store.store(ProviderId::Anthropic, &mat).unwrap();
    assert_eq!(store.load(ProviderId::Anthropic).unwrap(), mat);
}

#[test]
fn store_and_load_session_token() {
    let store = InMemoryStore::new();
    let mat = session("sess-token-xyz");
    store.store(ProviderId::Ollama, &mat).unwrap();
    assert_eq!(store.load(ProviderId::Ollama).unwrap(), mat);
}

#[test]
fn load_not_found() {
    let store = InMemoryStore::new();
    let err = store.load(ProviderId::OpenAi).unwrap_err();
    assert!(matches!(err, AuthError::NotFound { .. }));
}

#[test]
fn delete_credential() {
    let store = InMemoryStore::new();
    store.store(ProviderId::OpenAi, &api_key("key")).unwrap();
    store.delete(ProviderId::OpenAi).unwrap();
    let err = store.load(ProviderId::OpenAi).unwrap_err();
    assert!(matches!(err, AuthError::NotFound { .. }));
}

#[test]
fn delete_not_found() {
    let store = InMemoryStore::new();
    let err = store.delete(ProviderId::OpenAi).unwrap_err();
    assert!(matches!(err, AuthError::NotFound { .. }));
}

#[test]
fn status_configured() {
    let store = InMemoryStore::new();
    store.store(ProviderId::OpenAi, &api_key("key")).unwrap();
    let s = store.status(ProviderId::OpenAi);
    assert!(
        matches!(s, CredentialStatus::Configured { provider: ProviderId::OpenAi, ref kind } if kind == "api_key")
    );
}

#[test]
fn status_not_configured() {
    let store = InMemoryStore::new();
    assert_eq!(
        store.status(ProviderId::Anthropic),
        CredentialStatus::NotConfigured {
            provider: ProviderId::Anthropic
        }
    );
}

#[test]
fn list_configured() {
    let store = InMemoryStore::new();
    store.store(ProviderId::OpenAi, &api_key("k1")).unwrap();
    store.store(ProviderId::Anthropic, &api_key("k2")).unwrap();

    let statuses = store.list_configured();
    assert_eq!(statuses.len(), 3);

    let configured: Vec<_> = statuses
        .iter()
        .filter(|s| matches!(s, CredentialStatus::Configured { .. }))
        .collect();
    let not_configured: Vec<_> = statuses
        .iter()
        .filter(|s| matches!(s, CredentialStatus::NotConfigured { .. }))
        .collect();

    assert_eq!(configured.len(), 2);
    assert_eq!(not_configured.len(), 1);
}

#[test]
fn overwrite_credential() {
    let store = InMemoryStore::new();
    store
        .store(ProviderId::OpenAi, &api_key("old-key"))
        .unwrap();
    let new_mat = oauth("new-access-token");
    store.store(ProviderId::OpenAi, &new_mat).unwrap();
    assert_eq!(store.load(ProviderId::OpenAi).unwrap(), new_mat);
}

#[test]
fn redact_short() {
    assert_eq!(redact("abc"), "****");
}

#[test]
fn redact_long() {
    // "sk-1234567890abcdef" is 18 chars — first 4 = "sk-1", last 4 = "cdef"
    assert_eq!(redact("sk-1234567890abcdef"), "sk-1...cdef");
}

#[test]
fn auth_material_serde_roundtrip() {
    let cases = [
        api_key("my-api-key"),
        oauth("access"),
        AuthMaterial::OAuth {
            access_token: "tok".into(),
            refresh_token: None,
            expires_at: None,
        },
        session("sess"),
        AuthMaterial::SessionToken {
            token: "t".into(),
            expires_at: None,
        },
    ];

    for mat in &cases {
        let json = serde_json::to_string(mat).expect("serialize");
        let back: AuthMaterial = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(&back, mat);
    }
}

#[test]
fn provider_id_display() {
    assert_eq!(ProviderId::OpenAi.to_string(), "openai");
    assert_eq!(ProviderId::Anthropic.to_string(), "anthropic");
    assert_eq!(ProviderId::Ollama.to_string(), "ollama");
}
