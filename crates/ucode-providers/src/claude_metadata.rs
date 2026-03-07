//! Read metadata from `~/.claude.json` for Anthropic OAuth requests.
//!
//! When using Anthropic OAuth (Claude Max subscription), the API requires
//! a `metadata.user_id` field in the request body. This is constructed from
//! fields in `~/.claude.json` which is written by Claude Code.

use serde::Deserialize;

/// Relevant fields from `~/.claude.json`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeConfig {
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    oauth_account: Option<OAuthAccount>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OAuthAccount {
    #[serde(default)]
    account_uuid: Option<String>,
}

/// Build the metadata user_id string for Anthropic OAuth requests.
///
/// Format: `user_{userId}_account_{accountUuid}_session_{sessionId}`
///
/// Returns `None` if `~/.claude.json` doesn't exist or lacks required fields.
/// The `session_id` parameter is provided by the caller (from the current session).
pub fn build_metadata_user_id(session_id: &str) -> Option<String> {
    let home = dirs::home_dir()?;
    let path = home.join(".claude.json");

    let content = std::fs::read_to_string(&path).ok()?;
    let config: ClaudeConfig = serde_json::from_str(&content).ok()?;

    let user_id = config.user_id?;
    let account_uuid = config.oauth_account?.account_uuid?;

    Some(format!(
        "user_{user_id}_account_{account_uuid}_session_{session_id}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_claude_config_full() {
        let json = r#"{
            "userId": "user123",
            "oauthAccount": {
                "accountUuid": "acc-456"
            }
        }"#;
        let config: ClaudeConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.user_id.as_deref(), Some("user123"));
        assert_eq!(
            config.oauth_account.unwrap().account_uuid.as_deref(),
            Some("acc-456")
        );
    }

    #[test]
    fn parse_claude_config_missing_fields() {
        let json = r#"{}"#;
        let config: ClaudeConfig = serde_json::from_str(json).unwrap();
        assert!(config.user_id.is_none());
        assert!(config.oauth_account.is_none());
    }

    #[test]
    fn parse_claude_config_partial() {
        let json = r#"{"userId": "user123"}"#;
        let config: ClaudeConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.user_id.as_deref(), Some("user123"));
        assert!(config.oauth_account.is_none());
    }

    #[test]
    fn parse_claude_config_extra_fields_ignored() {
        let json = r#"{
            "userId": "user123",
            "oauthAccount": {
                "accountUuid": "acc-456",
                "extraField": "ignored"
            },
            "otherStuff": true
        }"#;
        let config: ClaudeConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.user_id.as_deref(), Some("user123"));
        assert_eq!(
            config.oauth_account.unwrap().account_uuid.as_deref(),
            Some("acc-456")
        );
    }
}
