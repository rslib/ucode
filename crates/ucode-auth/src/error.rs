/// Errors produced by credential store operations.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("no credentials found for provider '{provider}'")]
    NotFound { provider: String },

    #[error("keyring error: {message}")]
    Keyring { message: String },

    #[error("serialization error: {message}")]
    Serialization { message: String },

    #[error("unknown provider type: '{name}'")]
    InvalidProvider { name: String },

    #[error("file store error: {message}")]
    FileStore { message: String },

    #[error("missing credential for provider '{provider}': {detail}")]
    MissingCredential { provider: String, detail: String },

    #[error("auth flow error: {message}")]
    AuthFlow { message: String },

    #[error("device code flow timed out")]
    DeviceCodeTimeout,

    #[error("authorization denied by user")]
    AuthDenied,

    #[error("HTTP request failed: {message}")]
    Http { message: String },

    #[error("auth token expired for provider '{provider}': {detail}")]
    AuthExpired { provider: String, detail: String },
}
