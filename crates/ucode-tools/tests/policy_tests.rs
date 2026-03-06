use std::fs;
use std::os::unix::fs::symlink;

use tempfile::TempDir;
use ucode_tools::policy::{
    Capabilities, EffectivePolicy, NetworkPolicy, PolicyLayer, PolicyStack, SandboxTier,
    check_path_within_workspace,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn stack_with_root(dir: &TempDir) -> PolicyStack {
    PolicyStack::new(dir.path().to_path_buf())
}

// ── 1. default_policy_is_workspace_readonly ───────────────────────────────────

#[test]
fn default_policy_is_workspace_readonly() {
    let dir = TempDir::new().unwrap();
    let policy = stack_with_root(&dir).resolve();

    assert_eq!(policy.tier, SandboxTier::Workspace);
    assert!(policy.capabilities.file_read);
    assert!(!policy.capabilities.file_write);
    assert!(!policy.capabilities.cmd_exec);
    assert!(!policy.capabilities.network);
    assert!(!policy.capabilities.spawn_process);
}

// ── 2. global_tier_propagates ─────────────────────────────────────────────────

#[test]
fn global_tier_propagates() {
    let dir = TempDir::new().unwrap();
    let mut stack = stack_with_root(&dir);
    stack.push(PolicyLayer {
        tier: Some(SandboxTier::Strict),
        ..Default::default()
    });
    assert_eq!(stack.resolve().tier, SandboxTier::Strict);
}

// ── 3. most_restrictive_tier_wins ─────────────────────────────────────────────

#[test]
fn most_restrictive_tier_wins() {
    let dir = TempDir::new().unwrap();
    let mut stack = stack_with_root(&dir);
    stack.push(PolicyLayer {
        tier: Some(SandboxTier::Off),
        ..Default::default()
    });
    stack.push(PolicyLayer {
        tier: Some(SandboxTier::Workspace),
        ..Default::default()
    });
    // max(Off, Workspace) = Workspace
    assert_eq!(stack.resolve().tier, SandboxTier::Workspace);
}

// ── 4. tool_cannot_escalate_beyond_parent ────────────────────────────────────

#[test]
fn tool_cannot_escalate_beyond_parent() {
    let dir = TempDir::new().unwrap();
    let mut stack = stack_with_root(&dir);
    // Global layer denies file_write.
    stack.push(PolicyLayer {
        file_write: Some(false),
        ..Default::default()
    });
    // Tool layer tries to grant file_write — must not succeed.
    stack.push(PolicyLayer {
        file_write: Some(true),
        ..Default::default()
    });
    assert!(!stack.resolve().capabilities.file_write);
}

// ── 5. tool_can_further_restrict ─────────────────────────────────────────────

#[test]
fn tool_can_further_restrict() {
    let dir = TempDir::new().unwrap();
    let mut stack = stack_with_root(&dir);
    // Global layer allows cmd_exec.
    stack.push(PolicyLayer {
        cmd_exec: Some(true),
        ..Default::default()
    });
    // Tool layer revokes it.
    stack.push(PolicyLayer {
        cmd_exec: Some(false),
        ..Default::default()
    });
    assert!(!stack.resolve().capabilities.cmd_exec);
}

// ── 6. multiple_layers_merge_correctly ───────────────────────────────────────

#[test]
fn multiple_layers_merge_correctly() {
    let dir = TempDir::new().unwrap();
    let mut stack = stack_with_root(&dir);

    // Layer 1: global — strict tier, allow file_read, deny everything else.
    stack.push(PolicyLayer {
        tier: Some(SandboxTier::Strict),
        file_read: Some(true),
        file_write: Some(false),
        cmd_exec: Some(false),
        network: Some(false),
        spawn_process: Some(false),
        network_policy: None,
    });
    // Layer 2: session — tries to grant network (should be blocked by layer 1).
    stack.push(PolicyLayer {
        network: Some(true),
        ..Default::default()
    });
    // Layer 3: agent — grants cmd_exec (no prior denial).
    stack.push(PolicyLayer {
        cmd_exec: Some(true),
        ..Default::default()
    });
    // Layer 4: tool — revokes cmd_exec again.
    stack.push(PolicyLayer {
        cmd_exec: Some(false),
        ..Default::default()
    });

    let p = stack.resolve();
    assert_eq!(p.tier, SandboxTier::Strict);
    assert!(p.capabilities.file_read);
    assert!(!p.capabilities.file_write);
    assert!(!p.capabilities.cmd_exec); // denied by layer 1 and layer 4
    assert!(!p.capabilities.network); // denied by layer 1 despite layer 2 grant
    assert!(!p.capabilities.spawn_process);
}

// ── 7. path_within_workspace_ok ──────────────────────────────────────────────

#[test]
fn path_within_workspace_ok() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("file.txt");
    fs::write(&file, "").unwrap();

    let result = check_path_within_workspace(&file, dir.path());
    assert!(result.is_ok());
}

// ── 8. path_outside_workspace_denied ─────────────────────────────────────────

#[test]
fn path_outside_workspace_denied() {
    let dir = TempDir::new().unwrap();
    // /tmp itself is outside the temp sub-directory.
    let outside = std::path::Path::new("/tmp");
    let result = check_path_within_workspace(outside, dir.path());
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("escapes workspace"), "unexpected error: {msg}");
}

// ── 9. dotdot_traversal_denied ───────────────────────────────────────────────

#[test]
fn dotdot_traversal_denied() {
    let dir = TempDir::new().unwrap();
    // Construct a path that uses `..` to escape the workspace.
    let escape = dir.path().join("..").join("escape.txt");
    let result = check_path_within_workspace(&escape, dir.path());
    assert!(result.is_err());
}

// ── 10. symlink_escape_denied ─────────────────────────────────────────────────

#[test]
fn symlink_escape_denied() {
    let dir = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();

    // Create a symlink inside the workspace that points outside.
    let link = dir.path().join("evil_link");
    symlink(outside.path(), &link).unwrap();

    let result = check_path_within_workspace(&link, dir.path());
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("escapes workspace"), "unexpected error: {msg}");
}

// ── 11. check_file_read_allowed ───────────────────────────────────────────────

#[test]
fn check_file_read_allowed() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("read.txt");
    fs::write(&file, "data").unwrap();

    let policy = EffectivePolicy {
        tier: SandboxTier::Workspace,
        capabilities: Capabilities {
            file_read: true,
            ..Capabilities::default()
        },
        workspace_root: dir.path().to_path_buf(),
        network_policy: NetworkPolicy::allow_all(),
    };

    assert!(policy.check_file_access(&file, false).is_ok());
}

// ── 12. check_file_write_denied ───────────────────────────────────────────────

#[test]
fn check_file_write_denied() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("write.txt");
    fs::write(&file, "").unwrap();

    let policy = EffectivePolicy {
        tier: SandboxTier::Workspace,
        capabilities: Capabilities::default(), // file_write = false
        workspace_root: dir.path().to_path_buf(),
        network_policy: NetworkPolicy::allow_all(),
    };

    let result = policy.check_file_access(&file, true);
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("file_write"), "unexpected error: {msg}");
}

// ── 13. check_cmd_exec_denied ─────────────────────────────────────────────────

#[test]
fn check_cmd_exec_denied() {
    let dir = TempDir::new().unwrap();
    let policy = EffectivePolicy {
        tier: SandboxTier::Workspace,
        capabilities: Capabilities::default(), // cmd_exec = false
        workspace_root: dir.path().to_path_buf(),
        network_policy: NetworkPolicy::allow_all(),
    };

    let result = policy.check_cmd_exec();
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("cmd_exec"), "unexpected error: {msg}");
}

// ── 14. check_network_denied ──────────────────────────────────────────────────

#[test]
fn check_network_denied() {
    let dir = TempDir::new().unwrap();
    let policy = EffectivePolicy {
        tier: SandboxTier::Workspace,
        capabilities: Capabilities::default(), // network = false
        workspace_root: dir.path().to_path_buf(),
        network_policy: NetworkPolicy::allow_all(),
    };

    let result = policy.check_network();
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("network"), "unexpected error: {msg}");
}

// ── 15. tier_ordering ─────────────────────────────────────────────────────────

#[test]
fn tier_ordering() {
    assert!(SandboxTier::Off < SandboxTier::Workspace);
    assert!(SandboxTier::Workspace < SandboxTier::Networked);
    assert!(SandboxTier::Networked < SandboxTier::Strict);
    assert!(SandboxTier::Off < SandboxTier::Strict);
}
