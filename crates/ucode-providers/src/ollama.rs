use serde::Serialize;

use ucode_core::{CoreError, EventStream};

use crate::openai::{ToolCallAccumulator, parse_sse_line};
use crate::provider::{Capabilities, ChatRequest, Provider, ProviderFuture, ToolDef};

// ── Request body types ────────────────────────────────────────────────────────

#[derive(Serialize)]
struct OllamaRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<OllamaTool>,
}

#[derive(Serialize)]
struct OllamaMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct OllamaTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: OllamaFunction,
}

#[derive(Serialize)]
struct OllamaFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

fn to_ollama_messages(messages: &[ucode_core::Message]) -> Vec<OllamaMessage> {
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
            OllamaMessage {
                role: role.into(),
                content,
            }
        })
        .collect()
}

fn to_ollama_tools(tools: &[ToolDef]) -> Vec<OllamaTool> {
    tools
        .iter()
        .map(|t| OllamaTool {
            tool_type: "function".into(),
            function: OllamaFunction {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: t.parameters.clone(),
            },
        })
        .collect()
}

// ── Provider ──────────────────────────────────────────────────────────────────

/// Ollama local inference provider using the OpenAI-compatible endpoint.
pub struct OllamaProvider {
    client: reqwest::Client,
    base_url: String,
}

impl OllamaProvider {
    /// Create a new Ollama provider pointing at `http://localhost:11434`.
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: "http://localhost:11434".into(),
        }
    }

    /// Override the base URL (for remote Ollama instances or testing).
    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url;
        self
    }
}

impl Default for OllamaProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl Provider for OllamaProvider {
    fn name(&self) -> &str {
        "ollama"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            tool_calls: true,
            json_mode: false,
            max_context: 128_000,
            max_output: 4_096,
            streaming: true,
            token_counting: false,
        }
    }

    fn stream_chat(&self, req: ChatRequest) -> ProviderFuture<Result<EventStream, CoreError>> {
        let client = self.client.clone();
        let url = format!("{}/v1/chat/completions", self.base_url);

        let body = OllamaRequest {
            model: req.model,
            messages: to_ollama_messages(&req.messages),
            stream: true,
            temperature: req.temperature,
            tools: to_ollama_tools(&req.tools),
        };

        Box::pin(async move {
            let resp = client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| CoreError::Provider {
                    provider: "ollama".into(),
                    message: format!("HTTP request failed: {e}"),
                })?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body_text = resp.text().await.unwrap_or_default();
                if status.as_u16() == 401 || status.as_u16() == 403 {
                    return Err(CoreError::Auth {
                        provider: "ollama".into(),
                        auth_kind: ucode_core::AuthErrorKind::Invalid,
                    });
                }
                return Err(CoreError::Provider {
                    provider: "ollama".into(),
                    message: format!("HTTP {status}: {body_text}"),
                });
            }

            let byte_stream = resp.bytes_stream();

            let event_stream = futures_util::stream::unfold(
                (byte_stream, ToolCallAccumulator::default(), String::new()),
                |(mut byte_stream, mut accumulator, mut buffer)| async move {
                    use futures_util::StreamExt;

                    loop {
                        while let Some(newline_pos) = buffer.find('\n') {
                            let line = buffer[..newline_pos].to_string();
                            buffer = buffer[newline_pos + 1..].to_string();

                            let events = parse_sse_line(&line, &mut accumulator);
                            if !events.is_empty() {
                                return Some((events, (byte_stream, accumulator, buffer)));
                            }
                        }

                        match byte_stream.next().await {
                            Some(Ok(bytes)) => {
                                buffer.push_str(&String::from_utf8_lossy(&bytes));
                            }
                            Some(Err(_)) | None => {
                                if !buffer.trim().is_empty() {
                                    let events = parse_sse_line(&buffer, &mut accumulator);
                                    buffer.clear();
                                    if !events.is_empty() {
                                        return Some((events, (byte_stream, accumulator, buffer)));
                                    }
                                }
                                return None;
                            }
                        }
                    }
                },
            );

            let flat_stream = futures_util::stream::StreamExt::flat_map(event_stream, |events| {
                futures_util::stream::iter(events)
            });

            Ok(Box::pin(flat_stream) as EventStream)
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ucode_core::Message;

    // ── provider metadata ─────────────────────────────────────────────────────

    #[test]
    fn test_provider_name() {
        let provider = OllamaProvider::new();
        assert_eq!(provider.name(), "ollama");
    }

    #[test]
    fn test_provider_capabilities() {
        let caps = OllamaProvider::new().capabilities();
        assert!(caps.tool_calls);
        assert!(!caps.json_mode);
        assert_eq!(caps.max_context, 128_000);
        assert_eq!(caps.max_output, 4_096);
        assert!(caps.streaming);
        assert!(!caps.token_counting);
    }

    #[test]
    fn test_default_base_url() {
        let provider = OllamaProvider::new();
        assert_eq!(provider.base_url, "http://localhost:11434");
    }

    #[test]
    fn test_custom_base_url() {
        let provider = OllamaProvider::new().with_base_url("http://192.168.1.10:11434".into());
        assert_eq!(provider.base_url, "http://192.168.1.10:11434");
    }

    #[test]
    fn test_count_tokens_returns_none() {
        let provider = OllamaProvider::new();
        let messages = vec![Message::user("hello")];
        assert!(provider.count_tokens(&messages).is_none());
    }

    // ── message conversion ────────────────────────────────────────────────────

    #[test]
    fn test_message_conversion_user() {
        let messages = vec![Message::user("Hello, world!")];
        let converted = to_ollama_messages(&messages);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].role, "user");
        assert_eq!(converted[0].content, "Hello, world!");
    }

    #[test]
    fn test_message_conversion_system() {
        let messages = vec![Message::system("You are a helpful assistant.")];
        let converted = to_ollama_messages(&messages);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].role, "system");
        assert_eq!(converted[0].content, "You are a helpful assistant.");
    }

    #[test]
    fn test_message_conversion_assistant() {
        let messages = vec![Message::assistant("I can help with that.")];
        let converted = to_ollama_messages(&messages);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].role, "assistant");
        assert_eq!(converted[0].content, "I can help with that.");
    }

    #[test]
    fn test_message_conversion_mixed() {
        let messages = vec![
            Message::system("Be concise."),
            Message::user("What is 2+2?"),
            Message::assistant("4"),
            Message::user("Thanks!"),
        ];
        let converted = to_ollama_messages(&messages);
        assert_eq!(converted.len(), 4);
        assert_eq!(converted[0].role, "system");
        assert_eq!(converted[0].content, "Be concise.");
        assert_eq!(converted[1].role, "user");
        assert_eq!(converted[1].content, "What is 2+2?");
        assert_eq!(converted[2].role, "assistant");
        assert_eq!(converted[2].content, "4");
        assert_eq!(converted[3].role, "user");
        assert_eq!(converted[3].content, "Thanks!");
    }

    // ── request body serialization ────────────────────────────────────────────

    #[test]
    fn test_request_body_serialization() {
        let body = OllamaRequest {
            model: "llama3.2".into(),
            messages: vec![OllamaMessage {
                role: "user".into(),
                content: "Hello".into(),
            }],
            stream: true,
            temperature: None,
            tools: vec![],
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["model"], "llama3.2");
        assert_eq!(json["stream"], true);
        assert_eq!(json["messages"][0]["role"], "user");
        assert_eq!(json["messages"][0]["content"], "Hello");
        // Empty tools should be omitted.
        assert!(json.get("tools").is_none());
    }

    #[test]
    fn test_request_body_with_tools() {
        let tools = vec![ToolDef {
            name: "get_weather".into(),
            description: "Get current weather".into(),
            parameters: serde_json::json!({"type": "object", "properties": {"city": {"type": "string"}}}),
        }];
        let body = OllamaRequest {
            model: "llama3.2".into(),
            messages: vec![],
            stream: true,
            temperature: Some(0.5),
            tools: to_ollama_tools(&tools),
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["temperature"], 0.5);
        assert!(json.get("tools").is_some());
        assert_eq!(json["tools"][0]["type"], "function");
        assert_eq!(json["tools"][0]["function"]["name"], "get_weather");
        assert_eq!(
            json["tools"][0]["function"]["description"],
            "Get current weather"
        );
    }

    #[test]
    fn test_request_body_without_optional_fields() {
        let body = OllamaRequest {
            model: "mistral".into(),
            messages: vec![OllamaMessage {
                role: "user".into(),
                content: "Hi".into(),
            }],
            stream: true,
            temperature: None,
            tools: vec![],
        };
        let json = serde_json::to_value(&body).unwrap();
        // Optional fields absent when None/empty.
        assert!(json.get("temperature").is_none());
        assert!(json.get("tools").is_none());
        // Required fields present.
        assert_eq!(json["model"], "mistral");
        assert_eq!(json["stream"], true);
    }
}
