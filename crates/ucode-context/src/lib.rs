//! ucode-context: native context management -- strategies, knowledge base, session continuity

pub mod config;
pub mod continuity;
pub mod dedup;
pub mod embedder;
pub mod error;
pub mod knowledge;
pub mod purge;
pub mod sandbox;
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
pub use embedder::Embedder;
pub use error::ContextError;
pub use knowledge::{KnowledgeBase, KnowledgeEntry};
pub use purge::PurgeErrorsStrategy;
pub use sandbox::SandboxInterceptor;
pub use strategy::{ContextPipeline, ContextStrategy, StrategyContext, StrategyResult};
pub use supersede::SupersedeStrategy;

/// Construct a `ContextPipeline` from `config`, adding only enabled strategies in order:
/// dedup -> supersede -> purge -> sandbox.
pub fn build_pipeline(config: &ContextConfig) -> ContextPipeline {
    let mut pipeline = ContextPipeline::new();

    if config.strategies.dedup {
        pipeline.add_strategy(Box::new(DedupStrategy));
    }
    if config.strategies.supersede_writes {
        pipeline.add_strategy(Box::new(SupersedeStrategy));
    }
    if config.strategies.purge_errors {
        pipeline.add_strategy(Box::new(PurgeErrorsStrategy::new(
            config.strategies.purge_errors_after_turns,
        )));
    }
    if config.strategies.sandbox_large_outputs {
        pipeline.add_strategy(Box::new(SandboxInterceptor::new(
            config.strategies.sandbox_threshold_chars,
        )));
    }

    pipeline
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_from_config_respects_toggles() {
        let config = ContextConfig {
            strategies: StrategiesConfig {
                dedup: false,
                ..StrategiesConfig::default()
            },
            ..ContextConfig::default()
        };

        let pipeline = build_pipeline(&config);
        let names = pipeline.strategy_names();

        // dedup disabled -> 3 strategies remain
        assert_eq!(names.len(), 3);
        assert!(!names.contains(&"dedup"));
        assert!(names.contains(&"supersede"));
        assert!(names.contains(&"purge_errors"));
        assert!(names.contains(&"sandbox"));
    }
}
