use ucode_auth::{
    AuthError, AuthMaterial, CredentialStatus, CredentialStore, InMemoryStore, ProviderType, redact,
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

fn wellknown(env_key: &str, token: &str) -> AuthMaterial {
    AuthMaterial::WellKnown {
        env_key: env_key.into(),
        token: token.into(),
    }
}

fn aws_creds() -> AuthMaterial {
    AuthMaterial::AwsCredentials {
        access_key_id: "AKIAIOSFODNN7EXAMPLE".into(),
        secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
        session_token: Some("FwoGZXIvYXdzEBY...".into()),
        region: "us-east-1".into(),
    }
}

// ── Store / Load ─────────────────────────────────────────────────────────────

#[test]
fn store_and_load_api_key() {
    let store = InMemoryStore::new();
    let mat = api_key("sk-test-key");
    store.store("openai", &mat).unwrap();
    assert_eq!(store.load("openai").unwrap(), mat);
}

#[test]
fn store_and_load_oauth() {
    let store = InMemoryStore::new();
    let mat = oauth("access-token-abc");
    store.store("anthropic", &mat).unwrap();
    assert_eq!(store.load("anthropic").unwrap(), mat);
}

#[test]
fn store_and_load_session_token() {
    let store = InMemoryStore::new();
    let mat = session("sess-token-xyz");
    store.store("ollama", &mat).unwrap();
    assert_eq!(store.load("ollama").unwrap(), mat);
}

#[test]
fn store_and_load_wellknown() {
    let store = InMemoryStore::new();
    let mat = wellknown("CUSTOM_API_KEY", "tok-abc-123");
    store.store("custom-provider", &mat).unwrap();
    assert_eq!(store.load("custom-provider").unwrap(), mat);
}

#[test]
fn store_and_load_aws_credentials() {
    let store = InMemoryStore::new();
    let mat = aws_creds();
    store.store("aws-bedrock", &mat).unwrap();
    assert_eq!(store.load("aws-bedrock").unwrap(), mat);
}

#[test]
fn arbitrary_provider_name() {
    let store = InMemoryStore::new();
    let mat = api_key("sk-custom");
    store.store("my-custom-proxy", &mat).unwrap();
    assert_eq!(store.load("my-custom-proxy").unwrap(), mat);
}

#[test]
fn load_not_found() {
    let store = InMemoryStore::new();
    let err = store.load("openai").unwrap_err();
    assert!(matches!(err, AuthError::NotFound { .. }));
}

#[test]
fn delete_credential() {
    let store = InMemoryStore::new();
    store.store("openai", &api_key("key")).unwrap();
    store.delete("openai").unwrap();
    let err = store.load("openai").unwrap_err();
    assert!(matches!(err, AuthError::NotFound { .. }));
}

#[test]
fn delete_not_found() {
    let store = InMemoryStore::new();
    let err = store.delete("openai").unwrap_err();
    assert!(matches!(err, AuthError::NotFound { .. }));
}

#[test]
fn status_configured() {
    let store = InMemoryStore::new();
    store.store("openai", &api_key("key")).unwrap();
    let s = store.status("openai");
    assert!(
        matches!(s, CredentialStatus::Configured { ref provider, ref kind } if provider == "openai" && kind == "api_key")
    );
}

#[test]
fn status_not_configured() {
    let store = InMemoryStore::new();
    assert_eq!(
        store.status("anthropic"),
        CredentialStatus::NotConfigured {
            provider: "anthropic".into()
        }
    );
}

#[test]
fn list_configured_returns_only_stored() {
    let store = InMemoryStore::new();
    store.store("openai", &api_key("k1")).unwrap();
    store.store("anthropic", &api_key("k2")).unwrap();

    let statuses = store.list_configured();
    assert_eq!(statuses.len(), 2);

    for s in &statuses {
        assert!(matches!(s, CredentialStatus::Configured { .. }));
    }
}

#[test]
fn overwrite_credential() {
    let store = InMemoryStore::new();
    store.store("openai", &api_key("old-key")).unwrap();
    let new_mat = oauth("new-access-token");
    store.store("openai", &new_mat).unwrap();
    assert_eq!(store.load("openai").unwrap(), new_mat);
}

// ── Redact ───────────────────────────────────────────────────────────────────

#[test]
fn redact_short() {
    assert_eq!(redact("abc"), "****");
}

#[test]
fn redact_long() {
    assert_eq!(redact("sk-1234567890abcdef"), "sk-1...cdef");
}

// ── Serde ────────────────────────────────────────────────────────────────────

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
        wellknown("MY_KEY", "secret-value"),
        aws_creds(),
        AuthMaterial::AwsCredentials {
            access_key_id: "AKIA".into(),
            secret_access_key: "secret".into(),
            session_token: None,
            region: "eu-west-1".into(),
        },
    ];

    for mat in &cases {
        let json = serde_json::to_string(mat).expect("serialize");
        let back: AuthMaterial = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(&back, mat);
    }
}

// ── ProviderType ─────────────────────────────────────────────────────────────

#[test]
fn provider_type_from_str_valid() {
    assert_eq!(
        "openai".parse::<ProviderType>().unwrap(),
        ProviderType::OpenAi
    );
    assert_eq!(
        "anthropic".parse::<ProviderType>().unwrap(),
        ProviderType::Anthropic
    );
    assert_eq!(
        "ollama".parse::<ProviderType>().unwrap(),
        ProviderType::Ollama
    );
    assert_eq!(
        "gemini".parse::<ProviderType>().unwrap(),
        ProviderType::Gemini
    );
    // case-insensitive
    assert_eq!(
        "OpenAI".parse::<ProviderType>().unwrap(),
        ProviderType::OpenAi
    );
    assert_eq!(
        "ANTHROPIC".parse::<ProviderType>().unwrap(),
        ProviderType::Anthropic
    );
}

#[test]
fn provider_type_from_str_invalid() {
    assert!("unknown".parse::<ProviderType>().is_err());
}

#[test]
fn provider_type_display() {
    assert_eq!(ProviderType::OpenAi.to_string(), "openai");
    assert_eq!(ProviderType::Anthropic.to_string(), "anthropic");
    assert_eq!(ProviderType::Ollama.to_string(), "ollama");
    assert_eq!(ProviderType::Gemini.to_string(), "gemini");
}
