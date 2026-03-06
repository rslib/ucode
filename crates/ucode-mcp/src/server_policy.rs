use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ── Trust tier ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ServerTier {
    /// Full access within configured policy.
    Trusted,
    /// Restricted access; read-only tools only.
    Sandboxed,
    /// No tool execution until explicitly approved.
    #[default]
    Untrusted,
}

// ── Network policy ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServerNetworkPolicy {
    pub allowed: bool,
    pub domain_allowlist: Vec<String>,
    pub domain_denylist: Vec<String>,
}

// ── Tool permission ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ToolPermission {
    /// All tools from this server are permitted.
    AllowAll,
    /// Only the named tools are permitted.
    AllowList(Vec<String>),
    /// No tools from this server are permitted.
    #[default]
    DenyAll,
}

// ── Per-server policy profile ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerPolicy {
    pub server_name: String,
    pub tier: ServerTier,
    pub network: ServerNetworkPolicy,
    pub tool_permission: ToolPermission,
    /// Maximum number of automatic restart attempts before giving up.
    pub max_restart_attempts: u32,
    /// Base delay (ms) for exponential backoff between restart attempts.
    pub restart_backoff_base_ms: u64,
}

impl ServerPolicy {
    pub fn new(server_name: impl Into<String>) -> Self {
        Self {
            server_name: server_name.into(),
            tier: ServerTier::default(),
            network: ServerNetworkPolicy::default(),
            tool_permission: ToolPermission::default(),
            max_restart_attempts: 3,
            restart_backoff_base_ms: 1000,
        }
    }
}

// ── Lifecycle state ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerState {
    Stopped,
    Starting,
    Running,
    Crashed { error: String, crash_count: u32 },
    Restarting { attempt: u32 },
}

// ── Audit log ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditEventType {
    Launch,
    Shutdown,
    Crash,
    RestartAttempt,
    RestartSuccess,
    RestartFailed,
    ToolApproved,
    ToolDenied,
    PolicyChanged,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub server_name: String,
    pub event_type: AuditEventType,
    pub timestamp: DateTime<Utc>,
    pub details: Option<String>,
}

// ── Tool check result ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolCheckResult {
    Allowed,
    Denied { reason: String },
    ServerUntrusted,
}

// ── ServerLifecycle ───────────────────────────────────────────────────────────

pub struct ServerLifecycle {
    server_name: String,
    policy: ServerPolicy,
    state: ServerState,
    audit_log: Vec<AuditEvent>,
    crash_count: u32,
}

fn audit(server_name: &str, event_type: AuditEventType, details: Option<String>) -> AuditEvent {
    AuditEvent {
        server_name: server_name.to_owned(),
        event_type,
        timestamp: Utc::now(),
        details,
    }
}

impl ServerLifecycle {
    pub fn new(policy: ServerPolicy) -> Self {
        let server_name = policy.server_name.clone();
        Self {
            server_name,
            policy,
            state: ServerState::Stopped,
            audit_log: Vec::new(),
            crash_count: 0,
        }
    }

    pub fn state(&self) -> &ServerState {
        &self.state
    }

    pub fn policy(&self) -> &ServerPolicy {
        &self.policy
    }

    pub fn audit_log(&self) -> &[AuditEvent] {
        &self.audit_log
    }

    /// Transition to `Running` and record a `Launch` audit event.
    pub fn record_launch(&mut self) {
        self.state = ServerState::Running;
        let ev = audit(&self.server_name, AuditEventType::Launch, None);
        self.audit_log.push(ev);
    }

    /// Transition to `Stopped` and record a `Shutdown` audit event.
    pub fn record_shutdown(&mut self) {
        self.state = ServerState::Stopped;
        let ev = audit(&self.server_name, AuditEventType::Shutdown, None);
        self.audit_log.push(ev);
    }

    /// Record a crash.  Returns `true` when auto-restart should be attempted
    /// (i.e. `crash_count` is still below `max_restart_attempts`).
    pub fn record_crash(&mut self, error: &str) -> bool {
        self.crash_count += 1;
        self.state = ServerState::Crashed {
            error: error.to_owned(),
            crash_count: self.crash_count,
        };
        let ev = audit(
            &self.server_name,
            AuditEventType::Crash,
            Some(error.to_owned()),
        );
        self.audit_log.push(ev);
        self.crash_count < self.policy.max_restart_attempts
    }

    /// Transition to `Restarting` and return the exponential backoff delay in ms.
    ///
    /// Backoff = `restart_backoff_base_ms * 2^(attempt - 1)`.
    pub fn record_restart_attempt(&mut self) -> u64 {
        let attempt = self.crash_count; // attempt number mirrors crash count
        self.state = ServerState::Restarting { attempt };
        let ev = audit(
            &self.server_name,
            AuditEventType::RestartAttempt,
            Some(format!("attempt {attempt}")),
        );
        self.audit_log.push(ev);
        // 2^(attempt-1), capped at u64::MAX on overflow
        let shift = attempt.saturating_sub(1);
        let multiplier = 1u64.checked_shl(shift).unwrap_or(u64::MAX);
        self.policy
            .restart_backoff_base_ms
            .saturating_mul(multiplier)
    }

    /// Transition to `Running` and reset the crash counter after a successful restart.
    pub fn record_restart_success(&mut self) {
        self.state = ServerState::Running;
        self.crash_count = 0;
        let ev = audit(&self.server_name, AuditEventType::RestartSuccess, None);
        self.audit_log.push(ev);
    }

    /// Record a failed restart.  Transitions to `Stopped` when max attempts are exceeded.
    pub fn record_restart_failed(&mut self, error: &str) {
        let ev = audit(
            &self.server_name,
            AuditEventType::RestartFailed,
            Some(error.to_owned()),
        );
        self.audit_log.push(ev);
        if self.crash_count >= self.policy.max_restart_attempts {
            self.state = ServerState::Stopped;
        }
    }

    /// Check whether `tool_name` may be called according to this server's policy.
    pub fn check_tool_permission(&self, tool_name: &str) -> ToolCheckResult {
        if self.policy.tier == ServerTier::Untrusted {
            return ToolCheckResult::ServerUntrusted;
        }
        match &self.policy.tool_permission {
            ToolPermission::AllowAll => ToolCheckResult::Allowed,
            ToolPermission::DenyAll => ToolCheckResult::Denied {
                reason: "tool execution denied by policy (DenyAll)".to_owned(),
            },
            ToolPermission::AllowList(list) => {
                if list.iter().any(|t| t == tool_name) {
                    ToolCheckResult::Allowed
                } else {
                    ToolCheckResult::Denied {
                        reason: format!("tool '{tool_name}' is not in the allowlist"),
                    }
                }
            }
        }
    }

    /// Check whether outbound network access to `domain` is permitted.
    pub fn check_network(&self, domain: &str) -> bool {
        if !self.policy.network.allowed {
            return false;
        }
        if self
            .policy
            .network
            .domain_denylist
            .iter()
            .any(|d| d == domain)
        {
            return false;
        }
        if !self.policy.network.domain_allowlist.is_empty()
            && !self
                .policy
                .network
                .domain_allowlist
                .iter()
                .any(|d| d == domain)
        {
            return false;
        }
        true
    }

    /// Reset the crash counter (e.g. after a period of stability).
    pub fn reset_crash_count(&mut self) {
        self.crash_count = 0;
    }
}

// ── ServerPolicyStore ─────────────────────────────────────────────────────────

pub struct ServerPolicyStore {
    policies: HashMap<String, ServerLifecycle>,
}

impl Default for ServerPolicyStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ServerPolicyStore {
    pub fn new() -> Self {
        Self {
            policies: HashMap::new(),
        }
    }

    pub fn register(&mut self, policy: ServerPolicy) {
        let name = policy.server_name.clone();
        self.policies.insert(name, ServerLifecycle::new(policy));
    }

    pub fn get(&self, server_name: &str) -> Option<&ServerLifecycle> {
        self.policies.get(server_name)
    }

    pub fn get_mut(&mut self, server_name: &str) -> Option<&mut ServerLifecycle> {
        self.policies.get_mut(server_name)
    }

    /// Returns the registered server names in unspecified order.
    pub fn list(&self) -> Vec<&str> {
        self.policies.keys().map(String::as_str).collect()
    }

    /// Removes a server.  Returns `true` if it existed.
    pub fn remove(&mut self, server_name: &str) -> bool {
        self.policies.remove(server_name).is_some()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn trusted_policy(name: &str) -> ServerPolicy {
        ServerPolicy {
            server_name: name.to_owned(),
            tier: ServerTier::Trusted,
            network: ServerNetworkPolicy::default(),
            tool_permission: ToolPermission::AllowAll,
            max_restart_attempts: 3,
            restart_backoff_base_ms: 1000,
        }
    }

    fn sandboxed_policy(name: &str) -> ServerPolicy {
        ServerPolicy {
            server_name: name.to_owned(),
            tier: ServerTier::Sandboxed,
            network: ServerNetworkPolicy::default(),
            tool_permission: ToolPermission::DenyAll,
            max_restart_attempts: 3,
            restart_backoff_base_ms: 1000,
        }
    }

    // 1
    #[test]
    fn test_new_lifecycle_starts_stopped() {
        let lc = ServerLifecycle::new(trusted_policy("srv"));
        assert_eq!(lc.state(), &ServerState::Stopped);
    }

    // 2
    #[test]
    fn test_record_launch_transitions_to_running() {
        let mut lc = ServerLifecycle::new(trusted_policy("srv"));
        lc.record_launch();
        assert_eq!(lc.state(), &ServerState::Running);
    }

    // 3
    #[test]
    fn test_record_shutdown_transitions_to_stopped() {
        let mut lc = ServerLifecycle::new(trusted_policy("srv"));
        lc.record_launch();
        lc.record_shutdown();
        assert_eq!(lc.state(), &ServerState::Stopped);
    }

    // 4
    #[test]
    fn test_record_crash_increments_count() {
        let mut lc = ServerLifecycle::new(trusted_policy("srv"));
        lc.record_launch();
        lc.record_crash("oom");
        assert_eq!(lc.crash_count, 1);
        lc.record_crash("oom again");
        assert_eq!(lc.crash_count, 2);
    }

    // 5
    #[test]
    fn test_record_crash_allows_restart_within_limit() {
        let mut lc = ServerLifecycle::new(trusted_policy("srv")); // max = 3
        let should_restart = lc.record_crash("err"); // crash_count = 1 < 3
        assert!(should_restart);
    }

    // 6
    #[test]
    fn test_record_crash_denies_restart_at_limit() {
        let mut lc = ServerLifecycle::new(trusted_policy("srv")); // max = 3
        lc.record_crash("e1"); // 1
        lc.record_crash("e2"); // 2
        let should_restart = lc.record_crash("e3"); // 3 == max → false
        assert!(!should_restart);
    }

    // 7
    #[test]
    fn test_restart_backoff_exponential() {
        let mut lc = ServerLifecycle::new(trusted_policy("srv")); // base = 1000 ms
        lc.record_crash("e1"); // crash_count = 1
        let b1 = lc.record_restart_attempt(); // attempt=1 → 1000 * 2^0 = 1000
        lc.record_crash("e2"); // crash_count = 2
        let b2 = lc.record_restart_attempt(); // attempt=2 → 1000 * 2^1 = 2000
        lc.record_crash("e3"); // crash_count = 3
        let b3 = lc.record_restart_attempt(); // attempt=3 → 1000 * 2^2 = 4000
        assert_eq!(b1, 1000);
        assert_eq!(b2, 2000);
        assert_eq!(b3, 4000);
    }

    // 8
    #[test]
    fn test_restart_success_resets_crash_count() {
        let mut lc = ServerLifecycle::new(trusted_policy("srv"));
        lc.record_crash("e");
        lc.record_restart_attempt();
        lc.record_restart_success();
        assert_eq!(lc.crash_count, 0);
        assert_eq!(lc.state(), &ServerState::Running);
    }

    // 9
    #[test]
    fn test_restart_failed_at_max_stops() {
        let mut lc = ServerLifecycle::new(trusted_policy("srv")); // max = 3
        lc.record_crash("e1");
        lc.record_crash("e2");
        lc.record_crash("e3"); // crash_count == max_restart_attempts
        lc.record_restart_attempt();
        lc.record_restart_failed("still broken");
        assert_eq!(lc.state(), &ServerState::Stopped);
    }

    // 10
    #[test]
    fn test_tool_permission_allow_all() {
        let lc = ServerLifecycle::new(trusted_policy("srv")); // AllowAll + Trusted
        assert_eq!(
            lc.check_tool_permission("any_tool"),
            ToolCheckResult::Allowed
        );
        assert_eq!(
            lc.check_tool_permission("another"),
            ToolCheckResult::Allowed
        );
    }

    // 11
    #[test]
    fn test_tool_permission_deny_all() {
        let lc = ServerLifecycle::new(sandboxed_policy("srv")); // DenyAll + Sandboxed
        let result = lc.check_tool_permission("some_tool");
        assert!(matches!(result, ToolCheckResult::Denied { .. }));
    }

    // 12
    #[test]
    fn test_tool_permission_allowlist() {
        let policy = ServerPolicy {
            server_name: "srv".to_owned(),
            tier: ServerTier::Trusted,
            network: ServerNetworkPolicy::default(),
            tool_permission: ToolPermission::AllowList(vec!["read_file".to_owned()]),
            max_restart_attempts: 3,
            restart_backoff_base_ms: 1000,
        };
        let lc = ServerLifecycle::new(policy);
        assert_eq!(
            lc.check_tool_permission("read_file"),
            ToolCheckResult::Allowed
        );
        assert!(matches!(
            lc.check_tool_permission("write_file"),
            ToolCheckResult::Denied { .. }
        ));
    }

    // 13
    #[test]
    fn test_tool_permission_untrusted_server() {
        let policy = ServerPolicy {
            server_name: "srv".to_owned(),
            tier: ServerTier::Untrusted,
            network: ServerNetworkPolicy::default(),
            tool_permission: ToolPermission::AllowAll, // even AllowAll is blocked
            max_restart_attempts: 3,
            restart_backoff_base_ms: 1000,
        };
        let lc = ServerLifecycle::new(policy);
        assert_eq!(
            lc.check_tool_permission("any_tool"),
            ToolCheckResult::ServerUntrusted
        );
    }

    // 14
    #[test]
    fn test_network_check_denied_by_default() {
        let lc = ServerLifecycle::new(trusted_policy("srv")); // network.allowed = false
        assert!(!lc.check_network("example.com"));
    }

    // 15
    #[test]
    fn test_network_check_allowlist() {
        let policy = ServerPolicy {
            server_name: "srv".to_owned(),
            tier: ServerTier::Trusted,
            network: ServerNetworkPolicy {
                allowed: true,
                domain_allowlist: vec!["api.example.com".to_owned()],
                domain_denylist: Vec::new(),
            },
            tool_permission: ToolPermission::AllowAll,
            max_restart_attempts: 3,
            restart_backoff_base_ms: 1000,
        };
        let lc = ServerLifecycle::new(policy);
        assert!(lc.check_network("api.example.com"));
        assert!(!lc.check_network("evil.com"));
    }

    // 16
    #[test]
    fn test_network_check_denylist_precedence() {
        let policy = ServerPolicy {
            server_name: "srv".to_owned(),
            tier: ServerTier::Trusted,
            network: ServerNetworkPolicy {
                allowed: true,
                domain_allowlist: vec!["api.example.com".to_owned()],
                domain_denylist: vec!["api.example.com".to_owned()], // also denied
            },
            tool_permission: ToolPermission::AllowAll,
            max_restart_attempts: 3,
            restart_backoff_base_ms: 1000,
        };
        let lc = ServerLifecycle::new(policy);
        // denylist wins even though domain is in allowlist
        assert!(!lc.check_network("api.example.com"));
    }

    // 17
    #[test]
    fn test_audit_log_records_events() {
        let mut lc = ServerLifecycle::new(trusted_policy("srv"));
        lc.record_launch();
        lc.record_crash("boom");
        lc.record_shutdown();

        let log = lc.audit_log();
        assert_eq!(log.len(), 3);
        assert_eq!(log[0].event_type, AuditEventType::Launch);
        assert_eq!(log[1].event_type, AuditEventType::Crash);
        assert_eq!(log[2].event_type, AuditEventType::Shutdown);
        // All events carry the server name
        assert!(log.iter().all(|e| e.server_name == "srv"));
    }

    // 18
    #[test]
    fn test_policy_store_register_and_get() {
        let mut store = ServerPolicyStore::new();
        store.register(trusted_policy("alpha"));
        let lc = store.get("alpha");
        assert!(lc.is_some());
        assert_eq!(lc.unwrap().policy().server_name, "alpha");
        assert!(store.get("beta").is_none());
    }

    // 19
    #[test]
    fn test_policy_store_list() {
        let mut store = ServerPolicyStore::new();
        store.register(trusted_policy("alpha"));
        store.register(trusted_policy("beta"));
        let mut names = store.list();
        names.sort_unstable();
        assert_eq!(names, vec!["alpha", "beta"]);
    }
}
