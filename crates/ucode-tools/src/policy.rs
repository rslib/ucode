use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use ucode_core::CoreError;

/// Fine-grained network egress policy.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NetworkPolicy {
    /// Whether network access is allowed at all.
    pub allowed: bool,
    /// Allowed domains (empty = all domains when `allowed` is true).
    pub domain_allowlist: Vec<String>,
    /// Blocked domains — takes precedence over the allowlist.
    pub domain_denylist: Vec<String>,
    /// Allowed ports (empty = all ports when `allowed` is true).
    pub port_allowlist: Vec<u16>,
}

/// Result of a per-request network check.
#[derive(Debug, Clone, PartialEq)]
pub enum NetworkCheckResult {
    Allowed,
    Denied { reason: String },
}

impl NetworkPolicy {
    /// Block all network access.
    pub fn deny_all() -> Self {
        Self {
            allowed: false,
            ..Default::default()
        }
    }

    /// Allow all network access without restriction.
    pub fn allow_all() -> Self {
        Self {
            allowed: true,
            ..Default::default()
        }
    }

    /// Allow network access only to the listed domains (all ports).
    pub fn with_domains(domains: Vec<String>) -> Self {
        Self {
            allowed: true,
            domain_allowlist: domains,
            ..Default::default()
        }
    }

    /// Profile for local-only agents — no network access.
    pub fn local_only() -> Self {
        Self::deny_all()
    }

    /// Profile for research agents — network allowed only to the given domains.
    pub fn research(allowed_domains: Vec<String>) -> Self {
        Self::with_domains(allowed_domains)
    }

    /// Check whether a connection to `domain` on `port` is permitted.
    ///
    /// Rules applied in order:
    /// 1. If `allowed` is false → Denied.
    /// 2. If `domain` is in `domain_denylist` → Denied (takes precedence over allowlist).
    /// 3. If `domain_allowlist` is non-empty and `domain` is not in it → Denied.
    /// 4. If `port_allowlist` is non-empty and `port` is not in it → Denied.
    /// 5. Otherwise → Allowed.
    pub fn check(&self, domain: &str, port: Option<u16>) -> NetworkCheckResult {
        if !self.allowed {
            return NetworkCheckResult::Denied {
                reason: "network access is disabled".into(),
            };
        }

        if self.domain_denylist.iter().any(|d| d == domain) {
            return NetworkCheckResult::Denied {
                reason: format!("domain '{domain}' is in the denylist"),
            };
        }

        if !self.domain_allowlist.is_empty() && !self.domain_allowlist.iter().any(|d| d == domain) {
            return NetworkCheckResult::Denied {
                reason: format!("domain '{domain}' is not in the allowlist"),
            };
        }

        if let Some(p) = port
            && !self.port_allowlist.is_empty()
            && !self.port_allowlist.contains(&p)
        {
            return NetworkCheckResult::Denied {
                reason: format!("port {p} is not in the port allowlist"),
            };
        }

        NetworkCheckResult::Allowed
    }
}

/// Isolation tier — higher value means more restrictive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxTier {
    Off = 0,
    Workspace = 1,
    Networked = 2,
    Strict = 3,
}

/// Capability flags for a resolved policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capabilities {
    pub file_read: bool,
    pub file_write: bool,
    pub cmd_exec: bool,
    pub network: bool,
    pub spawn_process: bool,
}

impl Default for Capabilities {
    fn default() -> Self {
        Self {
            file_read: true,
            file_write: false,
            cmd_exec: false,
            network: false,
            spawn_process: false,
        }
    }
}

/// A single layer in the policy hierarchy.
/// `None` on any field means "inherit from parent / use default".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PolicyLayer {
    pub tier: Option<SandboxTier>,
    pub file_read: Option<bool>,
    pub file_write: Option<bool>,
    pub cmd_exec: Option<bool>,
    pub network: Option<bool>,
    pub spawn_process: Option<bool>,
    /// Fine-grained network policy for this layer.  `None` means no opinion.
    pub network_policy: Option<NetworkPolicy>,
}

/// The fully-resolved policy after merging all layers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectivePolicy {
    pub tier: SandboxTier,
    pub capabilities: Capabilities,
    pub workspace_root: PathBuf,
    /// Merged network policy (most-restrictive wins across all layers).
    pub network_policy: NetworkPolicy,
}

/// Merge an iterator of `NetworkPolicy` references using most-restrictive semantics:
///
/// - If **any** layer sets `allowed = false` → result is denied.
/// - `domain_denylist`: union of all layers (any deny wins).
/// - `domain_allowlist`: intersection of non-empty allowlists (must appear in every
///   layer that has an opinion); an empty allowlist means "no restriction from this layer".
/// - `port_allowlist`: same intersection logic as domain allowlist.
///
/// If the iterator is empty the result is `NetworkPolicy::allow_all()` (no restriction).
fn merge_network_policies<'a>(policies: impl Iterator<Item = &'a NetworkPolicy>) -> NetworkPolicy {
    let collected: Vec<&NetworkPolicy> = policies.collect();

    if collected.is_empty() {
        return NetworkPolicy::allow_all();
    }

    // Any explicit deny wins.
    let allowed = collected.iter().all(|p| p.allowed);

    // Union of all denylists.
    let mut domain_denylist: Vec<String> = Vec::new();
    for p in &collected {
        for d in &p.domain_denylist {
            if !domain_denylist.contains(d) {
                domain_denylist.push(d.clone());
            }
        }
    }

    // Intersection of non-empty allowlists.
    let domain_allowlist =
        intersect_string_allowlists(collected.iter().map(|p| p.domain_allowlist.as_slice()));
    let port_allowlist =
        intersect_port_allowlists(collected.iter().map(|p| p.port_allowlist.as_slice()));

    NetworkPolicy {
        allowed,
        domain_allowlist,
        domain_denylist,
        port_allowlist,
    }
}

/// Intersect non-empty string allowlists.  An empty slice from a layer means
/// "no restriction from this layer" and is skipped.
fn intersect_string_allowlists<'a>(lists: impl Iterator<Item = &'a [String]>) -> Vec<String> {
    let non_empty: Vec<&[String]> = lists.filter(|l| !l.is_empty()).collect();
    if non_empty.is_empty() {
        return Vec::new();
    }
    non_empty[0]
        .iter()
        .filter(|item| non_empty[1..].iter().all(|l| l.contains(item)))
        .cloned()
        .collect()
}

/// Intersect non-empty port allowlists.
fn intersect_port_allowlists<'a>(lists: impl Iterator<Item = &'a [u16]>) -> Vec<u16> {
    let non_empty: Vec<&[u16]> = lists.filter(|l| !l.is_empty()).collect();
    if non_empty.is_empty() {
        return Vec::new();
    }
    non_empty[0]
        .iter()
        .filter(|&&p| non_empty[1..].iter().all(|l| l.contains(&p)))
        .copied()
        .collect()
}

/// Ordered stack of policy layers (global first, session/tool last).
///
/// Resolution rules:
/// - `tier`: maximum (most restrictive) across all layers that set it; default `Workspace`.
/// - Each capability: `false` if **any** layer explicitly sets it to `false`; otherwise the
///   default for that capability applies.  A lower layer cannot escalate a capability that a
///   higher layer has denied.
/// - `network_policy`: merged with most-restrictive semantics (see [`merge_network_policies`]).
pub struct PolicyStack {
    workspace_root: PathBuf,
    layers: Vec<PolicyLayer>,
}

impl PolicyStack {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self {
            workspace_root,
            layers: Vec::new(),
        }
    }

    pub fn push(&mut self, layer: PolicyLayer) {
        self.layers.push(layer);
    }

    pub fn resolve(&self) -> EffectivePolicy {
        let defaults = Capabilities::default();

        let tier = self
            .layers
            .iter()
            .filter_map(|l| l.tier)
            .max()
            .unwrap_or(SandboxTier::Workspace);

        // Any explicit `false` in any layer denies the capability.
        let resolve_cap = |extract: fn(&PolicyLayer) -> Option<bool>, default: bool| -> bool {
            if self.layers.iter().any(|l| extract(l) == Some(false)) {
                return false;
            }
            // If at least one layer explicitly grants it, allow; otherwise fall back to default.
            if self.layers.iter().any(|l| extract(l) == Some(true)) {
                return true;
            }
            default
        };

        let capabilities = Capabilities {
            file_read: resolve_cap(|l| l.file_read, defaults.file_read),
            file_write: resolve_cap(|l| l.file_write, defaults.file_write),
            cmd_exec: resolve_cap(|l| l.cmd_exec, defaults.cmd_exec),
            network: resolve_cap(|l| l.network, defaults.network),
            spawn_process: resolve_cap(|l| l.spawn_process, defaults.spawn_process),
        };

        let network_policy =
            merge_network_policies(self.layers.iter().filter_map(|l| l.network_policy.as_ref()));

        EffectivePolicy {
            tier,
            capabilities,
            workspace_root: self.workspace_root.clone(),
            network_policy,
        }
    }
}

/// Canonicalize `path` and verify it resides within `workspace_root`.
///
/// For paths that do not yet exist on disk (e.g. a file about to be created),
/// the parent directory is canonicalized and the filename is appended.
///
/// Returns the canonical path on success, or `CoreError::Tool` if the path
/// escapes the workspace.
pub fn check_path_within_workspace(
    path: &Path,
    workspace_root: &Path,
) -> Result<PathBuf, CoreError> {
    let canonical_root = workspace_root.canonicalize().map_err(|e| {
        CoreError::Tool(format!(
            "cannot canonicalize workspace root '{}': {e}",
            workspace_root.display()
        ))
    })?;

    let canonical_path = if path.exists() {
        path.canonicalize().map_err(|e| {
            CoreError::Tool(format!("cannot canonicalize '{}': {e}", path.display()))
        })?
    } else {
        // Path doesn't exist yet — canonicalize the parent and re-attach the filename.
        let parent = path.parent().ok_or_else(|| {
            CoreError::Tool(format!("path '{}' has no parent directory", path.display()))
        })?;
        let filename = path.file_name().ok_or_else(|| {
            CoreError::Tool(format!(
                "path '{}' has no filename component",
                path.display()
            ))
        })?;
        let canonical_parent = parent.canonicalize().map_err(|e| {
            CoreError::Tool(format!(
                "cannot canonicalize parent '{}': {e}",
                parent.display()
            ))
        })?;
        canonical_parent.join(filename)
    };

    if !canonical_path.starts_with(&canonical_root) {
        return Err(CoreError::Tool(format!(
            "path '{}' escapes workspace '{}'",
            canonical_path.display(),
            canonical_root.display()
        )));
    }

    Ok(canonical_path)
}

impl EffectivePolicy {
    /// Check whether file access at `path` is permitted.
    ///
    /// Always checks `file_read`; also checks `file_write` when `write` is `true`.
    /// Returns the canonical path on success.
    pub fn check_file_access(&self, path: &Path, write: bool) -> Result<PathBuf, CoreError> {
        if !self.capabilities.file_read {
            return Err(CoreError::Tool("file_read capability denied".into()));
        }
        if write && !self.capabilities.file_write {
            return Err(CoreError::Tool("file_write capability denied".into()));
        }
        check_path_within_workspace(path, &self.workspace_root)
    }

    /// Check whether command execution is permitted.
    pub fn check_cmd_exec(&self) -> Result<(), CoreError> {
        if !self.capabilities.cmd_exec {
            return Err(CoreError::Tool("cmd_exec capability denied".into()));
        }
        Ok(())
    }

    /// Check whether network access is permitted (coarse capability gate only).
    pub fn check_network(&self) -> Result<(), CoreError> {
        if !self.capabilities.network {
            return Err(CoreError::Tool("network capability denied".into()));
        }
        Ok(())
    }

    /// Check whether a network connection to `domain:port` is permitted by the
    /// fine-grained [`NetworkPolicy`].  Pass `None` for `port` when the port is
    /// not yet known.
    pub fn check_network_domain(&self, domain: &str, port: Option<u16>) -> Result<(), CoreError> {
        match self.network_policy.check(domain, port) {
            NetworkCheckResult::Allowed => Ok(()),
            NetworkCheckResult::Denied { reason } => {
                Err(CoreError::Tool(format!("network denied: {reason}")))
            }
        }
    }

    /// Check whether spawning external processes is permitted.
    pub fn check_spawn(&self) -> Result<(), CoreError> {
        if !self.capabilities.spawn_process {
            return Err(CoreError::Tool("spawn_process capability denied".into()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── NetworkPolicy::check ──────────────────────────────────────────────────

    #[test]
    fn deny_all_blocks_everything() {
        let p = NetworkPolicy::deny_all();
        assert_eq!(
            p.check("example.com", None),
            NetworkCheckResult::Denied {
                reason: "network access is disabled".into()
            }
        );
        assert_eq!(
            p.check("example.com", Some(443)),
            NetworkCheckResult::Denied {
                reason: "network access is disabled".into()
            }
        );
    }

    #[test]
    fn allow_all_allows_everything() {
        let p = NetworkPolicy::allow_all();
        assert_eq!(
            p.check("example.com", Some(443)),
            NetworkCheckResult::Allowed
        );
        assert_eq!(p.check("anything.io", None), NetworkCheckResult::Allowed);
    }

    #[test]
    fn domain_allowlist_permits_only_listed_domains() {
        let p = NetworkPolicy::with_domains(vec!["api.example.com".into()]);
        assert_eq!(
            p.check("api.example.com", None),
            NetworkCheckResult::Allowed
        );
        assert!(matches!(
            p.check("evil.com", None),
            NetworkCheckResult::Denied { .. }
        ));
    }

    #[test]
    fn domain_denylist_takes_precedence_over_allowlist() {
        let p = NetworkPolicy {
            allowed: true,
            domain_allowlist: vec!["example.com".into()],
            domain_denylist: vec!["example.com".into()],
            port_allowlist: vec![],
        };
        // In both lists — deny wins.
        assert!(matches!(
            p.check("example.com", None),
            NetworkCheckResult::Denied { .. }
        ));
    }

    #[test]
    fn port_allowlist_blocks_unlisted_ports() {
        let p = NetworkPolicy {
            allowed: true,
            domain_allowlist: vec![],
            domain_denylist: vec![],
            port_allowlist: vec![443, 8443],
        };
        assert_eq!(
            p.check("example.com", Some(443)),
            NetworkCheckResult::Allowed
        );
        assert_eq!(
            p.check("example.com", Some(8443)),
            NetworkCheckResult::Allowed
        );
        assert!(matches!(
            p.check("example.com", Some(80)),
            NetworkCheckResult::Denied { .. }
        ));
        // No port specified — port allowlist is not checked.
        assert_eq!(p.check("example.com", None), NetworkCheckResult::Allowed);
    }

    // ── merge: most-restrictive wins ─────────────────────────────────────────

    #[test]
    fn merge_allow_all_plus_deny_all_is_denied() {
        let mut stack = PolicyStack::new("/tmp".into());
        stack.push(PolicyLayer {
            network_policy: Some(NetworkPolicy::allow_all()),
            ..Default::default()
        });
        stack.push(PolicyLayer {
            network_policy: Some(NetworkPolicy::deny_all()),
            ..Default::default()
        });
        let ep = stack.resolve();
        assert!(matches!(
            ep.network_policy.check("example.com", None),
            NetworkCheckResult::Denied { .. }
        ));
    }

    #[test]
    fn merge_domain_allowlists_intersect() {
        let mut stack = PolicyStack::new("/tmp".into());
        stack.push(PolicyLayer {
            network_policy: Some(NetworkPolicy::with_domains(vec![
                "a.com".into(),
                "b.com".into(),
            ])),
            ..Default::default()
        });
        stack.push(PolicyLayer {
            network_policy: Some(NetworkPolicy::with_domains(vec![
                "b.com".into(),
                "c.com".into(),
            ])),
            ..Default::default()
        });
        let ep = stack.resolve();
        // Only "b.com" is in both allowlists.
        assert_eq!(
            ep.network_policy.check("b.com", None),
            NetworkCheckResult::Allowed
        );
        assert!(matches!(
            ep.network_policy.check("a.com", None),
            NetworkCheckResult::Denied { .. }
        ));
        assert!(matches!(
            ep.network_policy.check("c.com", None),
            NetworkCheckResult::Denied { .. }
        ));
    }

    #[test]
    fn merge_denylists_union() {
        let mut stack = PolicyStack::new("/tmp".into());
        stack.push(PolicyLayer {
            network_policy: Some(NetworkPolicy {
                allowed: true,
                domain_denylist: vec!["bad1.com".into()],
                ..Default::default()
            }),
            ..Default::default()
        });
        stack.push(PolicyLayer {
            network_policy: Some(NetworkPolicy {
                allowed: true,
                domain_denylist: vec!["bad2.com".into()],
                ..Default::default()
            }),
            ..Default::default()
        });
        let ep = stack.resolve();
        assert!(matches!(
            ep.network_policy.check("bad1.com", None),
            NetworkCheckResult::Denied { .. }
        ));
        assert!(matches!(
            ep.network_policy.check("bad2.com", None),
            NetworkCheckResult::Denied { .. }
        ));
        assert_eq!(
            ep.network_policy.check("good.com", None),
            NetworkCheckResult::Allowed
        );
    }

    // ── profiles ─────────────────────────────────────────────────────────────

    #[test]
    fn local_only_profile_blocks_all_network() {
        let p = NetworkPolicy::local_only();
        assert!(matches!(
            p.check("localhost", Some(8080)),
            NetworkCheckResult::Denied { .. }
        ));
    }

    #[test]
    fn research_profile_allows_only_specified_domains() {
        let p = NetworkPolicy::research(vec!["scholar.google.com".into(), "arxiv.org".into()]);
        assert_eq!(p.check("arxiv.org", Some(443)), NetworkCheckResult::Allowed);
        assert_eq!(
            p.check("scholar.google.com", None),
            NetworkCheckResult::Allowed
        );
        assert!(matches!(
            p.check("twitter.com", None),
            NetworkCheckResult::Denied { .. }
        ));
    }

    // ── per-agent profiles in the same stack ─────────────────────────────────

    #[test]
    fn agent_a_local_only_agent_b_research_independent_stacks() {
        let mut stack_a = PolicyStack::new("/tmp".into());
        stack_a.push(PolicyLayer {
            network_policy: Some(NetworkPolicy::local_only()),
            ..Default::default()
        });

        let mut stack_b = PolicyStack::new("/tmp".into());
        stack_b.push(PolicyLayer {
            network_policy: Some(NetworkPolicy::research(vec!["arxiv.org".into()])),
            ..Default::default()
        });

        let ep_a = stack_a.resolve();
        let ep_b = stack_b.resolve();

        assert!(matches!(
            ep_a.network_policy.check("arxiv.org", None),
            NetworkCheckResult::Denied { .. }
        ));
        assert_eq!(
            ep_b.network_policy.check("arxiv.org", None),
            NetworkCheckResult::Allowed
        );
        assert!(matches!(
            ep_b.network_policy.check("evil.com", None),
            NetworkCheckResult::Denied { .. }
        ));
    }

    // ── PolicyLayer.network_policy integrates into resolve() ─────────────────

    #[test]
    fn policy_layer_network_policy_integrates_into_resolve() {
        let mut stack = PolicyStack::new("/tmp".into());
        // Global layer: allow all.
        stack.push(PolicyLayer {
            network_policy: Some(NetworkPolicy::allow_all()),
            ..Default::default()
        });
        // Tool layer: restrict to port 443 only.
        stack.push(PolicyLayer {
            network_policy: Some(NetworkPolicy {
                allowed: true,
                port_allowlist: vec![443],
                ..Default::default()
            }),
            ..Default::default()
        });

        let ep = stack.resolve();
        assert_eq!(
            ep.network_policy.check("example.com", Some(443)),
            NetworkCheckResult::Allowed
        );
        assert!(matches!(
            ep.network_policy.check("example.com", Some(80)),
            NetworkCheckResult::Denied { .. }
        ));
    }

    // ── no network_policy layers → allow_all default ─────────────────────────

    #[test]
    fn empty_stack_network_policy_defaults_to_allow_all() {
        let stack = PolicyStack::new("/tmp".into());
        let ep = stack.resolve();
        assert_eq!(
            ep.network_policy.check("example.com", Some(443)),
            NetworkCheckResult::Allowed
        );
    }

    #[test]
    fn stack_without_network_policy_layers_defaults_to_allow_all() {
        let mut stack = PolicyStack::new("/tmp".into());
        stack.push(PolicyLayer {
            tier: Some(SandboxTier::Networked),
            network: Some(true),
            ..Default::default()
        });
        let ep = stack.resolve();
        assert_eq!(
            ep.network_policy.check("anything.io", None),
            NetworkCheckResult::Allowed
        );
    }
}
