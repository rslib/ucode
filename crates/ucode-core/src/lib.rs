//! ucode-core: canonical messages, events, router, session state, subagent orchestration

pub mod error;
pub mod event;
pub mod message;

pub use error::{AuthErrorKind, CoreError};
pub use event::{Event, EventStream};
pub use message::{Message, Part, Role, ToolCall, ToolResult};
