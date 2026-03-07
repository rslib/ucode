use ucode_core::{Message, TokenBudget, TokenCounter};

/// Shared context passed to each strategy.
pub struct StrategyContext<'a> {
    pub session_id: &'a str,
    pub turn_count: usize,
    pub token_budget: &'a TokenBudget,
    pub counter: &'a dyn TokenCounter,
}

/// Result of applying a strategy.
#[derive(Debug, Default)]
pub struct StrategyResult {
    pub messages_removed: usize,
    pub messages_modified: usize,
    pub chars_saved: usize,
}

/// A message transform strategy.
pub trait ContextStrategy: Send + Sync {
    fn name(&self) -> &str;
    fn apply(&self, messages: &mut Vec<Message>, ctx: &StrategyContext) -> StrategyResult;
}

pub struct ContextPipeline {
    strategies: Vec<Box<dyn ContextStrategy>>,
}

impl ContextPipeline {
    pub fn new() -> Self {
        Self {
            strategies: Vec::new(),
        }
    }

    pub fn add_strategy(&mut self, strategy: Box<dyn ContextStrategy>) {
        self.strategies.push(strategy);
    }

    pub fn transform(
        &self,
        messages: &mut Vec<Message>,
        ctx: &StrategyContext,
    ) -> Vec<StrategyResult> {
        self.strategies
            .iter()
            .map(|s| s.apply(messages, ctx))
            .collect()
    }

    pub fn strategy_names(&self) -> Vec<&str> {
        self.strategies.iter().map(|s| s.name()).collect()
    }
}

impl Default for ContextPipeline {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use ucode_core::{CharEstimator, TokenBudget};

    use super::*;

    fn make_ctx<'a>(budget: &'a TokenBudget, counter: &'a dyn TokenCounter) -> StrategyContext<'a> {
        StrategyContext {
            session_id: "test-session",
            turn_count: 0,
            token_budget: budget,
            counter,
        }
    }

    #[test]
    fn empty_pipeline_returns_no_results() {
        let pipeline = ContextPipeline::new();
        let budget = TokenBudget::new(128_000, 4_096);
        let counter = CharEstimator::default();
        let ctx = make_ctx(&budget, &counter);
        let mut messages = vec![Message::user("hello")];

        let results = pipeline.transform(&mut messages, &ctx);
        assert!(results.is_empty());
    }

    #[test]
    fn pipeline_runs_strategies_in_order() {
        let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        struct OrderedStrategy {
            label: String,
            log: Arc<Mutex<Vec<String>>>,
        }

        impl ContextStrategy for OrderedStrategy {
            fn name(&self) -> &str {
                &self.label
            }

            fn apply(
                &self,
                _messages: &mut Vec<Message>,
                _ctx: &StrategyContext,
            ) -> StrategyResult {
                self.log.lock().unwrap().push(self.label.clone());
                StrategyResult::default()
            }
        }

        let mut pipeline = ContextPipeline::new();
        for label in ["alpha", "beta", "gamma"] {
            pipeline.add_strategy(Box::new(OrderedStrategy {
                label: label.to_string(),
                log: Arc::clone(&log),
            }));
        }

        let budget = TokenBudget::new(128_000, 4_096);
        let counter = CharEstimator::default();
        let ctx = make_ctx(&budget, &counter);
        let mut messages = vec![Message::user("test")];

        let results = pipeline.transform(&mut messages, &ctx);
        assert_eq!(results.len(), 3);

        let recorded = log.lock().unwrap();
        assert_eq!(*recorded, vec!["alpha", "beta", "gamma"]);
    }
}
