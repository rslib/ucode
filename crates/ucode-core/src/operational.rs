use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Status of a run (subagent, tool call, MCP request).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Success,
    Failed,
    Cancelled,
}

impl RunStatus {
    pub fn icon(self) -> char {
        match self {
            Self::Running => '⟳',
            Self::Success => '✓',
            Self::Failed => '✗',
            Self::Cancelled => '○',
        }
    }

    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Running)
    }
}

/// Severity level for session events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventLevel {
    Info,
    Warn,
    Error,
}

impl EventLevel {
    pub fn badge(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
}

/// A single subagent invocation and its output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentRun {
    pub id: String,
    pub agent_name: String,
    pub task_description: String,
    pub status: RunStatus,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub duration_ms: Option<u64>,
    pub token_count: Option<u64>,
    /// Full output (markdown).
    pub output: String,
    /// Tool call IDs cross-referencing ToolRun entries.
    pub tool_call_ids: Vec<String>,
}

/// A single tool invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolRun {
    pub id: String,
    pub tool_name: String,
    /// e.g. "file=src/main.rs, offset=1"
    pub args_summary: String,
    pub status: RunStatus,
    pub started_at: DateTime<Utc>,
    pub duration_ms: Option<u64>,
    /// Full input parameters (JSON or formatted string).
    pub input: String,
    pub output: Option<String>,
    pub thinking: Option<String>,
    /// Which subagent spawned this, if any.
    pub subagent_id: Option<String>,
}

/// An MCP request/response log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpLogEntry {
    pub id: String,
    pub server_name: String,
    /// e.g. "tools/call", "resources/read"
    pub method: String,
    pub request_summary: String,
    pub request_body: String,
    pub response_body: Option<String>,
    pub status: RunStatus,
    pub timestamp: DateTime<Utc>,
    pub duration_ms: Option<u64>,
}

/// A structured session event for the Logs tab.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEvent {
    pub timestamp: DateTime<Utc>,
    pub level: EventLevel,
    /// e.g. "model_switch", "agent_spawn", "approval", "budget_warning"
    pub event_type: String,
    pub summary: String,
    pub detail: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_status_default_is_running() {
        let s = RunStatus::Running;
        assert_eq!(s.icon(), '⟳');
        assert!(!s.is_terminal());
    }

    #[test]
    fn run_status_terminal_states() {
        assert!(RunStatus::Success.is_terminal());
        assert!(RunStatus::Failed.is_terminal());
        assert!(RunStatus::Cancelled.is_terminal());
        assert!(!RunStatus::Running.is_terminal());
    }

    #[test]
    fn event_level_badge() {
        assert_eq!(EventLevel::Info.badge(), "INFO");
        assert_eq!(EventLevel::Warn.badge(), "WARN");
        assert_eq!(EventLevel::Error.badge(), "ERROR");
    }

    #[test]
    fn subagent_run_roundtrip() {
        let run = SubagentRun {
            id: "sa-001".into(),
            agent_name: "rust-expert".into(),
            task_description: "Fix tests".into(),
            status: RunStatus::Success,
            started_at: chrono::Utc::now(),
            completed_at: Some(chrono::Utc::now()),
            duration_ms: Some(1200),
            token_count: Some(890),
            output: "# Summary\nAll tests pass.".into(),
            tool_call_ids: vec!["tc-001".into(), "tc-002".into()],
        };
        let json = serde_json::to_string(&run).unwrap();
        let decoded: SubagentRun = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.id, "sa-001");
        assert_eq!(decoded.agent_name, "rust-expert");
        assert_eq!(decoded.tool_call_ids.len(), 2);
    }

    #[test]
    fn tool_run_roundtrip() {
        let run = ToolRun {
            id: "tc-001".into(),
            tool_name: "Read".into(),
            args_summary: "file=src/main.rs".into(),
            status: RunStatus::Success,
            started_at: chrono::Utc::now(),
            duration_ms: Some(45),
            input: r#"{"file":"src/main.rs","offset":1}"#.into(),
            output: Some("fn main() {}".into()),
            thinking: None,
            subagent_id: Some("sa-001".into()),
        };
        let json = serde_json::to_string(&run).unwrap();
        let decoded: ToolRun = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.tool_name, "Read");
    }

    #[test]
    fn mcp_log_entry_roundtrip() {
        let entry = McpLogEntry {
            id: "mcp-001".into(),
            server_name: "context7".into(),
            method: "tools/call".into(),
            request_summary: "query-docs next.js".into(),
            request_body: "{}".into(),
            response_body: Some("{}".into()),
            status: RunStatus::Success,
            timestamp: chrono::Utc::now(),
            duration_ms: Some(120),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let decoded: McpLogEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.server_name, "context7");
    }

    #[test]
    fn session_event_roundtrip() {
        let event = SessionEvent {
            timestamp: chrono::Utc::now(),
            level: EventLevel::Warn,
            event_type: "budget_warning".into(),
            summary: "Token budget at 75%".into(),
            detail: Some("Used 150k of 200k tokens".into()),
        };
        let json = serde_json::to_string(&event).unwrap();
        let decoded: SessionEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.event_type, "budget_warning");
        assert_eq!(decoded.level, EventLevel::Warn);
    }
}
