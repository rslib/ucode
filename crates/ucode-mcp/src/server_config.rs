use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::reconnect::ReconnectConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "transport", rename_all = "kebab-case")]
pub enum TransportType {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: HashMap<String, String>,
    },
    Sse {
        url: String,
    },
    StreamableHttp {
        url: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub name: String,
    #[serde(flatten)]
    pub transport_type: TransportType,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub reconnect: ReconnectConfig,
    pub client_name: Option<String>,
    pub client_version: Option<String>,
}

impl ServerConfig {
    pub fn expanded_headers(&self) -> HashMap<String, String> {
        self.headers
            .iter()
            .map(|(k, v)| (k.clone(), expand_env_vars(v)))
            .collect()
    }
}

pub fn expand_env_vars(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '$' && chars.peek() == Some(&'{') {
            chars.next(); // consume '{'
            let var_name: String = chars.by_ref().take_while(|&c| c != '}').collect();
            let value = std::env::var(&var_name).unwrap_or_default();
            result.push_str(&value);
        } else {
            result.push(ch);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reconnect::ReconnectStrategy;

    #[test]
    fn parse_stdio_config() {
        let toml_str = r#"
            name = "test-server"
            transport = "stdio"
            command = "node"
            args = ["server.js", "--port", "3000"]
        "#;
        let config: ServerConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.name, "test-server");
        match &config.transport_type {
            TransportType::Stdio { command, args, .. } => {
                assert_eq!(command, "node");
                assert_eq!(args, &["server.js", "--port", "3000"]);
            }
            _ => panic!("expected Stdio"),
        }
    }

    #[test]
    fn parse_sse_config() {
        let toml_str = r#"
            name = "remote-sse"
            transport = "sse"
            url = "https://example.com/mcp/sse"
            reconnect = "persistent"
        "#;
        let config: ServerConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.name, "remote-sse");
        match &config.transport_type {
            TransportType::Sse { url } => {
                assert_eq!(url, "https://example.com/mcp/sse");
            }
            _ => panic!("expected Sse"),
        }
        assert_eq!(config.reconnect.strategy, ReconnectStrategy::Persistent);
    }

    #[test]
    fn parse_http_config() {
        let toml_str = r#"
            name = "remote-http"
            transport = "streamable-http"
            url = "https://example.com/mcp"
            client_name = "kimi-code"
            client_version = "2.0.0"
        "#;
        let config: ServerConfig = toml::from_str(toml_str).unwrap();
        match &config.transport_type {
            TransportType::StreamableHttp { url } => {
                assert_eq!(url, "https://example.com/mcp");
            }
            _ => panic!("expected StreamableHttp"),
        }
        assert_eq!(config.client_name.as_deref(), Some("kimi-code"));
        assert_eq!(config.client_version.as_deref(), Some("2.0.0"));
    }

    #[test]
    fn env_var_expansion() {
        // SAFETY: single-threaded test, no concurrent env access
        unsafe { std::env::set_var("TEST_MCP_TOKEN_55", "secret123") };
        let result = expand_env_vars("Bearer ${TEST_MCP_TOKEN_55}");
        assert_eq!(result, "Bearer secret123");
        // SAFETY: single-threaded test, no concurrent env access
        unsafe { std::env::remove_var("TEST_MCP_TOKEN_55") };
    }

    #[test]
    fn env_var_expansion_missing() {
        let result = expand_env_vars("Bearer ${NONEXISTENT_VAR_ZZZZZ}");
        assert_eq!(result, "Bearer ");
    }

    #[test]
    fn parse_headers_with_env_expansion() {
        // SAFETY: single-threaded test, no concurrent env access
        unsafe { std::env::set_var("TEST_AUTH_TOKEN_55", "tok_abc") };
        let toml_str = r#"
            name = "with-headers"
            transport = "sse"
            url = "https://example.com/mcp"

            [headers]
            Authorization = "Bearer ${TEST_AUTH_TOKEN_55}"
            X-Custom = "static-value"
        "#;
        let config: ServerConfig = toml::from_str(toml_str).unwrap();
        let headers = config.expanded_headers();
        assert_eq!(headers.get("Authorization").unwrap(), "Bearer tok_abc");
        assert_eq!(headers.get("X-Custom").unwrap(), "static-value");
        // SAFETY: single-threaded test, no concurrent env access
        unsafe { std::env::remove_var("TEST_AUTH_TOKEN_55") };
    }
}
