use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Safety classification for hook events, determining whether the host
/// auto-applies the hook's side-effects or requires user approval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverrideClass {
    /// Auto-applied; no approval needed (e.g., logging, observability).
    Safe,
    /// Bounded modifications allowed without explicit approval.
    Guarded,
    /// Requires explicit user approval before the host acts on the result.
    Risky,
}

/// All hook events emitted by the host runtime.
///
/// Serialized with `tag = "type"` so consumers can dispatch on the `type` field.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum HookEvent {
    // --- Session lifecycle ---
    SessionStart {
        session_id: String,
    },
    SessionEnd {
        session_id: String,
        duration_secs: f64,
    },

    // --- Tool lifecycle ---
    BeforeToolCall {
        tool_name: String,
        args: serde_json::Value,
    },
    AfterToolCall {
        tool_name: String,
        result: serde_json::Value,
        duration_ms: u64,
    },
    ToolError {
        tool_name: String,
        error: String,
    },

    // --- Model lifecycle ---
    BeforeModelCall {
        model: String,
        message_count: usize,
    },
    AfterModelCall {
        model: String,
        tokens_used: usize,
        duration_ms: u64,
    },
    ModelFallback {
        from_model: String,
        to_model: String,
        reason: String,
    },

    // --- Context ---
    ContextOverflow {
        current_tokens: usize,
        max_tokens: usize,
    },
    ContextCompaction {
        before_tokens: usize,
        after_tokens: usize,
    },

    // --- Approval ---
    ApprovalRequired {
        tool_name: String,
        risk_level: String,
    },
    ApprovalGranted {
        tool_name: String,
    },
    ApprovalDenied {
        tool_name: String,
        reason: String,
    },

    // --- Checkpoint ---
    CheckpointCreated {
        checkpoint_id: String,
    },
    CheckpointRestored {
        checkpoint_id: String,
    },

    // --- Plugin ---
    PluginLoaded {
        plugin_name: String,
    },
    PluginUnloaded {
        plugin_name: String,
    },
    PluginError {
        plugin_name: String,
        error: String,
    },

    // --- MCP ---
    McpServerConnected {
        server_name: String,
    },
    McpServerDisconnected {
        server_name: String,
        reason: String,
    },

    // --- Skill ---
    SkillActivated {
        skill_name: String,
    },
    SkillDeactivated {
        skill_name: String,
    },
}

impl HookEvent {
    /// Canonical snake_case name used for subscription matching.
    pub fn event_name(&self) -> &'static str {
        match self {
            Self::SessionStart { .. } => "session_start",
            Self::SessionEnd { .. } => "session_end",
            Self::BeforeToolCall { .. } => "before_tool_call",
            Self::AfterToolCall { .. } => "after_tool_call",
            Self::ToolError { .. } => "tool_error",
            Self::BeforeModelCall { .. } => "before_model_call",
            Self::AfterModelCall { .. } => "after_model_call",
            Self::ModelFallback { .. } => "model_fallback",
            Self::ContextOverflow { .. } => "context_overflow",
            Self::ContextCompaction { .. } => "context_compaction",
            Self::ApprovalRequired { .. } => "approval_required",
            Self::ApprovalGranted { .. } => "approval_granted",
            Self::ApprovalDenied { .. } => "approval_denied",
            Self::CheckpointCreated { .. } => "checkpoint_created",
            Self::CheckpointRestored { .. } => "checkpoint_restored",
            Self::PluginLoaded { .. } => "plugin_loaded",
            Self::PluginUnloaded { .. } => "plugin_unloaded",
            Self::PluginError { .. } => "plugin_error",
            Self::McpServerConnected { .. } => "mcp_server_connected",
            Self::McpServerDisconnected { .. } => "mcp_server_disconnected",
            Self::SkillActivated { .. } => "skill_activated",
            Self::SkillDeactivated { .. } => "skill_deactivated",
        }
    }

    /// Safety classification for this event variant.
    pub fn override_class(&self) -> OverrideClass {
        match self {
            // Pure observability — no side-effects on host state.
            Self::SessionStart { .. }
            | Self::SessionEnd { .. }
            | Self::AfterToolCall { .. }
            | Self::AfterModelCall { .. }
            | Self::ToolError { .. }
            | Self::PluginLoaded { .. }
            | Self::PluginUnloaded { .. }
            | Self::PluginError { .. }
            | Self::McpServerConnected { .. }
            | Self::McpServerDisconnected { .. }
            | Self::SkillActivated { .. }
            | Self::SkillDeactivated { .. }
            | Self::ApprovalGranted { .. }
            | Self::ApprovalDenied { .. } => OverrideClass::Safe,

            // Bounded modifications: plugin may alter args/tokens within limits.
            Self::BeforeToolCall { .. }
            | Self::BeforeModelCall { .. }
            | Self::ContextOverflow { .. }
            | Self::ContextCompaction { .. }
            | Self::ApprovalRequired { .. }
            | Self::CheckpointCreated { .. } => OverrideClass::Guarded,

            // Structural changes that can alter execution path or state.
            Self::ModelFallback { .. } | Self::CheckpointRestored { .. } => OverrideClass::Risky,
        }
    }
}

/// A timestamped wrapper around a [`HookEvent`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookRecord {
    pub event: HookEvent,
    pub timestamp: DateTime<Utc>,
}

impl HookRecord {
    /// Wrap `event` with the current UTC timestamp.
    pub fn new(event: HookEvent) -> Self {
        Self {
            event,
            timestamp: Utc::now(),
        }
    }
}

/// Tracks which plugin subscribes to which event name.
#[derive(Debug, Clone)]
pub struct HookSubscription {
    pub plugin_name: String,
    pub event_name: String,
}

/// Routes [`HookEvent`]s to subscribed plugins and maintains an event log.
#[derive(Debug, Default)]
pub struct HookDispatcher {
    subscriptions: Vec<HookSubscription>,
    log: Vec<HookRecord>,
}

impl HookDispatcher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `plugin_name` to receive events named `event_name`.
    pub fn subscribe(&mut self, plugin_name: &str, event_name: &str) {
        self.subscriptions.push(HookSubscription {
            plugin_name: plugin_name.to_owned(),
            event_name: event_name.to_owned(),
        });
    }

    /// Remove the subscription for `(plugin_name, event_name)`.
    ///
    /// Returns `false` if no matching subscription existed.
    pub fn unsubscribe(&mut self, plugin_name: &str, event_name: &str) -> bool {
        let before = self.subscriptions.len();
        self.subscriptions
            .retain(|s| !(s.plugin_name == plugin_name && s.event_name == event_name));
        self.subscriptions.len() < before
    }

    /// Remove all subscriptions for `plugin_name`.
    pub fn unsubscribe_all(&mut self, plugin_name: &str) {
        self.subscriptions.retain(|s| s.plugin_name != plugin_name);
    }

    /// Log `event` and return references to all matching subscriptions.
    pub fn dispatch(&mut self, event: HookEvent) -> Vec<&HookSubscription> {
        let name = event.event_name();
        self.log.push(HookRecord::new(event));
        // Collect indices first to avoid borrow conflicts.
        let indices: Vec<usize> = self
            .subscriptions
            .iter()
            .enumerate()
            .filter(|(_, s)| s.event_name == name)
            .map(|(i, _)| i)
            .collect();
        indices
            .into_iter()
            .map(|i| &self.subscriptions[i])
            .collect()
    }

    /// List all subscriptions for a given event name.
    pub fn subscribers_for(&self, event_name: &str) -> Vec<&HookSubscription> {
        self.subscriptions
            .iter()
            .filter(|s| s.event_name == event_name)
            .collect()
    }

    /// The full event log in insertion order.
    pub fn log(&self) -> &[HookRecord] {
        &self.log
    }

    /// Discard all logged events.
    pub fn clear_log(&mut self) {
        self.log.clear();
    }

    /// Total number of active subscriptions.
    pub fn subscription_count(&self) -> usize {
        self.subscriptions.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- HookEvent::event_name ----

    #[test]
    fn test_hook_event_names() {
        assert_eq!(
            HookEvent::SessionStart {
                session_id: "s1".into()
            }
            .event_name(),
            "session_start"
        );
        assert_eq!(
            HookEvent::BeforeToolCall {
                tool_name: "bash".into(),
                args: serde_json::Value::Null
            }
            .event_name(),
            "before_tool_call"
        );
        assert_eq!(
            HookEvent::ModelFallback {
                from_model: "a".into(),
                to_model: "b".into(),
                reason: "quota".into()
            }
            .event_name(),
            "model_fallback"
        );
        assert_eq!(
            HookEvent::McpServerDisconnected {
                server_name: "fs".into(),
                reason: "timeout".into()
            }
            .event_name(),
            "mcp_server_disconnected"
        );
        assert_eq!(
            HookEvent::SkillDeactivated {
                skill_name: "tdd".into()
            }
            .event_name(),
            "skill_deactivated"
        );
    }

    // ---- HookRecord ----

    #[test]
    fn test_hook_record_timestamp() {
        let before = Utc::now();
        let rec = HookRecord::new(HookEvent::SessionStart {
            session_id: "x".into(),
        });
        let after = Utc::now();
        assert!(rec.timestamp >= before);
        assert!(rec.timestamp <= after);
    }

    // ---- OverrideClass ----

    #[test]
    fn test_override_class_safe_events() {
        assert_eq!(
            HookEvent::SessionStart {
                session_id: "s".into()
            }
            .override_class(),
            OverrideClass::Safe
        );
        assert_eq!(
            HookEvent::PluginLoaded {
                plugin_name: "p".into()
            }
            .override_class(),
            OverrideClass::Safe
        );
        assert_eq!(
            HookEvent::SkillActivated {
                skill_name: "sk".into()
            }
            .override_class(),
            OverrideClass::Safe
        );
    }

    #[test]
    fn test_override_class_guarded_events() {
        assert_eq!(
            HookEvent::BeforeToolCall {
                tool_name: "bash".into(),
                args: serde_json::Value::Null
            }
            .override_class(),
            OverrideClass::Guarded
        );
        assert_eq!(
            HookEvent::ContextCompaction {
                before_tokens: 8000,
                after_tokens: 4000
            }
            .override_class(),
            OverrideClass::Guarded
        );
    }

    #[test]
    fn test_override_class_risky_events() {
        assert_eq!(
            HookEvent::ModelFallback {
                from_model: "a".into(),
                to_model: "b".into(),
                reason: "r".into()
            }
            .override_class(),
            OverrideClass::Risky
        );
        assert_eq!(
            HookEvent::CheckpointRestored {
                checkpoint_id: "c1".into()
            }
            .override_class(),
            OverrideClass::Risky
        );
    }

    // ---- HookDispatcher ----

    #[test]
    fn test_dispatcher_subscribe_and_dispatch() {
        let mut d = HookDispatcher::new();
        d.subscribe("logger", "session_start");
        let matches = d.dispatch(HookEvent::SessionStart {
            session_id: "s1".into(),
        });
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].plugin_name, "logger");
    }

    #[test]
    fn test_dispatcher_no_match() {
        let mut d = HookDispatcher::new();
        d.subscribe("logger", "session_end");
        let matches = d.dispatch(HookEvent::SessionStart {
            session_id: "s1".into(),
        });
        assert!(matches.is_empty());
    }

    #[test]
    fn test_dispatcher_multiple_subscribers() {
        let mut d = HookDispatcher::new();
        d.subscribe("plugin-a", "tool_error");
        d.subscribe("plugin-b", "tool_error");
        let matches = d.dispatch(HookEvent::ToolError {
            tool_name: "bash".into(),
            error: "exit 1".into(),
        });
        assert_eq!(matches.len(), 2);
        let names: Vec<&str> = matches.iter().map(|s| s.plugin_name.as_str()).collect();
        assert!(names.contains(&"plugin-a"));
        assert!(names.contains(&"plugin-b"));
    }

    #[test]
    fn test_dispatcher_unsubscribe() {
        let mut d = HookDispatcher::new();
        d.subscribe("logger", "session_start");
        let removed = d.unsubscribe("logger", "session_start");
        assert!(removed);
        let matches = d.dispatch(HookEvent::SessionStart {
            session_id: "s2".into(),
        });
        assert!(matches.is_empty());
    }

    #[test]
    fn test_dispatcher_unsubscribe_not_found() {
        let mut d = HookDispatcher::new();
        let removed = d.unsubscribe("ghost", "session_start");
        assert!(!removed);
    }

    #[test]
    fn test_dispatcher_unsubscribe_all() {
        let mut d = HookDispatcher::new();
        d.subscribe("plugin-a", "session_start");
        d.subscribe("plugin-a", "session_end");
        d.subscribe("plugin-b", "session_start");
        d.unsubscribe_all("plugin-a");
        assert_eq!(d.subscription_count(), 1);
        assert_eq!(d.subscribers_for("session_start").len(), 1);
        assert_eq!(
            d.subscribers_for("session_start")[0].plugin_name,
            "plugin-b"
        );
    }

    #[test]
    fn test_dispatcher_log() {
        let mut d = HookDispatcher::new();
        d.dispatch(HookEvent::SessionStart {
            session_id: "s1".into(),
        });
        d.dispatch(HookEvent::SessionEnd {
            session_id: "s1".into(),
            duration_secs: 1.5,
        });
        assert_eq!(d.log().len(), 2);
        assert_eq!(d.log()[0].event.event_name(), "session_start");
        assert_eq!(d.log()[1].event.event_name(), "session_end");
    }

    #[test]
    fn test_dispatcher_clear_log() {
        let mut d = HookDispatcher::new();
        d.dispatch(HookEvent::PluginLoaded {
            plugin_name: "p".into(),
        });
        assert_eq!(d.log().len(), 1);
        d.clear_log();
        assert!(d.log().is_empty());
    }

    #[test]
    fn test_dispatcher_subscription_count() {
        let mut d = HookDispatcher::new();
        assert_eq!(d.subscription_count(), 0);
        d.subscribe("a", "session_start");
        d.subscribe("b", "session_start");
        d.subscribe("a", "tool_error");
        assert_eq!(d.subscription_count(), 3);
        d.unsubscribe("a", "tool_error");
        assert_eq!(d.subscription_count(), 2);
    }

    #[test]
    fn test_hook_event_serialization() {
        let event = HookEvent::AfterToolCall {
            tool_name: "read_file".into(),
            result: serde_json::json!({"lines": 42}),
            duration_ms: 17,
        };
        let json = serde_json::to_string(&event).unwrap();
        let roundtrip: HookEvent = serde_json::from_str(&json).unwrap();
        // Verify the discriminant survived the roundtrip.
        assert_eq!(roundtrip.event_name(), "after_tool_call");
        // Verify the payload survived.
        if let HookEvent::AfterToolCall {
            tool_name,
            duration_ms,
            ..
        } = roundtrip
        {
            assert_eq!(tool_name, "read_file");
            assert_eq!(duration_ms, 17);
        } else {
            panic!("wrong variant after deserialization");
        }
    }
}
