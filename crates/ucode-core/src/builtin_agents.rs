use std::collections::HashMap;

use crate::agent_def::{
    AgentDef, AgentMode, AgentSource, PermissionAction, PermissionEntry, ToolPermissions,
    auto_color,
};

pub fn builtin_agents() -> Vec<AgentDef> {
    vec![
        AgentDef {
            name: "coder".into(),
            description: "General-purpose coding agent. Reads, writes, and edits code.".into(),
            system_prompt: "You are a coding assistant. Read, write, and edit code to complete tasks. Follow the project's conventions and style. Prefer minimal, correct changes over large rewrites.".into(),
            color: auto_color("coder"),
            model: None,
            temperature: Some(0.2),
            top_p: None,
            enabled: true,
            mode: AgentMode::Primary,
            hidden: false,
            tools: ToolPermissions::default(),
            permissions: HashMap::new(),
            max_steps: None,
            timeout_secs: None,
            max_retries: None,
            source: AgentSource::BuiltIn,
        },
        AgentDef {
            name: "explore".into(),
            description: "Fast read-only codebase explorer. Searches files and answers questions.".into(),
            system_prompt: "You are a read-only codebase explorer. Search, read, and analyze code to answer questions. Do not modify any files.".into(),
            color: auto_color("explore"),
            model: None,
            temperature: Some(0.1),
            top_p: None,
            enabled: true,
            mode: AgentMode::Subagent,
            hidden: false,
            tools: ToolPermissions {
                read: true,
                edit: false,
                write: false,
                bash: true,
                glob: true,
                grep: true,
                list: true,
            },
            permissions: HashMap::new(),
            max_steps: None,
            timeout_secs: None,
            max_retries: None,
            source: AgentSource::BuiltIn,
        },
        AgentDef {
            name: "planner".into(),
            description: "Breaks tasks into atomic, testable subtasks with dependency tracking.".into(),
            system_prompt: "You are a planning agent. Decompose tasks into atomic, independently testable subtasks. Track dependencies between subtasks. Output structured plans that other agents can execute.".into(),
            color: auto_color("planner"),
            model: None,
            temperature: Some(0.3),
            top_p: None,
            enabled: true,
            mode: AgentMode::Primary,
            hidden: false,
            tools: ToolPermissions {
                read: true,
                edit: false,
                write: true,
                bash: true,
                glob: true,
                grep: true,
                list: true,
            },
            permissions: HashMap::from([
                ("bash".to_string(), PermissionEntry::Flat(PermissionAction::Ask)),
                ("edit".to_string(), PermissionEntry::Flat(PermissionAction::Ask)),
                ("write".to_string(), PermissionEntry::Flat(PermissionAction::Ask)),
            ]),
            max_steps: None,
            timeout_secs: None,
            max_retries: None,
            source: AgentSource::BuiltIn,
        },
        AgentDef {
            name: "orchestrator".into(),
            description: "Primary orchestrator. Analyzes tasks, delegates to specialist agents.".into(),
            system_prompt: "You are the primary orchestrator. Analyze incoming tasks, break them into subtasks, and delegate each to the appropriate specialist agent. Coordinate results and synthesize a final response.".into(),
            color: auto_color("orchestrator"),
            model: None,
            temperature: Some(0.3),
            top_p: None,
            enabled: true,
            mode: AgentMode::Primary,
            hidden: false,
            tools: ToolPermissions::default(),
            permissions: HashMap::new(),
            max_steps: None,
            timeout_secs: None,
            max_retries: None,
            source: AgentSource::BuiltIn,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_def::AgentSource;

    #[test]
    fn builtin_agents_has_four() {
        let agents = builtin_agents();
        assert_eq!(agents.len(), 4);
    }

    #[test]
    fn builtin_agents_names() {
        let agents = builtin_agents();
        let names: Vec<&str> = agents.iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"coder"));
        assert!(names.contains(&"explore"));
        assert!(names.contains(&"planner"));
        assert!(names.contains(&"orchestrator"));
    }

    #[test]
    fn builtin_agents_are_builtin_source() {
        for agent in builtin_agents() {
            assert_eq!(agent.source, AgentSource::BuiltIn);
        }
    }

    #[test]
    fn explore_is_read_only() {
        let agents = builtin_agents();
        let explore = agents.iter().find(|a| a.name == "explore").unwrap();
        assert!(explore.tools.read);
        assert!(!explore.tools.edit);
        assert!(!explore.tools.write);
    }

    #[test]
    fn explore_is_subagent() {
        let agents = builtin_agents();
        let explore = agents.iter().find(|a| a.name == "explore").unwrap();
        assert_eq!(explore.mode, AgentMode::Subagent);
    }

    #[test]
    fn coder_and_orchestrator_are_primary() {
        let agents = builtin_agents();
        let coder = agents.iter().find(|a| a.name == "coder").unwrap();
        let orch = agents.iter().find(|a| a.name == "orchestrator").unwrap();
        assert_eq!(coder.mode, AgentMode::Primary);
        assert_eq!(orch.mode, AgentMode::Primary);
    }

    #[test]
    fn planner_has_ask_permissions() {
        let agents = builtin_agents();
        let planner = agents.iter().find(|a| a.name == "planner").unwrap();
        assert_eq!(planner.permissions.len(), 3);
        assert!(planner.permissions.contains_key("bash"));
        assert!(planner.permissions.contains_key("edit"));
        assert!(planner.permissions.contains_key("write"));
    }

    #[test]
    fn no_builtin_is_hidden() {
        for agent in builtin_agents() {
            assert!(!agent.hidden, "{} should not be hidden", agent.name);
        }
    }

    #[test]
    fn all_builtins_have_colors() {
        for agent in builtin_agents() {
            let c = agent.color;
            assert!(
                c.r != 0 || c.g != 0 || c.b != 0,
                "{} color should not be black",
                agent.name
            );
            let hex = c.to_hex();
            assert!(
                hex.starts_with('#'),
                "{} color hex should start with #",
                agent.name
            );
            assert_eq!(hex.len(), 7, "{} color hex should be 7 chars", agent.name);
        }
    }

    #[test]
    fn builtin_colors_are_distinct() {
        let agents = builtin_agents();
        let colors: Vec<ucode_themes::Rgb> = agents.iter().map(|a| a.color).collect();
        let unique: std::collections::HashSet<(u8, u8, u8)> =
            colors.iter().map(|c| (c.r, c.g, c.b)).collect();
        assert_eq!(
            colors.len(),
            unique.len(),
            "all built-in colors should be distinct"
        );
    }
}
