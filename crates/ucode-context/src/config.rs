use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContextConfig {
    pub enabled: bool,
    pub strategies: StrategiesConfig,
    pub knowledge_base: KnowledgeBaseConfig,
    pub pruning: PruningConfig,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            strategies: StrategiesConfig::default(),
            knowledge_base: KnowledgeBaseConfig::default(),
            pruning: PruningConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StrategiesConfig {
    pub dedup: bool,
    pub supersede_writes: bool,
    pub purge_errors: bool,
    pub purge_errors_after_turns: usize,
    pub sandbox_large_outputs: bool,
    pub sandbox_threshold_chars: usize,
    pub knowledge_base: bool,
    pub session_continuity: bool,
}

impl Default for StrategiesConfig {
    fn default() -> Self {
        Self {
            dedup: true,
            supersede_writes: true,
            purge_errors: true,
            purge_errors_after_turns: 3,
            sandbox_large_outputs: true,
            sandbox_threshold_chars: 2000,
            knowledge_base: true,
            session_continuity: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KnowledgeBaseConfig {
    pub enabled: bool,
    pub embedding: EmbeddingMode,
    pub embedding_endpoint: Option<EmbeddingEndpointConfig>,
}

impl Default for KnowledgeBaseConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            embedding: EmbeddingMode::Auto,
            embedding_endpoint: None,
        }
    }
}

/// How to generate embeddings for vector search.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingMode {
    Auto,
    Local,
    Endpoint,
    None,
}

/// Custom embedding endpoint configuration (OpenAI-compatible API).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EmbeddingEndpointConfig {
    pub url: String,
    pub model: String,
    pub dimensions: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PruningConfig {
    pub enabled: bool,
    pub trigger_threshold_pct: u8,
    pub model: String,
    pub overrides: HashMap<String, PruningOverride>,
}

impl Default for PruningConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            trigger_threshold_pct: 60,
            model: "auto".to_string(),
            overrides: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PruningOverride {
    pub enabled: Option<bool>,
    pub trigger_threshold_pct: Option<u8>,
}

impl PruningConfig {
    /// Resolve effective (enabled, threshold) for a given model name.
    ///
    /// Checks `overrides` first; falls back to the top-level config values.
    pub fn resolve(&self, model_name: &str) -> (bool, u8) {
        match self.overrides.get(model_name) {
            Some(ov) => {
                let enabled = ov.enabled.unwrap_or(self.enabled);
                let threshold = ov
                    .trigger_threshold_pct
                    .unwrap_or(self.trigger_threshold_pct);
                (enabled, threshold)
            }
            None => (self.enabled, self.trigger_threshold_pct),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_all_strategies_enabled() {
        let cfg = ContextConfig::default();
        assert!(cfg.enabled);

        let s = &cfg.strategies;
        assert!(s.dedup);
        assert!(s.supersede_writes);
        assert!(s.purge_errors);
        assert_eq!(s.purge_errors_after_turns, 3);
        assert!(s.sandbox_large_outputs);
        assert_eq!(s.sandbox_threshold_chars, 2000);
        assert!(s.knowledge_base);
        assert!(s.session_continuity);

        let kb = &cfg.knowledge_base;
        assert!(kb.enabled);
        assert_eq!(kb.embedding, EmbeddingMode::Auto);
        assert!(kb.embedding_endpoint.is_none());

        let p = &cfg.pruning;
        assert!(p.enabled);
        assert_eq!(p.trigger_threshold_pct, 60);
        assert_eq!(p.model, "auto");
        assert!(p.overrides.is_empty());
    }

    #[test]
    fn config_roundtrip_json() {
        let original = ContextConfig {
            enabled: true,
            strategies: StrategiesConfig {
                dedup: false,
                purge_errors_after_turns: 5,
                sandbox_threshold_chars: 4000,
                ..StrategiesConfig::default()
            },
            knowledge_base: KnowledgeBaseConfig {
                embedding: EmbeddingMode::Endpoint,
                embedding_endpoint: Some(EmbeddingEndpointConfig {
                    url: "http://localhost:8080".to_string(),
                    model: "text-embedding-3-small".to_string(),
                    dimensions: 1536,
                }),
                ..KnowledgeBaseConfig::default()
            },
            pruning: PruningConfig {
                model: "claude-3-haiku".to_string(),
                ..PruningConfig::default()
            },
        };

        let json = serde_json::to_string(&original).unwrap();
        let restored: ContextConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(original, restored);
    }

    #[test]
    fn embedding_mode_default_is_auto() {
        let kb = KnowledgeBaseConfig::default();
        assert_eq!(kb.embedding, EmbeddingMode::Auto);
    }

    #[test]
    fn pruning_override_resolves_correctly() {
        let mut cfg = PruningConfig::default(); // enabled=true, threshold=60
        cfg.overrides.insert(
            "gpt-4".to_string(),
            PruningOverride {
                enabled: Some(false),
                trigger_threshold_pct: None,
            },
        );
        cfg.overrides.insert(
            "claude-3-opus".to_string(),
            PruningOverride {
                enabled: None,
                trigger_threshold_pct: Some(80),
            },
        );

        // No override: falls back to defaults
        let (enabled, threshold) = cfg.resolve("unknown-model");
        assert!(enabled);
        assert_eq!(threshold, 60);

        // Override disables pruning, threshold falls back
        let (enabled, threshold) = cfg.resolve("gpt-4");
        assert!(!enabled);
        assert_eq!(threshold, 60);

        // Override changes threshold, enabled falls back
        let (enabled, threshold) = cfg.resolve("claude-3-opus");
        assert!(enabled);
        assert_eq!(threshold, 80);
    }
}
