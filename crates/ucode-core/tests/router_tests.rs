use ucode_core::{FallbackReason, ModelEndpoint, ModelGroup, RouteDecision, Router, RouterConfig};

fn test_config() -> RouterConfig {
    RouterConfig {
        endpoints: vec![
            ModelEndpoint {
                provider: "openai".into(),
                model: "gpt-4o-mini".into(),
                group: ModelGroup::Fast,
            },
            ModelEndpoint {
                provider: "anthropic".into(),
                model: "haiku".into(),
                group: ModelGroup::Fast,
            },
            ModelEndpoint {
                provider: "openai".into(),
                model: "gpt-4o".into(),
                group: ModelGroup::Strong,
            },
            ModelEndpoint {
                provider: "anthropic".into(),
                model: "sonnet".into(),
                group: ModelGroup::Strong,
            },
            ModelEndpoint {
                provider: "openai".into(),
                model: "gpt-4o-long".into(),
                group: ModelGroup::LongCtx,
            },
        ],
        max_fallbacks: 5,
    }
}

fn fast_ep(idx: usize) -> ModelEndpoint {
    let cfg = test_config();
    cfg.endpoints
        .into_iter()
        .filter(|e| e.group == ModelGroup::Fast)
        .nth(idx)
        .unwrap()
}

#[test]
fn select_preferred() {
    let mut r = Router::new(test_config());
    assert_eq!(r.select(ModelGroup::Fast), RouteDecision::Use(fast_ep(0)));
}

#[test]
fn select_skips_tried() {
    let mut r = Router::new(test_config());
    r.select(ModelGroup::Fast); // consumes first
    assert_eq!(r.select(ModelGroup::Fast), RouteDecision::Use(fast_ep(1)));
}

#[test]
fn select_exhausted() {
    let mut r = Router::new(test_config());
    r.select(ModelGroup::Fast);
    r.select(ModelGroup::Fast);
    let d = r.select(ModelGroup::Fast);
    assert!(matches!(d, RouteDecision::Exhausted { .. }));
}

#[test]
fn fallback_on_rate_limit() {
    let mut r = Router::new(test_config());
    let ep = match r.select(ModelGroup::Fast) {
        RouteDecision::Use(e) => e,
        _ => panic!("expected Use"),
    };
    let next = r.report_failure(&ep, FallbackReason::RateLimit, ModelGroup::Fast);
    assert_eq!(next, RouteDecision::Use(fast_ep(1)));
}

#[test]
fn fallback_on_auth_error() {
    let mut r = Router::new(test_config());
    let ep = match r.select(ModelGroup::Fast) {
        RouteDecision::Use(e) => e,
        _ => panic!("expected Use"),
    };
    let next = r.report_failure(&ep, FallbackReason::AuthError, ModelGroup::Fast);
    assert_eq!(next, RouteDecision::Use(fast_ep(1)));
}

#[test]
fn fallback_on_timeout() {
    let mut r = Router::new(test_config());
    let ep = match r.select(ModelGroup::Fast) {
        RouteDecision::Use(e) => e,
        _ => panic!("expected Use"),
    };
    let next = r.report_failure(&ep, FallbackReason::Timeout, ModelGroup::Fast);
    assert_eq!(next, RouteDecision::Use(fast_ep(1)));
}

#[test]
fn context_too_large_shrink_first() {
    let mut r = Router::new(test_config());
    let ep = match r.select(ModelGroup::Fast) {
        RouteDecision::Use(e) => e,
        _ => panic!("expected Use"),
    };
    let next = r.report_failure(&ep, FallbackReason::ContextTooLarge, ModelGroup::Fast);
    assert_eq!(next, RouteDecision::ShrinkAndRetry(fast_ep(0)));
}

#[test]
fn context_too_large_already_shrunk() {
    let mut r = Router::new(test_config());
    let ep = match r.select(ModelGroup::Fast) {
        RouteDecision::Use(e) => e,
        _ => panic!("expected Use"),
    };
    // First ContextTooLarge → shrink
    r.report_failure(&ep, FallbackReason::ContextTooLarge, ModelGroup::Fast);
    // Second ContextTooLarge → next endpoint
    let next = r.report_failure(&ep, FallbackReason::ContextTooLarge, ModelGroup::Fast);
    assert_eq!(next, RouteDecision::Use(fast_ep(1)));
}

#[test]
fn patch_failed_once() {
    let mut r = Router::new(test_config());
    let ep = match r.select(ModelGroup::Fast) {
        RouteDecision::Use(e) => e,
        _ => panic!("expected Use"),
    };
    let next = r.report_failure(&ep, FallbackReason::PatchFailed, ModelGroup::Fast);
    // Only one patch failure — no escalation, just next endpoint
    assert_eq!(next, RouteDecision::Use(fast_ep(1)));
}

#[test]
fn patch_failed_twice_escalates() {
    let mut r = Router::new(test_config());
    let ep0 = match r.select(ModelGroup::Fast) {
        RouteDecision::Use(e) => e,
        _ => panic!("expected Use"),
    };
    let ep1 = match r.report_failure(&ep0, FallbackReason::PatchFailed, ModelGroup::Fast) {
        RouteDecision::Use(e) => e,
        other => panic!("expected Use after first patch fail, got {other:?}"),
    };
    let next = r.report_failure(&ep1, FallbackReason::PatchFailed, ModelGroup::Fast);
    assert_eq!(
        next,
        RouteDecision::Escalate {
            from: ModelGroup::Fast,
            to: ModelGroup::Strong,
            reason: FallbackReason::PatchFailed,
        }
    );
}

#[test]
fn patch_failed_on_strong_no_escalation() {
    let mut r = Router::new(test_config());
    // Consume both Fast endpoints so we can work in Strong
    r.select(ModelGroup::Fast);
    r.select(ModelGroup::Fast);

    let ep0 = match r.select(ModelGroup::Strong) {
        RouteDecision::Use(e) => e,
        _ => panic!("expected Use"),
    };
    // First patch fail on Strong → next Strong endpoint
    let ep1 = match r.report_failure(&ep0, FallbackReason::PatchFailed, ModelGroup::Strong) {
        RouteDecision::Use(e) => e,
        other => panic!("expected Use, got {other:?}"),
    };
    // Second patch fail on Strong → still next Strong endpoint (no further escalation)
    let next = r.report_failure(&ep1, FallbackReason::PatchFailed, ModelGroup::Strong);
    // Both Strong endpoints consumed, so Exhausted
    assert!(matches!(next, RouteDecision::Exhausted { .. }));
}

#[test]
fn max_fallbacks_exceeded() {
    let cfg = RouterConfig {
        endpoints: vec![
            ModelEndpoint {
                provider: "openai".into(),
                model: "gpt-4o-mini".into(),
                group: ModelGroup::Fast,
            },
            ModelEndpoint {
                provider: "anthropic".into(),
                model: "haiku".into(),
                group: ModelGroup::Fast,
            },
            ModelEndpoint {
                provider: "openai".into(),
                model: "gpt-4o".into(),
                group: ModelGroup::Fast,
            },
        ],
        max_fallbacks: 2,
    };
    let mut r = Router::new(cfg);

    let ep = match r.select(ModelGroup::Fast) {
        RouteDecision::Use(e) => e,
        _ => panic!(),
    };
    // fallback 1
    let ep2 = match r.report_failure(&ep, FallbackReason::RateLimit, ModelGroup::Fast) {
        RouteDecision::Use(e) => e,
        _ => panic!(),
    };
    // fallback 2
    let ep3 = match r.report_failure(&ep2, FallbackReason::RateLimit, ModelGroup::Fast) {
        RouteDecision::Use(e) => e,
        _ => panic!(),
    };
    // fallback 3 — exceeds max_fallbacks=2
    let next = r.report_failure(&ep3, FallbackReason::RateLimit, ModelGroup::Fast);
    assert!(matches!(next, RouteDecision::Exhausted { .. }));
}

#[test]
fn reset_clears_state() {
    let mut r = Router::new(test_config());
    r.select(ModelGroup::Fast);
    r.select(ModelGroup::Fast);

    r.reset();

    // After reset, first Fast endpoint is available again
    assert_eq!(r.select(ModelGroup::Fast), RouteDecision::Use(fast_ep(0)));
}
