//! ucode-providers: Provider trait, capability model, and adapters (OpenAI, Anthropic, Ollama).

pub mod anthropic;
pub mod config;
pub mod mock;
pub mod ollama;
pub mod openai;
pub mod provider;
pub mod sse;

pub use anthropic::{AnthropicProvider, parse_anthropic_sse_line};
pub use config::{AdapterKind, ProviderConfig, ProvidersTable};
pub use mock::MockProvider;
pub use ollama::OllamaProvider;
pub use openai::{OpenaiProvider, parse_sse_line};
pub use provider::{Capabilities, ChatRequest, Provider, ProviderFuture, ToolDef};
