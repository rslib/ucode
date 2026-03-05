use serde::{Deserialize, Serialize};

use crate::budget::CompactionRecord;
use crate::error::CoreError;
use crate::message::{ToolCall, ToolResult};

/// A streaming event emitted by a provider or agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "value")]
pub enum Event {
    /// A text token from the model.
    Token(String),
    /// A tool invocation requested by the model.
    ToolCall(ToolCall),
    /// The result of a tool execution.
    ToolResult(ToolResult),
    /// A JSON patch to apply to structured output.
    Patch(String),
    /// A diagnostic log message.
    Log(String),
    /// A terminal error.
    Error(CoreError),
    /// A compaction/distillation step was applied.
    Compaction(CompactionRecord),
    /// Signals the end of the stream.
    Done,
}

/// A pinned, boxed, send-able stream of [`Event`]s.
pub type EventStream = std::pin::Pin<Box<dyn futures_core::Stream<Item = Event> + Send>>;
