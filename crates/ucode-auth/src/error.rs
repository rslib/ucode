/// Errors produced by credential store operations.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("no credentials found for provider '{provider}'")]
    NotFound { provider: String },

    #[error("keyring error: {message}")]
    Keyring { message: String },

    #[error("serialization error: {message}")]
    Serialization { message: String },
}
