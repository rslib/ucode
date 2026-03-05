//! ucode-core: canonical messages, events, router, session state, subagent orchestration

pub mod agent;
pub mod error;
pub mod event;
pub mod message;
pub mod router;
pub mod session;

pub use agent::{
    AgentHandle, AgentId, AgentInfo, AgentResult, AgentSpec, AgentState, Orchestrator,
};
pub use error::{AuthErrorKind, CoreError};
pub use event::{Event, EventStream};
pub use message::{Message, Part, Role, ToolCall, ToolResult};
pub use router::{FallbackReason, ModelEndpoint, ModelGroup, RouteDecision, Router, RouterConfig};
pub use session::{Session, SessionId, SessionMeta, ToolAuditEntry};
