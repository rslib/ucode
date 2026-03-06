//! Ed25519 signature verification for WASM plugin binaries.

#[cfg(feature = "signed-plugins")]
use ed25519_dalek::{Signature, Verifier, VerifyingKey};

use std::path::Path;

/// Signature verification policy.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignaturePolicy {
    /// Reject unsigned or invalid-signature plugins.
    Required,
    /// Warn on unsigned plugins, reject invalid signatures.
    WarnUnsigned,
    /// Skip signature verification entirely.
    Disabled,
}

impl Default for SignaturePolicy {
    fn default() -> Self {
        Self::Disabled
    }
}

/// Result of signature verification.
#[derive(Debug, Clone, PartialEq)]
pub enum SignatureCheckResult {
    /// Signature is valid.
    Valid,
    /// No signature file found.
    Unsigned,
    /// Signature file exists but is invalid.
    Invalid { reason: String },
}

/// Errors from signature verification.
#[derive(Debug)]
pub enum SignatureError {
    /// Plugin is unsigned and policy requires signatures.
    Unsigned,
    /// Signature is invalid.
    Invalid(String),
    /// I/O error reading signature file.
    Io(std::io::Error),
    /// Invalid key format.
    InvalidKey(String),
}

impl std::fmt::Display for SignatureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsigned => write!(f, "plugin is unsigned"),
            Self::Invalid(reason) => write!(f, "invalid signature: {reason}"),
            Self::Io(e) => write!(f, "signature I/O error: {e}"),
            Self::InvalidKey(reason) => write!(f, "invalid key: {reason}"),
        }
    }
}

impl std::error::Error for SignatureError {}

/// Apply signature policy to a check result.
pub fn apply_signature_policy(
    policy: &SignaturePolicy,
    result: &SignatureCheckResult,
) -> Result<(), SignatureError> {
    match policy {
        SignaturePolicy::Disabled => Ok(()),
        SignaturePolicy::WarnUnsigned => match result {
            SignatureCheckResult::Valid => Ok(()),
            SignatureCheckResult::Unsigned => {
                tracing::warn!("plugin is unsigned (policy: warn_unsigned)");
                Ok(())
            }
            SignatureCheckResult::Invalid { reason } => {
                Err(SignatureError::Invalid(reason.clone()))
            }
        },
        SignaturePolicy::Required => match result {
            SignatureCheckResult::Valid => Ok(()),
            SignatureCheckResult::Unsigned => Err(SignatureError::Unsigned),
            SignatureCheckResult::Invalid { reason } => {
                Err(SignatureError::Invalid(reason.clone()))
            }
        },
    }
}

/// Check if a `.wasm.sig` file exists for the given WASM path.
pub fn check_signature_file(wasm_path: &Path) -> Option<Vec<u8>> {
    let sig_path = wasm_path.with_extension("wasm.sig");
    std::fs::read(&sig_path).ok()
}

/// Verify a signature against WASM bytes using trusted keys.
#[cfg(feature = "signed-plugins")]
pub fn verify_signature(
    wasm_bytes: &[u8],
    signature_bytes: &[u8; 64],
    trusted_keys: &[VerifyingKey],
) -> SignatureCheckResult {
    let signature = Signature::from_bytes(signature_bytes);
    for key in trusted_keys {
        if key.verify(wasm_bytes, &signature).is_ok() {
            return SignatureCheckResult::Valid;
        }
    }
    SignatureCheckResult::Invalid {
        reason: "signature does not match any trusted key".into(),
    }
}

/// Full verification flow: check for sig file, verify if present, apply policy.
#[cfg(feature = "signed-plugins")]
pub fn verify_plugin_signature(
    wasm_path: &Path,
    wasm_bytes: &[u8],
    trusted_keys: &[VerifyingKey],
    policy: &SignaturePolicy,
) -> Result<(), SignatureError> {
    if matches!(policy, SignaturePolicy::Disabled) {
        return Ok(());
    }

    let check_result = match check_signature_file(wasm_path) {
        None => SignatureCheckResult::Unsigned,
        Some(sig_bytes) => {
            if sig_bytes.len() != 64 {
                SignatureCheckResult::Invalid {
                    reason: format!("signature file is {} bytes, expected 64", sig_bytes.len()),
                }
            } else {
                let mut sig_array = [0u8; 64];
                sig_array.copy_from_slice(&sig_bytes);
                verify_signature(wasm_bytes, &sig_array, trusted_keys)
            }
        }
    };

    apply_signature_policy(policy, &check_result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signature_policy_default_disabled() {
        assert_eq!(SignaturePolicy::default(), SignaturePolicy::Disabled);
    }

    #[test]
    fn test_signature_check_result_variants() {
        let valid = SignatureCheckResult::Valid;
        assert_eq!(valid, SignatureCheckResult::Valid);

        let unsigned = SignatureCheckResult::Unsigned;
        assert_eq!(unsigned, SignatureCheckResult::Unsigned);

        let invalid = SignatureCheckResult::Invalid {
            reason: "bad sig".into(),
        };
        assert!(matches!(invalid, SignatureCheckResult::Invalid { .. }));
    }

    #[test]
    fn test_apply_policy_disabled_allows_unsigned() {
        let result =
            apply_signature_policy(&SignaturePolicy::Disabled, &SignatureCheckResult::Unsigned);
        assert!(result.is_ok());
    }

    #[test]
    fn test_apply_policy_required_rejects_unsigned() {
        let result =
            apply_signature_policy(&SignaturePolicy::Required, &SignatureCheckResult::Unsigned);
        assert!(result.is_err());
    }

    #[test]
    fn test_apply_policy_warn_unsigned_allows() {
        let result = apply_signature_policy(
            &SignaturePolicy::WarnUnsigned,
            &SignatureCheckResult::Unsigned,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_apply_policy_required_allows_valid() {
        let result =
            apply_signature_policy(&SignaturePolicy::Required, &SignatureCheckResult::Valid);
        assert!(result.is_ok());
    }

    #[test]
    fn test_apply_policy_any_rejects_invalid() {
        let invalid = SignatureCheckResult::Invalid {
            reason: "bad".into(),
        };
        assert!(apply_signature_policy(&SignaturePolicy::Required, &invalid).is_err());
        assert!(apply_signature_policy(&SignaturePolicy::WarnUnsigned, &invalid).is_err());
        assert!(apply_signature_policy(&SignaturePolicy::Disabled, &invalid).is_ok());
    }

    #[cfg(feature = "signed-plugins")]
    #[test]
    fn test_verify_signature_roundtrip() {
        use ed25519_dalek::{Signer, SigningKey};

        // Fixed 32-byte secret for deterministic tests — never use in production.
        let secret = [0x42u8; 32];
        let signing_key = SigningKey::from_bytes(&secret);
        let verifying_key = signing_key.verifying_key();
        let wasm_bytes = b"fake wasm component bytes";
        let signature = signing_key.sign(wasm_bytes);
        let sig_bytes = signature.to_bytes();

        let result = verify_signature(wasm_bytes, &sig_bytes, &[verifying_key]);
        assert_eq!(result, SignatureCheckResult::Valid);
    }

    #[cfg(feature = "signed-plugins")]
    #[test]
    fn test_verify_signature_wrong_key() {
        use ed25519_dalek::{Signer, SigningKey};

        let signing_key = SigningKey::from_bytes(&[0x42u8; 32]);
        let wrong_key = SigningKey::from_bytes(&[0x99u8; 32]).verifying_key();
        let wasm_bytes = b"fake wasm component bytes";
        let signature = signing_key.sign(wasm_bytes);
        let sig_bytes = signature.to_bytes();

        let result = verify_signature(wasm_bytes, &sig_bytes, &[wrong_key]);
        assert!(matches!(result, SignatureCheckResult::Invalid { .. }));
    }
}
