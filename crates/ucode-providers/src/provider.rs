use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};

use ucode_core::{CoreError, EventStream, Message};

/// A boxed, send-able future for provider operations.
pub type ProviderFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// Chat completion request sent to a provider.
#[derive(Debug, Clone)]
pub struct ChatRequest {
    /// Model identifier (e.g., "gpt-4o", "claude-3-opus").
    pub model: String,
    /// Conversation messages.
    pub messages: Vec<Message>,
    /// Sampling temperature (0.0 - 2.0).
    pub temperature: Option<f64>,
    /// Maximum tokens to generate.
    pub max_tokens: Option<usize>,
    /// Tool definitions available to the model.
    pub tools: Vec<ToolDef>,
    /// Whether to request JSON output mode.
    pub json_mode: bool,
}

impl ChatRequest {
    /// Create a minimal chat request with just model and messages.
    pub fn new(model: impl Into<String>, messages: Vec<Message>) -> Self {
        Self {
            model: model.into(),
            messages,
            temperature: None,
            max_tokens: None,
            tools: Vec::new(),
            json_mode: false,
        }
    }
}

/// A tool definition provided to the model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    /// Tool name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// JSON Schema for the tool's parameters.
    pub parameters: serde_json::Value,
}

/// Provider capability flags.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Capabilities {
    /// Supports tool/function calling.
    pub tool_calls: bool,
    /// Supports JSON output mode.
    pub json_mode: bool,
    /// Maximum context window size (tokens).
    pub max_context: usize,
    /// Maximum output tokens.
    pub max_output: usize,
    /// Supports streaming responses.
    pub streaming: bool,
    /// Supports provider-native token counting.
    pub token_counting: bool,
}

/// A model available from a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    /// Model identifier (e.g., "gpt-4o", "claude-sonnet-4-20250514").
    pub id: String,
    /// Human-readable name, if different from id.
    pub name: Option<String>,
}

/// The core provider interface.
///
/// Providers translate chat requests into canonical `Event` streams.
/// Use `ProviderFuture` return type for object safety with `dyn Provider`.
pub trait Provider: Send + Sync {
    /// Human-readable provider name (e.g., "openai", "anthropic", "ollama").
    fn name(&self) -> &str;

    /// Report this provider's capabilities.
    fn capabilities(&self) -> Capabilities;

    /// Stream a chat completion as a series of [`Event`]s.
    fn stream_chat(&self, req: ChatRequest) -> ProviderFuture<Result<EventStream, CoreError>>;

    /// Optional: count tokens using the provider's native tokenizer.
    /// Returns `None` if the provider doesn't support native counting.
    fn count_tokens(&self, _messages: &[Message]) -> Option<usize> {
        None
    }

    /// Optional: list available models from this provider.
    /// Returns an empty vec if the provider doesn't support model listing.
    fn list_models(&self) -> ProviderFuture<Result<Vec<ModelInfo>, CoreError>> {
        Box::pin(async { Ok(vec![]) })
    }
}
