use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use ucode_core::CoreError;

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
}

/// The fully-resolved policy after merging all layers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectivePolicy {
    pub tier: SandboxTier,
    pub capabilities: Capabilities,
    pub workspace_root: PathBuf,
}

/// Ordered stack of policy layers (global first, session/tool last).
///
/// Resolution rules:
/// - `tier`: maximum (most restrictive) across all layers that set it; default `Workspace`.
/// - Each capability: `false` if **any** layer explicitly sets it to `false`; otherwise the
///   default for that capability applies.  A lower layer cannot escalate a capability that a
///   higher layer has denied.
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

        EffectivePolicy {
            tier,
            capabilities,
            workspace_root: self.workspace_root.clone(),
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

    /// Check whether network access is permitted.
    pub fn check_network(&self) -> Result<(), CoreError> {
        if !self.capabilities.network {
            return Err(CoreError::Tool("network capability denied".into()));
        }
        Ok(())
    }

    /// Check whether spawning external processes is permitted.
    pub fn check_spawn(&self) -> Result<(), CoreError> {
        if !self.capabilities.spawn_process {
            return Err(CoreError::Tool("spawn_process capability denied".into()));
        }
        Ok(())
    }
}
