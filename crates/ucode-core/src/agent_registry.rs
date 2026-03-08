use std::collections::HashMap;
use std::path::Path;

use crate::agent_def::{AgentDef, AgentMode, PermissionEntry};
use crate::builtin_agents::builtin_agents;

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct AgentConfigOverride {
    pub model: Option<String>,
    pub color: Option<ucode_themes::Rgb>,
    pub enabled: Option<bool>,
    pub mode: Option<AgentMode>,
    pub hidden: Option<bool>,
    #[serde(default)]
    pub permissions: Option<HashMap<String, PermissionEntry>>,
    pub max_steps: Option<u32>,
    pub timeout_secs: Option<u64>,
    pub max_retries: Option<u32>,
    pub top_p: Option<f32>,
}

pub struct AgentRegistry {
    // Keyed by agent name; insertion order doesn't matter since we sort on output.
    agents: HashMap<String, AgentDef>,
    /// Name of the default starting agent.
    default_agent: String,
}

impl AgentRegistry {
    pub fn new() -> Self {
        let agents = builtin_agents()
            .into_iter()
            .map(|a| (a.name.clone(), a))
            .collect();
        Self {
            agents,
            default_agent: "coder".into(),
        }
    }

    /// Get the name of the default starting agent.
    pub fn default_agent_name(&self) -> &str {
        &self.default_agent
    }

    /// Get the default starting agent definition.
    pub fn default_agent(&self) -> Option<&AgentDef> {
        self.agents.get(&self.default_agent)
    }

    /// Set the default starting agent. Returns false if the agent doesn't exist.
    pub fn set_default_agent(&mut self, name: &str) -> bool {
        if self.agents.contains_key(name) {
            self.default_agent = name.to_string();
            true
        } else {
            false
        }
    }

    /// Load user agent definitions from `*.md` files in `dir`.
    /// User agents override built-ins with the same name.
    pub fn discover_user_agents(&mut self, dir: &Path) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let name = match path.file_stem().and_then(|s| s.to_str()) {
                Some(n) => n.to_owned(),
                None => continue,
            };
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            match AgentDef::from_markdown(&name, &content) {
                Ok(def) => {
                    self.agents.insert(name, def);
                }
                Err(e) => {
                    tracing::warn!("skipping {}: {e}", path.display());
                }
            }
        }
    }

    /// Apply TOML config overrides to existing agents.
    pub fn apply_overrides(&mut self, overrides: &HashMap<String, AgentConfigOverride>) {
        for (name, ov) in overrides {
            if let Some(agent) = self.agents.get_mut(name) {
                if let Some(model) = &ov.model {
                    agent.model = Some(model.clone());
                }
                if let Some(color) = ov.color {
                    agent.color = color;
                }
                if let Some(enabled) = ov.enabled {
                    agent.enabled = enabled;
                }
                if let Some(mode) = ov.mode {
                    agent.mode = mode;
                }
                if let Some(hidden) = ov.hidden {
                    agent.hidden = hidden;
                }
                if let Some(permissions) = &ov.permissions {
                    agent.permissions = permissions.clone();
                }
                if let Some(max_steps) = ov.max_steps {
                    agent.max_steps = Some(max_steps);
                }
                if let Some(timeout_secs) = ov.timeout_secs {
                    agent.timeout_secs = Some(timeout_secs);
                }
                if let Some(max_retries) = ov.max_retries {
                    agent.max_retries = Some(max_retries);
                }
                if let Some(top_p) = ov.top_p {
                    agent.top_p = Some(top_p);
                }
            }
        }
    }

    /// Get an agent by name (even if disabled).
    pub fn get(&self, name: &str) -> Option<&AgentDef> {
        self.agents.get(name)
    }

    /// All agent names, sorted alphabetically.
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.agents.keys().cloned().collect();
        names.sort();
        names
    }

    /// Agents the user can Tab-cycle through: enabled, mode == Primary.
    pub fn cyclable_agents(&self) -> Vec<&AgentDef> {
        let mut results: Vec<&AgentDef> = self
            .agents
            .values()
            .filter(|a| a.enabled && a.mode == AgentMode::Primary)
            .collect();
        results.sort_by(|a, b| a.name.cmp(&b.name));
        results
    }

    /// Agents visible in @mention autocomplete: enabled and not hidden.
    pub fn visible_agents(&self) -> Vec<&AgentDef> {
        let mut results: Vec<&AgentDef> = self
            .agents
            .values()
            .filter(|a| a.enabled && !a.hidden)
            .collect();
        results.sort_by(|a, b| a.name.cmp(&b.name));
        results
    }

    /// Search enabled, non-hidden agents by name prefix. Empty query returns all visible.
    pub fn search(&self, query: &str) -> Vec<&AgentDef> {
        let mut results: Vec<&AgentDef> = self
            .agents
            .values()
            .filter(|a| a.enabled && !a.hidden && a.name.starts_with(query))
            .collect();
        results.sort_by(|a, b| a.name.cmp(&b.name));
        results
    }

    /// Names of all enabled, non-hidden agents (for passing to `parse_input` as `known_agents`).
    pub fn enabled_names(&self) -> Vec<String> {
        self.search("").iter().map(|a| a.name.clone()).collect()
    }
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_def::AgentSource;
    use std::io::Write as _;

    #[test]
    fn new_has_builtins() {
        let reg = AgentRegistry::new();
        assert!(reg.get("coder").is_some());
        assert!(reg.get("explore").is_some());
        assert!(reg.get("planner").is_some());
        assert!(reg.get("orchestrator").is_some());
    }

    #[test]
    fn names_returns_sorted() {
        let reg = AgentRegistry::new();
        let names = reg.names();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
    }

    #[test]
    fn search_filters_by_prefix() {
        let reg = AgentRegistry::new();
        let results = reg.search("ex");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "explore");
    }

    #[test]
    fn search_empty_returns_all_enabled() {
        let reg = AgentRegistry::new();
        let results = reg.search("");
        // All 4 builtins are enabled and none are hidden.
        assert_eq!(results.len(), 4);
    }

    #[test]
    fn user_agent_overrides_builtin() {
        let dir = tempfile::tempdir().unwrap();
        let agent_path = dir.path().join("explore.md");
        let mut f = std::fs::File::create(&agent_path).unwrap();
        write!(
            f,
            "---\ndescription: \"Custom explorer\"\n---\n\nCustom prompt."
        )
        .unwrap();

        let mut reg = AgentRegistry::new();
        reg.discover_user_agents(dir.path());
        let explore = reg.get("explore").unwrap();
        assert_eq!(explore.description, "Custom explorer");
        assert_eq!(explore.source, AgentSource::User);
    }

    #[test]
    fn apply_config_overrides_model_and_enabled() {
        let mut reg = AgentRegistry::new();
        let mut overrides = HashMap::new();
        overrides.insert(
            "explore".to_string(),
            AgentConfigOverride {
                model: Some("anthropic/claude-haiku-4-5".to_string()),
                enabled: Some(false),
                ..Default::default()
            },
        );
        reg.apply_overrides(&overrides);

        let explore = reg.get("explore").unwrap();
        assert_eq!(explore.model.as_deref(), Some("anthropic/claude-haiku-4-5"));
        assert!(!explore.enabled);
    }

    #[test]
    fn disabled_agents_excluded_from_search() {
        let mut reg = AgentRegistry::new();
        let mut overrides = HashMap::new();
        overrides.insert(
            "explore".to_string(),
            AgentConfigOverride {
                enabled: Some(false),
                ..Default::default()
            },
        );
        reg.apply_overrides(&overrides);

        let results = reg.search("");
        assert!(!results.iter().any(|a| a.name == "explore"));
    }

    #[test]
    fn enabled_names_matches_search() {
        let reg = AgentRegistry::new();
        let names = reg.enabled_names();
        let search_names: Vec<String> = reg.search("").iter().map(|a| a.name.clone()).collect();
        assert_eq!(names, search_names);
    }

    #[test]
    fn cyclable_agents_excludes_subagents() {
        let reg = AgentRegistry::new();
        let cyclable = reg.cyclable_agents();
        // explore is Subagent, so only coder, orchestrator, planner
        assert_eq!(cyclable.len(), 3);
        assert!(!cyclable.iter().any(|a| a.name == "explore"));
    }

    #[test]
    fn visible_agents_excludes_hidden() {
        let mut reg = AgentRegistry::new();
        let mut overrides = HashMap::new();
        overrides.insert(
            "explore".to_string(),
            AgentConfigOverride {
                hidden: Some(true),
                ..Default::default()
            },
        );
        reg.apply_overrides(&overrides);

        let visible = reg.visible_agents();
        assert!(!visible.iter().any(|a| a.name == "explore"));
        assert_eq!(visible.len(), 3);
    }

    #[test]
    fn search_excludes_hidden() {
        let mut reg = AgentRegistry::new();
        let mut overrides = HashMap::new();
        overrides.insert(
            "explore".to_string(),
            AgentConfigOverride {
                hidden: Some(true),
                ..Default::default()
            },
        );
        reg.apply_overrides(&overrides);

        let results = reg.search("ex");
        assert!(results.is_empty());
    }

    #[test]
    fn apply_overrides_mode_and_permissions() {
        use crate::agent_def::{PermissionAction, PermissionEntry};

        let mut reg = AgentRegistry::new();
        let mut overrides = HashMap::new();
        overrides.insert(
            "coder".to_string(),
            AgentConfigOverride {
                mode: Some(AgentMode::Subagent),
                permissions: Some(HashMap::from([(
                    "bash".to_string(),
                    PermissionEntry::Flat(PermissionAction::Deny),
                )])),
                ..Default::default()
            },
        );
        reg.apply_overrides(&overrides);

        let coder = reg.get("coder").unwrap();
        assert_eq!(coder.mode, AgentMode::Subagent);
        assert_eq!(coder.permissions.len(), 1);
    }

    #[test]
    fn apply_overrides_max_steps_and_timeout() {
        let mut reg = AgentRegistry::new();
        let mut overrides = HashMap::new();
        overrides.insert(
            "explore".to_string(),
            AgentConfigOverride {
                max_steps: Some(5),
                timeout_secs: Some(120),
                max_retries: Some(2),
                top_p: Some(0.95),
                ..Default::default()
            },
        );
        reg.apply_overrides(&overrides);

        let explore = reg.get("explore").unwrap();
        assert_eq!(explore.max_steps, Some(5));
        assert_eq!(explore.timeout_secs, Some(120));
        assert_eq!(explore.max_retries, Some(2));
        assert_eq!(explore.top_p, Some(0.95));
    }

    #[test]
    fn default_agent_is_coder() {
        let reg = AgentRegistry::new();
        assert_eq!(reg.default_agent_name(), "coder");
        assert!(reg.default_agent().is_some());
    }

    #[test]
    fn set_default_agent_works() {
        let mut reg = AgentRegistry::new();
        assert!(reg.set_default_agent("explore"));
        assert_eq!(reg.default_agent_name(), "explore");
    }

    #[test]
    fn set_default_agent_rejects_unknown() {
        let mut reg = AgentRegistry::new();
        assert!(!reg.set_default_agent("nonexistent"));
        assert_eq!(reg.default_agent_name(), "coder");
    }
}
