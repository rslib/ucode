use futures_util::stream;

use ucode_core::{CoreError, Event, EventStream};

use crate::provider::{Capabilities, ChatRequest, Provider, ProviderFuture};

/// A mock provider for testing that emits configurable token events.
pub struct MockProvider {
    /// Tokens to emit on each stream_chat call.
    pub tokens: Vec<String>,
    /// Capabilities to report.
    pub caps: Capabilities,
}

impl MockProvider {
    /// Create a mock that emits the given tokens then Done.
    pub fn new(tokens: Vec<String>) -> Self {
        Self {
            tokens,
            caps: Capabilities {
                tool_calls: false,
                json_mode: false,
                max_context: 128_000,
                max_output: 4_096,
                streaming: true,
                token_counting: false,
            },
        }
    }

    /// Create a mock with custom capabilities.
    pub fn with_capabilities(mut self, caps: Capabilities) -> Self {
        self.caps = caps;
        self
    }
}

impl Provider for MockProvider {
    fn name(&self) -> &str {
        "mock"
    }

    fn capabilities(&self) -> Capabilities {
        self.caps.clone()
    }

    fn stream_chat(&self, _req: ChatRequest) -> ProviderFuture<Result<EventStream, CoreError>> {
        let mut events: Vec<Event> = self
            .tokens
            .iter()
            .map(|t| Event::Token(t.clone()))
            .collect();
        events.push(Event::Done);

        Box::pin(async move { Ok(Box::pin(stream::iter(events)) as EventStream) })
    }
}
