use crate::app::ToolCallStatus;
use crate::theme::{ModelGroup, SandboxTier};

// ---------------------------------------------------------------------------
// Plugin sidebar section
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct PluginSidebarSection {
    pub plugin_name: String,
    pub section_id: String,
    pub title: String,
    pub lines: Vec<String>,
    /// Lower value = higher priority (rendered first). Default 100.
    pub priority: i32,
    pub collapsed: bool,
}

// ---------------------------------------------------------------------------
// 1. Router
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct RouterData {
    pub model_name: String,
    pub model_group: Option<ModelGroup>,
    /// Ordered provider names, e.g. ["anthropic", "openai", "ollama"].
    pub fallback_chain: Vec<String>,
    pub current_provider_index: usize,
    pub sandbox_tier: SandboxTier,
    /// Human-readable description of the last routing decision.
    pub last_decision: Option<String>,
}

// ---------------------------------------------------------------------------
// 2. Skill
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct SkillData {
    pub name: Option<String>,
    pub tools_allowed: u32,
    pub preferred_group: Option<String>,
}

// ---------------------------------------------------------------------------
// 3. Context
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct ContextData {
    pub tokens_used: u64,
    pub tokens_max: u64,
    pub cost_request: f64,
    pub cost_session: f64,
    /// "provider_count" or "local_estimate".
    pub count_source: String,
}

impl ContextData {
    pub fn usage_percent(&self) -> f64 {
        if self.tokens_max == 0 {
            return 0.0;
        }
        self.tokens_used as f64 / self.tokens_max as f64 * 100.0
    }
}

// ---------------------------------------------------------------------------
// 4. Workspace
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct FileDiff {
    pub path: String,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, Default)]
pub struct WorkspaceData {
    pub files: Vec<FileDiff>,
    /// Human-readable age of the last checkpoint, e.g. "2m ago".
    pub checkpoint_age: Option<String>,
}

impl WorkspaceData {
    pub fn total_additions(&self) -> u32 {
        self.files.iter().map(|f| f.additions).sum()
    }

    pub fn total_deletions(&self) -> u32 {
        self.files.iter().map(|f| f.deletions).sum()
    }
}

// ---------------------------------------------------------------------------
// 5. Tools
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ToolEntry {
    pub name: String,
    pub status: ToolCallStatus,
    pub duration: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ToolsData {
    pub entries: Vec<ToolEntry>,
}

// ---------------------------------------------------------------------------
// 6. Agents
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentStatus {
    Done,
    Running,
    Failed,
}

#[derive(Debug, Clone)]
pub struct AgentEntry {
    pub name: String,
    pub status: AgentStatus,
    pub duration: Option<String>,
    /// 0 = root, 1 = child, etc.
    pub depth: u8,
}

#[derive(Debug, Clone, Default)]
pub struct AgentsData {
    pub entries: Vec<AgentEntry>,
}

impl AgentsData {
    /// Returns `(done, running, failed)` counts.
    pub fn count_by_status(&self) -> (usize, usize, usize) {
        let done = self
            .entries
            .iter()
            .filter(|e| e.status == AgentStatus::Done)
            .count();
        let running = self
            .entries
            .iter()
            .filter(|e| e.status == AgentStatus::Running)
            .count();
        let failed = self
            .entries
            .iter()
            .filter(|e| e.status == AgentStatus::Failed)
            .count();
        (done, running, failed)
    }
}

// ---------------------------------------------------------------------------
// 7. Network
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct NetworkData {
    pub egress: bool,
    /// `(agent_name, host)` pairs for active connections.
    pub connections: Vec<(String, String)>,
}

// ---------------------------------------------------------------------------
// 8. Jobs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone)]
pub struct JobEntry {
    pub name: String,
    pub command: String,
    pub status: JobStatus,
    pub elapsed: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct JobsData {
    pub entries: Vec<JobEntry>,
}

impl JobsData {
    pub fn running_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.status == JobStatus::Running)
            .count()
    }
}

// ---------------------------------------------------------------------------
// 9. MCP Servers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpServerStatus {
    Running,
    Stopped,
    Failed,
}

#[derive(Debug, Clone)]
pub struct McpServerEntry {
    pub name: String,
    /// "trusted" or "untrusted".
    pub trust: String,
    pub status: McpServerStatus,
}

#[derive(Debug, Clone, Default)]
pub struct McpData {
    pub servers: Vec<McpServerEntry>,
}

impl McpData {
    pub fn active_count(&self) -> usize {
        self.servers
            .iter()
            .filter(|s| s.status == McpServerStatus::Running)
            .count()
    }
}
