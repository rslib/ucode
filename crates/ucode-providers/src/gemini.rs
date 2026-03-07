use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use ucode_auth::CredentialStore;
use ucode_core::{CoreError, Event, EventStream, ToolCall as CoreToolCall};

use crate::config::ProviderConfig;
use crate::provider::{Capabilities, ChatRequest, Provider, ProviderFuture, ToolDef};

// ── Request types ─────────────────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiRequest {
    contents: Vec<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<GenerationConfig>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<GeminiToolConfig>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct GeminiContent {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    parts: Vec<GeminiPart>,
}

/// A part in a Gemini content block.
/// Uses flat struct with optional fields (Gemini's JSON uses this pattern).
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
struct GeminiPart {
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    function_call: Option<GeminiFunctionCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    function_response: Option<GeminiFunctionResponse>,
    /// When true, this part contains thinking/reasoning content.
    #[serde(skip_serializing_if = "Option::is_none")]
    thought: Option<bool>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct GeminiFunctionCall {
    name: String,
    args: serde_json::Value,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct GeminiFunctionResponse {
    name: String,
    response: serde_json::Value,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<usize>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiToolConfig {
    function_declarations: Vec<GeminiFunctionDecl>,
}

#[derive(Serialize)]
struct GeminiFunctionDecl {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

// ── Response types ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiStreamResponse {
    #[serde(default)]
    candidates: Vec<GeminiCandidate>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiCandidate {
    #[serde(default)]
    content: Option<GeminiContent>,
    #[serde(default)]
    finish_reason: Option<String>,
}

// ── SSE line parser ───────────────────────────────────────────────────────────

/// Parse a Gemini SSE `data:` line into events.
pub fn parse_gemini_sse_line(line: &str, _acc: &mut ()) -> Vec<Event> {
    let line = line.trim();

    let data = if let Some(d) = line.strip_prefix("data: ") {
        d.trim()
    } else if let Some(d) = line.strip_prefix("data:") {
        d.trim()
    } else {
        return vec![];
    };

    if data == "[DONE]" {
        return vec![Event::Done];
    }

    let resp: GeminiStreamResponse = match serde_json::from_str(data) {
        Ok(r) => r,
        Err(_) => return vec![],
    };

    let mut events = Vec::new();

    for candidate in &resp.candidates {
        if let Some(ref content) = candidate.content {
            for part in &content.parts {
                // Skip thinking content.
                if part.thought == Some(true) {
                    continue;
                }
                if let Some(ref text) = part.text
                    && !text.is_empty()
                {
                    events.push(Event::Token(text.clone()));
                }
                if let Some(ref fc) = part.function_call {
                    events.push(Event::ToolCall(CoreToolCall::new(
                        format!("gemini_fc_{}", fc.name),
                        fc.name.clone(),
                        fc.args.clone(),
                    )));
                }
            }
        }

        if candidate.finish_reason.as_deref() == Some("STOP")
            || candidate.finish_reason.as_deref() == Some("MAX_TOKENS")
        {
            events.push(Event::Done);
        }
    }

    events
}

// ── Message conversion ────────────────────────────────────────────────────────

fn to_gemini_contents(
    messages: &[ucode_core::Message],
) -> (Option<GeminiContent>, Vec<GeminiContent>) {
    let mut system: Option<GeminiContent> = None;
    let mut contents = Vec::new();

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
                system = Some(GeminiContent {
                    role: None,
                    parts: vec![GeminiPart {
                        text: Some(text),
                        ..Default::default()
                    }],
                });
            }
            ucode_core::Role::User => {
                contents.push(GeminiContent {
                    role: Some("user".into()),
                    parts: vec![GeminiPart {
                        text: Some(text),
                        ..Default::default()
                    }],
                });
            }
            ucode_core::Role::Assistant => {
                contents.push(GeminiContent {
                    role: Some("model".into()),
                    parts: vec![GeminiPart {
                        text: Some(text),
                        ..Default::default()
                    }],
                });
            }
            // Tool results not yet mapped.
            ucode_core::Role::Tool => {}
        }
    }

    (system, contents)
}

fn to_gemini_tools(tools: &[ToolDef]) -> Vec<GeminiToolConfig> {
    if tools.is_empty() {
        return vec![];
    }
    vec![GeminiToolConfig {
        function_declarations: tools
            .iter()
            .map(|t| GeminiFunctionDecl {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: t.parameters.clone(),
            })
            .collect(),
    }]
}

// ── Provider ──────────────────────────────────────────────────────────────────

pub struct GeminiProvider {
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

impl GeminiProvider {
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
            provider_name: "gemini".into(),
            api_key: Some(api_key),
            base_url: "https://generativelanguage.googleapis.com".into(),
            headers: HashMap::new(),
            credential_store: None,
            api_key_env: None,
        }
    }
}

impl Provider for GeminiProvider {
    fn name(&self) -> &str {
        &self.provider_name
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            tool_calls: true,
            json_mode: true,
            max_context: 1_000_000,
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
        let custom_headers = self.headers.clone();

        // URL: /v1beta/models/{model}:streamGenerateContent?alt=sse
        // API key query param is appended inside the async block after resolution.
        let base_url = format!(
            "{}/v1beta/models/{}:streamGenerateContent?alt=sse",
            self.base_url, req.model
        );

        let (system_instruction, contents) = to_gemini_contents(&req.messages);
        let body = GeminiRequest {
            contents,
            system_instruction,
            generation_config: if req.temperature.is_some() || req.max_tokens.is_some() {
                Some(GenerationConfig {
                    temperature: req.temperature,
                    max_output_tokens: req.max_tokens,
                })
            } else {
                None
            },
            tools: to_gemini_tools(&req.tools),
        };

        Box::pin(async move {
            let api_key = crate::auth::resolve_provider_auth(
                &provider_name,
                api_key_env.as_deref(),
                credential_store.as_ref().map(|s| s.as_ref()),
                fallback_api_key.as_deref(),
            )?;

            let mut url = base_url;
            if let Some(ref key) = api_key {
                url.push_str(&format!("&key={key}"));
            }

            let mut request = client.post(&url).header("Content-Type", "application/json");

            if let Some(ref key) = api_key {
                request = request.header("x-goog-api-key", key);
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

            Ok(crate::sse::stream_lines(
                resp.bytes_stream(),
                (),
                parse_gemini_sse_line,
            ))
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AdapterKind, ProviderConfig};
    use ucode_core::{Event, Message};

    // ── provider metadata ─────────────────────────────────────────────────────

    #[test]
    fn provider_name_default() {
        let p = GeminiProvider::new("key".into());
        assert_eq!(p.name(), "gemini");
    }

    #[test]
    fn provider_name_from_config() {
        let config = ProviderConfig {
            adapter: AdapterKind::Gemini,
            base_url: None,
            api_key_env: None,
            headers: HashMap::new(),
        };
        let p = GeminiProvider::from_config("my-gemini", &config, Some("key".into()), None);
        assert_eq!(p.name(), "my-gemini");
    }

    #[test]
    fn capabilities() {
        let caps = GeminiProvider::new("key".into()).capabilities();
        assert!(caps.tool_calls);
        assert!(caps.json_mode);
        assert!(caps.streaming);
        assert!(!caps.token_counting);
        assert_eq!(caps.max_context, 1_000_000);
        assert_eq!(caps.max_output, 8_192);
    }

    // ── SSE line parser ───────────────────────────────────────────────────────

    #[test]
    fn text_token_emitted() {
        let line =
            r#"data: {"candidates":[{"content":{"role":"model","parts":[{"text":"Hello"}]}}]}"#;
        let events = parse_gemini_sse_line(line, &mut ());
        assert_eq!(events, vec![Event::Token("Hello".into())]);
    }

    #[test]
    fn empty_text_skipped() {
        let line = r#"data: {"candidates":[{"content":{"role":"model","parts":[{"text":""}]}}]}"#;
        let events = parse_gemini_sse_line(line, &mut ());
        assert!(events.is_empty());
    }

    #[test]
    fn stop_finish_reason_emits_done() {
        let line = r#"data: {"candidates":[{"content":{"role":"model","parts":[{"text":"end"}]},"finishReason":"STOP"}]}"#;
        let events = parse_gemini_sse_line(line, &mut ());
        assert_eq!(events, vec![Event::Token("end".into()), Event::Done]);
    }

    #[test]
    fn max_tokens_finish_reason_emits_done() {
        let line = r#"data: {"candidates":[{"finishReason":"MAX_TOKENS"}]}"#;
        let events = parse_gemini_sse_line(line, &mut ());
        assert_eq!(events, vec![Event::Done]);
    }

    #[test]
    fn function_call_emitted() {
        let line = r#"data: {"candidates":[{"content":{"role":"model","parts":[{"functionCall":{"name":"get_weather","args":{"city":"London"}}}]}}]}"#;
        let events = parse_gemini_sse_line(line, &mut ());
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::ToolCall(tc) => {
                assert_eq!(tc.id, "gemini_fc_get_weather");
                assert_eq!(tc.name, "get_weather");
                assert_eq!(tc.args, serde_json::json!({"city": "London"}));
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    #[test]
    fn non_data_lines_ignored() {
        assert!(parse_gemini_sse_line("", &mut ()).is_empty());
        assert!(parse_gemini_sse_line(": keep-alive", &mut ()).is_empty());
        assert!(parse_gemini_sse_line("   ", &mut ()).is_empty());
        assert!(parse_gemini_sse_line("event: something", &mut ()).is_empty());
    }

    #[test]
    fn done_marker_emits_done() {
        let events = parse_gemini_sse_line("data: [DONE]", &mut ());
        assert_eq!(events, vec![Event::Done]);
    }

    #[test]
    fn thinking_content_filtered_out() {
        let line = r#"data: {"candidates":[{"content":{"role":"model","parts":[{"text":"thinking...","thought":true},{"text":"actual answer"}]}}]}"#;
        let events = parse_gemini_sse_line(line, &mut ());
        // Only the non-thought part should be emitted.
        assert_eq!(events, vec![Event::Token("actual answer".into())]);
    }

    #[test]
    fn unparseable_data_ignored() {
        let events = parse_gemini_sse_line("data: not-json", &mut ());
        assert!(events.is_empty());
    }

    // ── message conversion ────────────────────────────────────────────────────

    #[test]
    fn system_extracted_from_contents() {
        let messages = vec![Message::system("You are helpful."), Message::user("Hello")];
        let (system, contents) = to_gemini_contents(&messages);
        assert!(system.is_some());
        let sys = system.unwrap();
        assert!(sys.role.is_none());
        assert_eq!(sys.parts[0].text.as_deref(), Some("You are helpful."));
        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0].role.as_deref(), Some("user"));
    }

    #[test]
    fn assistant_mapped_to_model_role() {
        let messages = vec![Message::user("Hi"), Message::assistant("Hello back")];
        let (system, contents) = to_gemini_contents(&messages);
        assert!(system.is_none());
        assert_eq!(contents.len(), 2);
        assert_eq!(contents[0].role.as_deref(), Some("user"));
        assert_eq!(contents[1].role.as_deref(), Some("model"));
        assert_eq!(contents[1].parts[0].text.as_deref(), Some("Hello back"));
    }

    #[test]
    fn no_system_gives_none() {
        let messages = vec![Message::user("Hi")];
        let (system, contents) = to_gemini_contents(&messages);
        assert!(system.is_none());
        assert_eq!(contents.len(), 1);
    }

    // ── tool config ───────────────────────────────────────────────────────────

    #[test]
    fn empty_tools_gives_empty_vec() {
        let result = to_gemini_tools(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn tools_wrapped_in_function_declarations() {
        let tools = vec![
            ToolDef {
                name: "search".into(),
                description: "Search the web".into(),
                parameters: serde_json::json!({"type": "object", "properties": {}}),
            },
            ToolDef {
                name: "calc".into(),
                description: "A calculator".into(),
                parameters: serde_json::json!({"type": "object", "properties": {}}),
            },
        ];
        let result = to_gemini_tools(&tools);
        // All tools go into a single GeminiToolConfig with functionDeclarations.
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].function_declarations.len(), 2);
        assert_eq!(result[0].function_declarations[0].name, "search");
        assert_eq!(result[0].function_declarations[1].name, "calc");
    }

    // ── request serialization ─────────────────────────────────────────────────

    #[test]
    fn request_serializes_contents() {
        let body = GeminiRequest {
            contents: vec![GeminiContent {
                role: Some("user".into()),
                parts: vec![GeminiPart {
                    text: Some("Hello".into()),
                    ..Default::default()
                }],
            }],
            system_instruction: None,
            generation_config: None,
            tools: vec![],
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["contents"][0]["role"], "user");
        assert_eq!(json["contents"][0]["parts"][0]["text"], "Hello");
        // Optional fields omitted when None/empty.
        assert!(json.get("systemInstruction").is_none());
        assert!(json.get("generationConfig").is_none());
        assert!(json.get("tools").is_none());
    }

    #[test]
    fn request_serializes_generation_config() {
        let body = GeminiRequest {
            contents: vec![],
            system_instruction: None,
            generation_config: Some(GenerationConfig {
                temperature: Some(0.7),
                max_output_tokens: Some(1024),
            }),
            tools: vec![],
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["generationConfig"]["temperature"], 0.7);
        assert_eq!(json["generationConfig"]["maxOutputTokens"], 1024);
    }

    #[test]
    fn request_serializes_system_instruction() {
        let body = GeminiRequest {
            contents: vec![],
            system_instruction: Some(GeminiContent {
                role: None,
                parts: vec![GeminiPart {
                    text: Some("Be helpful.".into()),
                    ..Default::default()
                }],
            }),
            generation_config: None,
            tools: vec![],
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["systemInstruction"]["parts"][0]["text"], "Be helpful.");
        // role is None so it should be omitted.
        assert!(json["systemInstruction"].get("role").is_none());
    }

    #[test]
    fn from_config_uses_base_url() {
        let config = ProviderConfig {
            adapter: AdapterKind::Gemini,
            base_url: Some("https://custom.example.com".into()),
            api_key_env: None,
            headers: HashMap::new(),
        };
        let p = GeminiProvider::from_config("gemini", &config, Some("key".into()), None);
        assert_eq!(p.base_url, "https://custom.example.com");
    }

    #[test]
    fn new_defaults_to_google_endpoint() {
        let p = GeminiProvider::new("test-key".into());
        assert_eq!(p.base_url, "https://generativelanguage.googleapis.com");
        assert_eq!(p.api_key, Some("test-key".into()));
    }
}
