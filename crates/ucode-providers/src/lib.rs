//! ucode-providers: Provider trait, capability model, and adapters (OpenAI, Anthropic, Ollama).

pub mod anthropic;
pub mod mock;
pub mod ollama;
pub mod openai;
pub mod provider;

pub use anthropic::{AnthropicProvider, parse_anthropic_sse_line};
pub use mock::MockProvider;
pub use ollama::OllamaProvider;
pub use openai::{OpenaiProvider, parse_sse_line};
pub use provider::{Capabilities, ChatRequest, Provider, ProviderFuture, ToolDef};
