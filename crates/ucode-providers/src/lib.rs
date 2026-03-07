//! ucode-providers: Provider trait, capability model, and adapters
//! (OpenAI-compat, Anthropic-compat, Ollama native, Gemini).

pub mod anthropic;
pub mod auth;
pub mod claude_metadata;
pub mod config;
pub mod factory;
pub mod gemini;
pub mod mock;
pub mod ollama;
pub mod openai;
pub mod provider;
pub mod sse;
pub mod tool_normalize;

pub use anthropic::{AnthropicCompatProvider, parse_anthropic_sse_line};
pub use config::{AdapterKind, ProviderConfig, ProvidersTable};
pub use factory::{create_all_providers, create_provider};
pub use gemini::{GeminiProvider, parse_gemini_sse_line};
pub use mock::MockProvider;
pub use ollama::{OllamaProvider, parse_ollama_line};
pub use openai::{OpenAiCompatProvider, parse_sse_line};
pub use provider::{Capabilities, ChatRequest, ModelInfo, Provider, ProviderFuture, ToolDef};

/// Backward-compat type aliases.
pub type OpenaiProvider = OpenAiCompatProvider;
pub type AnthropicProvider = AnthropicCompatProvider;
