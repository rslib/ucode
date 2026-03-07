//! Minimal JWT payload decoding (no signature verification).
//!
//! Used to extract claims from OAuth access tokens (e.g., OpenAI's
//! `chatgpt_account_id` claim). We only need to read the payload,
//! not verify the signature — the token was obtained via a trusted
//! OAuth flow.

use base64::Engine;
use serde_json::Value;

/// Decode the payload of a JWT without verifying the signature.
///
/// Returns the payload as a `serde_json::Value`, or `None` if the
/// token is malformed.
pub fn decode_jwt_payload(token: &str) -> Option<Value> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }

    let payload_b64 = parts[1];
    // JWT uses base64url encoding (no padding)
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let bytes = engine.decode(payload_b64).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Extract the ChatGPT account ID from an OpenAI JWT.
///
/// Checks two claim paths:
/// 1. `chatgpt_account_id` (direct claim)
/// 2. `https://api.openai.com/auth` -> `chatgpt_account_id` (nested)
pub fn extract_openai_account_id(token: &str) -> Option<String> {
    let payload = decode_jwt_payload(token)?;

    // Try direct claim
    if let Some(id) = payload.get("chatgpt_account_id").and_then(|v| v.as_str()) {
        return Some(id.to_owned());
    }

    // Try nested claim
    if let Some(auth) = payload.get("https://api.openai.com/auth")
        && let Some(id) = auth.get("chatgpt_account_id").and_then(|v| v.as_str())
    {
        return Some(id.to_owned());
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn make_jwt(payload: &Value) -> String {
        let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let header = engine.encode(r#"{"alg":"RS256","typ":"JWT"}"#.as_bytes());
        let payload_b64 = engine.encode(serde_json::to_vec(payload).unwrap());
        let sig = engine.encode(b"fake-signature");
        format!("{header}.{payload_b64}.{sig}")
    }

    #[test]
    fn decode_valid_jwt() {
        let payload = serde_json::json!({"sub": "user123", "exp": 9999999999u64});
        let token = make_jwt(&payload);
        let decoded = decode_jwt_payload(&token).unwrap();
        assert_eq!(decoded["sub"], "user123");
    }

    #[test]
    fn decode_invalid_jwt() {
        assert!(decode_jwt_payload("not-a-jwt").is_none());
        assert!(decode_jwt_payload("a.b").is_none());
        assert!(decode_jwt_payload("").is_none());
    }

    #[test]
    fn extract_direct_account_id() {
        let payload = serde_json::json!({
            "chatgpt_account_id": "acct-123",
            "sub": "user"
        });
        let token = make_jwt(&payload);
        assert_eq!(
            extract_openai_account_id(&token).as_deref(),
            Some("acct-123")
        );
    }

    #[test]
    fn extract_nested_account_id() {
        let payload = serde_json::json!({
            "sub": "user",
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acct-456"
            }
        });
        let token = make_jwt(&payload);
        assert_eq!(
            extract_openai_account_id(&token).as_deref(),
            Some("acct-456")
        );
    }

    #[test]
    fn extract_no_account_id() {
        let payload = serde_json::json!({"sub": "user"});
        let token = make_jwt(&payload);
        assert!(extract_openai_account_id(&token).is_none());
    }

    #[test]
    fn direct_claim_takes_precedence() {
        let payload = serde_json::json!({
            "chatgpt_account_id": "direct-id",
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "nested-id"
            }
        });
        let token = make_jwt(&payload);
        assert_eq!(
            extract_openai_account_id(&token).as_deref(),
            Some("direct-id")
        );
    }
}
