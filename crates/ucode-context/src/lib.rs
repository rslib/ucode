//! ucode-context: native context management -- strategies, knowledge base, session continuity

pub mod config;
pub mod error;
pub mod strategy;

pub use config::{
    ContextConfig, EmbeddingEndpointConfig, EmbeddingMode, KnowledgeBaseConfig, PruningConfig,
    PruningOverride, StrategiesConfig,
};
pub use error::ContextError;
pub use strategy::{ContextPipeline, ContextStrategy, StrategyContext, StrategyResult};
