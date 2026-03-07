use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use ucode_auth::CredentialStore;
use ucode_core::{CoreError, Event, EventStream, ToolCall as CoreToolCall};

use crate::config::ProviderConfig;
use crate::provider::{Capabilities, ChatRequest, Provider, ProviderFuture, ToolDef};
use crate::sse::stream_lines;

// ── SSE response types ────────────────────────────────────────────────────────

/// Anthropic SSE event types.
#[derive(Debug, Clone, PartialEq)]
enum AnthropicEventType {
    MessageStart,
    ContentBlockStart,
    ContentBlockDelta,
    ContentBlockStop,
    MessageDelta,
    MessageStop,
    Unknown,
}

impl AnthropicEventType {
    fn from_str(s: &str) -> Self {
        match s {
            "message_start" => Self::MessageStart,
            "content_block_start" => Self::ContentBlockStart,
            "content_block_delta" => Self::ContentBlockDelta,
            "content_block_stop" => Self::ContentBlockStop,
            "message_delta" => Self::MessageDelta,
            "message_stop" => Self::MessageStop,
            _ => Self::Unknown,
        }
    }
}

/// Payload for `content_block_start` events.
#[derive(Debug, Deserialize)]
struct ContentBlockStart {
    index: usize,
    content_block: ContentBlock,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ContentBlock {
    Text,
    ToolUse { id: String, name: String },
}

/// Payload for `content_block_delta` events.
#[derive(Debug, Deserialize)]
struct ContentBlockDelta {
    index: usize,
    delta: Delta,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Delta {
    TextDelta { text: String },
    InputJsonDelta { partial_json: String },
}

// ── Tool call accumulator ─────────────────────────────────────────────────────

/// State for accumulating streamed Anthropic tool calls across multiple events.
#[derive(Debug, Default)]
pub struct AnthropicToolAccumulator {
    /// Pending tool calls indexed by content block index.
    calls: Vec<Option<PendingToolCall>>,
    /// The current SSE event type (set by `event:` lines, consumed by `data:` lines).
    current_event: Option<AnthropicEventType>,
}

#[derive(Debug, Default)]
struct PendingToolCall {
    id: String,
    name: String,
    partial_json: String,
}

impl AnthropicToolAccumulator {
    fn ensure_slot(&mut self, index: usize) {
        while self.calls.len() <= index {
            self.calls.push(None);
        }
    }

    fn start_tool_use(&mut self, index: usize, id: String, name: String) {
        self.ensure_slot(index);
        self.calls[index] = Some(PendingToolCall {
            id,
            name,
            partial_json: String::new(),
        });
    }

    fn append_json(&mut self, index: usize, fragment: &str) {
        if let Some(slot) = self.calls.get_mut(index).and_then(|s| s.as_mut()) {
            slot.partial_json.push_str(fragment);
        }
    }

    fn finish_block(&mut self, index: usize) -> Option<Event> {
        let slot = self.calls.get_mut(index)?.take()?;
        let args = serde_json::from_str(&slot.partial_json)
            .unwrap_or(serde_json::Value::String(slot.partial_json));
        Some(Event::ToolCall(CoreToolCall::new(slot.id, slot.name, args)))
    }
}

// ── SSE line parser ───────────────────────────────────────────────────────────

/// Parse a single SSE line into zero or more events.
///
/// Anthropic SSE uses a two-line format per event:
/// ```text
/// event: <type>
/// data: <json>
/// ```
/// The accumulator carries the current event type across calls so the `data:`
/// line can be interpreted correctly.
pub fn parse_anthropic_sse_line(
    line: &str,
    accumulator: &mut AnthropicToolAccumulator,
) -> Vec<Event> {
    let line = line.trim();

    // `event:` line — record type for the next `data:` line.
    if let Some(event_type) = line
        .strip_prefix("event: ")
        .or_else(|| line.strip_prefix("event:"))
    {
        accumulator.current_event = Some(AnthropicEventType::from_str(event_type.trim()));
        return vec![];
    }

    let data = if let Some(d) = line.strip_prefix("data: ") {
        d.trim()
    } else if let Some(d) = line.strip_prefix("data:") {
        d.trim()
    } else {
        return vec![];
    };

    let event_type = match accumulator.current_event.take() {
        Some(t) => t,
        None => return vec![],
    };

    match event_type {
        AnthropicEventType::ContentBlockStart => {
            let Ok(payload) = serde_json::from_str::<ContentBlockStart>(data) else {
                return vec![];
            };
            if let ContentBlock::ToolUse { id, name } = payload.content_block {
                accumulator.start_tool_use(payload.index, id, name);
            }
            vec![]
        }

        AnthropicEventType::ContentBlockDelta => {
            let Ok(payload) = serde_json::from_str::<ContentBlockDelta>(data) else {
                return vec![];
            };
            match payload.delta {
                Delta::TextDelta { text } if !text.is_empty() => vec![Event::Token(text)],
                Delta::InputJsonDelta { partial_json } => {
                    accumulator.append_json(payload.index, &partial_json);
                    vec![]
                }
                _ => vec![],
            }
        }

        AnthropicEventType::ContentBlockStop => {
            // `data:` payload is `{"type":"content_block_stop","index":<n>}`.
            #[derive(Deserialize)]
            struct StopPayload {
                index: usize,
            }
            let Ok(payload) = serde_json::from_str::<StopPayload>(data) else {
                return vec![];
            };
            accumulator
                .finish_block(payload.index)
                .map(|e| vec![e])
                .unwrap_or_default()
        }

        AnthropicEventType::MessageStop => vec![Event::Done],

        // message_start, message_delta, unknown — no events to emit.
        _ => vec![],
    }
}

// ── Request body types ────────────────────────────────────────────────────────

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: usize,
    messages: Vec<AnthropicMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<AnthropicTool>,
}

#[derive(Serialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
}

/// Extract the system prompt (if any) and return non-system messages.
fn split_messages(messages: &[ucode_core::Message]) -> (Option<String>, Vec<AnthropicMessage>) {
    let mut system: Option<String> = None;
    let mut out = Vec::with_capacity(messages.len());

    for m in messages {
        let text: String = m
            .parts
            .iter()
            .filter_map(|p| match p {
                ucode_core::Part::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");

        match m.role {
            ucode_core::Role::System => {
                // Concatenate multiple system messages with a newline.
                match system.as_mut() {
                    Some(s) => {
                        s.push('\n');
                        s.push_str(&text);
                    }
                    None => system = Some(text),
                }
            }
            ucode_core::Role::User => out.push(AnthropicMessage {
                role: "user".into(),
                content: text,
            }),
            ucode_core::Role::Assistant => out.push(AnthropicMessage {
                role: "assistant".into(),
                content: text,
            }),
            // Tool results are not yet mapped to Anthropic's tool_result format;
            // skip them to avoid sending malformed requests.
            ucode_core::Role::Tool => {}
        }
    }

    (system, out)
}

fn to_anthropic_tools(tools: &[ToolDef]) -> Vec<AnthropicTool> {
    tools
        .iter()
        .map(|t| AnthropicTool {
            name: t.name.clone(),
            description: t.description.clone(),
            input_schema: t.parameters.clone(),
        })
        .collect()
}

// ── Provider ──────────────────────────────────────────────────────────────────

/// Anthropic Messages API provider.
///
/// Works with Anthropic and any Anthropic-compatible API endpoint.
pub struct AnthropicCompatProvider {
    client: reqwest::Client,
    provider_name: String,
    api_key: Option<String>,
    base_url: String,
    headers: HashMap<String, String>,
    /// Optional credential store for dynamic auth resolution.
    credential_store: Option<Arc<dyn CredentialStore>>,
    /// Environment variable name for API key lookup.
    api_key_env: Option<String>,
}

impl AnthropicCompatProvider {
    pub fn from_config(
        name: &str,
        config: &ProviderConfig,
        api_key: Option<String>,
        credential_store: Option<Arc<dyn CredentialStore>>,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            provider_name: name.to_owned(),
            api_key,
            base_url: config.base_url().to_owned(),
            headers: config.headers.clone(),
            credential_store,
            api_key_env: config.api_key_env.clone(),
        }
    }

    pub fn new(api_key: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            provider_name: "anthropic".into(),
            api_key: Some(api_key),
            base_url: "https://api.anthropic.com/v1".into(),
            headers: HashMap::new(),
            credential_store: None,
            api_key_env: None,
        }
    }

    /// Override the base URL (for testing or proxies).
    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url;
        self
    }
}

impl Provider for AnthropicCompatProvider {
    fn name(&self) -> &str {
        &self.provider_name
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            tool_calls: true,
            json_mode: false,
            max_context: 200_000,
            max_output: 8_192,
            streaming: true,
            token_counting: false,
        }
    }

    fn stream_chat(&self, req: ChatRequest) -> ProviderFuture<Result<EventStream, CoreError>> {
        let client = self.client.clone();
        let credential_store = self.credential_store.clone();
        let api_key_env = self.api_key_env.clone();
        let fallback_api_key = self.api_key.clone();
        let provider_name = self.provider_name.clone();
        let extra_headers = self.headers.clone();
        let url = format!("{}/messages", self.base_url);

        let (system, messages) = split_messages(&req.messages);
        let body = AnthropicRequest {
            model: req.model,
            max_tokens: req.max_tokens.unwrap_or(4096),
            messages,
            stream: true,
            system,
            temperature: req.temperature,
            tools: to_anthropic_tools(&req.tools),
        };

        Box::pin(async move {
            let api_key = crate::auth::resolve_provider_auth(
                &provider_name,
                api_key_env.as_deref(),
                credential_store.as_ref().map(|s| s.as_ref()),
                fallback_api_key.as_deref(),
            )?;

            let mut builder = client.post(&url);

            if let Some(ref key) = api_key {
                builder = builder.header("x-api-key", key);
            }
            builder = builder
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json");
            for (k, v) in &extra_headers {
                builder = builder.header(k.as_str(), v.as_str());
            }

            let resp = builder
                .json(&body)
                .send()
                .await
                .map_err(|e| CoreError::Provider {
                    provider: provider_name.clone(),
                    message: format!("HTTP request failed: {e}"),
                })?;

            let status = resp.status();
            if !status.is_success() {
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

            let byte_stream = resp.bytes_stream();

            Ok(stream_lines(
                byte_stream,
                AnthropicToolAccumulator::default(),
                parse_anthropic_sse_line,
            ))
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AdapterKind;
    use ucode_core::{Event, Message};

    fn feed(lines: &[&str], acc: &mut AnthropicToolAccumulator) -> Vec<Event> {
        lines
            .iter()
            .flat_map(|l| parse_anthropic_sse_line(l, acc))
            .collect()
    }

    // ── text streaming ────────────────────────────────────────────────────────

    #[test]
    fn text_delta_emits_token() {
        let mut acc = AnthropicToolAccumulator::default();
        let events = feed(
            &[
                "event: content_block_delta",
                r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#,
            ],
            &mut acc,
        );
        assert_eq!(events, vec![Event::Token("Hello".into())]);
    }

    #[test]
    fn empty_text_delta_skipped() {
        let mut acc = AnthropicToolAccumulator::default();
        let events = feed(
            &[
                "event: content_block_delta",
                r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":""}}"#,
            ],
            &mut acc,
        );
        assert!(events.is_empty());
    }

    #[test]
    fn message_stop_emits_done() {
        let mut acc = AnthropicToolAccumulator::default();
        let events = feed(
            &["event: message_stop", r#"data: {"type":"message_stop"}"#],
            &mut acc,
        );
        assert_eq!(events, vec![Event::Done]);
    }

    #[test]
    fn multiple_text_tokens_in_sequence() {
        let mut acc = AnthropicToolAccumulator::default();
        let events = feed(
            &[
                "event: content_block_delta",
                r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#,
                "event: content_block_delta",
                r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":" world"}}"#,
                "event: message_stop",
                r#"data: {"type":"message_stop"}"#,
            ],
            &mut acc,
        );
        assert_eq!(
            events,
            vec![
                Event::Token("Hello".into()),
                Event::Token(" world".into()),
                Event::Done,
            ]
        );
    }

    #[test]
    fn non_event_lines_ignored() {
        let mut acc = AnthropicToolAccumulator::default();
        assert!(parse_anthropic_sse_line("", &mut acc).is_empty());
        assert!(parse_anthropic_sse_line(": keep-alive", &mut acc).is_empty());
        assert!(parse_anthropic_sse_line("   ", &mut acc).is_empty());
    }

    #[test]
    fn data_without_preceding_event_ignored() {
        let mut acc = AnthropicToolAccumulator::default();
        // No `event:` line before this `data:` line.
        let events = parse_anthropic_sse_line(
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"oops"}}"#,
            &mut acc,
        );
        assert!(events.is_empty());
    }

    // ── tool use ──────────────────────────────────────────────────────────────

    #[test]
    fn tool_use_full_sequence() {
        let mut acc = AnthropicToolAccumulator::default();
        let events = feed(
            &[
                // content_block_start: tool_use
                "event: content_block_start",
                r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_01","name":"get_weather"}}"#,
                // partial JSON fragments
                "event: content_block_delta",
                r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"city\":"}}"#,
                "event: content_block_delta",
                r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"\"London\"}"}}"#,
                // content_block_stop: emits ToolCall
                "event: content_block_stop",
                r#"data: {"type":"content_block_stop","index":0}"#,
            ],
            &mut acc,
        );

        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::ToolCall(tc) => {
                assert_eq!(tc.id, "toolu_01");
                assert_eq!(tc.name, "get_weather");
                assert_eq!(tc.args, serde_json::json!({"city": "London"}));
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    #[test]
    fn tool_use_invalid_json_stored_as_string() {
        let mut acc = AnthropicToolAccumulator::default();
        feed(
            &[
                "event: content_block_start",
                r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_02","name":"broken"}}"#,
                "event: content_block_delta",
                r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"not-json"}}"#,
            ],
            &mut acc,
        );

        let events = feed(
            &[
                "event: content_block_stop",
                r#"data: {"type":"content_block_stop","index":0}"#,
            ],
            &mut acc,
        );

        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::ToolCall(tc) => {
                assert_eq!(tc.id, "toolu_02");
                assert_eq!(tc.args, serde_json::Value::String("not-json".into()));
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    #[test]
    fn content_block_stop_without_tool_use_emits_nothing() {
        let mut acc = AnthropicToolAccumulator::default();
        // A text block stop — no pending tool call at index 0.
        let events = feed(
            &[
                "event: content_block_stop",
                r#"data: {"type":"content_block_stop","index":0}"#,
            ],
            &mut acc,
        );
        assert!(events.is_empty());
    }

    // ── mixed text + tool use ─────────────────────────────────────────────────

    #[test]
    fn mixed_text_then_tool_use() {
        let mut acc = AnthropicToolAccumulator::default();
        let events = feed(
            &[
                // Text block (index 0)
                "event: content_block_delta",
                r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Sure, let me check."}}"#,
                "event: content_block_stop",
                r#"data: {"type":"content_block_stop","index":0}"#,
                // Tool use block (index 1)
                "event: content_block_start",
                r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_03","name":"search"}}"#,
                "event: content_block_delta",
                r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"q\":\"rust\"}"}}"#,
                "event: content_block_stop",
                r#"data: {"type":"content_block_stop","index":1}"#,
                "event: message_stop",
                r#"data: {"type":"message_stop"}"#,
            ],
            &mut acc,
        );

        assert_eq!(events.len(), 3);
        assert_eq!(events[0], Event::Token("Sure, let me check.".into()));
        match &events[1] {
            Event::ToolCall(tc) => {
                assert_eq!(tc.id, "toolu_03");
                assert_eq!(tc.name, "search");
                assert_eq!(tc.args, serde_json::json!({"q": "rust"}));
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
        assert_eq!(events[2], Event::Done);
    }

    // ── request body serialization ────────────────────────────────────────────

    #[test]
    fn system_message_extracted_to_top_level() {
        let messages = vec![Message::system("You are helpful."), Message::user("Hello")];
        let (system, msgs) = split_messages(&messages);
        assert_eq!(system.as_deref(), Some("You are helpful."));
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].content, "Hello");
    }

    #[test]
    fn multiple_system_messages_concatenated() {
        let messages = vec![
            Message::system("Part one."),
            Message::system("Part two."),
            Message::user("Go"),
        ];
        let (system, msgs) = split_messages(&messages);
        assert_eq!(system.as_deref(), Some("Part one.\nPart two."));
        assert_eq!(msgs.len(), 1);
    }

    #[test]
    fn no_system_message_gives_none() {
        let messages = vec![Message::user("Hi")];
        let (system, msgs) = split_messages(&messages);
        assert!(system.is_none());
        assert_eq!(msgs.len(), 1);
    }

    #[test]
    fn tool_def_serialized_with_input_schema() {
        let tools = vec![ToolDef {
            name: "calc".into(),
            description: "A calculator".into(),
            parameters: serde_json::json!({"type": "object", "properties": {}}),
        }];
        let anthropic_tools = to_anthropic_tools(&tools);
        let json = serde_json::to_value(&anthropic_tools).unwrap();
        // Must use `input_schema`, not `parameters`.
        assert!(json[0].get("input_schema").is_some());
        assert!(json[0].get("parameters").is_none());
        assert_eq!(json[0]["name"], "calc");
        assert_eq!(json[0]["description"], "A calculator");
    }

    #[test]
    fn request_body_serialization_roundtrip() {
        let body = AnthropicRequest {
            model: "claude-sonnet-4-20250514".into(),
            max_tokens: 4096,
            messages: vec![AnthropicMessage {
                role: "user".into(),
                content: "Hello".into(),
            }],
            stream: true,
            system: Some("Be helpful.".into()),
            temperature: Some(0.7),
            tools: vec![],
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["model"], "claude-sonnet-4-20250514");
        assert_eq!(json["max_tokens"], 4096);
        assert_eq!(json["stream"], true);
        assert_eq!(json["system"], "Be helpful.");
        assert_eq!(json["temperature"], 0.7);
        // Empty tools array should be omitted.
        assert!(json.get("tools").is_none());
    }

    #[test]
    fn request_body_no_system_omits_field() {
        let body = AnthropicRequest {
            model: "claude-haiku-3".into(),
            max_tokens: 1024,
            messages: vec![],
            stream: true,
            system: None,
            temperature: None,
            tools: vec![],
        };
        let json = serde_json::to_value(&body).unwrap();
        assert!(json.get("system").is_none());
        assert!(json.get("temperature").is_none());
    }

    // ── provider metadata ─────────────────────────────────────────────────────

    #[test]
    fn provider_name_and_capabilities() {
        let provider = AnthropicCompatProvider::new("test-key".into());
        assert_eq!(provider.name(), "anthropic");

        let caps = provider.capabilities();
        assert!(caps.tool_calls);
        assert!(!caps.json_mode);
        assert!(caps.streaming);
        assert!(!caps.token_counting);
        assert_eq!(caps.max_context, 200_000);
        assert_eq!(caps.max_output, 8_192);
    }

    #[test]
    fn provider_with_base_url() {
        let provider = AnthropicCompatProvider::new("key".into())
            .with_base_url("http://localhost:8080".into());
        assert_eq!(provider.base_url, "http://localhost:8080");
    }

    #[test]
    fn from_config_uses_provider_name() {
        let config = ProviderConfig {
            adapter: AdapterKind::Anthropic,
            base_url: None,
            api_key_env: None,
            headers: HashMap::new(),
        };
        let provider =
            AnthropicCompatProvider::from_config("my-anthropic", &config, Some("key".into()), None);
        assert_eq!(provider.name(), "my-anthropic");
        assert_eq!(provider.base_url, "https://api.anthropic.com/v1");
    }

    #[test]
    fn from_config_with_custom_base_url() {
        let config = ProviderConfig {
            adapter: AdapterKind::Anthropic,
            base_url: Some("https://proxy.example.com/anthropic".into()),
            api_key_env: None,
            headers: HashMap::new(),
        };
        let provider =
            AnthropicCompatProvider::from_config("proxy", &config, Some("key".into()), None);
        assert_eq!(provider.base_url, "https://proxy.example.com/anthropic");
    }

    #[test]
    fn new_defaults_to_anthropic() {
        let provider = AnthropicCompatProvider::new("test-key".into());
        assert_eq!(provider.name(), "anthropic");
        assert_eq!(provider.base_url, "https://api.anthropic.com/v1");
        assert_eq!(provider.api_key, Some("test-key".into()));
    }

    #[test]
    fn no_api_key_allowed() {
        let config = ProviderConfig {
            adapter: AdapterKind::Anthropic,
            base_url: None,
            api_key_env: None,
            headers: HashMap::new(),
        };
        let provider = AnthropicCompatProvider::from_config("proxy", &config, None, None);
        assert!(provider.api_key.is_none());
    }

    // ── message_start / message_delta ignored ─────────────────────────────────

    #[test]
    fn message_start_emits_nothing() {
        let mut acc = AnthropicToolAccumulator::default();
        let events = feed(
            &[
                "event: message_start",
                r#"data: {"type":"message_start","message":{"id":"msg_01","type":"message","role":"assistant","content":[],"model":"claude-3-opus-20240229","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":25,"output_tokens":1}}}"#,
            ],
            &mut acc,
        );
        assert!(events.is_empty());
    }

    #[test]
    fn message_delta_emits_nothing() {
        let mut acc = AnthropicToolAccumulator::default();
        let events = feed(
            &[
                "event: message_delta",
                r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":15}}"#,
            ],
            &mut acc,
        );
        assert!(events.is_empty());
    }
}
