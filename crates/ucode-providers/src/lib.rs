//! ucode-providers: Provider trait, capability model, and adapters (OpenAI, Anthropic, Ollama).

pub mod anthropic;
pub mod config;
pub mod mock;
pub mod ollama;
pub mod openai;
pub mod provider;
pub mod sse;

pub use anthropic::{AnthropicCompatProvider, parse_anthropic_sse_line};

/// Backward-compat alias.
pub type AnthropicProvider = AnthropicCompatProvider;
pub use config::{AdapterKind, ProviderConfig, ProvidersTable};
pub use mock::MockProvider;
pub use ollama::{OllamaProvider, parse_ollama_line};
pub use openai::{OpenAiCompatProvider, parse_sse_line};

/// Backward-compat alias.
pub type OpenaiProvider = OpenAiCompatProvider;
pub use provider::{Capabilities, ChatRequest, Provider, ProviderFuture, ToolDef};
