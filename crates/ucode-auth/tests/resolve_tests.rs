use ucode_auth::{AuthError, AuthMaterial, CredentialStore, InMemoryStore, resolve_auth};

fn api_key(key: &str) -> AuthMaterial {
    AuthMaterial::ApiKey { key: key.into() }
}

#[test]
fn env_var_takes_precedence() {
    let store = InMemoryStore::new();
    store
        .store("test-provider", &api_key("stored-key"))
        .unwrap();

    let env_name = "UCODE_TEST_RESOLVE_PRECEDENCE";
    // SAFETY: single-threaded test; no other thread reads this var concurrently.
    unsafe { std::env::set_var(env_name, "env-key") };
    let result = resolve_auth("test-provider", Some(env_name), &store);
    // SAFETY: same as above.
    unsafe { std::env::remove_var(env_name) };

    assert_eq!(result.unwrap(), api_key("env-key"));
}

#[test]
fn falls_back_to_store() {
    let store = InMemoryStore::new();
    store
        .store("test-provider", &api_key("stored-key"))
        .unwrap();

    let result = resolve_auth("test-provider", None, &store);
    assert_eq!(result.unwrap(), api_key("stored-key"));
}

#[test]
fn env_var_empty_falls_through() {
    let store = InMemoryStore::new();
    store
        .store("test-provider", &api_key("stored-key"))
        .unwrap();

    let env_name = "UCODE_TEST_RESOLVE_EMPTY";
    // SAFETY: single-threaded test; no other thread reads this var concurrently.
    unsafe { std::env::set_var(env_name, "") };
    let result = resolve_auth("test-provider", Some(env_name), &store);
    // SAFETY: same as above.
    unsafe { std::env::remove_var(env_name) };

    assert_eq!(result.unwrap(), api_key("stored-key"));
}

#[test]
fn missing_everywhere_returns_error() {
    let store = InMemoryStore::new();
    let result = resolve_auth(
        "test-provider",
        Some("UCODE_TEST_NONEXISTENT_VAR_12345"),
        &store,
    );
    assert!(matches!(
        result.unwrap_err(),
        AuthError::MissingCredential { .. }
    ));
}

#[test]
fn no_env_var_name_and_no_store_returns_error() {
    let store = InMemoryStore::new();
    let result = resolve_auth("test-provider", None, &store);
    assert!(matches!(
        result.unwrap_err(),
        AuthError::MissingCredential { .. }
    ));
}

#[test]
fn env_var_unset_falls_to_store() {
    let store = InMemoryStore::new();
    store
        .store("test-provider", &api_key("stored-key"))
        .unwrap();

    // SAFETY: single-threaded test; no other thread reads this var concurrently.
    unsafe { std::env::remove_var("UCODE_TEST_UNSET_VAR") };
    let result = resolve_auth("test-provider", Some("UCODE_TEST_UNSET_VAR"), &store);
    assert_eq!(result.unwrap(), api_key("stored-key"));
}

#[test]
fn error_message_includes_env_var_hint() {
    let store = InMemoryStore::new();
    let err = resolve_auth("groq", Some("GROQ_API_KEY"), &store).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("GROQ_API_KEY"));
    assert!(msg.contains("groq"));
}

#[test]
fn error_message_without_env_var() {
    let store = InMemoryStore::new();
    let err = resolve_auth("custom", None, &store).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("ucode auth login custom"));
}
