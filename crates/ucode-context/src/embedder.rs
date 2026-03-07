use crate::error::ContextError;

/// Abstraction for embedding generation.
pub trait Embedder: Send + Sync {
    fn embed(&self, text: &str) -> Result<Vec<f32>, ContextError>;
    fn dimensions(&self) -> usize;
}
