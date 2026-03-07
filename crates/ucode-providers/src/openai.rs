use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use ucode_core::{CoreError, Event, EventStream, ToolCall as CoreToolCall};

use crate::config::ProviderConfig;
use crate::provider::{Capabilities, ChatRequest, Provider, ProviderFuture, ToolDef};
use crate::sse::stream_lines;

// ── SSE response types ────────────────────────────────────────────────────────

/// OpenAI streaming chunk (SSE `data:` payload).
#[derive(Debug, Deserialize)]
pub(crate) struct ChatCompletionChunk {
    pub choices: Vec<ChunkChoice>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChunkChoice {
    pub delta: Delta,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct Delta {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<DeltaToolCall>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DeltaToolCall {
    pub index: usize,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub function: Option<DeltaFunction>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DeltaFunction {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

// ── Tool call accumulator ─────────────────────────────────────────────────────

/// State for accumulating streamed tool calls across multiple chunks.
#[derive(Debug, Default)]
pub struct ToolCallAccumulator {
    calls: Vec<PendingToolCall>,
}

#[derive(Debug, Default)]
struct PendingToolCall {
    id: String,
    name: String,
    arguments: String,
}

impl ToolCallAccumulator {
    /// Process a delta tool call chunk. Accumulates arguments across chunks.
    fn process(&mut self, dtc: &DeltaToolCall) {
        while self.calls.len() <= dtc.index {
            self.calls.push(PendingToolCall::default());
        }
        let pending = &mut self.calls[dtc.index];
        if let Some(ref id) = dtc.id {
            pending.id = id.clone();
        }
        if let Some(ref func) = dtc.function {
            if let Some(ref name) = func.name {
                pending.name = name.clone();
            }
            if let Some(ref args) = func.arguments {
                pending.arguments.push_str(args);
            }
        }
    }

    /// Drain all accumulated tool calls into canonical Events.
    fn drain(&mut self) -> Vec<Event> {
        self.calls
            .drain(..)
            .filter(|tc| !tc.id.is_empty())
            .map(|tc| {
                let args = serde_json::from_str(&tc.arguments)
                    .unwrap_or(serde_json::Value::String(tc.arguments));
                Event::ToolCall(CoreToolCall::new(tc.id, tc.name, args))
            })
            .collect()
    }
}

// ── SSE line parser ───────────────────────────────────────────────────────────

/// Parse a single SSE data line into events.
/// Returns empty vec for keep-alive or unparseable lines.
pub fn parse_sse_line(line: &str, accumulator: &mut ToolCallAccumulator) -> Vec<Event> {
    let line = line.trim();

    let data = if let Some(d) = line.strip_prefix("data: ") {
        d.trim()
    } else if let Some(d) = line.strip_prefix("data:") {
        d.trim()
    } else {
        return vec![];
    };

    if data == "[DONE]" {
        let mut events = accumulator.drain();
        events.push(Event::Done);
        return events;
    }

    let chunk: ChatCompletionChunk = match serde_json::from_str(data) {
        Ok(c) => c,
        Err(_) => return vec![],
    };

    let mut events = Vec::new();

    for choice in &chunk.choices {
        if let Some(ref content) = choice.delta.content
            && !content.is_empty()
        {
            events.push(Event::Token(content.clone()));
        }

        if let Some(ref tool_calls) = choice.delta.tool_calls {
            for dtc in tool_calls {
                accumulator.process(dtc);
            }
        }

        if choice.finish_reason.as_deref() == Some("tool_calls") {
            events.extend(accumulator.drain());
        }
    }

    events
}

// ── Request body types ────────────────────────────────────────────────────────

#[derive(Serialize)]
struct OpenAiRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<usize>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<OpenAiTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormat>,
}

#[derive(Serialize)]
struct OpenAiMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct OpenAiTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: OpenAiFunction,
}

#[derive(Serialize)]
struct OpenAiFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    format_type: String,
}

fn to_openai_messages(messages: &[ucode_core::Message]) -> Vec<OpenAiMessage> {
    messages
        .iter()
        .map(|m| {
            let role = match m.role {
                ucode_core::Role::System => "system",
                ucode_core::Role::User => "user",
                ucode_core::Role::Assistant => "assistant",
                ucode_core::Role::Tool => "tool",
            };
            let content = m
                .parts
                .iter()
                .filter_map(|p| match p {
                    ucode_core::Part::Text(t) => Some(t.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");
            OpenAiMessage {
                role: role.into(),
                content,
            }
        })
        .collect()
}

fn to_openai_tools(tools: &[ToolDef]) -> Vec<OpenAiTool> {
    tools
        .iter()
        .map(|t| OpenAiTool {
            tool_type: "function".into(),
            function: OpenAiFunction {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: t.parameters.clone(),
            },
        })
        .collect()
}

// ── Provider ──────────────────────────────────────────────────────────────────

/// OpenAI-compatible chat provider.
///
/// Works with OpenAI, Groq, Together, Fireworks, DeepSeek, Mistral,
/// OpenRouter, vLLM, LiteLLM, Azure OpenAI, and any endpoint that
/// implements the `/v1/chat/completions` streaming SSE protocol.
pub struct OpenAiCompatProvider {
    client: reqwest::Client,
    /// Provider instance name (from TOML key, e.g., "groq", "openai").
    provider_name: String,
    api_key: Option<String>,
    base_url: String,
    /// Extra headers sent with every request.
    headers: HashMap<String, String>,
}

impl OpenAiCompatProvider {
    /// Create from a provider config and resolved API key.
    pub fn from_config(name: &str, config: &ProviderConfig, api_key: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            provider_name: name.to_owned(),
            api_key,
            base_url: config.base_url().to_owned(),
            headers: config.headers.clone(),
        }
    }

    /// Create with just an API key (backward compat, defaults to OpenAI endpoint).
    pub fn new(api_key: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            provider_name: "openai".into(),
            api_key: Some(api_key),
            base_url: "https://api.openai.com/v1".into(),
            headers: HashMap::new(),
        }
    }

    /// Override the base URL.
    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url;
        self
    }
}

impl Provider for OpenAiCompatProvider {
    fn name(&self) -> &str {
        &self.provider_name
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            tool_calls: true,
            json_mode: true,
            max_context: 128_000,
            max_output: 16_384,
            streaming: true,
            token_counting: false,
        }
    }

    fn stream_chat(&self, req: ChatRequest) -> ProviderFuture<Result<EventStream, CoreError>> {
        let client = self.client.clone();
        let api_key = self.api_key.clone();
        let url = format!("{}/chat/completions", self.base_url);
        let provider_name = self.provider_name.clone();
        let custom_headers = self.headers.clone();

        let body = OpenAiRequest {
            model: req.model,
            messages: to_openai_messages(&req.messages),
            stream: true,
            temperature: req.temperature,
            max_tokens: req.max_tokens,
            tools: to_openai_tools(&req.tools),
            response_format: if req.json_mode {
                Some(ResponseFormat {
                    format_type: "json_object".into(),
                })
            } else {
                None
            },
        };

        Box::pin(async move {
            let mut request = client.post(&url).header("Content-Type", "application/json");

            if let Some(ref key) = api_key {
                request = request.header("Authorization", format!("Bearer {key}"));
            }

            for (k, v) in &custom_headers {
                request = request.header(k.as_str(), v.as_str());
            }

            let resp = request
                .json(&body)
                .send()
                .await
                .map_err(|e| CoreError::Provider {
                    provider: provider_name.clone(),
                    message: format!("HTTP request failed: {e}"),
                })?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body_text = resp.text().await.unwrap_or_default();
                if status.as_u16() == 401 || status.as_u16() == 403 {
                    return Err(CoreError::Auth {
                        provider: provider_name,
                        auth_kind: ucode_core::AuthErrorKind::Invalid,
                    });
                }
                return Err(CoreError::Provider {
                    provider: provider_name,
                    message: format!("HTTP {status}: {body_text}"),
                });
            }

            Ok(stream_lines(
                resp.bytes_stream(),
                ToolCallAccumulator::default(),
                parse_sse_line,
            ))
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AdapterKind, ProviderConfig};
    use std::collections::HashMap;

    #[test]
    fn from_config_uses_provider_name() {
        let config = ProviderConfig {
            adapter: AdapterKind::Openai,
            base_url: Some("https://api.groq.com/openai/v1".into()),
            api_key_env: None,
            headers: HashMap::new(),
        };
        let provider = OpenAiCompatProvider::from_config("groq", &config, Some("key".into()));
        assert_eq!(provider.name(), "groq");
        assert_eq!(provider.base_url, "https://api.groq.com/openai/v1");
    }

    #[test]
    fn from_config_with_custom_headers() {
        let mut headers = HashMap::new();
        headers.insert("api-version".into(), "2024-10-21".into());
        let config = ProviderConfig {
            adapter: AdapterKind::Openai,
            base_url: Some("https://azure.example.com".into()),
            api_key_env: None,
            headers,
        };
        let provider = OpenAiCompatProvider::from_config("azure", &config, Some("key".into()));
        assert_eq!(provider.headers.get("api-version").unwrap(), "2024-10-21");
    }

    #[test]
    fn new_defaults_to_openai() {
        let provider = OpenAiCompatProvider::new("test-key".into());
        assert_eq!(provider.name(), "openai");
        assert_eq!(provider.base_url, "https://api.openai.com/v1");
        assert_eq!(provider.api_key, Some("test-key".into()));
    }

    #[test]
    fn no_api_key_allowed() {
        let config = ProviderConfig {
            adapter: AdapterKind::Openai,
            base_url: None,
            api_key_env: None,
            headers: HashMap::new(),
        };
        let provider = OpenAiCompatProvider::from_config("local-vllm", &config, None);
        assert!(provider.api_key.is_none());
    }

    #[test]
    fn with_base_url_overrides() {
        let provider =
            OpenAiCompatProvider::new("key".into()).with_base_url("http://localhost:8080".into());
        assert_eq!(provider.base_url, "http://localhost:8080");
    }

    #[test]
    fn capabilities_unchanged() {
        let caps = OpenAiCompatProvider::new("key".into()).capabilities();
        assert!(caps.tool_calls);
        assert!(caps.json_mode);
        assert_eq!(caps.max_context, 128_000);
        assert_eq!(caps.max_output, 16_384);
        assert!(caps.streaming);
    }
}
