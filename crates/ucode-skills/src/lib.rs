//! ucode-skills: SKILL.md discovery and parsing (Claude Code + OpenCode compatible)

pub mod binding;
pub mod discovery;
pub mod parser;

pub use binding::{SkillBinding, SkillManager, ToolFilter};
pub use discovery::discover_skills;
pub use parser::{SkillError, load_all_skills, parse_skill};

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A parsed skill definition from a SKILL.md file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    /// Skill name (from frontmatter).
    pub name: String,
    /// Short description (from frontmatter).
    pub description: String,
    /// Full instruction text (markdown body after frontmatter).
    pub instructions: String,
    /// Source file path.
    pub source: PathBuf,
    /// Optional ucode-specific extensions.
    pub ucode: Option<UcodeSkillConfig>,
}

/// Optional ucode-specific configuration from frontmatter.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UcodeSkillConfig {
    /// Tool names this skill allows (empty = all tools).
    #[serde(default)]
    pub tool_allowlist: Vec<String>,
    /// Routing hints (e.g., preferred model group).
    #[serde(default)]
    pub routing_hints: HashMap<String, String>,
}
