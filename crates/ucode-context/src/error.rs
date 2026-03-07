use thiserror::Error;

#[derive(Debug, Error)]
pub enum ContextError {
    #[error("knowledge base error: {0}")]
    KnowledgeBase(String),

    #[error("embedding error: {0}")]
    Embedding(String),

    #[error("continuity error: {0}")]
    Continuity(String),

    #[error("config error: {0}")]
    Config(String),

    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}
