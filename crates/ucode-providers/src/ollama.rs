use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use ucode_auth::CredentialStore;
use ucode_core::{CoreError, Event, EventStream, ToolCall as CoreToolCall};

use crate::config::ProviderConfig;
use crate::provider::{Capabilities, ChatRequest, Provider, ProviderFuture, ToolDef};
use crate::sse::stream_lines;

// ── Request types ─────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct OllamaNativeRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<OllamaOptions>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<OllamaTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    think: Option<bool>,
}

#[derive(Serialize)]
struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    num_ctx: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_k: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    min_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<i64>,
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

// ── Response types (NDJSON) ───────────────────────────────────────────────────

// Performance stats fields are deserialized for completeness but not yet surfaced upstream.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct OllamaChatResponse {
    #[serde(default)]
    message: Option<OllamaResponseMessage>,
    #[serde(default)]
    done: bool,
    #[serde(default)]
    done_reason: Option<String>,
    #[serde(default)]
    total_duration: Option<u64>,
    #[serde(default)]
    load_duration: Option<u64>,
    #[serde(default)]
    prompt_eval_count: Option<u64>,
    #[serde(default)]
    eval_count: Option<u64>,
    #[serde(default)]
    eval_duration: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct OllamaResponseMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    thinking: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OllamaResponseToolCall>>,
}

#[derive(Debug, Deserialize)]
struct OllamaResponseToolCall {
    function: OllamaResponseFunction,
}

#[derive(Debug, Deserialize)]
struct OllamaResponseFunction {
    name: String,
    arguments: serde_json::Value,
}

// ── Conversion helpers ────────────────────────────────────────────────────────

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

// ── NDJSON line parser ────────────────────────────────────────────────────────

/// Parse a single NDJSON line from Ollama's native `/api/chat` response.
pub fn parse_ollama_line(line: &str, _acc: &mut ()) -> Vec<Event> {
    let line = line.trim();
    if line.is_empty() {
        return vec![];
    }

    let resp: OllamaChatResponse = match serde_json::from_str(line) {
        Ok(r) => r,
        Err(_) => return vec![],
    };

    let mut events = Vec::new();

    if let Some(ref msg) = resp.message {
        if let Some(ref content) = msg.content
            && !content.is_empty()
        {
            events.push(Event::Token(content.clone()));
        }

        if let Some(ref tool_calls) = msg.tool_calls {
            for (i, tc) in tool_calls.iter().enumerate() {
                events.push(Event::ToolCall(CoreToolCall::new(
                    format!("ollama_tc_{i}"),
                    tc.function.name.clone(),
                    tc.function.arguments.clone(),
                )));
            }
        }
    }

    if resp.done {
        events.push(Event::Done);
    }

    events
}

// ── Provider ──────────────────────────────────────────────────────────────────

/// Ollama local inference provider using the native `/api/chat` endpoint with NDJSON streaming.
pub struct OllamaProvider {
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

impl OllamaProvider {
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

    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            provider_name: "ollama".into(),
            api_key: None,
            base_url: "http://localhost:11434".into(),
            headers: HashMap::new(),
            credential_store: None,
            api_key_env: None,
        }
    }

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
        &self.provider_name
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
        let credential_store = self.credential_store.clone();
        let api_key_env = self.api_key_env.clone();
        let fallback_api_key = self.api_key.clone();
        let url = format!("{}/api/chat", self.base_url);
        let provider_name = self.provider_name.clone();
        let extra_headers = self.headers.clone();

        let options = req.temperature.map(|temperature| OllamaOptions {
            temperature: Some(temperature),
            num_ctx: None,
            top_k: None,
            top_p: None,
            min_p: None,
            seed: None,
        });

        let body = OllamaNativeRequest {
            model: req.model,
            messages: to_ollama_messages(&req.messages),
            stream: true,
            options,
            tools: to_ollama_tools(&req.tools),
            think: None,
        };

        Box::pin(async move {
            let api_key = crate::auth::resolve_provider_auth(
                &provider_name,
                api_key_env.as_deref(),
                credential_store.as_ref().map(|s| s.as_ref()),
                fallback_api_key.as_deref(),
            )?;

            let mut request = client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&body);

            if let Some(ref key) = api_key {
                request = request.header("Authorization", format!("Bearer {key}"));
            }

            for (key, value) in &extra_headers {
                request = request.header(key.as_str(), value.as_str());
            }

            let resp = request.send().await.map_err(|e| CoreError::Provider {
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

            Ok(stream_lines(resp.bytes_stream(), (), parse_ollama_line))
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use ucode_core::{Event, Message};

    use crate::config::{AdapterKind, ProviderConfig};
    use crate::provider::ToolDef;

    // ── provider metadata ─────────────────────────────────────────────────────

    #[test]
    fn provider_name_default() {
        assert_eq!(OllamaProvider::new().name(), "ollama");
    }

    #[test]
    fn provider_name_from_config() {
        let config = ProviderConfig {
            adapter: AdapterKind::Ollama,
            base_url: None,
            api_key_env: None,
            headers: HashMap::new(),
        };
        let p = OllamaProvider::from_config("my-ollama", &config, None, None);
        assert_eq!(p.name(), "my-ollama");
    }

    #[test]
    fn default_base_url() {
        assert_eq!(OllamaProvider::new().base_url, "http://localhost:11434");
    }

    #[test]
    fn custom_base_url() {
        let p = OllamaProvider::new().with_base_url("http://192.168.1.10:11434".into());
        assert_eq!(p.base_url, "http://192.168.1.10:11434");
    }

    #[test]
    fn capabilities() {
        let caps = OllamaProvider::new().capabilities();
        assert!(caps.tool_calls);
        assert!(!caps.json_mode);
        assert_eq!(caps.max_context, 128_000);
        assert!(caps.streaming);
    }

    // ── NDJSON line parser ────────────────────────────────────────────────────

    #[test]
    fn parse_text_token() {
        let line = r#"{"message":{"role":"assistant","content":"Hello"},"done":false}"#;
        let events = parse_ollama_line(line, &mut ());
        assert_eq!(events, vec![Event::Token("Hello".into())]);
    }

    #[test]
    fn parse_empty_content_skipped() {
        let line = r#"{"message":{"role":"assistant","content":""},"done":false}"#;
        let events = parse_ollama_line(line, &mut ());
        assert!(events.is_empty());
    }

    #[test]
    fn parse_done_emits_done() {
        let line = r#"{"message":{"role":"assistant","content":""},"done":true,"done_reason":"stop","total_duration":1234567,"eval_count":42}"#;
        let events = parse_ollama_line(line, &mut ());
        assert_eq!(events, vec![Event::Done]);
    }

    #[test]
    fn parse_tool_call() {
        let line = r#"{"message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":"get_weather","arguments":{"city":"London"}}}]},"done":false}"#;
        let events = parse_ollama_line(line, &mut ());
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::ToolCall(tc) => {
                assert_eq!(tc.name, "get_weather");
                assert_eq!(tc.args, serde_json::json!({"city": "London"}));
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    #[test]
    fn parse_multiple_tool_calls() {
        let line = r#"{"message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":"a","arguments":{}}},{"function":{"name":"b","arguments":{}}}]},"done":false}"#;
        let events = parse_ollama_line(line, &mut ());
        assert_eq!(events.len(), 2);
        match (&events[0], &events[1]) {
            (Event::ToolCall(a), Event::ToolCall(b)) => {
                assert_eq!(a.id, "ollama_tc_0");
                assert_eq!(b.id, "ollama_tc_1");
            }
            _ => panic!("expected two ToolCalls"),
        }
    }

    #[test]
    fn parse_empty_line_ignored() {
        assert!(parse_ollama_line("", &mut ()).is_empty());
        assert!(parse_ollama_line("   ", &mut ()).is_empty());
    }

    #[test]
    fn parse_invalid_json_ignored() {
        assert!(parse_ollama_line("not json", &mut ()).is_empty());
    }

    #[test]
    fn parse_content_with_done() {
        // Final chunk can have both content and done=true
        let line = r#"{"message":{"role":"assistant","content":"end"},"done":true}"#;
        let events = parse_ollama_line(line, &mut ());
        assert_eq!(events, vec![Event::Token("end".into()), Event::Done]);
    }

    // ── message conversion ────────────────────────────────────────────────────

    #[test]
    fn message_conversion() {
        let messages = vec![
            Message::system("Be helpful."),
            Message::user("Hello"),
            Message::assistant("Hi"),
        ];
        let converted = to_ollama_messages(&messages);
        assert_eq!(converted.len(), 3);
        assert_eq!(converted[0].role, "system");
        assert_eq!(converted[1].role, "user");
        assert_eq!(converted[2].role, "assistant");
    }

    // ── request serialization ─────────────────────────────────────────────────

    #[test]
    fn native_request_minimal() {
        let body = OllamaNativeRequest {
            model: "llama3.2".into(),
            messages: vec![OllamaMessage {
                role: "user".into(),
                content: "Hello".into(),
            }],
            stream: true,
            options: None,
            tools: vec![],
            think: None,
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["model"], "llama3.2");
        assert_eq!(json["stream"], true);
        assert!(json.get("options").is_none());
        assert!(json.get("tools").is_none());
        assert!(json.get("think").is_none());
    }

    #[test]
    fn native_request_with_options() {
        let body = OllamaNativeRequest {
            model: "llama3.2".into(),
            messages: vec![],
            stream: true,
            options: Some(OllamaOptions {
                temperature: Some(0.7),
                num_ctx: Some(4096),
                top_k: None,
                top_p: None,
                min_p: None,
                seed: None,
            }),
            tools: vec![],
            think: Some(true),
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["options"]["temperature"], 0.7);
        assert_eq!(json["options"]["num_ctx"], 4096);
        assert!(json["options"].get("top_k").is_none());
        assert_eq!(json["think"], true);
    }

    #[test]
    fn native_request_with_tools() {
        let tools = vec![ToolDef {
            name: "calc".into(),
            description: "Calculator".into(),
            parameters: serde_json::json!({"type": "object"}),
        }];
        let body = OllamaNativeRequest {
            model: "llama3.2".into(),
            messages: vec![],
            stream: true,
            options: None,
            tools: to_ollama_tools(&tools),
            think: None,
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["tools"][0]["type"], "function");
        assert_eq!(json["tools"][0]["function"]["name"], "calc");
    }
}
