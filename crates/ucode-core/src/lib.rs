//! ucode-core: canonical messages, events, router, session state, subagent orchestration

pub mod agent;
pub mod agent_def;
pub mod agent_registry;
pub mod budget;
pub mod builtin_agents;
pub mod comm;
pub mod directive;
pub mod error;
pub mod event;
pub mod jobs;
pub mod logging;
pub mod message;
pub mod operational;
pub mod router;
pub mod session;

pub use agent::{
    AgentHandle, AgentId, AgentInfo, AgentResult, AgentSpec, AgentState, Orchestrator,
};
pub use budget::{
    BudgetAction, BudgetCheck, BudgetManager, CharEstimator, CompactionPolicy, CompactionRecord,
    CompactionStep, CostBudget, CostGovernor, CountSource, SessionUsage, TokenBudget, TokenCounter,
    UsageRecord,
};
pub use comm::{AgentMessage, CommBus, CommError, CommPolicy};
pub use directive::{Directive, ParsedInput, Span, parse_input};
pub use error::{AuthErrorKind, CoreError};
pub use event::{Event, EventStream};
pub use jobs::{JobController, JobId, JobInfo, JobResult, JobState};
pub use logging::{
    LogConfig, LogGuard, LogLevel, default_config_home, default_log_dir, init_logging,
};
pub use message::{Message, Part, Role, ToolCall, ToolResult};
pub use router::{FallbackReason, ModelEndpoint, ModelGroup, RouteDecision, Router, RouterConfig};
pub use session::{Session, SessionId, SessionMeta, SessionStore, TitleSource, ToolAuditEntry};
