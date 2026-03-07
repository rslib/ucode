use ucode_auth::{AuthError, AuthMaterial, ChainStore, CredentialStore, InMemoryStore};

fn api_key(key: &str) -> AuthMaterial {
    AuthMaterial::ApiKey { key: key.into() }
}

#[test]
fn load_from_primary() {
    let primary = InMemoryStore::new();
    let fallback = InMemoryStore::new();
    primary.store("openai", &api_key("primary-key")).unwrap();
    fallback.store("openai", &api_key("fallback-key")).unwrap();

    let chain = ChainStore::new(Box::new(primary), Box::new(fallback));
    assert_eq!(chain.load("openai").unwrap(), api_key("primary-key"));
}

#[test]
fn load_falls_back() {
    let primary = InMemoryStore::new();
    let fallback = InMemoryStore::new();
    fallback.store("openai", &api_key("fallback-key")).unwrap();

    let chain = ChainStore::new(Box::new(primary), Box::new(fallback));
    assert_eq!(chain.load("openai").unwrap(), api_key("fallback-key"));
}

#[test]
fn load_not_found_in_either() {
    let primary = InMemoryStore::new();
    let fallback = InMemoryStore::new();
    let chain = ChainStore::new(Box::new(primary), Box::new(fallback));
    assert!(matches!(
        chain.load("openai").unwrap_err(),
        AuthError::NotFound { .. }
    ));
}

#[test]
fn store_writes_to_primary() {
    let primary = InMemoryStore::new();
    let fallback = InMemoryStore::new();
    let chain = ChainStore::new(Box::new(primary), Box::new(fallback));

    chain.store("openai", &api_key("new-key")).unwrap();
    assert_eq!(chain.load("openai").unwrap(), api_key("new-key"));
}

#[test]
fn delete_from_both() {
    let primary = InMemoryStore::new();
    let fallback = InMemoryStore::new();
    primary.store("openai", &api_key("k1")).unwrap();
    fallback.store("openai", &api_key("k2")).unwrap();

    let chain = ChainStore::new(Box::new(primary), Box::new(fallback));
    chain.delete("openai").unwrap();
    assert!(chain.load("openai").is_err());
}

#[test]
fn list_configured_merges() {
    let primary = InMemoryStore::new();
    let fallback = InMemoryStore::new();
    primary.store("openai", &api_key("k1")).unwrap();
    fallback.store("anthropic", &api_key("k2")).unwrap();

    let chain = ChainStore::new(Box::new(primary), Box::new(fallback));
    let statuses = chain.list_configured();
    assert_eq!(statuses.len(), 2);
}

#[test]
fn list_configured_deduplicates() {
    let primary = InMemoryStore::new();
    let fallback = InMemoryStore::new();
    primary.store("openai", &api_key("primary")).unwrap();
    fallback.store("openai", &api_key("fallback")).unwrap();

    let chain = ChainStore::new(Box::new(primary), Box::new(fallback));
    let statuses = chain.list_configured();
    // Should only have 1 entry for "openai" (from primary)
    assert_eq!(statuses.len(), 1);
}
