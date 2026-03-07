//! Legacy SSE transport for MCP (2024-11-05 spec).
//!
//! Client opens GET `{base_url}/sse`; server sends an `endpoint` event with
//! the POST URL, then `message` events for JSON-RPC responses.  Client POSTs
//! requests to the endpoint URL.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use futures_util::StreamExt;
use reqwest::header::HeaderMap;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::error::McpError;
use crate::jsonrpc::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use crate::reconnect::{self, ReconnectConfig};
use crate::transport::Transport;
use crate::transport_http::build_header_map;

pub struct SseTransport {
    base_url: String,
    post_url: Option<String>,
    client: reqwest::Client,
    headers: HeaderMap,
    next_id: u64,
    reconnect: ReconnectConfig,
    message_rx: Option<mpsc::UnboundedReceiver<String>>,
    sse_task: Option<tokio::task::JoinHandle<()>>,
}

/// Resolve an endpoint data line against the base URL.
///
/// If the data payload is an absolute URL it is returned as-is.
/// A relative path is prepended with `base_url` (trailing slash on base is
/// normalised away before joining).
pub fn extract_endpoint_url(line: &str, base_url: &str) -> Option<String> {
    let data = line.strip_prefix("data:")?;
    let data = data.strip_prefix(' ').unwrap_or(data).trim();
    if data.is_empty() {
        return None;
    }
    if data.starts_with("http://") || data.starts_with("https://") {
        return Some(data.to_owned());
    }
    // Relative path: join with base_url, stripping any trailing slash from base.
    let base = base_url.trim_end_matches('/');
    Some(format!("{base}{data}"))
}

impl SseTransport {
    /// Open the SSE stream and wait for the `endpoint` event before returning.
    pub async fn connect(
        base_url: impl Into<String>,
        custom_headers: &HashMap<String, String>,
        reconnect: ReconnectConfig,
    ) -> Result<Self, McpError> {
        let base_url = base_url.into();
        let headers = build_header_map(custom_headers)?;
        let client = reqwest::Client::new();

        let mut transport = Self {
            base_url,
            post_url: None,
            client,
            headers,
            next_id: 1,
            reconnect,
            message_rx: None,
            sse_task: None,
        };

        transport.open_sse_stream().await?;
        Ok(transport)
    }

    /// GET `{base_url}/sse`, parse until `endpoint` event found, then spawn a
    /// background task that feeds `message` event payloads into an mpsc channel.
    async fn open_sse_stream(&mut self) -> Result<(), McpError> {
        let sse_url = format!("{}/sse", self.base_url.trim_end_matches('/'));

        let resp = self
            .client
            .get(&sse_url)
            .headers(self.headers.clone())
            .send()
            .await
            .map_err(|e| McpError::SseConnection(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(McpError::Http { status, body });
        }

        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        let mut current_event: Option<String> = None;
        let mut post_url: Option<String> = None;

        // Read until we find the `endpoint` event.
        'outer: while let Some(chunk) = stream.next().await {
            let bytes = chunk.map_err(|e| McpError::SseConnection(e.to_string()))?;
            buf.push_str(&String::from_utf8_lossy(&bytes));

            while let Some(pos) = buf.find('\n') {
                let line = buf[..pos].trim_end_matches('\r').to_owned();
                buf.drain(..=pos);

                if line.is_empty() {
                    current_event = None;
                    continue;
                }

                if let Some(event_type) = line.strip_prefix("event:") {
                    current_event = Some(event_type.trim().to_owned());
                    continue;
                }

                if current_event.as_deref() == Some("endpoint")
                    && let Some(url) = extract_endpoint_url(&line, &self.base_url)
                {
                    post_url = Some(url);
                    break 'outer;
                }
            }
        }

        let post_url = post_url.ok_or_else(|| {
            McpError::SseConnection("SSE stream ended before endpoint event".into())
        })?;

        self.post_url = Some(post_url);

        // Spawn background task to drain `message` events into the channel.
        let (tx, rx) = mpsc::unbounded_channel::<String>();
        let base_url = self.base_url.clone();

        let task = tokio::spawn(async move {
            // `buf` may still hold partial data from the initial read; we start
            // fresh here because the stream object was consumed above.  The
            // background task re-uses the same stream continuation.
            let _ = base_url; // carried for potential reconnect logging
            let mut remaining = buf;
            let mut evt = current_event;

            while let Some(chunk) = stream.next().await {
                let bytes = match chunk {
                    Ok(b) => b,
                    Err(_) => break,
                };
                remaining.push_str(&String::from_utf8_lossy(&bytes));

                while let Some(pos) = remaining.find('\n') {
                    let line = remaining[..pos].trim_end_matches('\r').to_owned();
                    remaining.drain(..=pos);

                    if line.is_empty() {
                        evt = None;
                        continue;
                    }

                    if let Some(event_type) = line.strip_prefix("event:") {
                        evt = Some(event_type.trim().to_owned());
                        continue;
                    }

                    if evt.as_deref() == Some("message")
                        && let Some(data) = crate::transport_http::extract_sse_data(&line)
                    {
                        let _ = tx.send(data.to_owned());
                    }
                }
            }
        });

        self.message_rx = Some(rx);
        self.sse_task = Some(task);
        Ok(())
    }

    /// POST `body` to the endpoint URL with retry logic.
    async fn post_to_endpoint(&mut self, body: String) -> Result<(), McpError> {
        let post_url = self
            .post_url
            .clone()
            .ok_or_else(|| McpError::Protocol("SSE transport not connected".into()))?;

        let mut attempt = 0usize;
        loop {
            let req = self
                .client
                .post(&post_url)
                .headers(self.headers.clone())
                .body(body.clone());

            match req.send().await {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    if resp.status().is_success() {
                        return Ok(());
                    }
                    if reconnect::is_permanent_error(status) {
                        let body_text = resp.text().await.unwrap_or_default();
                        return Err(McpError::Http {
                            status,
                            body: body_text,
                        });
                    }
                    if !self.reconnect.should_retry(attempt) {
                        let body_text = resp.text().await.unwrap_or_default();
                        return Err(McpError::Http {
                            status,
                            body: body_text,
                        });
                    }
                    let backoff_ms = self.reconnect.backoff_ms(attempt);
                    tracing::warn!(
                        transport = "sse",
                        url = %post_url,
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
                        transport = "sse",
                        url = %post_url,
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

    /// Read from the mpsc channel until a response matching `expected_id` arrives.
    async fn recv_response(&mut self, expected_id: u64) -> Result<Value, McpError> {
        let rx = self
            .message_rx
            .as_mut()
            .ok_or_else(|| McpError::Protocol("SSE transport not connected".into()))?;

        loop {
            let raw = rx
                .recv()
                .await
                .ok_or_else(|| McpError::Protocol("SSE message channel closed".into()))?;

            let rpc: JsonRpcResponse = serde_json::from_str(&raw)?;
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
            // Discard responses for other ids.
        }
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
        self.post_to_endpoint(body).await?;
        self.recv_response(id).await
    }

    async fn send_notify(&mut self, method: &str, params: Option<Value>) -> Result<(), McpError> {
        let notif = JsonRpcNotification::new(method, params);
        let body = serde_json::to_string(&notif)?;
        self.post_to_endpoint(body).await
    }
}

impl Transport for SseTransport {
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
            if let Some(task) = self.sse_task.take() {
                task.abort();
            }
            self.message_rx = None;
            self.post_url = None;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_endpoint_relative() {
        let result = extract_endpoint_url("data: /messages?session_id=abc", "https://example.com");
        assert_eq!(
            result,
            Some("https://example.com/messages?session_id=abc".to_string())
        );
    }

    #[test]
    fn parse_endpoint_absolute() {
        let result =
            extract_endpoint_url("data: https://other.com/messages", "https://example.com");
        assert_eq!(result, Some("https://other.com/messages".to_string()));
    }

    #[test]
    fn parse_endpoint_base_trailing_slash() {
        let result = extract_endpoint_url("data: /messages", "https://example.com/");
        assert_eq!(result, Some("https://example.com/messages".to_string()));
    }

    #[test]
    fn parse_endpoint_not_data_line() {
        assert!(extract_endpoint_url("event: endpoint", "https://example.com").is_none());
        assert!(extract_endpoint_url("", "https://example.com").is_none());
    }
}
