pub mod sections;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::app::ToolCallStatus;
use crate::layout::SidebarMode;
use crate::theme::UcodeTheme;

use sections::{
    AgentStatus, AgentsData, ContextData, JobStatus, JobsData, McpData, McpServerStatus,
    NetworkData, RouterData, SkillData, ToolsData, WorkspaceData,
};

// ---------------------------------------------------------------------------
// SectionId
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SectionId {
    Router,
    Skill,
    Context,
    Workspace,
    Tools,
    Agents,
    Network,
    Jobs,
    Mcp,
}

impl SectionId {
    pub fn title(self) -> &'static str {
        match self {
            SectionId::Router => "ROUTER",
            SectionId::Skill => "SKILL",
            SectionId::Context => "CONTEXT",
            SectionId::Workspace => "WORKSPACE",
            SectionId::Tools => "TOOLS",
            SectionId::Agents => "AGENTS",
            SectionId::Network => "NETWORK",
            SectionId::Jobs => "JOBS",
            SectionId::Mcp => "MCP SERVERS",
        }
    }

    pub fn icon(self) -> char {
        match self {
            SectionId::Router => 'R',
            SectionId::Skill => 'S',
            SectionId::Context => 'C',
            SectionId::Workspace => 'W',
            SectionId::Tools => 'T',
            SectionId::Agents => 'A',
            SectionId::Network => 'N',
            SectionId::Jobs => 'J',
            SectionId::Mcp => 'M',
        }
    }

    pub fn all() -> &'static [SectionId] {
        &[
            SectionId::Router,
            SectionId::Skill,
            SectionId::Context,
            SectionId::Workspace,
            SectionId::Tools,
            SectionId::Agents,
            SectionId::Network,
            SectionId::Jobs,
            SectionId::Mcp,
        ]
    }
}

// ---------------------------------------------------------------------------
// SectionState
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SectionState {
    pub id: SectionId,
    pub collapsed: bool,
}

impl SectionState {
    pub fn toggle(&mut self) {
        self.collapsed = !self.collapsed;
    }
}

// ---------------------------------------------------------------------------
// SidebarData
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SidebarData {
    pub sections: Vec<SectionState>,
    pub router: RouterData,
    pub skill: SkillData,
    pub context: ContextData,
    pub workspace: WorkspaceData,
    pub tools: ToolsData,
    pub agents: AgentsData,
    pub network: NetworkData,
    pub jobs: JobsData,
    pub mcp: McpData,
}

impl SidebarData {
    pub fn new() -> Self {
        let sections = SectionId::all()
            .iter()
            .map(|&id| SectionState {
                id,
                collapsed: false,
            })
            .collect();

        Self {
            sections,
            router: RouterData::default(),
            skill: SkillData::default(),
            context: ContextData::default(),
            workspace: WorkspaceData::default(),
            tools: ToolsData::default(),
            agents: AgentsData::default(),
            network: NetworkData::default(),
            jobs: JobsData::default(),
            mcp: McpData::default(),
        }
    }

    pub fn toggle_section(&mut self, id: SectionId) {
        if let Some(s) = self.sections.iter_mut().find(|s| s.id == id) {
            s.toggle();
        }
    }

    pub fn is_collapsed(&self, id: SectionId) -> bool {
        self.sections
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.collapsed)
            .unwrap_or(false)
    }
}

impl Default for SidebarData {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Sidebar widget
// ---------------------------------------------------------------------------

pub struct Sidebar<'a> {
    pub data: &'a SidebarData,
    pub theme: &'a UcodeTheme,
    pub mode: SidebarMode,
}

impl<'a> Sidebar<'a> {
    pub fn new(data: &'a SidebarData, theme: &'a UcodeTheme, mode: SidebarMode) -> Self {
        Self { data, theme, mode }
    }
}

impl Widget for Sidebar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        match self.mode {
            SidebarMode::Hidden => {}
            SidebarMode::IconStrip => render_icon_strip(self.data, self.theme, area, buf),
            SidebarMode::Full => render_full(self.data, self.theme, area, buf),
        }
    }
}

// ---------------------------------------------------------------------------
// Icon-strip rendering
// ---------------------------------------------------------------------------

fn render_icon_strip(data: &SidebarData, theme: &UcodeTheme, area: Rect, buf: &mut Buffer) {
    for (row, section) in data.sections.iter().enumerate() {
        let y = area.y + row as u16;
        if y >= area.y + area.height {
            break;
        }
        let icon = section.id.icon().to_string();
        let style = if section.collapsed {
            theme.muted_style()
        } else {
            theme.accent_style()
        };
        let line = Line::from(Span::styled(icon, style));
        let row_area = Rect {
            x: area.x,
            y,
            width: area.width,
            height: 1,
        };
        line.render(row_area, buf);
    }
}

// ---------------------------------------------------------------------------
// Full rendering
// ---------------------------------------------------------------------------

fn render_full(data: &SidebarData, theme: &UcodeTheme, area: Rect, buf: &mut Buffer) {
    let mut y = area.y;
    let bottom = area.y + area.height;

    for (i, section) in data.sections.iter().enumerate() {
        if y >= bottom {
            break;
        }

        // Divider between sections (not before the first).
        if i > 0 && y < bottom {
            let divider = "─".repeat(area.width as usize);
            let line = Line::from(Span::styled(divider, theme.muted_style()));
            let row_area = Rect {
                x: area.x,
                y,
                width: area.width,
                height: 1,
            };
            line.render(row_area, buf);
            y += 1;
        }

        if y >= bottom {
            break;
        }

        // Header line.
        let header = section_header(section, data, theme, area.width);
        let row_area = Rect {
            x: area.x,
            y,
            width: area.width,
            height: 1,
        };
        header.render(row_area, buf);
        y += 1;

        // Content lines when expanded.
        if !section.collapsed {
            let lines = render_section_lines(section.id, data, theme, area.width);
            for line in lines {
                if y >= bottom {
                    break;
                }
                let row_area = Rect {
                    x: area.x,
                    y,
                    width: area.width,
                    height: 1,
                };
                line.render(row_area, buf);
                y += 1;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Section header
// ---------------------------------------------------------------------------

fn section_header<'a>(
    section: &SectionState,
    data: &SidebarData,
    theme: &'a UcodeTheme,
    width: u16,
) -> Line<'a> {
    if section.collapsed {
        let summary = section_summary(section.id, data);
        let text = format!("▶ {}  {}", section.id.title(), summary);
        // Truncate to width.
        let text = truncate_str(&text, width as usize);
        Line::from(Span::styled(text, theme.dim_style()))
    } else {
        let text = format!("▼ {}", section.id.title());
        Line::from(Span::styled(text, theme.accent_style()))
    }
}

// ---------------------------------------------------------------------------
// Section content dispatcher
// ---------------------------------------------------------------------------

pub fn render_section_lines<'a>(
    id: SectionId,
    data: &SidebarData,
    theme: &'a UcodeTheme,
    width: u16,
) -> Vec<Line<'a>> {
    match id {
        SectionId::Router => router_lines(&data.router, theme),
        SectionId::Skill => skill_lines(&data.skill, theme),
        SectionId::Context => context_lines(&data.context, theme, width),
        SectionId::Workspace => workspace_lines(&data.workspace, theme),
        SectionId::Tools => tools_lines(&data.tools, theme),
        SectionId::Agents => agents_lines(&data.agents, theme),
        SectionId::Network => network_lines(&data.network, theme),
        SectionId::Jobs => jobs_lines(&data.jobs, theme),
        SectionId::Mcp => mcp_lines(&data.mcp, theme),
    }
}

// ---------------------------------------------------------------------------
// Per-section content renderers
// ---------------------------------------------------------------------------

fn router_lines<'a>(data: &RouterData, theme: &'a UcodeTheme) -> Vec<Line<'a>> {
    let mut lines = Vec::new();

    // Model name + optional group badge.
    let badge = data
        .model_group
        .map(|g| format!("{} ", g.badge()))
        .unwrap_or_default();
    lines.push(Line::from(vec![
        Span::styled(badge, theme.accent_style()),
        Span::styled(data.model_name.clone(), theme.text_style()),
    ]));

    // Fallback chain.
    if !data.fallback_chain.is_empty() {
        let chain = data.fallback_chain.join(" → ");
        lines.push(Line::from(Span::styled(
            format!("  chain: {chain}"),
            theme.dim_style(),
        )));
        // Highlight the active provider.
        if let Some(active) = data.fallback_chain.get(data.current_provider_index) {
            lines.push(Line::from(Span::styled(
                format!("  active: {active}"),
                theme.safe_style(),
            )));
        }
    }

    // Sandbox tier.
    let sandbox_color = data.sandbox_tier.color(theme);
    lines.push(Line::from(Span::styled(
        format!("  sandbox: {}", data.sandbox_tier.symbol()),
        Style::new().fg(sandbox_color),
    )));

    // Last decision.
    if let Some(ref decision) = data.last_decision {
        lines.push(Line::from(Span::styled(
            format!("  {decision}"),
            theme.muted_style(),
        )));
    }

    lines
}

fn skill_lines<'a>(data: &SkillData, theme: &'a UcodeTheme) -> Vec<Line<'a>> {
    let mut lines = Vec::new();

    let name = data.name.as_deref().unwrap_or("none");
    lines.push(Line::from(Span::styled(
        format!("  skill: {name}"),
        theme.text_style(),
    )));
    lines.push(Line::from(Span::styled(
        format!("  tools: {}", data.tools_allowed),
        theme.dim_style(),
    )));

    if let Some(ref group) = data.preferred_group {
        lines.push(Line::from(Span::styled(
            format!("  group: {group}"),
            theme.dim_style(),
        )));
    }

    lines
}

fn context_lines<'a>(data: &ContextData, theme: &'a UcodeTheme, width: u16) -> Vec<Line<'a>> {
    let mut lines = Vec::new();

    // Token counts.
    lines.push(Line::from(Span::styled(
        format!("  {}/{} tokens", data.tokens_used, data.tokens_max),
        theme.text_style(),
    )));

    // Progress bar — leave 2 chars indent on each side.
    let bar_width = (width as usize).saturating_sub(4).max(1);
    let pct = data.usage_percent().clamp(0.0, 100.0);
    let filled = ((pct / 100.0) * bar_width as f64).round() as usize;
    let empty = bar_width.saturating_sub(filled);
    let bar = format!("  {}{}", "█".repeat(filled), "░".repeat(empty));
    let bar_style = if pct >= 90.0 {
        theme.danger_style()
    } else if pct >= 70.0 {
        theme.warning_style()
    } else {
        theme.safe_style()
    };
    lines.push(Line::from(Span::styled(bar, bar_style)));

    // Cost.
    lines.push(Line::from(Span::styled(
        format!(
            "  req ${:.4}  session ${:.4}",
            data.cost_request, data.cost_session
        ),
        theme.dim_style(),
    )));

    // Count source.
    lines.push(Line::from(Span::styled(
        format!("  source: {}", data.count_source),
        theme.muted_style(),
    )));

    lines
}

fn workspace_lines<'a>(data: &WorkspaceData, theme: &'a UcodeTheme) -> Vec<Line<'a>> {
    let mut lines = Vec::new();

    if data.files.is_empty() {
        lines.push(Line::from(Span::styled(
            "  no changes",
            theme.muted_style(),
        )));
    } else {
        for f in data.files.iter().take(5) {
            lines.push(Line::from(vec![
                Span::styled(format!("  {}", f.path), theme.text_style()),
                Span::styled(format!(" +{}", f.additions), theme.safe_style()),
                Span::styled(format!(" -{}", f.deletions), theme.danger_style()),
            ]));
        }
        if data.files.len() > 5 {
            lines.push(Line::from(Span::styled(
                format!("  … {} more", data.files.len() - 5),
                theme.muted_style(),
            )));
        }
    }

    if let Some(ref age) = data.checkpoint_age {
        lines.push(Line::from(Span::styled(
            format!("  checkpoint: {age}"),
            theme.dim_style(),
        )));
    }

    lines
}

fn tools_lines<'a>(data: &ToolsData, theme: &'a UcodeTheme) -> Vec<Line<'a>> {
    if data.entries.is_empty() {
        return vec![Line::from(Span::styled(
            "  no recent tools",
            theme.muted_style(),
        ))];
    }

    data.entries
        .iter()
        .take(8)
        .map(|e| {
            let (symbol, style) = match e.status {
                ToolCallStatus::Running => ("⟳", theme.warning_style()),
                ToolCallStatus::Success => ("✓", theme.safe_style()),
                ToolCallStatus::Failed => ("✗", theme.danger_style()),
                ToolCallStatus::PendingApproval => ("?", theme.accent_style()),
            };
            let dur = e
                .duration
                .as_deref()
                .map(|d| format!(" {d}"))
                .unwrap_or_default();
            Line::from(vec![
                Span::styled(format!("  {symbol} "), style),
                Span::styled(format!("{}{}", e.name, dur), theme.text_style()),
            ])
        })
        .collect()
}

fn agents_lines<'a>(data: &AgentsData, theme: &'a UcodeTheme) -> Vec<Line<'a>> {
    if data.entries.is_empty() {
        return vec![Line::from(Span::styled("  no agents", theme.muted_style()))];
    }

    data.entries
        .iter()
        .take(8)
        .map(|e| {
            let (symbol, style) = match e.status {
                AgentStatus::Done => ("✓", theme.safe_style()),
                AgentStatus::Running => ("⟳", theme.warning_style()),
                AgentStatus::Failed => ("✗", theme.danger_style()),
            };
            let indent = "  ".repeat(e.depth as usize + 1);
            let dur = e
                .duration
                .as_deref()
                .map(|d| format!(" {d}"))
                .unwrap_or_default();
            Line::from(vec![
                Span::styled(format!("{indent}{symbol} "), style),
                Span::styled(format!("{}{}", e.name, dur), theme.text_style()),
            ])
        })
        .collect()
}

fn network_lines<'a>(data: &NetworkData, theme: &'a UcodeTheme) -> Vec<Line<'a>> {
    let mut lines = Vec::new();

    let (egress_label, egress_style) = if data.egress {
        ("egress: allowed", theme.warning_style())
    } else {
        ("egress: blocked", theme.safe_style())
    };
    lines.push(Line::from(Span::styled(
        format!("  {egress_label}"),
        egress_style,
    )));

    for (agent, host) in data.connections.iter().take(6) {
        lines.push(Line::from(Span::styled(
            format!("  {agent} → {host}"),
            theme.dim_style(),
        )));
    }

    if data.connections.is_empty() {
        lines.push(Line::from(Span::styled(
            "  no connections",
            theme.muted_style(),
        )));
    }

    lines
}

fn jobs_lines<'a>(data: &JobsData, theme: &'a UcodeTheme) -> Vec<Line<'a>> {
    if data.entries.is_empty() {
        return vec![Line::from(Span::styled("  no jobs", theme.muted_style()))];
    }

    data.entries
        .iter()
        .take(8)
        .map(|e| {
            let (symbol, style) = match e.status {
                JobStatus::Running => ("⟳", theme.warning_style()),
                JobStatus::Done => ("✓", theme.safe_style()),
                JobStatus::Failed => ("✗", theme.danger_style()),
            };
            let elapsed = e
                .elapsed
                .as_deref()
                .map(|d| format!(" {d}"))
                .unwrap_or_default();
            Line::from(vec![
                Span::styled(format!("  {symbol} "), style),
                Span::styled(format!("{}{}", e.name, elapsed), theme.text_style()),
            ])
        })
        .collect()
}

fn mcp_lines<'a>(data: &McpData, theme: &'a UcodeTheme) -> Vec<Line<'a>> {
    if data.servers.is_empty() {
        return vec![Line::from(Span::styled(
            "  no servers",
            theme.muted_style(),
        ))];
    }

    data.servers
        .iter()
        .take(8)
        .map(|s| {
            let (symbol, style) = match s.status {
                McpServerStatus::Running => ("●", theme.safe_style()),
                McpServerStatus::Stopped => ("○", theme.muted_style()),
                McpServerStatus::Failed => ("✗", theme.danger_style()),
            };
            Line::from(vec![
                Span::styled(format!("  {symbol} "), style),
                Span::styled(s.name.clone(), theme.text_style()),
                Span::styled(format!(" [{}]", s.trust), theme.dim_style()),
            ])
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Collapsed summary
// ---------------------------------------------------------------------------

pub fn section_summary(id: SectionId, data: &SidebarData) -> String {
    match id {
        SectionId::Router => {
            let badge = data
                .router
                .model_group
                .map(|g| format!("{} ", g.badge()))
                .unwrap_or_default();
            format!("{}{}", badge, data.router.model_name)
        }
        SectionId::Skill => data.skill.name.as_deref().unwrap_or("none").to_owned(),
        SectionId::Context => {
            let pct = data.context.usage_percent();
            format!(
                "{}/{} ({:.0}%)",
                data.context.tokens_used, data.context.tokens_max, pct
            )
        }
        SectionId::Workspace => {
            let n = data.workspace.files.len();
            let add = data.workspace.total_additions();
            let del = data.workspace.total_deletions();
            format!("{n} files +{add} -{del}")
        }
        SectionId::Tools => {
            let n = data.tools.entries.len();
            let pending = data
                .tools
                .entries
                .iter()
                .filter(|e| e.status == ToolCallStatus::PendingApproval)
                .count();
            if pending > 0 {
                format!("{n} calls, {pending} pending")
            } else {
                format!("{n} calls")
            }
        }
        SectionId::Agents => {
            let (done, running, failed) = data.agents.count_by_status();
            format!("{running} running, {done} done, {failed} failed")
        }
        SectionId::Network => {
            let n = data.network.connections.len();
            if data.network.egress {
                format!("{n} conn, egress on")
            } else {
                format!("{n} conn")
            }
        }
        SectionId::Jobs => {
            let running = data.jobs.running_count();
            format!("{running} running")
        }
        SectionId::Mcp => {
            let active = data.mcp.active_count();
            let total = data.mcp.servers.len();
            format!("{active}/{total} active")
        }
    }
}

// ---------------------------------------------------------------------------
// Utility
// ---------------------------------------------------------------------------

/// Truncate `s` to at most `max_chars` characters (by char count, not bytes).
fn truncate_str(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_owned()
    } else {
        s.chars().take(max_chars).collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ToolCallStatus;
    use crate::theme::ModelGroup;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use sections::{
        AgentEntry, AgentStatus, FileDiff, JobEntry, JobStatus, McpServerEntry, McpServerStatus,
        ToolEntry,
    };

    // -----------------------------------------------------------------------
    // SectionId
    // -----------------------------------------------------------------------

    #[test]
    fn section_id_titles() {
        let expected = [
            (SectionId::Router, "ROUTER"),
            (SectionId::Skill, "SKILL"),
            (SectionId::Context, "CONTEXT"),
            (SectionId::Workspace, "WORKSPACE"),
            (SectionId::Tools, "TOOLS"),
            (SectionId::Agents, "AGENTS"),
            (SectionId::Network, "NETWORK"),
            (SectionId::Jobs, "JOBS"),
            (SectionId::Mcp, "MCP SERVERS"),
        ];
        for (id, title) in expected {
            assert_eq!(id.title(), title, "title mismatch for {id:?}");
        }
    }

    #[test]
    fn section_id_icons() {
        let expected = [
            (SectionId::Router, 'R'),
            (SectionId::Skill, 'S'),
            (SectionId::Context, 'C'),
            (SectionId::Workspace, 'W'),
            (SectionId::Tools, 'T'),
            (SectionId::Agents, 'A'),
            (SectionId::Network, 'N'),
            (SectionId::Jobs, 'J'),
            (SectionId::Mcp, 'M'),
        ];
        for (id, icon) in expected {
            assert_eq!(id.icon(), icon, "icon mismatch for {id:?}");
        }
    }

    // -----------------------------------------------------------------------
    // SectionState
    // -----------------------------------------------------------------------

    #[test]
    fn section_toggle() {
        let mut s = SectionState {
            id: SectionId::Router,
            collapsed: false,
        };
        assert!(!s.collapsed);
        s.toggle();
        assert!(s.collapsed);
        s.toggle();
        assert!(!s.collapsed);
    }

    // -----------------------------------------------------------------------
    // SidebarData
    // -----------------------------------------------------------------------

    #[test]
    fn sidebar_data_new_all_expanded() {
        let data = SidebarData::new();
        assert_eq!(data.sections.len(), 9);
        for s in &data.sections {
            assert!(!s.collapsed, "section {:?} should start expanded", s.id);
        }
    }

    #[test]
    fn sidebar_data_toggle_section() {
        let mut data = SidebarData::new();
        assert!(!data.is_collapsed(SectionId::Router));
        data.toggle_section(SectionId::Router);
        assert!(data.is_collapsed(SectionId::Router));
        // Other sections unaffected.
        assert!(!data.is_collapsed(SectionId::Skill));
    }

    // -----------------------------------------------------------------------
    // ContextData
    // -----------------------------------------------------------------------

    #[test]
    fn context_usage_percent() {
        let ctx = ContextData {
            tokens_used: 50_000,
            tokens_max: 200_000,
            ..ContextData::default()
        };
        assert!((ctx.usage_percent() - 25.0).abs() < f64::EPSILON);
    }

    #[test]
    fn context_usage_percent_zero_max() {
        let ctx = ContextData::default(); // tokens_max == 0
        assert_eq!(ctx.usage_percent(), 0.0);
    }

    // -----------------------------------------------------------------------
    // WorkspaceData
    // -----------------------------------------------------------------------

    #[test]
    fn workspace_totals() {
        let data = WorkspaceData {
            files: vec![
                FileDiff {
                    path: "a.rs".into(),
                    additions: 10,
                    deletions: 2,
                },
                FileDiff {
                    path: "b.rs".into(),
                    additions: 5,
                    deletions: 3,
                },
            ],
            checkpoint_age: None,
        };
        assert_eq!(data.total_additions(), 15);
        assert_eq!(data.total_deletions(), 5);
    }

    // -----------------------------------------------------------------------
    // AgentsData
    // -----------------------------------------------------------------------

    #[test]
    fn agents_count_by_status() {
        let data = AgentsData {
            entries: vec![
                AgentEntry {
                    name: "a".into(),
                    status: AgentStatus::Done,
                    duration: None,
                    depth: 0,
                },
                AgentEntry {
                    name: "b".into(),
                    status: AgentStatus::Running,
                    duration: None,
                    depth: 1,
                },
                AgentEntry {
                    name: "c".into(),
                    status: AgentStatus::Running,
                    duration: None,
                    depth: 1,
                },
                AgentEntry {
                    name: "d".into(),
                    status: AgentStatus::Failed,
                    duration: None,
                    depth: 0,
                },
            ],
        };
        assert_eq!(data.count_by_status(), (1, 2, 1));
    }

    // -----------------------------------------------------------------------
    // JobsData
    // -----------------------------------------------------------------------

    #[test]
    fn jobs_running_count() {
        let data = JobsData {
            entries: vec![
                JobEntry {
                    name: "build".into(),
                    command: "cargo build".into(),
                    status: JobStatus::Running,
                    elapsed: None,
                },
                JobEntry {
                    name: "test".into(),
                    command: "cargo test".into(),
                    status: JobStatus::Done,
                    elapsed: Some("3s".into()),
                },
                JobEntry {
                    name: "lint".into(),
                    command: "cargo clippy".into(),
                    status: JobStatus::Running,
                    elapsed: None,
                },
            ],
        };
        assert_eq!(data.running_count(), 2);
    }

    // -----------------------------------------------------------------------
    // McpData
    // -----------------------------------------------------------------------

    #[test]
    fn mcp_active_count() {
        let data = McpData {
            servers: vec![
                McpServerEntry {
                    name: "fs".into(),
                    trust: "trusted".into(),
                    status: McpServerStatus::Running,
                },
                McpServerEntry {
                    name: "web".into(),
                    trust: "untrusted".into(),
                    status: McpServerStatus::Stopped,
                },
                McpServerEntry {
                    name: "db".into(),
                    trust: "trusted".into(),
                    status: McpServerStatus::Running,
                },
            ],
        };
        assert_eq!(data.active_count(), 2);
    }

    // -----------------------------------------------------------------------
    // Render helpers
    // -----------------------------------------------------------------------

    fn render_sidebar(data: &SidebarData, mode: SidebarMode, width: u16, height: u16) -> Buffer {
        let theme = UcodeTheme::default();
        let widget = Sidebar::new(data, &theme, mode);
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);
        buf
    }

    fn buf_text(buf: &Buffer) -> String {
        let area = buf.area;
        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn sidebar_renders_full_mode() {
        let mut data = SidebarData::new();
        data.router.model_name = "claude-opus".into();
        let buf = render_sidebar(&data, SidebarMode::Full, 40, 60);
        let text = buf_text(&buf);
        assert!(text.contains("ROUTER"), "expected ROUTER in:\n{text}");
        assert!(text.contains("SKILL"), "expected SKILL in:\n{text}");
        assert!(text.contains("CONTEXT"), "expected CONTEXT in:\n{text}");
        assert!(text.contains("WORKSPACE"), "expected WORKSPACE in:\n{text}");
        assert!(text.contains("TOOLS"), "expected TOOLS in:\n{text}");
        assert!(text.contains("AGENTS"), "expected AGENTS in:\n{text}");
        assert!(text.contains("NETWORK"), "expected NETWORK in:\n{text}");
        assert!(text.contains("JOBS"), "expected JOBS in:\n{text}");
        assert!(text.contains("MCP"), "expected MCP in:\n{text}");
    }

    #[test]
    fn sidebar_renders_icon_strip() {
        let data = SidebarData::new();
        let buf = render_sidebar(&data, SidebarMode::IconStrip, 6, 20);
        let text = buf_text(&buf);
        for icon in ['R', 'S', 'C', 'W', 'T', 'A', 'N', 'J', 'M'] {
            assert!(text.contains(icon), "expected icon '{icon}' in:\n{text}");
        }
    }

    #[test]
    fn sidebar_hidden_renders_nothing() {
        let data = SidebarData::new();
        let buf = render_sidebar(&data, SidebarMode::Hidden, 40, 20);
        let text = buf_text(&buf);
        // All cells should be blank.
        assert!(
            text.chars().all(|c| c == ' ' || c == '\n'),
            "expected blank buffer for Hidden mode"
        );
    }

    // -----------------------------------------------------------------------
    // section_summary
    // -----------------------------------------------------------------------

    #[test]
    fn section_summary_router() {
        let mut data = SidebarData::new();
        data.router.model_name = "claude-opus-4".into();
        data.router.model_group = Some(ModelGroup::Strong);
        let summary = section_summary(SectionId::Router, &data);
        assert!(
            summary.contains("claude-opus-4"),
            "expected model name in summary: {summary:?}"
        );
    }

    #[test]
    fn section_summary_context() {
        let mut data = SidebarData::new();
        data.context.tokens_used = 80_000;
        data.context.tokens_max = 200_000;
        let summary = section_summary(SectionId::Context, &data);
        assert!(
            summary.contains("80000"),
            "expected token count in summary: {summary:?}"
        );
        assert!(
            summary.contains("200000"),
            "expected max tokens in summary: {summary:?}"
        );
    }

    #[test]
    fn sidebar_collapsed_section_shows_summary() {
        let mut data = SidebarData::new();
        data.router.model_name = "gpt-4o".into();
        data.toggle_section(SectionId::Router);
        let buf = render_sidebar(&data, SidebarMode::Full, 50, 60);
        let text = buf_text(&buf);
        assert!(
            text.contains("gpt-4o"),
            "expected model name in collapsed summary:\n{text}"
        );
    }

    #[test]
    fn sidebar_zero_size_does_not_panic() {
        let data = SidebarData::new();
        let _ = render_sidebar(&data, SidebarMode::Full, 0, 0);
        let _ = render_sidebar(&data, SidebarMode::IconStrip, 0, 0);
    }

    #[test]
    fn tool_entry_status_symbols() {
        let mut data = SidebarData::new();
        data.tools.entries = vec![
            ToolEntry {
                name: "read_file".into(),
                status: ToolCallStatus::Success,
                duration: Some("0.1s".into()),
            },
            ToolEntry {
                name: "write_file".into(),
                status: ToolCallStatus::Running,
                duration: None,
            },
        ];
        let buf = render_sidebar(&data, SidebarMode::Full, 40, 60);
        let text = buf_text(&buf);
        assert!(text.contains("read_file"), "expected read_file in:\n{text}");
        assert!(
            text.contains("write_file"),
            "expected write_file in:\n{text}"
        );
    }

    #[test]
    fn context_progress_bar_appears() {
        let mut data = SidebarData::new();
        data.context.tokens_used = 100_000;
        data.context.tokens_max = 200_000;
        let buf = render_sidebar(&data, SidebarMode::Full, 40, 60);
        let text = buf_text(&buf);
        assert!(text.contains('█'), "expected filled bar chars in:\n{text}");
        assert!(text.contains('░'), "expected empty bar chars in:\n{text}");
    }

    #[test]
    fn sandbox_tier_default_is_off() {
        use crate::theme::SandboxTier;
        assert_eq!(SandboxTier::default(), SandboxTier::Off);
    }

    #[test]
    fn section_summary_tools_no_pending() {
        let mut data = SidebarData::new();
        data.tools.entries = vec![ToolEntry {
            name: "read_file".into(),
            status: ToolCallStatus::Success,
            duration: Some("0.1s".into()),
        }];
        let summary = section_summary(SectionId::Tools, &data);
        assert_eq!(summary, "1 calls");
    }

    #[test]
    fn section_summary_tools_with_pending() {
        let mut data = SidebarData::new();
        data.tools.entries = vec![
            ToolEntry {
                name: "read_file".into(),
                status: ToolCallStatus::Success,
                duration: Some("0.1s".into()),
            },
            ToolEntry {
                name: "run_cmd".into(),
                status: ToolCallStatus::PendingApproval,
                duration: None,
            },
        ];
        let summary = section_summary(SectionId::Tools, &data);
        assert_eq!(summary, "2 calls, 1 pending");
    }
}
