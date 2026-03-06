// Public API for the headless runner is intentionally ahead of the full agent
// loop integration; suppress dead-code lints until callers are wired up.
#![allow(dead_code)]

use chrono::Utc;
use serde::{Deserialize, Serialize};
use ucode_core::Event;

// ── Output types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeadlessOutput {
    pub events: Vec<HeadlessEvent>,
    pub usage: HeadlessUsage,
    pub exit_code: u8,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeadlessEvent {
    /// One of: "token", "tool_call", "tool_result", "patch", "log",
    /// "error", "compaction", "done".
    pub event_type: String,
    pub data: serde_json::Value,
    /// ISO 8601 timestamp at the moment the event was recorded.
    pub timestamp: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HeadlessUsage {
    pub total_tokens: usize,
    pub tool_calls: usize,
    pub duration_ms: u64,
}

// ── Exit codes ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ExitCode {
    Success = 0,
    PolicyDeny = 1,
    ToolFailure = 2,
    Timeout = 3,
    InternalError = 4,
}

impl ExitCode {
    pub fn as_u8(&self) -> u8 {
        *self as u8
    }

    pub fn from_u8(code: u8) -> Self {
        match code {
            0 => Self::Success,
            1 => Self::PolicyDeny,
            2 => Self::ToolFailure,
            3 => Self::Timeout,
            4 => Self::InternalError,
            _ => Self::InternalError,
        }
    }
}

// ── Runner ────────────────────────────────────────────────────────────────────

pub struct HeadlessRunner {
    /// When true, `format_output` emits pretty-printed JSON.
    pub json_output: bool,
    pub session_id: Option<String>,
}

impl HeadlessRunner {
    pub fn new(json_output: bool) -> Self {
        Self {
            json_output,
            session_id: None,
        }
    }

    pub fn with_session_id(mut self, id: String) -> Self {
        self.session_id = Some(id);
        self
    }

    /// Convert a core [`Event`] into a [`HeadlessEvent`] stamped with the
    /// current UTC time.
    pub fn record_event(&self, event: &Event) -> HeadlessEvent {
        let timestamp = Utc::now().to_rfc3339();

        let (event_type, data) = match event {
            Event::Token(text) => ("token".to_owned(), serde_json::json!({ "text": text })),
            Event::ToolCall(tc) => (
                "tool_call".to_owned(),
                serde_json::json!({
                    "id":   tc.id,
                    "name": tc.name,
                    "args": tc.args,
                }),
            ),
            Event::ToolResult(tr) => (
                "tool_result".to_owned(),
                serde_json::json!({
                    "id":       tr.id,
                    "name":     tr.name,
                    "result":   tr.result,
                    "is_error": tr.is_error,
                }),
            ),
            Event::Patch(patch) => ("patch".to_owned(), serde_json::json!({ "patch": patch })),
            Event::Log(msg) => ("log".to_owned(), serde_json::json!({ "message": msg })),
            Event::Error(err) => (
                "error".to_owned(),
                // CoreError implements Display + Serialize; capture both.
                serde_json::json!({
                    "message": err.to_string(),
                    "detail":  serde_json::to_value(err).unwrap_or(serde_json::Value::Null),
                }),
            ),
            Event::Compaction(rec) => (
                "compaction".to_owned(),
                serde_json::to_value(rec).unwrap_or(serde_json::Value::Null),
            ),
            Event::Done => ("done".to_owned(), serde_json::Value::Null),
        };

        HeadlessEvent {
            event_type,
            data,
            timestamp,
        }
    }

    pub fn build_output(
        &self,
        events: Vec<HeadlessEvent>,
        usage: HeadlessUsage,
        exit_code: ExitCode,
    ) -> HeadlessOutput {
        HeadlessOutput {
            events,
            usage,
            exit_code: exit_code.as_u8(),
            session_id: self.session_id.clone(),
        }
    }

    /// Serialize `output` to a JSON string.  Always pretty-printed for
    /// readability in CI logs.
    pub fn format_output(&self, output: &HeadlessOutput) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(output)
    }

    /// Scan `events` and return the most-specific non-success exit code, or
    /// [`ExitCode::Success`] if no error events are present.
    ///
    /// Priority (highest first): PolicyDeny > Timeout > ToolFailure > InternalError.
    pub fn determine_exit_code(events: &[HeadlessEvent]) -> ExitCode {
        let mut code = ExitCode::Success;

        for ev in events {
            if ev.event_type != "error" {
                continue;
            }
            let msg = ev
                .data
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_ascii_lowercase();

            let candidate = if msg.contains("policy") {
                ExitCode::PolicyDeny
            } else if msg.contains("timeout") || msg.contains("timed out") {
                ExitCode::Timeout
            } else if msg.contains("tool") {
                ExitCode::ToolFailure
            } else {
                ExitCode::InternalError
            };

            // Keep the highest-priority (lowest numeric) non-success code.
            if candidate.as_u8() < code.as_u8() || code == ExitCode::Success {
                code = candidate;
            }
        }

        code
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use ucode_core::message::{ToolCall, ToolResult};

    use super::*;

    // ── ExitCode ──────────────────────────────────────────────────────────────

    #[test]
    fn test_exit_code_values() {
        assert_eq!(ExitCode::Success as u8, 0);
        assert_eq!(ExitCode::PolicyDeny as u8, 1);
        assert_eq!(ExitCode::ToolFailure as u8, 2);
        assert_eq!(ExitCode::Timeout as u8, 3);
        assert_eq!(ExitCode::InternalError as u8, 4);
    }

    #[test]
    fn test_exit_code_from_u8() {
        assert_eq!(ExitCode::from_u8(0), ExitCode::Success);
        assert_eq!(ExitCode::from_u8(1), ExitCode::PolicyDeny);
        assert_eq!(ExitCode::from_u8(2), ExitCode::ToolFailure);
        assert_eq!(ExitCode::from_u8(3), ExitCode::Timeout);
        assert_eq!(ExitCode::from_u8(4), ExitCode::InternalError);
    }

    #[test]
    fn test_exit_code_from_u8_unknown() {
        assert_eq!(ExitCode::from_u8(99), ExitCode::InternalError);
        assert_eq!(ExitCode::from_u8(255), ExitCode::InternalError);
    }

    // ── HeadlessRunner construction ───────────────────────────────────────────

    #[test]
    fn test_headless_runner_new() {
        let r = HeadlessRunner::new(false);
        assert!(!r.json_output);
        assert!(r.session_id.is_none());
    }

    #[test]
    fn test_headless_runner_with_session_id() {
        let r = HeadlessRunner::new(true).with_session_id("abc-123".to_owned());
        assert_eq!(r.session_id.as_deref(), Some("abc-123"));
    }

    // ── record_event ──────────────────────────────────────────────────────────

    #[test]
    fn test_record_event_token() {
        let r = HeadlessRunner::new(false);
        let ev = r.record_event(&Event::Token("hello".to_owned()));
        assert_eq!(ev.event_type, "token");
        assert_eq!(ev.data["text"], "hello");
        // Timestamp must be a non-empty string.
        assert!(!ev.timestamp.is_empty());
    }

    #[test]
    fn test_record_event_done() {
        let r = HeadlessRunner::new(false);
        let ev = r.record_event(&Event::Done);
        assert_eq!(ev.event_type, "done");
        assert_eq!(ev.data, serde_json::Value::Null);
    }

    #[test]
    fn test_record_event_error() {
        let r = HeadlessRunner::new(false);
        let core_err = ucode_core::error::CoreError::Internal {
            message: "something went wrong".to_owned(),
        };
        let ev = r.record_event(&Event::Error(core_err));
        assert_eq!(ev.event_type, "error");
        let msg = ev.data["message"].as_str().unwrap();
        assert!(msg.contains("something went wrong"), "msg={msg}");
    }

    // ── build_output ──────────────────────────────────────────────────────────

    #[test]
    fn test_build_output() {
        let r = HeadlessRunner::new(false).with_session_id("s1".to_owned());
        let events = vec![r.record_event(&Event::Done)];
        let usage = HeadlessUsage {
            total_tokens: 42,
            tool_calls: 1,
            duration_ms: 500,
        };
        let out = r.build_output(events, usage, ExitCode::Success);
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.session_id.as_deref(), Some("s1"));
        assert_eq!(out.events.len(), 1);
        assert_eq!(out.usage.total_tokens, 42);
    }

    // ── format_output ─────────────────────────────────────────────────────────

    #[test]
    fn test_format_output_json() {
        let r = HeadlessRunner::new(true);
        let out = r.build_output(vec![], HeadlessUsage::default(), ExitCode::Success);
        let json = r.format_output(&out).unwrap();
        // Must be valid JSON and contain expected keys.
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed.get("events").is_some());
        assert!(parsed.get("usage").is_some());
        assert_eq!(parsed["exit_code"], 0);
    }

    // ── determine_exit_code ───────────────────────────────────────────────────

    fn error_event(msg: &str) -> HeadlessEvent {
        HeadlessEvent {
            event_type: "error".to_owned(),
            data: serde_json::json!({ "message": msg }),
            timestamp: Utc::now().to_rfc3339(),
        }
    }

    #[test]
    fn test_determine_exit_code_success() {
        let events = vec![
            HeadlessEvent {
                event_type: "token".to_owned(),
                data: serde_json::json!({ "text": "hi" }),
                timestamp: Utc::now().to_rfc3339(),
            },
            HeadlessEvent {
                event_type: "done".to_owned(),
                data: serde_json::Value::Null,
                timestamp: Utc::now().to_rfc3339(),
            },
        ];
        assert_eq!(
            HeadlessRunner::determine_exit_code(&events),
            ExitCode::Success
        );
    }

    #[test]
    fn test_determine_exit_code_policy_deny() {
        let events = vec![error_event("policy violation detected")];
        assert_eq!(
            HeadlessRunner::determine_exit_code(&events),
            ExitCode::PolicyDeny
        );
    }

    #[test]
    fn test_determine_exit_code_tool_failure() {
        let events = vec![error_event("tool 'bash' failed: permission denied")];
        assert_eq!(
            HeadlessRunner::determine_exit_code(&events),
            ExitCode::ToolFailure
        );
    }

    #[test]
    fn test_determine_exit_code_timeout() {
        let events = vec![error_event("operation timed out after 300s")];
        // "timeout" appears in the message → Timeout
        assert_eq!(
            HeadlessRunner::determine_exit_code(&events),
            ExitCode::Timeout
        );
    }

    // ── record_event for tool_call / tool_result ──────────────────────────────

    #[test]
    fn test_record_event_tool_call() {
        let r = HeadlessRunner::new(false);
        let tc = ToolCall::new("id1", "bash", serde_json::json!({"cmd": "ls"}));
        let ev = r.record_event(&Event::ToolCall(tc));
        assert_eq!(ev.event_type, "tool_call");
        assert_eq!(ev.data["name"], "bash");
        assert_eq!(ev.data["id"], "id1");
    }

    #[test]
    fn test_record_event_tool_result() {
        let r = HeadlessRunner::new(false);
        let tr = ToolResult::new("id1", "bash", serde_json::json!("ok"), false);
        let ev = r.record_event(&Event::ToolResult(tr));
        assert_eq!(ev.event_type, "tool_result");
        assert_eq!(ev.data["is_error"], false);
    }
}
