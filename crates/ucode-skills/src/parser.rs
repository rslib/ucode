use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::{Skill, UcodeSkillConfig};

/// Raw frontmatter shape — unknown keys are silently ignored by serde.
///
/// `serde_yml` (like serde_yaml) ignores unknown fields by default when
/// `deny_unknown_fields` is absent, so no flatten trick is needed.
#[derive(Debug, Deserialize)]
struct Frontmatter {
    name: Option<String>,
    description: Option<String>,
    ucode: Option<RawUcodeConfig>,
}

#[derive(Debug, Deserialize)]
struct RawUcodeConfig {
    #[serde(default)]
    tool_allowlist: Vec<String>,
    #[serde(default)]
    routing_hints: HashMap<String, String>,
}

/// Split `---`-delimited YAML frontmatter from markdown content.
///
/// Returns `Some((yaml_str, body_str))` when the file starts with `---\n`
/// and contains a closing `---` line; `None` otherwise.
fn split_frontmatter(content: &str) -> Option<(&str, &str)> {
    let rest = content
        .strip_prefix("---\n")
        .or_else(|| content.strip_prefix("---\r\n"))?;

    // Find the closing delimiter.
    let close = rest.find("\n---\n").or_else(|| rest.find("\n---\r\n"));
    if let Some(pos) = close {
        let yaml = &rest[..pos];
        let after = &rest[pos + "\n---\n".len()..];
        return Some((yaml, after));
    }

    // Handle trailing `---` at end of file (no newline after).
    if let Some(pos) = rest.find("\n---") {
        let tail = &rest[pos + "\n---".len()..];
        if tail.trim().is_empty() {
            return Some((&rest[..pos], ""));
        }
    }

    None
}

/// Parse a SKILL.md file into a [`Skill`].
///
/// Returns [`SkillError::MissingField`] when `name` or `description` are absent,
/// [`SkillError::Parse`] when the YAML is malformed, and [`SkillError::Io`] on
/// read failures.
pub fn parse_skill(path: &Path) -> Result<Skill, SkillError> {
    let content = std::fs::read_to_string(path).map_err(|e| SkillError::Io {
        path: path.to_owned(),
        source: e,
    })?;

    let (yaml, body) = split_frontmatter(&content).ok_or_else(|| SkillError::Parse {
        path: path.to_owned(),
        message: "no YAML frontmatter found (file must start with ---)".into(),
    })?;

    let fm: Frontmatter = serde_yml::from_str(yaml).map_err(|e| SkillError::Parse {
        path: path.to_owned(),
        message: e.to_string(),
    })?;

    let name = fm.name.ok_or_else(|| SkillError::MissingField {
        path: path.to_owned(),
        field: "name".into(),
    })?;

    let description = fm.description.ok_or_else(|| SkillError::MissingField {
        path: path.to_owned(),
        field: "description".into(),
    })?;

    let ucode = fm.ucode.map(|raw| UcodeSkillConfig {
        tool_allowlist: raw.tool_allowlist,
        routing_hints: raw.routing_hints,
    });

    Ok(Skill {
        name,
        description,
        instructions: body.trim().to_owned(),
        source: path.to_owned(),
        ucode,
    })
}

/// Discover and parse all skills from standard paths.
///
/// Each entry is `Ok(Skill)` on success or `Err(SkillError)` for files that
/// fail to parse, so callers can log bad files without aborting.
pub fn load_all_skills(project_root: &Path, user_config: &Path) -> Vec<Result<Skill, SkillError>> {
    crate::discover_skills(project_root, user_config)
        .into_iter()
        .map(|p| parse_skill(&p))
        .collect()
}

/// Canonical error type for skill operations.
#[derive(Debug, thiserror::Error)]
pub enum SkillError {
    #[error("failed to read skill file '{path}': {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("invalid frontmatter in '{path}': {message}")]
    Parse { path: PathBuf, message: String },

    #[error("missing required field '{field}' in '{path}'")]
    MissingField { path: PathBuf, field: String },
}
