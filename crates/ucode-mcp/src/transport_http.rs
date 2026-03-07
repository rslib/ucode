//! Streamable HTTP transport for MCP (2025-03-26 spec).
//!
//! Single POST endpoint: client sends JSON-RPC as POST body, server responds
//! with either `application/json` (direct) or `text/event-stream` (SSE stream).
//! Session continuity via `Mcp-Session-Id` header.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;

use crate::error::McpError;
use crate::jsonrpc::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use crate::reconnect::{self, ReconnectConfig};
use crate::transport::Transport;

pub struct HttpTransport {
    url: String,
    client: reqwest::Client,
    headers: HeaderMap,
    next_id: u64,
    session_id: Option<String>,
    reconnect: ReconnectConfig,
}

/// Build a `HeaderMap` from a string→string map, always injecting
/// `Content-Type: application/json`.
pub fn build_header_map(custom: &HashMap<String, String>) -> Result<HeaderMap, McpError> {
    let mut map = HeaderMap::new();
    map.insert(
        reqwest::header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    for (k, v) in custom {
        let name = HeaderName::from_bytes(k.as_bytes())
            .map_err(|e| McpError::InvalidConfig(format!("invalid header name {k:?}: {e}")))?;
        let value = HeaderValue::from_str(v)
            .map_err(|e| McpError::InvalidConfig(format!("invalid header value for {k:?}: {e}")))?;
        map.insert(name, value);
    }
    Ok(map)
}

/// Extract the payload from a `data: …` SSE line.
///
/// Handles both `data: payload` (with space) and `data:payload` (no space).
/// Returns `None` for any line that does not start with `data:`.
pub fn extract_sse_data(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("data:")?;
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

impl HttpTransport {
    pub fn new(
        url: impl Into<String>,
        custom_headers: &HashMap<String, String>,
        reconnect: ReconnectConfig,
    ) -> Result<Self, McpError> {
        let headers = build_header_map(custom_headers)?;
        Ok(Self {
            url: url.into(),
            client: reqwest::Client::new(),
            headers,
            next_id: 1,
            session_id: None,
            reconnect,
        })
    }

    /// POST `body` to the endpoint with retry logic.
    ///
    /// - 2xx → return response
    /// - 5xx / 429 / connection error → retry per `ReconnectConfig`
    /// - 4xx (except 429) → fail immediately
    async fn post_request(&mut self, body: String) -> Result<reqwest::Response, McpError> {
        let mut attempt = 0usize;
        loop {
            let mut req = self
                .client
                .post(&self.url)
                .headers(self.headers.clone())
                .body(body.clone());

            if let Some(sid) = &self.session_id {
                req = req.header("Mcp-Session-Id", sid.as_str());
            }

            match req.send().await {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    if resp.status().is_success() {
                        // Capture session id if server sent one.
                        if let Some(sid) = resp.headers().get("Mcp-Session-Id")
                            && let Ok(s) = sid.to_str()
                        {
                            self.session_id = Some(s.to_owned());
                        }
                        return Ok(resp);
                    }
                    // Permanent 4xx (not 429) — fail immediately.
                    if reconnect::is_permanent_error(status) {
                        let body_text = resp.text().await.unwrap_or_default();
                        return Err(McpError::Http {
                            status,
                            body: body_text,
                        });
                    }
                    // Transient: 5xx or 429 — fall through to retry.
                    if !self.reconnect.should_retry(attempt) {
                        let body_text = resp.text().await.unwrap_or_default();
                        return Err(McpError::Http {
                            status,
                            body: body_text,
                        });
                    }
                    let backoff_ms = self.reconnect.backoff_ms(attempt);
                    tracing::warn!(
                        transport = "http",
                        url = %self.url,
                        attempt,
                        backoff_ms,
                        "transient HTTP {status}, retrying"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                }
                Err(e) => {
                    if !self.reconnect.should_retry(attempt) {
                        return Err(McpError::SseConnection(e.to_string()));
                    }
                    let backoff_ms = self.reconnect.backoff_ms(attempt);
                    tracing::warn!(
                        transport = "http",
                        url = %self.url,
                        attempt,
                        backoff_ms,
                        "connection error: {e}, retrying"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                }
            }
            attempt += 1;
        }
    }

    /// POST `body` and parse the response — either direct JSON or SSE stream.
    async fn send_and_parse(&mut self, body: String, expected_id: u64) -> Result<Value, McpError> {
        let resp = self.post_request(body).await?;
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned();

        if content_type.contains("text/event-stream") {
            self.parse_sse_response(resp, expected_id).await
        } else {
            // Direct JSON response.
            let text = resp
                .text()
                .await
                .map_err(|e| McpError::SseConnection(e.to_string()))?;
            let rpc: JsonRpcResponse = serde_json::from_str(&text)?;
            if let Some(err) = rpc.error {
                return Err(McpError::JsonRpc {
                    code: err.code,
                    message: err.message,
                });
            }
            rpc.result
                .ok_or_else(|| McpError::Protocol("response has neither result nor error".into()))
        }
    }

    async fn parse_sse_response(
        &self,
        resp: reqwest::Response,
        expected_id: u64,
    ) -> Result<Value, McpError> {
        let mut stream = resp.bytes_stream();
        let mut buf = String::new();

        while let Some(chunk) = stream.next().await {
            let bytes = chunk.map_err(|e| McpError::SseConnection(e.to_string()))?;
            buf.push_str(&String::from_utf8_lossy(&bytes));

            // Process all complete lines in the buffer.
            while let Some(pos) = buf.find('\n') {
                let line = buf[..pos].trim_end_matches('\r').to_owned();
                buf.drain(..=pos);

                if let Some(data) = extract_sse_data(&line) {
                    let rpc: JsonRpcResponse = serde_json::from_str(data)?;
                    if rpc.id == Some(expected_id) {
                        if let Some(err) = rpc.error {
                            return Err(McpError::JsonRpc {
                                code: err.code,
                                message: err.message,
                            });
                        }
                        return rpc.result.ok_or_else(|| {
                            McpError::Protocol("response has neither result nor error".into())
                        });
                    }
                }
            }
        }

        Err(McpError::Protocol(format!(
            "SSE stream ended without response for id={expected_id}"
        )))
    }

    async fn send_request(
        &mut self,
        method: &str,
        params: Option<Value>,
    ) -> Result<Value, McpError> {
        let id = self.next_id;
        self.next_id += 1;
        let req = JsonRpcRequest::new(id, method, params);
        let body = serde_json::to_string(&req)?;
        self.send_and_parse(body, id).await
    }

    async fn send_notify(&mut self, method: &str, params: Option<Value>) -> Result<(), McpError> {
        let notif = JsonRpcNotification::new(method, params);
        let body = serde_json::to_string(&notif)?;
        self.post_request(body).await?;
        Ok(())
    }
}

impl Transport for HttpTransport {
    fn request(
        &mut self,
        method: &str,
        params: Option<Value>,
    ) -> Pin<Box<dyn Future<Output = Result<Value, McpError>> + Send + '_>> {
        let method = method.to_owned();
        Box::pin(async move { self.send_request(&method, params).await })
    }

    fn notify(
        &mut self,
        method: &str,
        params: Option<Value>,
    ) -> Pin<Box<dyn Future<Output = Result<(), McpError>> + Send + '_>> {
        let method = method.to_owned();
        Box::pin(async move { self.send_notify(&method, params).await })
    }

    fn shutdown(&mut self) -> Pin<Box<dyn Future<Output = Result<(), McpError>> + Send + '_>> {
        Box::pin(async move {
            self.session_id = None;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_request_headers() {
        let mut custom = HashMap::new();
        custom.insert("Authorization".into(), "Bearer tok123".into());
        custom.insert("X-Custom".into(), "value".into());
        let header_map = build_header_map(&custom).unwrap();
        assert_eq!(
            header_map.get("Authorization").unwrap().to_str().unwrap(),
            "Bearer tok123"
        );
        assert_eq!(
            header_map.get("X-Custom").unwrap().to_str().unwrap(),
            "value"
        );
        assert_eq!(
            header_map.get("Content-Type").unwrap().to_str().unwrap(),
            "application/json"
        );
    }

    #[test]
    fn extract_sse_data_valid() {
        assert_eq!(extract_sse_data("data: {\"id\":1}"), Some("{\"id\":1}"));
        assert_eq!(extract_sse_data("data:{\"id\":1}"), Some("{\"id\":1}"));
    }

    #[test]
    fn extract_sse_data_invalid() {
        assert!(extract_sse_data("event: message").is_none());
        assert!(extract_sse_data(": comment").is_none());
        assert!(extract_sse_data("").is_none());
    }

    #[test]
    fn parse_json_response_from_body() {
        let body = r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[]}}"#;
        let resp: JsonRpcResponse = serde_json::from_str(body).unwrap();
        assert_eq!(resp.id, Some(1));
        assert!(resp.result.is_some());
    }
}
