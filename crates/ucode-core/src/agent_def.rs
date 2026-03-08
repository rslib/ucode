use std::collections::HashMap;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// Whether an agent can be selected directly by the user or only invoked by other agents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentMode {
    /// User can select this agent directly (Tab cycles through these).
    #[default]
    Primary,
    /// Only callable by other agents — not shown in Tab cycle.
    Subagent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentSource {
    BuiltIn,
    User,
}

impl AgentSource {
    pub fn badge(&self) -> &'static str {
        match self {
            AgentSource::BuiltIn => "[built-in]",
            AgentSource::User => "[user]",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolPermissions {
    pub read: bool,
    pub edit: bool,
    pub write: bool,
    pub bash: bool,
    pub glob: bool,
    pub grep: bool,
    pub list: bool,
}

impl Default for ToolPermissions {
    fn default() -> Self {
        Self {
            read: true,
            edit: true,
            write: true,
            bash: true,
            glob: true,
            grep: true,
            list: true,
        }
    }
}

/// What to do when a tool action is requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionAction {
    Allow,
    Ask,
    Deny,
}

/// A permission entry: either a flat action or a map of glob patterns to actions.
/// Flat: `"bash": "allow"`
/// Granular: `"bash": { "*": "ask", "git *": "allow", "rm *": "deny" }`
/// Last matching rule wins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PermissionEntry {
    Flat(PermissionAction),
    Granular(IndexMap<String, PermissionAction>),
}

impl PermissionEntry {
    /// Resolve the permission for a given input string.
    /// For Flat, always returns the flat action.
    /// For Granular, evaluates rules in order; last matching glob wins.
    /// Returns None if no rule matches (Granular with no matching pattern).
    pub fn resolve(&self, input: &str) -> Option<PermissionAction> {
        match self {
            Self::Flat(action) => Some(*action),
            Self::Granular(rules) => {
                let mut result = None;
                for (pattern, action) in rules {
                    if glob_match(pattern, input) {
                        result = Some(*action);
                    }
                }
                result
            }
        }
    }
}

/// Simple glob matching: `*` matches any sequence of chars, `?` matches exactly one char.
fn glob_match(pattern: &str, input: &str) -> bool {
    let pat: Vec<char> = pattern.chars().collect();
    let inp: Vec<char> = input.chars().collect();
    glob_match_inner(&pat, &inp)
}

fn glob_match_inner(pat: &[char], inp: &[char]) -> bool {
    match (pat.first(), inp.first()) {
        (None, None) => true,
        (Some('*'), _) => {
            glob_match_inner(&pat[1..], inp)
                || (!inp.is_empty() && glob_match_inner(pat, &inp[1..]))
        }
        (Some('?'), Some(_)) => glob_match_inner(&pat[1..], &inp[1..]),
        (Some(a), Some(b)) if a == b => glob_match_inner(&pat[1..], &inp[1..]),
        _ => false,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDef {
    pub name: String,
    pub description: String,
    pub system_prompt: String,
    pub color: ucode_themes::Rgb,
    pub model: Option<String>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub enabled: bool,
    #[serde(default)]
    pub mode: AgentMode,
    #[serde(default)]
    pub hidden: bool,
    pub tools: ToolPermissions,
    #[serde(default)]
    pub permissions: HashMap<String, PermissionEntry>,
    pub max_steps: Option<u32>,
    pub timeout_secs: Option<u64>,
    pub max_retries: Option<u32>,
    #[serde(default = "default_source")]
    pub source: AgentSource,
}

/// Parse a hex color string like `"#ff6b6b"` or `"#FFF"` to `Rgb`.
pub fn parse_hex_color(s: &str) -> Result<ucode_themes::Rgb, String> {
    let s = s.strip_prefix('#').unwrap_or(s);
    match s.len() {
        6 => {
            let r = u8::from_str_radix(&s[0..2], 16).map_err(|e| e.to_string())?;
            let g = u8::from_str_radix(&s[2..4], 16).map_err(|e| e.to_string())?;
            let b = u8::from_str_radix(&s[4..6], 16).map_err(|e| e.to_string())?;
            Ok(ucode_themes::Rgb::new(r, g, b))
        }
        3 => {
            let r = u8::from_str_radix(&s[0..1], 16).map_err(|e| e.to_string())?;
            let g = u8::from_str_radix(&s[1..2], 16).map_err(|e| e.to_string())?;
            let b = u8::from_str_radix(&s[2..3], 16).map_err(|e| e.to_string())?;
            Ok(ucode_themes::Rgb::new(r * 17, g * 17, b * 17))
        }
        _ => Err(format!("invalid hex color: #{s}")),
    }
}

/// RGB to HSL conversion. Returns (h: 0..360, s: 0..1, l: 0..1).
fn rgb_to_hsl(c: ucode_themes::Rgb) -> (f32, f32, f32) {
    let r = c.r as f32 / 255.0;
    let g = c.g as f32 / 255.0;
    let b = c.b as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;

    if (max - min).abs() < f32::EPSILON {
        return (0.0, 0.0, l);
    }

    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };

    let h = if (max - r).abs() < f32::EPSILON {
        let mut h = (g - b) / d;
        if g < b {
            h += 6.0;
        }
        h
    } else if (max - g).abs() < f32::EPSILON {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };

    (h * 60.0, s, l)
}

/// HSL to RGB conversion. h: 0..360, s: 0..1, l: 0..1.
fn hsl_to_rgb(h: f32, s: f32, l: f32) -> ucode_themes::Rgb {
    if s.abs() < f32::EPSILON {
        let v = (l * 255.0).round() as u8;
        return ucode_themes::Rgb::new(v, v, v);
    }

    let h = ((h % 360.0) + 360.0) % 360.0;
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;

    let sector = (h / 60.0) as u32;
    let (r1, g1, b1) = match sector {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };

    ucode_themes::Rgb::new(
        ((r1 + m) * 255.0).round() as u8,
        ((g1 + m) * 255.0).round() as u8,
        ((b1 + m) * 255.0).round() as u8,
    )
}

/// Generate a deterministic color from an agent name (theme-independent fallback).
///
/// Uses golden-angle hue rotation with fixed saturation/lightness suitable
/// for dark terminals.
pub fn auto_color(name: &str) -> ucode_themes::Rgb {
    let h = name_to_hue(name, 0.0);
    hsl_to_rgb(h, 0.65, 0.65)
}

/// Generate a deterministic, beautiful agent color from the theme.
///
/// Uses golden-angle hue rotation starting from the theme's accent color,
/// with saturation and lightness tuned to the theme's brightness.
/// Scales to arbitrary numbers of agents with good visual separation.
pub fn theme_agent_color(name: &str, theme: &ucode_themes::ThemeDef) -> ucode_themes::Rgb {
    let (accent_h, _accent_s, _accent_l) = rgb_to_hsl(theme.accent);

    let (s, l) = if theme.is_dark() {
        (0.65, 0.65) // Vibrant but not neon on dark backgrounds
    } else {
        (0.55, 0.45) // Richer, darker tones on light backgrounds
    };

    let h = name_to_hue(name, accent_h);
    hsl_to_rgb(h, s, l)
}

/// Compute a golden-angle hue offset for `name`, starting from `base_hue`.
///
/// Performs the rotation in millidegrees (integer arithmetic) to avoid f32
/// precision loss when the hash value is large.
fn name_to_hue(name: &str, base_hue: f32) -> f32 {
    // Golden angle in millidegrees (137.508° × 1000), kept as u64 to avoid overflow.
    const GOLDEN_MDEG: u64 = 137_508;
    const CIRCLE_MDEG: u64 = 360_000;

    let hash = name
        .bytes()
        .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));

    let offset_mdeg = (GOLDEN_MDEG.wrapping_mul(hash)) % CIRCLE_MDEG;
    let base_mdeg = (base_hue * 1000.0) as u64 % CIRCLE_MDEG;
    let h_mdeg = (base_mdeg + offset_mdeg) % CIRCLE_MDEG;

    h_mdeg as f32 / 1000.0
}

fn default_source() -> AgentSource {
    AgentSource::User
}

#[derive(Debug, Deserialize)]
struct AgentFrontmatter {
    description: String,
    color: Option<String>,
    model: Option<String>,
    temperature: Option<f32>,
    top_p: Option<f32>,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    mode: AgentMode,
    #[serde(default)]
    hidden: bool,
    #[serde(default)]
    tools: ToolPermissions,
    #[serde(default)]
    permissions: HashMap<String, PermissionEntry>,
    max_steps: Option<u32>,
    timeout_secs: Option<u64>,
    max_retries: Option<u32>,
}

fn default_true() -> bool {
    true
}

impl AgentDef {
    pub fn from_markdown(name: &str, content: &str) -> Result<Self, String> {
        // Content must start with "---" to have frontmatter
        let rest = content
            .strip_prefix("---")
            .ok_or("missing opening --- delimiter")?;

        // Find the closing "---"
        let (yaml_part, body) = rest
            .split_once("\n---")
            .ok_or("missing closing --- delimiter")?;

        let fm: AgentFrontmatter = serde_yaml::from_str(yaml_part)
            .map_err(|e| format!("invalid frontmatter YAML: {e}"))?;

        // Strip leading newline from body
        let system_prompt = body.trim_start_matches('\n').to_owned();
        let color = match fm.color {
            Some(hex) => {
                parse_hex_color(&hex).map_err(|e| format!("invalid color in frontmatter: {e}"))?
            }
            None => auto_color(name),
        };

        Ok(AgentDef {
            name: name.to_owned(),
            description: fm.description,
            color,
            model: fm.model,
            temperature: fm.temperature,
            top_p: fm.top_p,
            enabled: fm.enabled,
            mode: fm.mode,
            hidden: fm.hidden,
            tools: fm.tools,
            permissions: fm.permissions,
            max_steps: fm.max_steps,
            timeout_secs: fm.timeout_secs,
            max_retries: fm.max_retries,
            system_prompt,
            source: AgentSource::User,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_def_defaults() {
        let def = AgentDef {
            name: "explore".into(),
            description: "Fast codebase explorer".into(),
            system_prompt: "You are a read-only explorer.".into(),
            color: ucode_themes::Rgb::new(0x4e, 0xc9, 0xb0),
            model: None,
            temperature: None,
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
        };
        assert_eq!(def.name, "explore");
        assert!(def.enabled);
        assert!(def.model.is_none());
        assert_eq!(def.source, AgentSource::BuiltIn);
        assert!(!def.hidden);
        assert!(def.permissions.is_empty());
        assert!(def.max_steps.is_none());
        assert!(def.timeout_secs.is_none());
        assert!(def.max_retries.is_none());
        assert!(def.top_p.is_none());
    }

    #[test]
    fn tool_permissions_default_all_true() {
        let tp = ToolPermissions::default();
        assert!(tp.read);
        assert!(tp.edit);
        assert!(tp.write);
        assert!(tp.bash);
        assert!(tp.glob);
        assert!(tp.grep);
    }

    #[test]
    fn tool_permissions_default_includes_list() {
        let tp = ToolPermissions::default();
        assert!(tp.list);
    }

    #[test]
    fn agent_source_display() {
        assert_eq!(AgentSource::BuiltIn.badge(), "[built-in]");
        assert_eq!(AgentSource::User.badge(), "[user]");
    }

    #[test]
    fn parse_agent_markdown_full() {
        let md = r#"---
description: "Fast codebase explorer"
model: "anthropic/claude-haiku-4-5"
temperature: 0.1
tools:
  read: true
  edit: false
  write: false
  bash: true
  glob: true
  grep: true
  list: true
---

You are a read-only explorer."#;

        let def = AgentDef::from_markdown("explore", md).unwrap();
        assert_eq!(def.name, "explore");
        assert_eq!(def.description, "Fast codebase explorer");
        assert_eq!(def.model.as_deref(), Some("anthropic/claude-haiku-4-5"));
        assert!(!def.tools.edit);
        assert!(def.tools.read);
        assert_eq!(def.system_prompt.trim(), "You are a read-only explorer.");
        assert_eq!(def.source, AgentSource::User);
    }

    #[test]
    fn parse_agent_markdown_minimal() {
        let md = r#"---
description: "Simple agent"
---

Do stuff."#;

        let def = AgentDef::from_markdown("simple", md).unwrap();
        assert_eq!(def.name, "simple");
        assert!(def.tools.edit); // default true
        assert!(def.model.is_none());
        assert!(def.enabled); // default true
    }

    #[test]
    fn parse_agent_markdown_subagent_mode() {
        let md = r#"---
description: "Rust expert"
mode: subagent
---

You are a Rust expert."#;

        let def = AgentDef::from_markdown("rust-expert", md).unwrap();
        assert_eq!(def.mode, AgentMode::Subagent);
    }

    #[test]
    fn parse_agent_markdown_default_mode_is_primary() {
        let md = r#"---
description: "Simple agent"
---

Do stuff."#;

        let def = AgentDef::from_markdown("simple", md).unwrap();
        assert_eq!(def.mode, AgentMode::Primary);
    }

    #[test]
    fn parse_agent_markdown_no_frontmatter() {
        let md = "Just a system prompt with no frontmatter.";
        let result = AgentDef::from_markdown("bad", md);
        assert!(result.is_err());
    }

    #[test]
    fn permission_entry_flat_resolve() {
        let entry = PermissionEntry::Flat(PermissionAction::Allow);
        assert_eq!(entry.resolve("anything"), Some(PermissionAction::Allow));
    }

    #[test]
    fn permission_entry_granular_last_match_wins() {
        let mut rules = IndexMap::new();
        rules.insert("*".to_string(), PermissionAction::Ask);
        rules.insert("git *".to_string(), PermissionAction::Allow);
        rules.insert("rm *".to_string(), PermissionAction::Deny);
        let entry = PermissionEntry::Granular(rules);

        assert_eq!(entry.resolve("git status"), Some(PermissionAction::Allow));
        assert_eq!(entry.resolve("rm -rf /"), Some(PermissionAction::Deny));
        assert_eq!(entry.resolve("echo hello"), Some(PermissionAction::Ask));
    }

    #[test]
    fn permission_entry_granular_no_match() {
        let mut rules = IndexMap::new();
        rules.insert("git *".to_string(), PermissionAction::Allow);
        let entry = PermissionEntry::Granular(rules);
        assert_eq!(entry.resolve("echo hello"), None);
    }

    #[test]
    fn glob_match_star() {
        assert!(glob_match("git *", "git status"));
        assert!(glob_match("git *", "git push origin main"));
        assert!(!glob_match("git *", "echo hello"));
        assert!(glob_match("*", "anything"));
        assert!(glob_match("*", ""));
    }

    #[test]
    fn glob_match_question() {
        assert!(glob_match("?.txt", "a.txt"));
        assert!(!glob_match("?.txt", "ab.txt"));
    }

    #[test]
    fn glob_match_exact() {
        assert!(glob_match("hello", "hello"));
        assert!(!glob_match("hello", "world"));
    }

    #[test]
    fn parse_agent_markdown_with_permissions() {
        let md = r#"---
description: "Plan agent"
mode: primary
permissions:
  bash:
    "*": "ask"
    "git *": "allow"
  edit: "deny"
---

You are a planning agent."#;

        let def = AgentDef::from_markdown("plan", md).unwrap();
        assert_eq!(def.mode, AgentMode::Primary);
        assert_eq!(def.permissions.len(), 2);

        // Check flat permission
        let edit_perm = &def.permissions["edit"];
        assert_eq!(edit_perm.resolve("anything"), Some(PermissionAction::Deny));

        // Check granular permission
        let bash_perm = &def.permissions["bash"];
        assert_eq!(
            bash_perm.resolve("git status"),
            Some(PermissionAction::Allow)
        );
        assert_eq!(bash_perm.resolve("rm -rf /"), Some(PermissionAction::Ask));
    }

    #[test]
    fn parse_agent_markdown_with_new_fields() {
        let md = r#"---
description: "Test agent"
hidden: true
max_steps: 10
timeout_secs: 300
max_retries: 3
top_p: 0.9
---

Test prompt."#;

        let def = AgentDef::from_markdown("test", md).unwrap();
        assert!(def.hidden);
        assert_eq!(def.max_steps, Some(10));
        assert_eq!(def.timeout_secs, Some(300));
        assert_eq!(def.max_retries, Some(3));
        assert_eq!(def.top_p, Some(0.9));
    }

    #[test]
    fn auto_color_deterministic() {
        let c1 = auto_color("coder");
        let c2 = auto_color("coder");
        assert_eq!(c1, c2);
    }

    #[test]
    fn auto_color_different_names_differ() {
        let c1 = auto_color("coder");
        let c2 = auto_color("explore");
        assert_ne!(c1, c2);
    }

    #[test]
    fn parse_hex_color_6digit() {
        let c = parse_hex_color("#ff6b6b").unwrap();
        assert_eq!(c, ucode_themes::Rgb::new(0xff, 0x6b, 0x6b));
    }

    #[test]
    fn parse_hex_color_3digit() {
        let c = parse_hex_color("#fff").unwrap();
        assert_eq!(c, ucode_themes::Rgb::new(0xff, 0xff, 0xff));
    }

    #[test]
    fn parse_hex_color_no_hash() {
        let c = parse_hex_color("4fc1ff").unwrap();
        assert_eq!(c, ucode_themes::Rgb::new(0x4f, 0xc1, 0xff));
    }

    #[test]
    fn parse_hex_color_invalid() {
        assert!(parse_hex_color("xyz").is_err());
    }

    #[test]
    fn parse_agent_markdown_with_color() {
        let md = r##"---
description: "Colorful agent"
color: "#ff6b6b"
---

Colorful prompt."##;

        let def = AgentDef::from_markdown("colorful", md).unwrap();
        assert_eq!(def.color, ucode_themes::Rgb::new(0xff, 0x6b, 0x6b));
    }

    #[test]
    fn parse_agent_markdown_auto_color() {
        let md = r#"---
description: "No color specified"
---

Auto color."#;

        // auto_color always returns a valid Rgb — just check it round-trips to a hex string
        let def = AgentDef::from_markdown("myagent", md).unwrap();
        let hex = def.color.to_hex();
        assert!(hex.starts_with('#'));
        assert_eq!(hex.len(), 7);
    }

    #[test]
    fn theme_agent_color_deterministic() {
        let theme = ucode_themes::builtin_theme("ucode").unwrap();
        let c1 = theme_agent_color("coder", &theme);
        let c2 = theme_agent_color("coder", &theme);
        assert_eq!(c1, c2);
    }

    #[test]
    fn theme_agent_color_different_names() {
        let theme = ucode_themes::builtin_theme("ucode").unwrap();
        let c1 = theme_agent_color("coder", &theme);
        let c2 = theme_agent_color("explore", &theme);
        assert_ne!(c1, c2);
    }

    #[test]
    fn theme_agent_color_adapts_to_theme() {
        let ucode = ucode_themes::builtin_theme("ucode").unwrap();
        let dracula = ucode_themes::builtin_theme("dracula").unwrap();
        let c1 = theme_agent_color("coder", &ucode);
        let c2 = theme_agent_color("coder", &dracula);
        // Different themes have different accent hues, so colors should differ
        assert_ne!(c1, c2);
    }

    #[test]
    fn hsl_roundtrip() {
        let original = ucode_themes::Rgb::new(0x4f, 0xc1, 0xff);
        let (h, s, l) = rgb_to_hsl(original);
        let back = hsl_to_rgb(h, s, l);
        assert!((original.r as i16 - back.r as i16).abs() <= 1);
        assert!((original.g as i16 - back.g as i16).abs() <= 1);
        assert!((original.b as i16 - back.b as i16).abs() <= 1);
    }

    #[test]
    fn hsl_pure_colors() {
        let red = hsl_to_rgb(0.0, 1.0, 0.5);
        assert_eq!(red.r, 255);
        assert_eq!(red.g, 0);
        assert_eq!(red.b, 0);

        let green = hsl_to_rgb(120.0, 1.0, 0.5);
        assert_eq!(green.r, 0);
        assert_eq!(green.g, 255);
        assert_eq!(green.b, 0);

        let blue = hsl_to_rgb(240.0, 1.0, 0.5);
        assert_eq!(blue.r, 0);
        assert_eq!(blue.g, 0);
        assert_eq!(blue.b, 255);
    }

    #[test]
    fn hsl_grayscale() {
        let gray = hsl_to_rgb(0.0, 0.0, 0.5);
        assert_eq!(gray.r, 128);
        assert_eq!(gray.g, 128);
        assert_eq!(gray.b, 128);
    }
}
