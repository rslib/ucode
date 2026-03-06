use std::fs;

use tempfile::TempDir;
use ucode_tools::approval::{
    ApprovalAction, ApprovalDecision, ApprovalScope, ApprovalStore, BoundaryGate,
};
use ucode_tools::policy::{Capabilities, EffectivePolicy, NetworkPolicy, SandboxTier};

// ── helpers ───────────────────────────────────────────────────────────────────

fn permissive_policy(workspace: &TempDir) -> EffectivePolicy {
    EffectivePolicy {
        tier: SandboxTier::Workspace,
        capabilities: Capabilities {
            file_read: true,
            file_write: true,
            cmd_exec: true,
            network: true,
            spawn_process: true,
        },
        workspace_root: workspace.path().to_path_buf(),
        network_policy: NetworkPolicy::allow_all(),
    }
}

fn readonly_policy(workspace: &TempDir) -> EffectivePolicy {
    EffectivePolicy {
        tier: SandboxTier::Workspace,
        capabilities: Capabilities::default(), // file_read=true, rest=false
        workspace_root: workspace.path().to_path_buf(),
        network_policy: NetworkPolicy::allow_all(),
    }
}

// ── 1. in_workspace_file_access_allowed ──────────────────────────────────────

#[test]
fn in_workspace_file_access_allowed() {
    let ws = TempDir::new().unwrap();
    let file = ws.path().join("hello.txt");
    fs::write(&file, "hi").unwrap();

    let gate = BoundaryGate::new(permissive_policy(&ws));
    let result = gate.check_file_access(&file, false);
    assert!(result.is_ok(), "expected Ok, got {result:?}");
}

// ── 2. out_of_workspace_file_requires_approval ───────────────────────────────

#[test]
fn out_of_workspace_file_requires_approval() {
    let ws = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let file = outside.path().join("secret.txt");
    fs::write(&file, "data").unwrap();

    let gate = BoundaryGate::new(permissive_policy(&ws));
    let err = gate.check_file_access(&file, false).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("approval_required:file_access:"),
        "expected approval_required, got: {msg}"
    );
}

// ── 3. session_approved_file_access_passes ───────────────────────────────────

#[test]
fn session_approved_file_access_passes() {
    let ws = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let file = outside.path().join("allowed.txt");
    fs::write(&file, "data").unwrap();

    let gate = BoundaryGate::new(permissive_policy(&ws));

    gate.approve(
        ApprovalAction::FileAccess {
            path: file.clone(),
            write: false,
        },
        ApprovalDecision::Approved(ApprovalScope::Session),
        "user approved",
    );

    let result = gate.check_file_access(&file, false);
    assert!(
        result.is_ok(),
        "expected Ok after session approval, got {result:?}"
    );
}

// ── 4. denied_action_stays_denied ────────────────────────────────────────────

#[test]
fn denied_action_stays_denied() {
    let ws = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let file = outside.path().join("blocked.txt");
    fs::write(&file, "data").unwrap();

    let gate = BoundaryGate::new(permissive_policy(&ws));

    gate.approve(
        ApprovalAction::FileAccess {
            path: file.clone(),
            write: false,
        },
        ApprovalDecision::Denied,
        "user denied",
    );

    let err = gate.check_file_access(&file, false).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("denied"), "expected denied error, got: {msg}");
}

// ── 5. once_approval_not_reused ───────────────────────────────────────────────

#[test]
fn once_approval_not_reused() {
    let store = ApprovalStore::new();
    let action = ApprovalAction::ProcessSpawn {
        program: "ls".to_owned(),
    };

    store.record(
        action.clone(),
        ApprovalDecision::Approved(ApprovalScope::Once),
        "one-time",
    );

    // Once-scoped approval must NOT be returned by lookup.
    let result = store.lookup(&action);
    assert!(
        result.is_none(),
        "Once-scoped approval should not be reusable via lookup, got {result:?}"
    );
}

// ── 6. cmd_exec_policy_denied ─────────────────────────────────────────────────

#[test]
fn cmd_exec_policy_denied() {
    let ws = TempDir::new().unwrap();
    // readonly_policy has cmd_exec = false
    let gate = BoundaryGate::new(readonly_policy(&ws));

    let err = gate.check_cmd_exec("ls", None).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("cmd_exec"),
        "expected cmd_exec denial, got: {msg}"
    );
}

// ── 7. cmd_exec_out_of_workspace_cwd ─────────────────────────────────────────

#[test]
fn cmd_exec_out_of_workspace_cwd() {
    let ws = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();

    let gate = BoundaryGate::new(permissive_policy(&ws));
    let err = gate.check_cmd_exec("ls", Some(outside.path())).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("approval_required:cmd_exec:"),
        "expected approval_required for out-of-workspace cwd, got: {msg}"
    );
}

// ── 8. spawn_requires_approval ────────────────────────────────────────────────

#[test]
fn spawn_requires_approval() {
    let ws = TempDir::new().unwrap();
    let gate = BoundaryGate::new(permissive_policy(&ws));

    let err = gate.check_spawn("curl").unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("approval_required:spawn:"),
        "expected approval_required for spawn, got: {msg}"
    );
}

// ── 9. spawn_session_approved ─────────────────────────────────────────────────

#[test]
fn spawn_session_approved() {
    let ws = TempDir::new().unwrap();
    let gate = BoundaryGate::new(permissive_policy(&ws));

    gate.approve(
        ApprovalAction::ProcessSpawn {
            program: "curl".to_owned(),
        },
        ApprovalDecision::Approved(ApprovalScope::Session),
        "user approved spawn",
    );

    let result = gate.check_spawn("curl");
    assert!(
        result.is_ok(),
        "expected Ok after session approval, got {result:?}"
    );
}

// ── 10. network_check_delegates_to_policy ────────────────────────────────────

#[test]
fn network_check_delegates_to_policy() {
    let ws = TempDir::new().unwrap();

    // Policy with network disabled.
    let no_net = EffectivePolicy {
        tier: SandboxTier::Workspace,
        capabilities: Capabilities {
            file_read: true,
            network: false,
            ..Capabilities::default()
        },
        workspace_root: ws.path().to_path_buf(),
        network_policy: NetworkPolicy::deny_all(),
    };
    let gate = BoundaryGate::new(no_net);
    let err = gate.check_network().unwrap_err();
    assert!(err.to_string().contains("network"), "got: {err}");

    // Policy with network enabled.
    let net_ok = EffectivePolicy {
        tier: SandboxTier::Networked,
        capabilities: Capabilities {
            file_read: true,
            network: true,
            ..Capabilities::default()
        },
        workspace_root: ws.path().to_path_buf(),
        network_policy: NetworkPolicy::allow_all(),
    };
    let gate2 = BoundaryGate::new(net_ok);
    assert!(gate2.check_network().is_ok());
}

// ── 11. audit_log_records_all_decisions ──────────────────────────────────────

#[test]
fn audit_log_records_all_decisions() {
    let store = ApprovalStore::new();

    let a1 = ApprovalAction::NetworkAccess;
    let a2 = ApprovalAction::ProcessSpawn {
        program: "git".to_owned(),
    };

    store.record(
        a1.clone(),
        ApprovalDecision::Approved(ApprovalScope::Session),
        "ok",
    );
    store.record(a2.clone(), ApprovalDecision::Denied, "no");

    let log = store.audit_log();
    assert_eq!(log.len(), 2, "expected 2 audit entries, got {}", log.len());
    assert_eq!(log[0].action, a1);
    assert_eq!(log[1].action, a2);
    assert_eq!(log[1].decision, ApprovalDecision::Denied);
}

// ── 12. dotdot_escape_denied ──────────────────────────────────────────────────

#[test]
fn dotdot_escape_denied() {
    let ws = TempDir::new().unwrap();
    // Construct a path that uses `..` to escape the workspace.
    let escape = ws.path().join("..").join("escape.txt");

    let gate = BoundaryGate::new(permissive_policy(&ws));
    let err = gate.check_file_access(&escape, false).unwrap_err();
    let msg = err.to_string();
    // Must be a hard denial, NOT an approval_required.
    assert!(
        !msg.contains("approval_required"),
        "dotdot escape should be hard-denied, not approval_required; got: {msg}"
    );
    assert!(
        msg.contains("denied") || msg.contains("traversal") || msg.contains("escapes"),
        "expected denial message, got: {msg}"
    );
}
