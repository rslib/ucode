//! ucode-agent: orchestration loop connecting user messages to LLM providers.

pub mod agent_loop;
pub mod config;

pub use agent_loop::{AgentEvent, AgentLoopConfig, AgentMessage, FileContext, run_agent_loop};
pub use config::{AppConfig, ConfigError, ensure_config_file};
pub use tokio_util::sync::CancellationToken;
