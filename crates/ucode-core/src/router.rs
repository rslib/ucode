use std::collections::HashSet;

use serde::{Deserialize, Serialize};

/// Which capability tier the request needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelGroup {
    Fast,
    Strong,
    LongCtx,
}

/// Why the router decided to fall back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackReason {
    RateLimit,
    Timeout,
    AuthError,
    ContextTooLarge,
    PatchFailed,
    ProviderError,
}

/// A single model endpoint the router can try.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelEndpoint {
    pub provider: String,
    pub model: String,
    pub group: ModelGroup,
}

/// Outcome of a routing decision.
#[derive(Debug, Clone, PartialEq)]
pub enum RouteDecision {
    /// Use this endpoint.
    Use(ModelEndpoint),
    /// Escalate to a stronger model group.
    Escalate {
        from: ModelGroup,
        to: ModelGroup,
        reason: FallbackReason,
    },
    /// Shrink context and retry with the same endpoint.
    ShrinkAndRetry(ModelEndpoint),
    /// All options exhausted.
    Exhausted { reason: FallbackReason },
}

/// Configuration for the router.
#[derive(Debug, Clone)]
pub struct RouterConfig {
    /// Endpoints per group, in priority order (first = preferred).
    pub endpoints: Vec<ModelEndpoint>,
    /// Max consecutive fallbacks before giving up.
    pub max_fallbacks: usize,
}

/// Tracks routing state for a single request attempt.
pub struct Router {
    config: RouterConfig,
    /// Which endpoints have been tried (by index into config.endpoints).
    tried: HashSet<usize>,
    /// How many fallbacks have occurred.
    fallback_count: usize,
    /// Whether context shrink has been attempted.
    context_shrunk: bool,
    /// Count of patch failures for escalation logic.
    patch_fail_count: usize,
}

impl Router {
    pub fn new(config: RouterConfig) -> Self {
        Self {
            config,
            tried: HashSet::new(),
            fallback_count: 0,
            context_shrunk: false,
            patch_fail_count: 0,
        }
    }

    /// Pick the first untried endpoint in the requested group.
    pub fn select(&mut self, group: ModelGroup) -> RouteDecision {
        for (i, ep) in self.config.endpoints.iter().enumerate() {
            if ep.group == group && !self.tried.contains(&i) {
                self.tried.insert(i);
                return RouteDecision::Use(ep.clone());
            }
        }
        RouteDecision::Exhausted {
            reason: FallbackReason::ProviderError,
        }
    }

    /// Report a failure and get the next decision.
    ///
    /// Fallback logic:
    /// - RateLimit | Timeout | ProviderError | AuthError → try next endpoint in same group
    /// - ContextTooLarge → if not yet shrunk, ShrinkAndRetry; else try next endpoint
    /// - PatchFailed → increment patch_fail_count; if >= 2, Escalate from current group to Strong
    ///   (unless already Strong, in which case try next endpoint)
    /// - If no more endpoints in group, Exhausted
    /// - If max_fallbacks exceeded, Exhausted
    pub fn report_failure(
        &mut self,
        failed_endpoint: &ModelEndpoint,
        reason: FallbackReason,
        current_group: ModelGroup,
    ) -> RouteDecision {
        self.fallback_count += 1;
        if self.fallback_count > self.config.max_fallbacks {
            return RouteDecision::Exhausted {
                reason: reason.clone(),
            };
        }

        // Mark the failed endpoint as tried (it may already be marked, but ensure it is).
        if let Some(i) = self
            .config
            .endpoints
            .iter()
            .position(|ep| ep == failed_endpoint)
        {
            self.tried.insert(i);
        }

        match &reason {
            FallbackReason::ContextTooLarge => {
                if !self.context_shrunk {
                    self.context_shrunk = true;
                    return RouteDecision::ShrinkAndRetry(failed_endpoint.clone());
                }
                self.select(current_group)
            }
            FallbackReason::PatchFailed => {
                self.patch_fail_count += 1;
                if self.patch_fail_count >= 2 && current_group != ModelGroup::Strong {
                    return RouteDecision::Escalate {
                        from: current_group,
                        to: ModelGroup::Strong,
                        reason,
                    };
                }
                self.select(current_group)
            }
            FallbackReason::RateLimit
            | FallbackReason::Timeout
            | FallbackReason::AuthError
            | FallbackReason::ProviderError => self.select(current_group),
        }
    }

    /// Reset state for a new request (keeps config).
    pub fn reset(&mut self) {
        self.tried.clear();
        self.fallback_count = 0;
        self.context_shrunk = false;
        self.patch_fail_count = 0;
    }
}
