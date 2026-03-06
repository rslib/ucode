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

    // --- Session (extended) ---
    SessionTitleGenerated {
        session_id: String,
        title: String,
    },
    SessionTitleUpdated {
        session_id: String,
        title: String,
    },
    ConfigReloaded,

    // --- Message flow ---
    UserMessageReceived {
        message_len: usize,
    },
    AssistantResponseStarted {
        model: String,
    },
    AssistantResponseCompleted {
        model: String,
        tokens: usize,
        duration_ms: u64,
    },
    MessageRetry {
        reason: String,
        attempt: u32,
    },

    // --- Model (extended) ---
    BeforeModelSelect {
        candidates: Vec<String>,
    },
    RouterDecision {
        model: String,
        reason: String,
    },
    ModelRateLimited {
        model: String,
        retry_after_ms: Option<u64>,
    },
    ModelQuotaExhausted {
        model: String,
    },

    // --- Tool specific ---
    ToolTimeout {
        tool_name: String,
        timeout_ms: u64,
    },
    BeforeApplyPatch {
        file_path: String,
        patch_summary: String,
    },
    AfterApplyPatch {
        file_path: String,
        lines_changed: usize,
    },
    BeforeRunCmd {
        command: String,
    },
    AfterRunCmd {
        command: String,
        exit_code: i32,
        duration_ms: u64,
    },
    BeforeFileRead {
        path: String,
    },
    AfterFileRead {
        path: String,
        size_bytes: u64,
    },
    BeforeFileWrite {
        path: String,
    },
    AfterFileWrite {
        path: String,
        size_bytes: u64,
    },

    // --- Context (extended) ---
    ContextDistilled {
        before_tokens: usize,
        after_tokens: usize,
    },
    TokenUsageUpdated {
        total_tokens: usize,
        max_tokens: usize,
    },

    // --- Agent ---
    AgentSpawned {
        agent_id: String,
        task: String,
    },
    AgentMessage {
        agent_id: String,
        message: String,
    },
    AgentCompleted {
        agent_id: String,
        duration_ms: u64,
    },
    AgentFailed {
        agent_id: String,
        error: String,
    },
    AgentCancelled {
        agent_id: String,
        reason: String,
    },

    // --- Approval/Sandbox (extended) ---
    SandboxDecision {
        tool_name: String,
        allowed: bool,
        reason: String,
    },
    PermissionDecision {
        action: String,
        allowed: bool,
        reason: String,
    },

    // --- Auth ---
    AuthChanged {
        provider: String,
    },
    AuthFailed {
        provider: String,
        error: String,
    },
    ProviderSwitched {
        from: String,
        to: String,
    },

    // --- MCP (extended) ---
    McpServerLaunch {
        server_name: String,
    },
    McpServerRestart {
        server_name: String,
        reason: String,
    },
    McpServerCrash {
        server_name: String,
        error: String,
    },
    McpToolInvoked {
        server_name: String,
        tool_name: String,
    },

    // --- Budget ---
    BudgetThresholdWarning {
        current_cost: f64,
        threshold: f64,
    },
    BudgetThresholdReached {
        current_cost: f64,
        limit: f64,
    },
    CostIncurred {
        model: String,
        cost_usd: f64,
        tokens: usize,
    },

    // --- Background ---
    BackgroundJobStateChanged {
        job_id: String,
        state: String,
    },

    // --- Commands/UI ---
    CommandInvoked {
        command: String,
    },
    PaletteCommandExecuted {
        command: String,
    },

    // --- Diagnostics ---
    UnhandledError {
        error: String,
        context: String,
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
            // Session (extended)
            Self::SessionTitleGenerated { .. } => "session_title_generated",
            Self::SessionTitleUpdated { .. } => "session_title_updated",
            Self::ConfigReloaded => "config_reloaded",
            // Message flow
            Self::UserMessageReceived { .. } => "user_message_received",
            Self::AssistantResponseStarted { .. } => "assistant_response_started",
            Self::AssistantResponseCompleted { .. } => "assistant_response_completed",
            Self::MessageRetry { .. } => "message_retry",
            // Model (extended)
            Self::BeforeModelSelect { .. } => "before_model_select",
            Self::RouterDecision { .. } => "router_decision",
            Self::ModelRateLimited { .. } => "model_rate_limited",
            Self::ModelQuotaExhausted { .. } => "model_quota_exhausted",
            // Tool specific
            Self::ToolTimeout { .. } => "tool_timeout",
            Self::BeforeApplyPatch { .. } => "before_apply_patch",
            Self::AfterApplyPatch { .. } => "after_apply_patch",
            Self::BeforeRunCmd { .. } => "before_run_cmd",
            Self::AfterRunCmd { .. } => "after_run_cmd",
            Self::BeforeFileRead { .. } => "before_file_read",
            Self::AfterFileRead { .. } => "after_file_read",
            Self::BeforeFileWrite { .. } => "before_file_write",
            Self::AfterFileWrite { .. } => "after_file_write",
            // Context (extended)
            Self::ContextDistilled { .. } => "context_distilled",
            Self::TokenUsageUpdated { .. } => "token_usage_updated",
            // Agent
            Self::AgentSpawned { .. } => "agent_spawned",
            Self::AgentMessage { .. } => "agent_message",
            Self::AgentCompleted { .. } => "agent_completed",
            Self::AgentFailed { .. } => "agent_failed",
            Self::AgentCancelled { .. } => "agent_cancelled",
            // Approval/Sandbox (extended)
            Self::SandboxDecision { .. } => "sandbox_decision",
            Self::PermissionDecision { .. } => "permission_decision",
            // Auth
            Self::AuthChanged { .. } => "auth_changed",
            Self::AuthFailed { .. } => "auth_failed",
            Self::ProviderSwitched { .. } => "provider_switched",
            // MCP (extended)
            Self::McpServerLaunch { .. } => "mcp_server_launch",
            Self::McpServerRestart { .. } => "mcp_server_restart",
            Self::McpServerCrash { .. } => "mcp_server_crash",
            Self::McpToolInvoked { .. } => "mcp_tool_invoked",
            // Budget
            Self::BudgetThresholdWarning { .. } => "budget_threshold_warning",
            Self::BudgetThresholdReached { .. } => "budget_threshold_reached",
            Self::CostIncurred { .. } => "cost_incurred",
            // Background
            Self::BackgroundJobStateChanged { .. } => "background_job_state_changed",
            // Commands/UI
            Self::CommandInvoked { .. } => "command_invoked",
            Self::PaletteCommandExecuted { .. } => "palette_command_executed",
            // Diagnostics
            Self::UnhandledError { .. } => "unhandled_error",
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

            // Safe: pure observability, no side-effects on host state.
            Self::SessionTitleGenerated { .. }
            | Self::SessionTitleUpdated { .. }
            | Self::ConfigReloaded
            | Self::UserMessageReceived { .. }
            | Self::AssistantResponseStarted { .. }
            | Self::AssistantResponseCompleted { .. }
            | Self::ToolTimeout { .. }
            | Self::AfterApplyPatch { .. }
            | Self::AfterRunCmd { .. }
            | Self::AfterFileRead { .. }
            | Self::AfterFileWrite { .. }
            | Self::RouterDecision { .. }
            | Self::ModelRateLimited { .. }
            | Self::ModelQuotaExhausted { .. }
            | Self::ContextDistilled { .. }
            | Self::TokenUsageUpdated { .. }
            | Self::AgentSpawned { .. }
            | Self::AgentMessage { .. }
            | Self::AgentCompleted { .. }
            | Self::AgentFailed { .. }
            | Self::AgentCancelled { .. }
            | Self::SandboxDecision { .. }
            | Self::PermissionDecision { .. }
            | Self::AuthChanged { .. }
            | Self::AuthFailed { .. }
            | Self::ProviderSwitched { .. }
            | Self::McpServerLaunch { .. }
            | Self::McpServerRestart { .. }
            | Self::McpServerCrash { .. }
            | Self::McpToolInvoked { .. }
            | Self::BudgetThresholdWarning { .. }
            | Self::CostIncurred { .. }
            | Self::BackgroundJobStateChanged { .. }
            | Self::CommandInvoked { .. }
            | Self::PaletteCommandExecuted { .. }
            | Self::UnhandledError { .. } => OverrideClass::Safe,

            // Guarded: bounded modifications allowed without explicit approval.
            Self::MessageRetry { .. }
            | Self::BeforeModelSelect { .. }
            | Self::BeforeApplyPatch { .. }
            | Self::BeforeRunCmd { .. }
            | Self::BeforeFileRead { .. }
            | Self::BeforeFileWrite { .. }
            | Self::BudgetThresholdReached { .. } => OverrideClass::Guarded,
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

    #[test]
    fn test_new_session_events() {
        assert_eq!(
            HookEvent::SessionTitleGenerated {
                session_id: "s".into(),
                title: "t".into()
            }
            .event_name(),
            "session_title_generated"
        );
        assert_eq!(
            HookEvent::SessionTitleUpdated {
                session_id: "s".into(),
                title: "t".into()
            }
            .event_name(),
            "session_title_updated"
        );
        assert_eq!(HookEvent::ConfigReloaded.event_name(), "config_reloaded");
    }

    #[test]
    fn test_message_flow_events() {
        assert_eq!(
            HookEvent::UserMessageReceived { message_len: 100 }.event_name(),
            "user_message_received"
        );
        assert_eq!(
            HookEvent::AssistantResponseStarted {
                model: "gpt-4".into()
            }
            .event_name(),
            "assistant_response_started"
        );
        assert_eq!(
            HookEvent::AssistantResponseCompleted {
                model: "gpt-4".into(),
                tokens: 500,
                duration_ms: 1200
            }
            .event_name(),
            "assistant_response_completed"
        );
        assert_eq!(
            HookEvent::MessageRetry {
                reason: "rate_limit".into(),
                attempt: 2
            }
            .event_name(),
            "message_retry"
        );
    }

    #[test]
    fn test_new_model_events() {
        assert_eq!(
            HookEvent::BeforeModelSelect {
                candidates: vec!["a".into(), "b".into()]
            }
            .event_name(),
            "before_model_select"
        );
        assert_eq!(
            HookEvent::RouterDecision {
                model: "gpt-4".into(),
                reason: "cost".into()
            }
            .event_name(),
            "router_decision"
        );
        assert_eq!(
            HookEvent::ModelRateLimited {
                model: "gpt-4".into(),
                retry_after_ms: Some(5000)
            }
            .event_name(),
            "model_rate_limited"
        );
        assert_eq!(
            HookEvent::ModelQuotaExhausted {
                model: "gpt-4".into()
            }
            .event_name(),
            "model_quota_exhausted"
        );
    }

    #[test]
    fn test_tool_specific_events() {
        assert_eq!(
            HookEvent::ToolTimeout {
                tool_name: "bash".into(),
                timeout_ms: 30000
            }
            .event_name(),
            "tool_timeout"
        );
        assert_eq!(
            HookEvent::BeforeApplyPatch {
                file_path: "src/main.rs".into(),
                patch_summary: "add fn".into()
            }
            .event_name(),
            "before_apply_patch"
        );
        assert_eq!(
            HookEvent::AfterApplyPatch {
                file_path: "src/main.rs".into(),
                lines_changed: 10
            }
            .event_name(),
            "after_apply_patch"
        );
        assert_eq!(
            HookEvent::BeforeRunCmd {
                command: "cargo test".into()
            }
            .event_name(),
            "before_run_cmd"
        );
        assert_eq!(
            HookEvent::AfterRunCmd {
                command: "cargo test".into(),
                exit_code: 0,
                duration_ms: 5000
            }
            .event_name(),
            "after_run_cmd"
        );
        assert_eq!(
            HookEvent::BeforeFileRead {
                path: "foo.rs".into()
            }
            .event_name(),
            "before_file_read"
        );
        assert_eq!(
            HookEvent::AfterFileRead {
                path: "foo.rs".into(),
                size_bytes: 1024
            }
            .event_name(),
            "after_file_read"
        );
        assert_eq!(
            HookEvent::BeforeFileWrite {
                path: "foo.rs".into()
            }
            .event_name(),
            "before_file_write"
        );
        assert_eq!(
            HookEvent::AfterFileWrite {
                path: "foo.rs".into(),
                size_bytes: 2048
            }
            .event_name(),
            "after_file_write"
        );
    }

    #[test]
    fn test_new_context_events() {
        assert_eq!(
            HookEvent::ContextDistilled {
                before_tokens: 8000,
                after_tokens: 3000
            }
            .event_name(),
            "context_distilled"
        );
        assert_eq!(
            HookEvent::TokenUsageUpdated {
                total_tokens: 5000,
                max_tokens: 128000
            }
            .event_name(),
            "token_usage_updated"
        );
    }

    #[test]
    fn test_agent_events() {
        assert_eq!(
            HookEvent::AgentSpawned {
                agent_id: "a1".into(),
                task: "review".into()
            }
            .event_name(),
            "agent_spawned"
        );
        assert_eq!(
            HookEvent::AgentMessage {
                agent_id: "a1".into(),
                message: "done".into()
            }
            .event_name(),
            "agent_message"
        );
        assert_eq!(
            HookEvent::AgentCompleted {
                agent_id: "a1".into(),
                duration_ms: 3000
            }
            .event_name(),
            "agent_completed"
        );
        assert_eq!(
            HookEvent::AgentFailed {
                agent_id: "a1".into(),
                error: "timeout".into()
            }
            .event_name(),
            "agent_failed"
        );
        assert_eq!(
            HookEvent::AgentCancelled {
                agent_id: "a1".into(),
                reason: "user".into()
            }
            .event_name(),
            "agent_cancelled"
        );
    }

    #[test]
    fn test_sandbox_permission_events() {
        assert_eq!(
            HookEvent::SandboxDecision {
                tool_name: "bash".into(),
                allowed: true,
                reason: "policy".into()
            }
            .event_name(),
            "sandbox_decision"
        );
        assert_eq!(
            HookEvent::PermissionDecision {
                action: "file_write".into(),
                allowed: false,
                reason: "denied".into()
            }
            .event_name(),
            "permission_decision"
        );
    }

    #[test]
    fn test_auth_events() {
        assert_eq!(
            HookEvent::AuthChanged {
                provider: "openai".into()
            }
            .event_name(),
            "auth_changed"
        );
        assert_eq!(
            HookEvent::AuthFailed {
                provider: "openai".into(),
                error: "expired".into()
            }
            .event_name(),
            "auth_failed"
        );
        assert_eq!(
            HookEvent::ProviderSwitched {
                from: "openai".into(),
                to: "anthropic".into()
            }
            .event_name(),
            "provider_switched"
        );
    }

    #[test]
    fn test_new_mcp_events() {
        assert_eq!(
            HookEvent::McpServerLaunch {
                server_name: "fs".into()
            }
            .event_name(),
            "mcp_server_launch"
        );
        assert_eq!(
            HookEvent::McpServerRestart {
                server_name: "fs".into(),
                reason: "crash".into()
            }
            .event_name(),
            "mcp_server_restart"
        );
        assert_eq!(
            HookEvent::McpServerCrash {
                server_name: "fs".into(),
                error: "segfault".into()
            }
            .event_name(),
            "mcp_server_crash"
        );
        assert_eq!(
            HookEvent::McpToolInvoked {
                server_name: "fs".into(),
                tool_name: "read".into()
            }
            .event_name(),
            "mcp_tool_invoked"
        );
    }

    #[test]
    fn test_budget_events() {
        assert_eq!(
            HookEvent::BudgetThresholdWarning {
                current_cost: 4.50,
                threshold: 5.00
            }
            .event_name(),
            "budget_threshold_warning"
        );
        assert_eq!(
            HookEvent::BudgetThresholdReached {
                current_cost: 5.00,
                limit: 5.00
            }
            .event_name(),
            "budget_threshold_reached"
        );
        assert_eq!(
            HookEvent::CostIncurred {
                model: "gpt-4".into(),
                cost_usd: 0.03,
                tokens: 1000
            }
            .event_name(),
            "cost_incurred"
        );
    }

    #[test]
    fn test_background_job_event() {
        assert_eq!(
            HookEvent::BackgroundJobStateChanged {
                job_id: "j1".into(),
                state: "completed".into()
            }
            .event_name(),
            "background_job_state_changed"
        );
    }

    #[test]
    fn test_command_ui_events() {
        assert_eq!(
            HookEvent::CommandInvoked {
                command: "/test".into()
            }
            .event_name(),
            "command_invoked"
        );
        assert_eq!(
            HookEvent::PaletteCommandExecuted {
                command: "toggle_theme".into()
            }
            .event_name(),
            "palette_command_executed"
        );
    }

    #[test]
    fn test_diagnostic_events() {
        assert_eq!(
            HookEvent::UnhandledError {
                error: "panic".into(),
                context: "tool_dispatch".into()
            }
            .event_name(),
            "unhandled_error"
        );
    }

    #[test]
    fn test_all_new_override_classes() {
        // Message flow
        assert_eq!(
            HookEvent::UserMessageReceived { message_len: 1 }.override_class(),
            OverrideClass::Safe
        );
        assert_eq!(
            HookEvent::MessageRetry {
                reason: "r".into(),
                attempt: 1
            }
            .override_class(),
            OverrideClass::Guarded
        );

        // Model
        assert_eq!(
            HookEvent::BeforeModelSelect { candidates: vec![] }.override_class(),
            OverrideClass::Guarded
        );
        assert_eq!(
            HookEvent::RouterDecision {
                model: "m".into(),
                reason: "r".into()
            }
            .override_class(),
            OverrideClass::Safe
        );
        assert_eq!(
            HookEvent::ModelRateLimited {
                model: "m".into(),
                retry_after_ms: None
            }
            .override_class(),
            OverrideClass::Safe
        );

        // Tool specific
        assert_eq!(
            HookEvent::BeforeApplyPatch {
                file_path: "f".into(),
                patch_summary: "s".into()
            }
            .override_class(),
            OverrideClass::Guarded
        );
        assert_eq!(
            HookEvent::AfterApplyPatch {
                file_path: "f".into(),
                lines_changed: 0
            }
            .override_class(),
            OverrideClass::Safe
        );
        assert_eq!(
            HookEvent::BeforeRunCmd {
                command: "c".into()
            }
            .override_class(),
            OverrideClass::Guarded
        );
        assert_eq!(
            HookEvent::AfterRunCmd {
                command: "c".into(),
                exit_code: 0,
                duration_ms: 0
            }
            .override_class(),
            OverrideClass::Safe
        );
        assert_eq!(
            HookEvent::BeforeFileRead { path: "p".into() }.override_class(),
            OverrideClass::Guarded
        );
        assert_eq!(
            HookEvent::BeforeFileWrite { path: "p".into() }.override_class(),
            OverrideClass::Guarded
        );

        // Budget
        assert_eq!(
            HookEvent::BudgetThresholdReached {
                current_cost: 0.0,
                limit: 0.0
            }
            .override_class(),
            OverrideClass::Guarded
        );
        assert_eq!(
            HookEvent::BudgetThresholdWarning {
                current_cost: 0.0,
                threshold: 0.0
            }
            .override_class(),
            OverrideClass::Safe
        );
    }
}
