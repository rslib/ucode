use std::collections::{HashMap, HashSet};

use crate::Skill;
use crate::parser::SkillError;

/// Controls which tools a skill permits.
#[derive(Debug, Clone)]
pub enum ToolFilter {
    /// No restriction — all tools are permitted.
    AllowAll,
    /// Only the named tools are permitted.
    AllowList(HashSet<String>),
}

impl ToolFilter {
    /// Returns `true` when `tool_name` is permitted by this filter.
    pub fn is_allowed(&self, tool_name: &str) -> bool {
        match self {
            ToolFilter::AllowAll => true,
            ToolFilter::AllowList(set) => set.contains(tool_name),
        }
    }
}

/// Runtime binding for an active skill — encapsulates the effects it has on
/// system prompt construction, tool access, and model routing.
#[derive(Debug, Clone)]
pub struct SkillBinding {
    skill: Skill,
}

impl SkillBinding {
    pub fn new(skill: Skill) -> Self {
        Self { skill }
    }

    /// Formats the skill as a system prompt prefix section.
    ///
    /// ```text
    /// [Active Skill: {name}]
    /// {description}
    ///
    /// {instructions}
    /// ```
    pub fn system_prompt_prefix(&self) -> String {
        format!(
            "[Active Skill: {}]\n{}\n\n{}",
            self.skill.name, self.skill.description, self.skill.instructions
        )
    }

    /// Returns a [`ToolFilter`] derived from the skill's `tool_allowlist`.
    ///
    /// An empty allowlist means the skill imposes no tool restrictions.
    pub fn tool_filter(&self) -> ToolFilter {
        let allowlist = self
            .skill
            .ucode
            .as_ref()
            .map(|u| u.tool_allowlist.as_slice())
            .unwrap_or(&[]);

        if allowlist.is_empty() {
            ToolFilter::AllowAll
        } else {
            ToolFilter::AllowList(allowlist.iter().cloned().collect())
        }
    }

    /// Returns the routing hints map (may be empty).
    pub fn routing_hints(&self) -> &HashMap<String, String> {
        self.skill
            .ucode
            .as_ref()
            .map(|u| &u.routing_hints)
            .unwrap_or_else(|| {
                // SAFETY: we need a reference to an empty map with 'self lifetime.
                // The only way to do this without storing a field is a static.
                static EMPTY: std::sync::OnceLock<HashMap<String, String>> =
                    std::sync::OnceLock::new();
                EMPTY.get_or_init(HashMap::new)
            })
    }

    /// Convenience: returns `routing_hints["model_group"]` if present.
    pub fn preferred_model_group(&self) -> Option<&str> {
        self.routing_hints().get("model_group").map(String::as_str)
    }

    pub fn skill(&self) -> &Skill {
        &self.skill
    }

    pub fn name(&self) -> &str {
        &self.skill.name
    }
}

/// Manages the set of available skills and tracks which one (if any) is active.
#[derive(Debug)]
pub struct SkillManager {
    available: Vec<Skill>,
    active: Option<SkillBinding>,
}

impl SkillManager {
    /// Creates a manager with the given skill set; no skill is active initially.
    pub fn new(skills: Vec<Skill>) -> Self {
        Self {
            available: skills,
            active: None,
        }
    }

    /// Returns the full list of available skills.
    pub fn available(&self) -> &[Skill] {
        &self.available
    }

    /// Activates the skill with the given `name`.
    ///
    /// Returns a reference to the new [`SkillBinding`], or
    /// [`SkillError::NotFound`] when no skill with that name exists.
    pub fn activate(&mut self, name: &str) -> Result<&SkillBinding, SkillError> {
        let skill = self
            .available
            .iter()
            .find(|s| s.name == name)
            .ok_or_else(|| SkillError::NotFound {
                name: name.to_owned(),
            })?
            .clone();

        self.active = Some(SkillBinding::new(skill));
        // SAFETY: we just assigned Some above.
        Ok(self.active.as_ref().expect("just set"))
    }

    /// Clears the active skill.
    pub fn deactivate(&mut self) {
        self.active = None;
    }

    /// Returns the active binding, if any.
    pub fn active(&self) -> Option<&SkillBinding> {
        self.active.as_ref()
    }

    /// Returns the active skill's tool filter, or [`ToolFilter::AllowAll`] when
    /// no skill is active.
    pub fn active_tool_filter(&self) -> ToolFilter {
        self.active
            .as_ref()
            .map(|b| b.tool_filter())
            .unwrap_or(ToolFilter::AllowAll)
    }

    /// Returns the active skill's system prompt prefix, or `None` when no skill
    /// is active.
    pub fn active_system_prefix(&self) -> Option<String> {
        self.active.as_ref().map(|b| b.system_prompt_prefix())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::*;
    use crate::{Skill, UcodeSkillConfig};

    fn make_skill(name: &str, description: &str, instructions: &str) -> Skill {
        Skill {
            name: name.to_owned(),
            description: description.to_owned(),
            instructions: instructions.to_owned(),
            source: PathBuf::from(format!("/fake/{name}/SKILL.md")),
            ucode: None,
        }
    }

    fn make_skill_with_ucode(
        name: &str,
        description: &str,
        instructions: &str,
        tool_allowlist: Vec<String>,
        routing_hints: HashMap<String, String>,
    ) -> Skill {
        Skill {
            name: name.to_owned(),
            description: description.to_owned(),
            instructions: instructions.to_owned(),
            source: PathBuf::from(format!("/fake/{name}/SKILL.md")),
            ucode: Some(UcodeSkillConfig {
                tool_allowlist,
                routing_hints,
            }),
        }
    }

    // 1
    #[test]
    fn test_system_prompt_prefix_format() {
        let skill = make_skill("my-skill", "Does things", "Step 1\nStep 2");
        let binding = SkillBinding::new(skill);
        let prefix = binding.system_prompt_prefix();
        assert!(
            prefix.contains("[Active Skill: my-skill]"),
            "missing header"
        );
        assert!(prefix.contains("Does things"), "missing description");
        assert!(prefix.contains("Step 1\nStep 2"), "missing instructions");
    }

    // 2
    #[test]
    fn test_system_prompt_prefix_empty_instructions() {
        let skill = make_skill("empty-skill", "No body", "");
        let binding = SkillBinding::new(skill);
        let prefix = binding.system_prompt_prefix();
        assert!(prefix.contains("[Active Skill: empty-skill]"));
        assert!(prefix.contains("No body"));
        // Should not panic and should still be well-formed.
        assert!(prefix.starts_with("[Active Skill:"));
    }

    // 3
    #[test]
    fn test_tool_filter_allow_all() {
        assert!(ToolFilter::AllowAll.is_allowed("anything"));
        assert!(ToolFilter::AllowAll.is_allowed("run_cmd"));
        assert!(ToolFilter::AllowAll.is_allowed(""));
    }

    // 4
    #[test]
    fn test_tool_filter_allowlist_match() {
        let filter = ToolFilter::AllowList(
            ["read_file".to_owned(), "search".to_owned()]
                .into_iter()
                .collect(),
        );
        assert!(filter.is_allowed("read_file"));
        assert!(filter.is_allowed("search"));
    }

    // 5
    #[test]
    fn test_tool_filter_allowlist_deny() {
        let filter = ToolFilter::AllowList(["read_file".to_owned()].into_iter().collect());
        assert!(!filter.is_allowed("run_cmd"));
        assert!(!filter.is_allowed("write_file"));
    }

    // 6
    #[test]
    fn test_tool_filter_empty_allowlist_means_all() {
        let skill = make_skill_with_ucode("no-restrict", "desc", "", vec![], HashMap::new());
        let binding = SkillBinding::new(skill);
        assert!(
            matches!(binding.tool_filter(), ToolFilter::AllowAll),
            "empty allowlist should produce AllowAll"
        );
    }

    // 7
    #[test]
    fn test_routing_hints_present() {
        let mut hints = HashMap::new();
        hints.insert("model_group".to_owned(), "strong".to_owned());
        hints.insert("priority".to_owned(), "high".to_owned());
        let skill = make_skill_with_ucode("hint-skill", "desc", "", vec![], hints.clone());
        let binding = SkillBinding::new(skill);
        assert_eq!(binding.routing_hints(), &hints);
    }

    // 8
    #[test]
    fn test_routing_hints_empty() {
        let skill = make_skill("plain", "desc", "");
        let binding = SkillBinding::new(skill);
        assert!(binding.routing_hints().is_empty());
    }

    // 9
    #[test]
    fn test_preferred_model_group_some() {
        let mut hints = HashMap::new();
        hints.insert("model_group".to_owned(), "strong".to_owned());
        let skill = make_skill_with_ucode("mg-skill", "desc", "", vec![], hints);
        let binding = SkillBinding::new(skill);
        assert_eq!(binding.preferred_model_group(), Some("strong"));
    }

    // 10
    #[test]
    fn test_preferred_model_group_none() {
        let skill = make_skill("no-mg", "desc", "");
        let binding = SkillBinding::new(skill);
        assert_eq!(binding.preferred_model_group(), None);
    }

    // 11
    #[test]
    fn test_skill_manager_activate() {
        let skills = vec![
            make_skill("alpha", "Alpha skill", "do alpha"),
            make_skill("beta", "Beta skill", "do beta"),
        ];
        let mut mgr = SkillManager::new(skills);
        let binding = mgr.activate("alpha").expect("alpha should be found");
        assert_eq!(binding.name(), "alpha");
        assert!(mgr.active().is_some());
    }

    // 12
    #[test]
    fn test_skill_manager_activate_not_found() {
        let mut mgr = SkillManager::new(vec![make_skill("only", "desc", "")]);
        let err = mgr.activate("missing").unwrap_err();
        assert!(
            matches!(&err, SkillError::NotFound { name } if name == "missing"),
            "expected NotFound, got: {err}"
        );
    }

    // 13
    #[test]
    fn test_skill_manager_deactivate() {
        let mut mgr = SkillManager::new(vec![make_skill("s", "d", "")]);
        mgr.activate("s").unwrap();
        assert!(mgr.active().is_some());
        mgr.deactivate();
        assert!(mgr.active().is_none());
    }

    // 14
    #[test]
    fn test_skill_manager_active_tool_filter_no_skill() {
        let mgr = SkillManager::new(vec![]);
        assert!(
            matches!(mgr.active_tool_filter(), ToolFilter::AllowAll),
            "no active skill should yield AllowAll"
        );
    }

    // 15
    #[test]
    fn test_skill_manager_active_system_prefix() {
        let mut mgr = SkillManager::new(vec![make_skill("s", "desc", "instructions")]);
        assert!(mgr.active_system_prefix().is_none(), "no active skill yet");
        mgr.activate("s").unwrap();
        let prefix = mgr.active_system_prefix().expect("should have prefix now");
        assert!(prefix.contains("[Active Skill: s]"));
        assert!(prefix.contains("instructions"));
    }

    // 16
    #[test]
    fn test_skill_manager_switch_skill() {
        let skills = vec![
            make_skill("first", "First", "first instructions"),
            make_skill("second", "Second", "second instructions"),
        ];
        let mut mgr = SkillManager::new(skills);

        mgr.activate("first").unwrap();
        assert_eq!(mgr.active().unwrap().name(), "first");

        mgr.activate("second").unwrap();
        assert_eq!(mgr.active().unwrap().name(), "second");

        let prefix = mgr.active_system_prefix().unwrap();
        assert!(prefix.contains("second instructions"));
        assert!(!prefix.contains("first instructions"));
    }
}
