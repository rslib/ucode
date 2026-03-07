use serde::{Deserialize, Serialize};

use ucode_core::{CoreError, Event, EventStream, ToolCall as CoreToolCall};

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
pub struct OpenaiProvider {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
}

impl OpenaiProvider {
    /// Create a new OpenAI provider with the given API key.
    pub fn new(api_key: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            base_url: "https://api.openai.com/v1".into(),
        }
    }

    /// Override the base URL (for proxies, Azure, etc.).
    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url;
        self
    }
}

impl Provider for OpenaiProvider {
    fn name(&self) -> &str {
        "openai"
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
            let resp = client
                .post(&url)
                .header("Authorization", format!("Bearer {api_key}"))
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| CoreError::Provider {
                    provider: "openai".into(),
                    message: format!("HTTP request failed: {e}"),
                })?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body_text = resp.text().await.unwrap_or_default();
                if status.as_u16() == 401 || status.as_u16() == 403 {
                    return Err(CoreError::Auth {
                        provider: "openai".into(),
                        auth_kind: ucode_core::AuthErrorKind::Invalid,
                    });
                }
                return Err(CoreError::Provider {
                    provider: "openai".into(),
                    message: format!("HTTP {status}: {body_text}"),
                });
            }

            let byte_stream = resp.bytes_stream();

            Ok(stream_lines(
                byte_stream,
                ToolCallAccumulator::default(),
                parse_sse_line,
            ))
        })
    }
}
