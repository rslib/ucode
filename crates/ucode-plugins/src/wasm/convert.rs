//! Conversion between HookEvent and WIT-generated payload types.

use super::ucode::hooks_types::types as wit;
use crate::api::HookResponse as NativeHookResponse;

/// Convert WIT hook-response to our native HookResponse.
pub fn wit_response_to_native(resp: wit::HookResponse) -> NativeHookResponse {
    match resp.kind {
        wit::HookResponseKind::Ok => NativeHookResponse::Ok,
        wit::HookResponseKind::Modify => NativeHookResponse::Modify {
            changes: resp
                .data
                .as_deref()
                .and_then(|d| serde_json::from_str(d).ok())
                .unwrap_or(serde_json::Value::Null),
        },
        wit::HookResponseKind::Veto => NativeHookResponse::Veto {
            reason: resp.data.as_deref().unwrap_or("").to_string(),
        },
    }
}

/// Maps each HookEvent variant name to its WIT export interface path.
///
/// Returns the interface path used for export probing (e.g., `"ucode:hooks-session/on-start"`).
pub fn event_to_wit_interface(event_name: &str) -> Option<&'static str> {
    Some(match event_name {
        // Session
        "session_start" => "ucode:hooks-session/on-start",
        "session_end" => "ucode:hooks-session/on-end",
        "session_title_generated" => "ucode:hooks-session/on-title-generated",
        "session_title_updated" => "ucode:hooks-session/on-title-updated",
        "config_reloaded" => "ucode:hooks-session/on-config-reloaded",
        // Message
        "user_message_received" => "ucode:hooks-message/on-user-message",
        "assistant_response_started" => "ucode:hooks-message/on-response-started",
        "assistant_response_completed" => "ucode:hooks-message/on-response-completed",
        "message_retry" => "ucode:hooks-message/on-retry",
        // Model
        "before_model_call" => "ucode:hooks-model/on-before-call",
        "after_model_call" => "ucode:hooks-model/on-after-call",
        "before_model_select" => "ucode:hooks-model/on-before-select",
        "model_fallback" => "ucode:hooks-model/on-fallback",
        "router_decision" => "ucode:hooks-model/on-router-decision",
        "model_rate_limited" => "ucode:hooks-model/on-rate-limited",
        "model_quota_exhausted" => "ucode:hooks-model/on-quota-exhausted",
        // Tool
        "before_tool_call" => "ucode:hooks-tool/on-before-call",
        "after_tool_call" => "ucode:hooks-tool/on-after-call",
        "tool_error" => "ucode:hooks-tool/on-error",
        "tool_timeout" => "ucode:hooks-tool/on-timeout",
        // Tool FS
        "before_file_read" => "ucode:hooks-tool-fs/on-before-read",
        "after_file_read" => "ucode:hooks-tool-fs/on-after-read",
        "before_file_write" => "ucode:hooks-tool-fs/on-before-write",
        "after_file_write" => "ucode:hooks-tool-fs/on-after-write",
        // Tool CMD
        "before_run_cmd" => "ucode:hooks-tool-cmd/on-before-run",
        "after_run_cmd" => "ucode:hooks-tool-cmd/on-after-run",
        // Tool Patch
        "before_apply_patch" => "ucode:hooks-tool-patch/on-before-apply",
        "after_apply_patch" => "ucode:hooks-tool-patch/on-after-apply",
        // Context
        "context_overflow" => "ucode:hooks-context/on-overflow",
        "context_compaction" => "ucode:hooks-context/on-compaction",
        "context_distilled" => "ucode:hooks-context/on-distilled",
        "token_usage_updated" => "ucode:hooks-context/on-usage-updated",
        // Agent
        "agent_spawned" => "ucode:hooks-agent/on-spawned",
        "agent_message" => "ucode:hooks-agent/on-message",
        "agent_completed" => "ucode:hooks-agent/on-completed",
        "agent_failed" => "ucode:hooks-agent/on-failed",
        "agent_cancelled" => "ucode:hooks-agent/on-cancelled",
        // Approval
        "approval_required" => "ucode:hooks-approval/on-required",
        "approval_granted" => "ucode:hooks-approval/on-granted",
        "approval_denied" => "ucode:hooks-approval/on-denied",
        "sandbox_decision" => "ucode:hooks-approval/on-sandbox-decision",
        "permission_decision" => "ucode:hooks-approval/on-permission-decision",
        // Auth
        "auth_changed" => "ucode:hooks-auth/on-changed",
        "auth_failed" => "ucode:hooks-auth/on-failed",
        "provider_switched" => "ucode:hooks-auth/on-provider-switched",
        // MCP
        "mcp_server_connected" => "ucode:hooks-mcp/on-connected",
        "mcp_server_disconnected" => "ucode:hooks-mcp/on-disconnected",
        "mcp_server_launch" => "ucode:hooks-mcp/on-launch",
        "mcp_server_restart" => "ucode:hooks-mcp/on-restart",
        "mcp_server_crash" => "ucode:hooks-mcp/on-crash",
        "mcp_tool_invoked" => "ucode:hooks-mcp/on-tool-invoked",
        // Skill
        "skill_activated" => "ucode:hooks-skill/on-activated",
        "skill_deactivated" => "ucode:hooks-skill/on-deactivated",
        // Plugin
        "plugin_loaded" => "ucode:hooks-plugin/on-loaded",
        "plugin_unloaded" => "ucode:hooks-plugin/on-unloaded",
        "plugin_error" => "ucode:hooks-plugin/on-error",
        // Checkpoint
        "checkpoint_created" => "ucode:hooks-checkpoint/on-created",
        "checkpoint_restored" => "ucode:hooks-checkpoint/on-restored",
        // Budget
        "budget_threshold_warning" => "ucode:hooks-budget/on-warning",
        "budget_threshold_reached" => "ucode:hooks-budget/on-reached",
        "cost_incurred" => "ucode:hooks-budget/on-cost-incurred",
        // Job
        "background_job_state_changed" => "ucode:hooks-job/on-state-changed",
        // Command
        "command_invoked" => "ucode:hooks-command/on-invoked",
        "palette_command_executed" => "ucode:hooks-command/on-palette-executed",
        // Diagnostic
        "unhandled_error" => "ucode:hooks-diagnostic/on-unhandled-error",
        // Transform
        "transform_messages" => "ucode:hooks-transform/on-transform-messages",
        "transform_system_prompt" => "ucode:hooks-transform/on-transform-system-prompt",
        _ => return None,
    })
}

/// All 65 event names paired with their WIT interface paths.
///
/// Used by the host to probe which interfaces a component exports at load time.
pub const EVENT_INTERFACE_MAP: &[(&str, &str)] = &[
    ("session_start", "ucode:hooks-session/on-start"),
    ("session_end", "ucode:hooks-session/on-end"),
    (
        "session_title_generated",
        "ucode:hooks-session/on-title-generated",
    ),
    (
        "session_title_updated",
        "ucode:hooks-session/on-title-updated",
    ),
    ("config_reloaded", "ucode:hooks-session/on-config-reloaded"),
    (
        "user_message_received",
        "ucode:hooks-message/on-user-message",
    ),
    (
        "assistant_response_started",
        "ucode:hooks-message/on-response-started",
    ),
    (
        "assistant_response_completed",
        "ucode:hooks-message/on-response-completed",
    ),
    ("message_retry", "ucode:hooks-message/on-retry"),
    ("before_model_call", "ucode:hooks-model/on-before-call"),
    ("after_model_call", "ucode:hooks-model/on-after-call"),
    ("before_model_select", "ucode:hooks-model/on-before-select"),
    ("model_fallback", "ucode:hooks-model/on-fallback"),
    ("router_decision", "ucode:hooks-model/on-router-decision"),
    ("model_rate_limited", "ucode:hooks-model/on-rate-limited"),
    (
        "model_quota_exhausted",
        "ucode:hooks-model/on-quota-exhausted",
    ),
    ("before_tool_call", "ucode:hooks-tool/on-before-call"),
    ("after_tool_call", "ucode:hooks-tool/on-after-call"),
    ("tool_error", "ucode:hooks-tool/on-error"),
    ("tool_timeout", "ucode:hooks-tool/on-timeout"),
    ("before_file_read", "ucode:hooks-tool-fs/on-before-read"),
    ("after_file_read", "ucode:hooks-tool-fs/on-after-read"),
    ("before_file_write", "ucode:hooks-tool-fs/on-before-write"),
    ("after_file_write", "ucode:hooks-tool-fs/on-after-write"),
    ("before_run_cmd", "ucode:hooks-tool-cmd/on-before-run"),
    ("after_run_cmd", "ucode:hooks-tool-cmd/on-after-run"),
    (
        "before_apply_patch",
        "ucode:hooks-tool-patch/on-before-apply",
    ),
    ("after_apply_patch", "ucode:hooks-tool-patch/on-after-apply"),
    ("context_overflow", "ucode:hooks-context/on-overflow"),
    ("context_compaction", "ucode:hooks-context/on-compaction"),
    ("context_distilled", "ucode:hooks-context/on-distilled"),
    (
        "token_usage_updated",
        "ucode:hooks-context/on-usage-updated",
    ),
    ("agent_spawned", "ucode:hooks-agent/on-spawned"),
    ("agent_message", "ucode:hooks-agent/on-message"),
    ("agent_completed", "ucode:hooks-agent/on-completed"),
    ("agent_failed", "ucode:hooks-agent/on-failed"),
    ("agent_cancelled", "ucode:hooks-agent/on-cancelled"),
    ("approval_required", "ucode:hooks-approval/on-required"),
    ("approval_granted", "ucode:hooks-approval/on-granted"),
    ("approval_denied", "ucode:hooks-approval/on-denied"),
    (
        "sandbox_decision",
        "ucode:hooks-approval/on-sandbox-decision",
    ),
    (
        "permission_decision",
        "ucode:hooks-approval/on-permission-decision",
    ),
    ("auth_changed", "ucode:hooks-auth/on-changed"),
    ("auth_failed", "ucode:hooks-auth/on-failed"),
    ("provider_switched", "ucode:hooks-auth/on-provider-switched"),
    ("mcp_server_connected", "ucode:hooks-mcp/on-connected"),
    ("mcp_server_disconnected", "ucode:hooks-mcp/on-disconnected"),
    ("mcp_server_launch", "ucode:hooks-mcp/on-launch"),
    ("mcp_server_restart", "ucode:hooks-mcp/on-restart"),
    ("mcp_server_crash", "ucode:hooks-mcp/on-crash"),
    ("mcp_tool_invoked", "ucode:hooks-mcp/on-tool-invoked"),
    ("skill_activated", "ucode:hooks-skill/on-activated"),
    ("skill_deactivated", "ucode:hooks-skill/on-deactivated"),
    ("plugin_loaded", "ucode:hooks-plugin/on-loaded"),
    ("plugin_unloaded", "ucode:hooks-plugin/on-unloaded"),
    ("plugin_error", "ucode:hooks-plugin/on-error"),
    ("checkpoint_created", "ucode:hooks-checkpoint/on-created"),
    ("checkpoint_restored", "ucode:hooks-checkpoint/on-restored"),
    ("budget_threshold_warning", "ucode:hooks-budget/on-warning"),
    ("budget_threshold_reached", "ucode:hooks-budget/on-reached"),
    ("cost_incurred", "ucode:hooks-budget/on-cost-incurred"),
    (
        "background_job_state_changed",
        "ucode:hooks-job/on-state-changed",
    ),
    ("command_invoked", "ucode:hooks-command/on-invoked"),
    (
        "palette_command_executed",
        "ucode:hooks-command/on-palette-executed",
    ),
    (
        "unhandled_error",
        "ucode:hooks-diagnostic/on-unhandled-error",
    ),
    (
        "transform_messages",
        "ucode:hooks-transform/on-transform-messages",
    ),
    (
        "transform_system_prompt",
        "ucode:hooks-transform/on-transform-system-prompt",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_interface_map_has_64_entries() {
        // world.wit exports 67 hook interfaces — verified by counting
        // `export ucode:hooks*` lines in world.wit.
        assert_eq!(EVENT_INTERFACE_MAP.len(), 67);
    }

    #[test]
    fn test_event_interface_map_unique_names() {
        let mut names: Vec<&str> = EVENT_INTERFACE_MAP.iter().map(|(n, _)| *n).collect();
        let original_len = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), original_len, "duplicate event names found");
    }

    #[test]
    fn test_event_interface_map_unique_paths() {
        let mut paths: Vec<&str> = EVENT_INTERFACE_MAP.iter().map(|(_, p)| *p).collect();
        let original_len = paths.len();
        paths.sort_unstable();
        paths.dedup();
        assert_eq!(paths.len(), original_len, "duplicate WIT paths found");
    }

    #[test]
    fn test_event_to_wit_interface_known() {
        assert_eq!(
            event_to_wit_interface("session_start"),
            Some("ucode:hooks-session/on-start")
        );
        assert_eq!(
            event_to_wit_interface("before_tool_call"),
            Some("ucode:hooks-tool/on-before-call")
        );
        assert_eq!(
            event_to_wit_interface("unhandled_error"),
            Some("ucode:hooks-diagnostic/on-unhandled-error")
        );
    }

    #[test]
    fn test_event_to_wit_interface_unknown() {
        assert_eq!(event_to_wit_interface("nonexistent_event"), None);
    }

    #[test]
    fn test_wit_response_ok() {
        let resp = wit::HookResponse {
            kind: wit::HookResponseKind::Ok,
            data: None,
        };
        assert!(matches!(
            wit_response_to_native(resp),
            NativeHookResponse::Ok
        ));
    }

    #[test]
    fn test_wit_response_modify() {
        let resp = wit::HookResponse {
            kind: wit::HookResponseKind::Modify,
            data: Some(r#"{"key":"val"}"#.to_string()),
        };
        match wit_response_to_native(resp) {
            NativeHookResponse::Modify { changes } => {
                assert_eq!(changes, serde_json::json!({"key": "val"}));
            }
            _ => panic!("expected Modify"),
        }
    }

    #[test]
    fn test_wit_response_veto() {
        let resp = wit::HookResponse {
            kind: wit::HookResponseKind::Veto,
            data: Some("blocked".to_string()),
        };
        match wit_response_to_native(resp) {
            NativeHookResponse::Veto { reason } => assert_eq!(reason, "blocked"),
            _ => panic!("expected Veto"),
        }
    }

    #[test]
    fn test_wit_response_modify_invalid_json() {
        let resp = wit::HookResponse {
            kind: wit::HookResponseKind::Modify,
            data: Some("not-json".to_string()),
        };
        match wit_response_to_native(resp) {
            NativeHookResponse::Modify { changes } => {
                assert_eq!(changes, serde_json::Value::Null);
            }
            _ => panic!("expected Modify"),
        }
    }

    #[test]
    fn test_wit_response_veto_no_data() {
        let resp = wit::HookResponse {
            kind: wit::HookResponseKind::Veto,
            data: None,
        };
        match wit_response_to_native(resp) {
            NativeHookResponse::Veto { reason } => assert_eq!(reason, ""),
            _ => panic!("expected Veto"),
        }
    }

    #[test]
    fn test_all_event_names_have_interface() {
        for &(name, expected_path) in EVENT_INTERFACE_MAP {
            assert_eq!(
                event_to_wit_interface(name),
                Some(expected_path),
                "mismatch for event: {name}"
            );
        }
    }
}
