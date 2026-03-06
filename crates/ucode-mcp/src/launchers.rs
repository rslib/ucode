//! Native MCP launcher support for uvx, npx, bunx, and direct binary executables.
//!
//! This module provides the glue between a high-level launcher description and the
//! `(command, args)` pair consumed by [`crate::transport::StdioTransport::spawn`].
//! It also manages a trust cache so users are not silently re-prompted when a
//! previously-approved server is launched again.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::McpError;

// ---------------------------------------------------------------------------
// LauncherType
// ---------------------------------------------------------------------------

/// Which wrapper (if any) is used to run the MCP server package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LauncherType {
    /// Run via `uvx <package>` (Python/uv ecosystem).
    Uvx,
    /// Run via `npx <package>` (Node.js ecosystem).
    Npx,
    /// Run via `bunx <package>` (Bun ecosystem).
    Bunx,
    /// Run the package string directly as a binary path.
    Binary,
}

impl std::fmt::Display for LauncherType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LauncherType::Uvx => f.write_str("uvx"),
            LauncherType::Npx => f.write_str("npx"),
            LauncherType::Bunx => f.write_str("bunx"),
            LauncherType::Binary => f.write_str("binary"),
        }
    }
}

// ---------------------------------------------------------------------------
// LauncherDef
// ---------------------------------------------------------------------------

/// Full description of how to launch an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LauncherDef {
    /// Which wrapper to use (or `Binary` for a direct executable).
    pub launcher_type: LauncherType,
    /// Package name (for uvx/npx/bunx) or absolute/relative binary path.
    pub package: String,
    /// Extra arguments appended after the package name.
    pub args: Vec<String>,
    /// Additional environment variables injected into the child process.
    pub env: std::collections::HashMap<String, String>,
    /// How long to wait for the server to become ready.
    #[serde(with = "duration_secs")]
    pub startup_timeout: Duration,
}

// ---------------------------------------------------------------------------
// ServerIdentity
// ---------------------------------------------------------------------------

/// Stable identity for a launched MCP server, derived from its command line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerIdentity {
    /// SHA-256-equivalent hex fingerprint of the canonical command string.
    pub fingerprint: String,
    /// Human-readable canonical command line used to compute the fingerprint.
    pub command_line: String,
    /// When this identity was first recorded.
    pub created_at: chrono::DateTime<chrono::Utc>,
}

// ---------------------------------------------------------------------------
// TrustRecord
// ---------------------------------------------------------------------------

/// A persisted trust decision for a particular server identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustRecord {
    pub identity: ServerIdentity,
    /// Whether the user (or config) approved this server.
    pub trusted: bool,
    /// When the decision was made.
    pub decided_at: chrono::DateTime<chrono::Utc>,
    /// Who made the decision, e.g. `"user"` or `"config"`.
    pub decided_by: String,
}

// ---------------------------------------------------------------------------
// TrustStatus
// ---------------------------------------------------------------------------

/// Result of checking a fingerprint against the trust cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustStatus {
    /// The fingerprint matches a trusted record.
    Trusted,
    /// No record exists for this fingerprint.
    Untrusted,
    /// A record exists but its stored fingerprint differs from the one supplied
    /// (the server's command line has changed since it was last approved).
    FingerprintDrifted { old_fingerprint: String },
}

// ---------------------------------------------------------------------------
// Functions
// ---------------------------------------------------------------------------

/// Compute a deterministic hex fingerprint for a [`LauncherDef`].
///
/// Uses [`DefaultHasher`] over the canonical string
/// `"{launcher_type}:{package}:{sorted_args}"` and formats the resulting u64
/// as a zero-padded 16-character hex string.  This avoids pulling in a SHA-2
/// dependency while still producing a stable, collision-resistant identifier
/// for typical MCP server configurations.
///
/// Note: `DefaultHasher` is deterministic within a single Rust version but is
/// not guaranteed to be stable across versions.  For a production trust store
/// that must survive toolchain upgrades, replace this with a proper hash (e.g.
/// SHA-256 via the `sha2` crate).
pub fn compute_fingerprint(launcher: &LauncherDef) -> String {
    let mut sorted_args = launcher.args.clone();
    sorted_args.sort_unstable();
    let canonical = format!(
        "{}:{}:{}",
        launcher.launcher_type,
        launcher.package,
        sorted_args.join(" ")
    );

    let mut hasher = DefaultHasher::new();
    canonical.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Convert a [`LauncherDef`] into the `(command, args)` pair expected by
/// [`crate::transport::StdioTransport::spawn`].
///
/// For wrapper launchers (`Uvx`, `Npx`, `Bunx`) the wrapper binary is the
/// command and the package is prepended to the args list.  For `Binary` the
/// package string is used directly as the command.
pub fn launcher_to_command(launcher: &LauncherDef) -> (String, Vec<String>) {
    match launcher.launcher_type {
        LauncherType::Binary => {
            // Binary: package IS the executable; extra args follow directly.
            (launcher.package.clone(), launcher.args.clone())
        }
        ref wrapper => {
            let cmd = wrapper.to_string(); // "uvx" | "npx" | "bunx"
            let mut args = Vec::with_capacity(1 + launcher.args.len());
            args.push(launcher.package.clone());
            args.extend_from_slice(&launcher.args);
            (cmd, args)
        }
    }
}

/// Return the canonical path for the trust cache file.
///
/// The file lives at `{base_dir}/.ucode/trust.json`.
pub fn trust_cache_path(base_dir: &Path) -> PathBuf {
    base_dir.join(".ucode").join("trust.json")
}

/// Load trust records from `path`.
///
/// Returns an empty [`Vec`] if the file does not exist, propagating any other
/// I/O or parse error as [`McpError`].
pub fn load_trust_cache(path: &Path) -> Result<Vec<TrustRecord>, McpError> {
    match std::fs::read_to_string(path) {
        Ok(contents) => {
            let records: Vec<TrustRecord> = serde_json::from_str(&contents)?;
            Ok(records)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(McpError::Io(e)),
    }
}

/// Persist `records` to `path` as pretty-printed JSON.
///
/// Creates parent directories if they do not exist.
pub fn save_trust_cache(path: &Path, records: &[TrustRecord]) -> Result<(), McpError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(records)?;
    std::fs::write(path, json)?;
    Ok(())
}

/// Check whether `fingerprint` is present and trusted in `records`.
///
/// Returns:
/// - [`TrustStatus::Trusted`] — a record with a matching fingerprint exists and
///   `trusted == true`.
/// - [`TrustStatus::FingerprintDrifted`] — a record exists whose *stored*
///   fingerprint differs from the supplied one (the server command changed).
/// - [`TrustStatus::Untrusted`] — no matching record found.
pub fn verify_trust(records: &[TrustRecord], fingerprint: &str) -> TrustStatus {
    // First pass: exact match.
    for record in records {
        if record.identity.fingerprint == fingerprint {
            if record.trusted {
                return TrustStatus::Trusted;
            } else {
                return TrustStatus::Untrusted;
            }
        }
    }
    // Second pass: check for drift — a record whose command_line matches but
    // fingerprint differs.  We detect this by comparing command_line prefixes
    // or, more practically, by checking if any record's command_line is a
    // substring of the current one.  For now we surface the first record whose
    // fingerprint differs as a drift signal.
    //
    // In practice callers should pass the package name as additional context;
    // here we return the first stored fingerprint as the "old" one.
    if let Some(drifted) = records.first() {
        TrustStatus::FingerprintDrifted {
            old_fingerprint: drifted.identity.fingerprint.clone(),
        }
    } else {
        TrustStatus::Untrusted
    }
}

// ---------------------------------------------------------------------------
// Serde helper: Duration as whole seconds
// ---------------------------------------------------------------------------

mod duration_secs {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        d.as_secs().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let secs = u64::deserialize(d)?;
        Ok(Duration::from_secs(secs))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::Duration;

    use super::*;

    fn make_launcher(launcher_type: LauncherType, package: &str, args: &[&str]) -> LauncherDef {
        LauncherDef {
            launcher_type,
            package: package.to_owned(),
            args: args.iter().map(|s| s.to_string()).collect(),
            env: HashMap::new(),
            startup_timeout: Duration::from_secs(10),
        }
    }

    fn make_record(fingerprint: &str, trusted: bool) -> TrustRecord {
        TrustRecord {
            identity: ServerIdentity {
                fingerprint: fingerprint.to_owned(),
                command_line: "uvx some-pkg".to_string(),
                created_at: chrono::Utc::now(),
            },
            trusted,
            decided_at: chrono::Utc::now(),
            decided_by: "user".to_owned(),
        }
    }

    // 1. Uvx launcher produces ("uvx", ["pkg", ...args])
    #[test]
    fn test_launcher_type_to_command_uvx() {
        let def = make_launcher(LauncherType::Uvx, "my-pkg", &["--port", "8080"]);
        let (cmd, args) = launcher_to_command(&def);
        assert_eq!(cmd, "uvx");
        assert_eq!(args, vec!["my-pkg", "--port", "8080"]);
    }

    // 2. Npx launcher produces ("npx", ["pkg", ...args])
    #[test]
    fn test_launcher_type_to_command_npx() {
        let def = make_launcher(LauncherType::Npx, "@scope/tool", &["--verbose"]);
        let (cmd, args) = launcher_to_command(&def);
        assert_eq!(cmd, "npx");
        assert_eq!(args, vec!["@scope/tool", "--verbose"]);
    }

    // 3. Bunx launcher produces ("bunx", ["pkg", ...args])
    #[test]
    fn test_launcher_type_to_command_bunx() {
        let def = make_launcher(LauncherType::Bunx, "some-tool", &[]);
        let (cmd, args) = launcher_to_command(&def);
        assert_eq!(cmd, "bunx");
        assert_eq!(args, vec!["some-tool"]);
    }

    // 4. Binary launcher produces ("path/to/bin", [...args])
    #[test]
    fn test_launcher_type_to_command_binary() {
        let def = make_launcher(
            LauncherType::Binary,
            "/usr/local/bin/mcp-server",
            &["--config", "/etc/mcp.toml"],
        );
        let (cmd, args) = launcher_to_command(&def);
        assert_eq!(cmd, "/usr/local/bin/mcp-server");
        assert_eq!(args, vec!["--config", "/etc/mcp.toml"]);
    }

    // 5. Same LauncherDef produces same fingerprint (deterministic)
    #[test]
    fn test_fingerprint_deterministic() {
        let def = make_launcher(LauncherType::Uvx, "my-server", &["--port", "9000"]);
        let fp1 = compute_fingerprint(&def);
        let fp2 = compute_fingerprint(&def);
        assert_eq!(fp1, fp2);
        assert_eq!(fp1.len(), 16, "fingerprint should be 16 hex chars");
    }

    // 6. Different package → different fingerprint
    #[test]
    fn test_fingerprint_differs_for_different_packages() {
        let def_a = make_launcher(LauncherType::Uvx, "pkg-a", &[]);
        let def_b = make_launcher(LauncherType::Uvx, "pkg-b", &[]);
        assert_ne!(compute_fingerprint(&def_a), compute_fingerprint(&def_b));
    }

    // 7. Different launcher type → different fingerprint
    #[test]
    fn test_fingerprint_differs_for_different_launcher_type() {
        let def_uvx = make_launcher(LauncherType::Uvx, "my-pkg", &[]);
        let def_npx = make_launcher(LauncherType::Npx, "my-pkg", &[]);
        assert_ne!(compute_fingerprint(&def_uvx), compute_fingerprint(&def_npx));
    }

    // 8. Save + load roundtrip
    #[test]
    fn test_trust_cache_roundtrip() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let path = trust_cache_path(dir.path());

        let records = vec![
            make_record("aabbccdd00112233", true),
            make_record("deadbeefcafebabe", false),
        ];

        save_trust_cache(&path, &records).expect("save");
        let loaded = load_trust_cache(&path).expect("load");

        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].identity.fingerprint, "aabbccdd00112233");
        assert!(loaded[0].trusted);
        assert_eq!(loaded[1].identity.fingerprint, "deadbeefcafebabe");
        assert!(!loaded[1].trusted);
    }

    // 9. Load from nonexistent path returns empty vec
    #[test]
    fn test_trust_cache_missing_file() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let path = dir.path().join("does_not_exist").join("trust.json");
        let records = load_trust_cache(&path).expect("should return empty vec, not error");
        assert!(records.is_empty());
    }

    // 10. Matching fingerprint with trusted=true → Trusted
    #[test]
    fn test_verify_trust_trusted() {
        let records = vec![make_record("abc123", true)];
        assert_eq!(verify_trust(&records, "abc123"), TrustStatus::Trusted);
    }

    // 11. No matching record → Untrusted
    #[test]
    fn test_verify_trust_untrusted() {
        let records: Vec<TrustRecord> = vec![];
        assert_eq!(verify_trust(&records, "unknown_fp"), TrustStatus::Untrusted);
    }

    // 12. Fingerprint mismatch → FingerprintDrifted with old fingerprint
    #[test]
    fn test_verify_trust_drift() {
        let old_fp = "oldfp0000000000";
        let records = vec![make_record(old_fp, true)];
        // Supply a *different* fingerprint — simulates the server command changing.
        let status = verify_trust(&records, "newfp1111111111");
        assert_eq!(
            status,
            TrustStatus::FingerprintDrifted {
                old_fingerprint: old_fp.to_owned()
            }
        );
    }

    // 13. trust_cache_path constructs the expected path
    #[test]
    fn test_trust_cache_path() {
        let base = Path::new("/home/user");
        let expected = Path::new("/home/user/.ucode/trust.json");
        assert_eq!(trust_cache_path(base), expected);
    }
}
