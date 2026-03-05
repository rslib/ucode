//! ucode-providers: Provider trait, capability model, and adapters (OpenAI, Anthropic, Ollama).

pub mod mock;
pub mod openai;
pub mod provider;

pub use mock::MockProvider;
pub use openai::{OpenaiProvider, parse_sse_line};
pub use provider::{Capabilities, ChatRequest, Provider, ProviderFuture, ToolDef};
