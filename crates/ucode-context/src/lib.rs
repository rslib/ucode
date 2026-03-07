//! ucode-context: native context management -- strategies, knowledge base, session continuity

pub mod config;
pub mod continuity;
pub mod dedup;
pub mod error;
pub mod purge;
pub mod strategy;
pub mod supersede;

pub use config::{
    ContextConfig, EmbeddingEndpointConfig, EmbeddingMode, KnowledgeBaseConfig, PruningConfig,
    PruningOverride, StrategiesConfig,
};
pub use continuity::{
    CompactionSnapshot, ContinuityEvent, ContinuityEventType, ErrorRecord, GitState,
    SessionContinuity,
};
pub use dedup::DedupStrategy;
pub use error::ContextError;
pub use purge::PurgeErrorsStrategy;
pub use strategy::{ContextPipeline, ContextStrategy, StrategyContext, StrategyResult};
pub use supersede::SupersedeStrategy;
