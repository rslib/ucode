# @ Mentions and Agent Registry Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add `@` autocomplete for file paths and agents, an agent registry with 4 built-in agents + user-defined agents from `~/.config/ucode/agents/*.md`, and TOML config overrides for agent model/enabled status.

**Architecture:** The agent registry lives in a new `crates/ucode-agents` crate (not to be confused with `ucode-agent` which is the agent loop). It loads built-in agent definitions, discovers user markdown files, applies TOML overrides, and exposes a searchable list. The TUI mirrors the existing slash-command autocomplete pattern for `@` — same `AutocompleteEntry` type, same dropdown UI, just different data source. The directive parser already handles `@name` → `Mention` vs `@path/to/file` → `FileRef`; we pass the agent registry's name list as `known_agents`.

**Tech Stack:** Rust, serde, toml, existing `AutocompleteState`/`AutocompleteEntry` from `ucode-tui`

---

## Milestone ordering

```
A (agent definition types) → B (agent registry) → C (@ autocomplete in TUI) → D (wire to agent loop)
```

---

### Task A1: Agent definition types in ucode-core

**Files:**
- Create: `crates/ucode-core/src/agent_def.rs`
- Modify: `crates/ucode-core/src/lib.rs`

**Step 1: Write the failing test**

In `crates/ucode-core/src/agent_def.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_def_defaults() {
        let def = AgentDef {
            name: "explore".into(),
            description: "Fast codebase explorer".into(),
            system_prompt: "You are a read-only explorer.".into(),
            model: None,
            temperature: None,
            enabled: true,
            tools: ToolPermissions::default(),
            source: AgentSource::BuiltIn,
        };
        assert_eq!(def.name, "explore");
        assert!(def.enabled);
        assert!(def.model.is_none());
        assert_eq!(def.source, AgentSource::BuiltIn);
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
    fn agent_source_display() {
        assert_eq!(AgentSource::BuiltIn.badge(), "[built-in]");
        assert_eq!(AgentSource::User.badge(), "[user]");
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p ucode-core -- agent_def`
Expected: FAIL — module doesn't exist

**Step 3: Write the types**

In `crates/ucode-core/src/agent_def.rs`:

```rust
use serde::{Deserialize, Serialize};

/// Where an agent definition came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentSource {
    BuiltIn,
    User,
}

impl AgentSource {
    pub fn badge(&self) -> &'static str {
        match self {
            Self::BuiltIn => "[built-in]",
            Self::User => "[user]",
        }
    }
}

/// Which tools an agent is allowed to use.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolPermissions {
    pub read: bool,
    pub edit: bool,
    pub write: bool,
    pub bash: bool,
    pub glob: bool,
    pub grep: bool,
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
        }
    }
}

/// A named agent definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDef {
    pub name: String,
    pub description: String,
    pub system_prompt: String,
    /// Override the global model. `None` = use the session's active model.
    pub model: Option<String>,
    pub temperature: Option<f32>,
    pub enabled: bool,
    pub tools: ToolPermissions,
    #[serde(default = "default_source")]
    pub source: AgentSource,
}

fn default_source() -> AgentSource {
    AgentSource::User
}
```

Add `pub mod agent_def;` to `crates/ucode-core/src/lib.rs`.

**Step 4: Run test to verify it passes**

Run: `cargo test -p ucode-core -- agent_def`
Expected: PASS

**Step 5: Commit**

```
feat(core): add AgentDef, ToolPermissions, and AgentSource types
```

---

### Task A2: Parse agent markdown frontmatter

**Files:**
- Modify: `crates/ucode-core/src/agent_def.rs`

Agent markdown files use YAML frontmatter (between `---` delimiters) with the body as the system prompt. Example:

```markdown
---
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
---

You are a read-only explorer.
```

**Step 1: Write the failing test**

```rust
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
fn parse_agent_markdown_no_frontmatter() {
    let md = "Just a system prompt with no frontmatter.";
    let result = AgentDef::from_markdown("bad", md);
    assert!(result.is_err());
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p ucode-core -- parse_agent_markdown`
Expected: FAIL — `from_markdown` doesn't exist

**Step 3: Implement the parser**

Add `serde_yaml` dependency: `cargo add serde_yaml -p ucode-core`

```rust
/// YAML frontmatter from agent markdown files.
#[derive(Debug, Deserialize)]
struct AgentFrontmatter {
    description: String,
    model: Option<String>,
    temperature: Option<f32>,
    #[serde(default = "default_enabled")]
    enabled: bool,
    #[serde(default)]
    tools: ToolPermissions,
}

fn default_enabled() -> bool {
    true
}

impl AgentDef {
    /// Parse an agent definition from a markdown file with YAML frontmatter.
    /// The `name` is derived from the filename (e.g., `explore.md` → `"explore"`).
    pub fn from_markdown(name: &str, content: &str) -> Result<Self, String> {
        let content = content.trim();
        if !content.starts_with("---") {
            return Err(format!("agent {name}: missing YAML frontmatter"));
        }
        let end = content[3..]
            .find("---")
            .ok_or_else(|| format!("agent {name}: unclosed frontmatter"))?;
        let yaml = &content[3..3 + end];
        let body = content[3 + end + 3..].trim();

        let fm: AgentFrontmatter =
            serde_yaml::from_str(yaml).map_err(|e| format!("agent {name}: {e}"))?;

        Ok(Self {
            name: name.to_string(),
            description: fm.description,
            system_prompt: body.to_string(),
            model: fm.model,
            temperature: fm.temperature,
            enabled: fm.enabled,
            tools: fm.tools,
            source: AgentSource::User,
        })
    }
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p ucode-core -- agent_def`
Expected: PASS

**Step 5: Commit**

```
feat(core): parse agent definitions from markdown frontmatter
```

---

### Task B1: Built-in agent definitions

**Files:**
- Create: `crates/ucode-core/src/builtin_agents.rs`
- Modify: `crates/ucode-core/src/lib.rs`

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

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
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p ucode-core -- builtin_agents`
Expected: FAIL

**Step 3: Implement built-in agents**

In `crates/ucode-core/src/builtin_agents.rs`:

```rust
use crate::agent_def::{AgentDef, AgentSource, ToolPermissions};

/// Returns the 4 built-in agent definitions shipped with ucode.
pub fn builtin_agents() -> Vec<AgentDef> {
    vec![
        AgentDef {
            name: "coder".into(),
            description: "General-purpose coding agent. Reads, writes, and edits code.".into(),
            system_prompt: concat!(
                "You are a coding assistant. You read, write, and edit code.\n",
                "Follow the project's conventions. Write clean, minimal code.\n",
                "Verify your changes compile and tests pass before returning.",
            ).into(),
            model: None,
            temperature: Some(0.2),
            enabled: true,
            tools: ToolPermissions::default(),
            source: AgentSource::BuiltIn,
        },
        AgentDef {
            name: "explore".into(),
            description: "Fast read-only codebase explorer. Searches files and answers questions.".into(),
            system_prompt: concat!(
                "You are a read-only codebase explorer.\n",
                "Search files, read code, answer questions about project structure.\n",
                "You do NOT edit, write, or execute anything that modifies state.",
            ).into(),
            model: None,
            temperature: Some(0.1),
            enabled: true,
            tools: ToolPermissions {
                read: true,
                edit: false,
                write: false,
                bash: true,
                glob: true,
                grep: true,
            },
            source: AgentSource::BuiltIn,
        },
        AgentDef {
            name: "planner".into(),
            description: "Breaks tasks into atomic, testable subtasks with dependency tracking.".into(),
            system_prompt: concat!(
                "You are a task planner. Break complex features into atomic subtasks.\n",
                "Each subtask must have clear exit criteria and test requirements.\n",
                "If a subtask cannot be verified, break it further.\n",
                "Output a numbered task list with dependencies.",
            ).into(),
            model: None,
            temperature: Some(0.3),
            enabled: true,
            tools: ToolPermissions {
                read: true,
                edit: false,
                write: true,
                bash: true,
                glob: true,
                grep: true,
            },
            source: AgentSource::BuiltIn,
        },
        AgentDef {
            name: "orchestrator".into(),
            description: "Primary orchestrator. Analyzes tasks, delegates to specialist agents.".into(),
            system_prompt: concat!(
                "You are the orchestrator. Analyze user requests, break them into tasks,\n",
                "delegate to specialist agents, and verify results.\n",
                "Use the cheapest capable agent for each task.\n",
                "Verify each result before moving to the next task.",
            ).into(),
            model: None,
            temperature: Some(0.3),
            enabled: true,
            tools: ToolPermissions::default(),
            source: AgentSource::BuiltIn,
        },
    ]
}
```

Add `pub mod builtin_agents;` to `crates/ucode-core/src/lib.rs`.

**Step 4: Run test to verify it passes**

Run: `cargo test -p ucode-core -- builtin_agents`
Expected: PASS

**Step 5: Commit**

```
feat(core): add 4 built-in agent definitions (coder, explore, planner, orchestrator)
```

---

### Task B2: Agent registry with discovery

**Files:**
- Create: `crates/ucode-core/src/agent_registry.rs`
- Modify: `crates/ucode-core/src/lib.rs`

The registry loads built-ins, discovers user agents from `~/.config/ucode/agents/*.md`, and applies TOML overrides.

**Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
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
        assert_eq!(results.len(), 4);
    }

    #[test]
    fn user_agent_overrides_builtin() {
        let dir = tempfile::tempdir().unwrap();
        let agent_path = dir.path().join("explore.md");
        let mut f = std::fs::File::create(&agent_path).unwrap();
        write!(f, "---\ndescription: \"Custom explorer\"\n---\n\nCustom prompt.").unwrap();

        let mut reg = AgentRegistry::new();
        reg.discover_user_agents(dir.path());
        let explore = reg.get("explore").unwrap();
        assert_eq!(explore.description, "Custom explorer");
        assert_eq!(explore.source, AgentSource::User);
    }

    #[test]
    fn apply_config_overrides_model_and_enabled() {
        let mut reg = AgentRegistry::new();
        let mut overrides = std::collections::HashMap::new();
        overrides.insert("explore".to_string(), AgentConfigOverride {
            model: Some("anthropic/claude-haiku-4-5".to_string()),
            enabled: Some(false),
        });
        reg.apply_overrides(&overrides);

        let explore = reg.get("explore").unwrap();
        assert_eq!(explore.model.as_deref(), Some("anthropic/claude-haiku-4-5"));
        assert!(!explore.enabled);
    }

    #[test]
    fn disabled_agents_excluded_from_search() {
        let mut reg = AgentRegistry::new();
        let mut overrides = std::collections::HashMap::new();
        overrides.insert("explore".to_string(), AgentConfigOverride {
            model: None,
            enabled: Some(false),
        });
        reg.apply_overrides(&overrides);

        let results = reg.search("");
        assert!(!results.iter().any(|a| a.name == "explore"));
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p ucode-core -- agent_registry`
Expected: FAIL

**Step 3: Implement the registry**

In `crates/ucode-core/src/agent_registry.rs`:

```rust
use std::collections::HashMap;
use std::path::Path;

use crate::agent_def::{AgentDef, AgentSource};
use crate::builtin_agents::builtin_agents;

/// TOML override for an agent (from `[agents.<name>]` in ucode.toml).
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct AgentConfigOverride {
    pub model: Option<String>,
    pub enabled: Option<bool>,
}

/// Registry of all known agents (built-in + user-defined).
#[derive(Debug, Clone)]
pub struct AgentRegistry {
    agents: HashMap<String, AgentDef>,
}

impl AgentRegistry {
    /// Creates a registry with only the built-in agents.
    pub fn new() -> Self {
        let mut agents = HashMap::new();
        for agent in builtin_agents() {
            agents.insert(agent.name.clone(), agent);
        }
        Self { agents }
    }

    /// Load user agent definitions from `*.md` files in `dir`.
    /// User agents override built-ins with the same name.
    pub fn discover_user_agents(&mut self, dir: &Path) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            match AgentDef::from_markdown(stem, &content) {
                Ok(def) => {
                    self.agents.insert(def.name.clone(), def);
                }
                Err(e) => {
                    eprintln!("warning: skipping agent {}: {e}", path.display());
                }
            }
        }
    }

    /// Apply TOML config overrides (model, enabled) to existing agents.
    pub fn apply_overrides(&mut self, overrides: &HashMap<String, AgentConfigOverride>) {
        for (name, ov) in overrides {
            if let Some(agent) = self.agents.get_mut(name) {
                if let Some(model) = &ov.model {
                    agent.model = Some(model.clone());
                }
                if let Some(enabled) = ov.enabled {
                    agent.enabled = enabled;
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

    /// Search enabled agents by name prefix. Empty query returns all enabled.
    pub fn search(&self, query: &str) -> Vec<&AgentDef> {
        let q = query.to_lowercase();
        let mut results: Vec<&AgentDef> = self
            .agents
            .values()
            .filter(|a| a.enabled)
            .filter(|a| q.is_empty() || a.name.to_lowercase().starts_with(&q))
            .collect();
        results.sort_by(|a, b| a.name.cmp(&b.name));
        results
    }

    /// Names of all enabled agents (for passing to `parse_input` as `known_agents`).
    pub fn enabled_names(&self) -> Vec<String> {
        self.search("")
            .into_iter()
            .map(|a| a.name.clone())
            .collect()
    }
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self::new()
    }
}
```

Add `pub mod agent_registry;` to `crates/ucode-core/src/lib.rs`.

**Step 4: Run test to verify it passes**

Run: `cargo test -p ucode-core -- agent_registry`
Expected: PASS

**Step 5: Commit**

```
feat(core): add AgentRegistry with built-in + user agent discovery and config overrides
```

---

### Task B3: Add `[agents]` section to AppConfig TOML

**Files:**
- Modify: `crates/ucode-agent/src/config.rs`
- Modify: `crates/ucode-providers/src/config.rs` (add agents to ProvidersTable, or rename it)

**Step 1: Write the failing test**

In `crates/ucode-agent/src/config.rs`:

```rust
#[test]
fn from_file_with_agent_overrides() {
    let f = write_temp_toml(
        r#"
        [agents.explore]
        model = "anthropic/claude-haiku-4-5"
        enabled = false

        [agents.coder]
        model = "anthropic/claude-sonnet-4-6"
        "#,
    );
    let cfg = AppConfig::from_file(f.path()).expect("valid TOML");
    assert_eq!(cfg.agent_overrides.len(), 2);
    assert_eq!(
        cfg.agent_overrides["explore"].enabled,
        Some(false)
    );
    assert_eq!(
        cfg.agent_overrides["coder"].model.as_deref(),
        Some("anthropic/claude-sonnet-4-6")
    );
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p ucode-agent -- from_file_with_agent`
Expected: FAIL — `agent_overrides` field doesn't exist

**Step 3: Implement**

Rename `ProvidersTable` to `ConfigTable` (or add `agents` field alongside `providers`). In `crates/ucode-providers/src/config.rs`, add:

```rust
/// Top-level TOML structure.
#[derive(Debug, Clone, Deserialize)]
pub struct ConfigTable {
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
    #[serde(default)]
    pub agents: HashMap<String, ucode_core::agent_registry::AgentConfigOverride>,
}
```

Update `AppConfig`:

```rust
pub struct AppConfig {
    pub providers: HashMap<String, ProviderConfig>,
    pub agent_overrides: HashMap<String, AgentConfigOverride>,
    pub config_path: Option<PathBuf>,
}
```

Update `from_file` to parse `table.agents` into `agent_overrides`.

Update `DEFAULT_CONFIG_TEMPLATE` to include commented-out `[agents]` examples.

**Step 4: Run test to verify it passes**

Run: `cargo test -p ucode-agent -- config`
Expected: PASS (all existing + new tests)

**Step 5: Commit**

```
feat(config): add [agents] section to ucode.toml for model/enabled overrides
```

---

### Task C1: @ mention completions in AppState

**Files:**
- Modify: `crates/ucode-tui/src/app.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn mention_completions_filters_agents() {
    let mut app = AppState::new(/* ... */);
    // AppState should have an agent_registry field
    let results = app.mention_completions("@ex");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "explore");
    assert_eq!(results[0].source, "[built-in]");
}

#[test]
fn mention_completions_empty_returns_all() {
    let app = AppState::new(/* ... */);
    let results = app.mention_completions("@");
    assert!(results.len() >= 4); // at least the 4 built-ins
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p ucode-tui -- mention_completions`
Expected: FAIL

**Step 3: Implement**

Add `agent_registry: AgentRegistry` field to `AppState`.

Add method:

```rust
pub fn mention_completions(&self, input: &str) -> Vec<AutocompleteEntry> {
    let query = input.strip_prefix('@').unwrap_or(input);

    // Agent completions
    let mut entries: Vec<AutocompleteEntry> = self
        .agent_registry
        .search(query)
        .into_iter()
        .map(|agent| {
            AutocompleteEntry::new(
                &agent.name,
                &agent.description,
                agent.source.badge(),
            )
        })
        .collect();

    // File completions (if query contains '/' or '.' or is empty)
    if query.is_empty() || query.contains('/') || query.contains('.') {
        let file_entries = self.file_completions(query);
        entries.extend(file_entries);
    }

    entries
}

fn file_completions(&self, query: &str) -> Vec<AutocompleteEntry> {
    // List files matching the query prefix in the working directory.
    // Use std::fs::read_dir, filter by prefix, return up to 20 results.
    let base = std::path::Path::new(".");
    let (dir, prefix) = if let Some(pos) = query.rfind('/') {
        let dir_part = &query[..pos];
        let file_part = &query[pos + 1..];
        (base.join(dir_part), file_part.to_string())
    } else {
        (base.to_path_buf(), query.to_string())
    };

    let Ok(entries) = std::fs::read_dir(&dir) else {
        return vec![];
    };

    let mut results: Vec<AutocompleteEntry> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if !prefix.is_empty() && !name.to_lowercase().starts_with(&prefix.to_lowercase()) {
                return None;
            }
            // Skip hidden files
            if name.starts_with('.') {
                return None;
            }
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            let display_path = if dir == base {
                if is_dir { format!("{name}/") } else { name.clone() }
            } else {
                let dir_str = dir.strip_prefix(base).unwrap_or(&dir).display();
                if is_dir {
                    format!("{dir_str}/{name}/")
                } else {
                    format!("{dir_str}/{name}")
                }
            };
            let desc = if is_dir { "directory" } else { "file" };
            Some(AutocompleteEntry::new(&display_path, desc, "[fs]"))
        })
        .collect();

    results.sort_by(|a, b| a.name.cmp(&b.name));
    results.truncate(20);
    results
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p ucode-tui -- mention_completions`
Expected: PASS

**Step 5: Commit**

```
feat(tui): add @mention completions for agents and file paths
```

---

### Task C2: Wire @ autocomplete into event loop

**Files:**
- Modify: `crates/ucode-tui/src/event_loop.rs`

This mirrors the existing slash-command autocomplete wiring exactly.

**Step 1: Identify all locations where `has_slash_prefix()` triggers autocomplete**

There are 3 locations in the event loop where slash completions are triggered:
1. After character insertion (Char key handler)
2. After backspace/delete
3. After Tab completion acceptance

At each location, extend the condition:

**Step 2: Implement**

Replace every occurrence of:
```rust
if input_box.has_slash_prefix() {
    let entries = app.slash_completions(&input_box.content);
    input_box.autocomplete.show(entries);
} else {
    input_box.autocomplete.hide();
}
```

with:

```rust
if input_box.has_slash_prefix() {
    let entries = app.slash_completions(&input_box.content);
    input_box.autocomplete.show(entries);
} else if input_box.has_mention_prefix() {
    let entries = app.mention_completions(&input_box.content);
    input_box.autocomplete.show(entries);
} else {
    input_box.autocomplete.hide();
}
```

Extract this into a helper to avoid repetition:

```rust
fn update_autocomplete(app: &AppState, input_box: &mut InputBoxState) {
    if input_box.has_slash_prefix() {
        let entries = app.slash_completions(&input_box.content);
        input_box.autocomplete.show(entries);
    } else if input_box.has_mention_prefix() {
        let entries = app.mention_completions(&input_box.content);
        input_box.autocomplete.show(entries);
    } else {
        input_box.autocomplete.hide();
    }
}
```

Call `update_autocomplete(app, input_box)` at all 3 locations.

**Step 3: Add Ctrl+P / Ctrl+N autocomplete navigation**

In the input box Ctrl key handler (around line 1284), the existing Ctrl+P and Ctrl+N
map to `move_up()` / `move_down()` (emacs line navigation). Add the same autocomplete
priority guard that Up/Down arrow keys already use:

```rust
// Ctrl+P in input box
crossterm::event::KeyCode::Char('p') => {
    if input_box.autocomplete.visible {
        input_box.autocomplete.prev();
    } else {
        input_box.move_up();
    }
    app.mark_dirty();
    return false;
}
// Ctrl+N in input box
crossterm::event::KeyCode::Char('n') => {
    if input_box.autocomplete.visible {
        input_box.autocomplete.next();
    } else {
        input_box.move_down();
    }
    app.mark_dirty();
    return false;
}
```

No conflict: Ctrl+P/N already mean "up/down" in the input box. When autocomplete is
visible, they navigate the dropdown instead. Same pattern as Up/Down arrow keys.

**Step 4: Handle Tab/Enter completion acceptance for @ mentions**

When the user selects an autocomplete entry and presses Tab/Enter:
- For agents: replace input content with `@agent_name ` (with trailing space)
- For files: replace input content with `@file/path ` (with trailing space)

The existing Tab handler replaces with the slash command name. Mirror this for @ mentions.

**Step 5: Run tests**

Run: `cargo test -p ucode-tui`
Expected: PASS (all existing tests + new behavior)

**Step 6: Commit**

```
feat(tui): wire @mention autocomplete into event loop with Ctrl+P/N navigation
```

---

### Task C3: Handle @ mentions on message send

**Files:**
- Modify: `crates/ucode-tui/src/event_loop.rs`

When the user sends a message containing `@agent_name`, the TUI needs to:
1. Parse the input with `parse_input()` passing `agent_registry.enabled_names()` as `known_agents`
2. For `FileRef` directives: read the file content and prepend it to the message as context
3. For `Mention` directives: route the message to that agent (set the target agent on the `AgentMessage`)

**Step 1: Expand AgentMessage to carry target agent and file context**

In `crates/ucode-agent/src/agent_loop.rs`, change:

```rust
pub enum AgentMessage {
    UserMessage(String),
    // ...
}
```

to:

```rust
pub enum AgentMessage {
    UserMessage {
        text: String,
        /// If set, route to this agent instead of the default.
        target_agent: Option<String>,
        /// File contents to prepend as context.
        file_context: Vec<FileContext>,
    },
    // ...
}

pub struct FileContext {
    pub path: String,
    pub content: String,
}
```

Update all existing `AgentMessage::UserMessage(text)` call sites to use the struct form with `target_agent: None, file_context: vec![]`.

**Step 2: In the send handler, parse and resolve**

In the event loop's send-message handler, before sending `AgentMessage::UserMessage`:

```rust
let agent_names: Vec<String> = app.agent_registry.enabled_names();
let agent_refs: Vec<&str> = agent_names.iter().map(|s| s.as_str()).collect();
let parsed = ucode_core::directive::parse_input(&text, &agent_refs);

let mut target_agent = None;
let mut file_context = Vec::new();
let mut display_text = String::new();

for directive in &parsed.directives {
    match directive {
        Directive::Mention { name, .. } => {
            target_agent = Some(name.clone());
        }
        Directive::FileRef { path, .. } => {
            if let Ok(content) = std::fs::read_to_string(path) {
                file_context.push(FileContext {
                    path: path.clone(),
                    content,
                });
            }
        }
        Directive::Text { content, .. } => {
            display_text.push_str(content);
        }
        _ => {}
    }
}
```

**Step 3: Run tests**

Run: `cargo test -p ucode-tui && cargo test -p ucode-agent`
Expected: PASS

**Step 4: Commit**

```
feat: resolve @mentions on send — file context and agent routing
```

---

### Task D1: Agent loop handles target agent routing

**Files:**
- Modify: `crates/ucode-agent/src/agent_loop.rs`

**Step 1: Implement**

In `run_agent_loop`, when handling `AgentMessage::UserMessage`:

```rust
AgentMessage::UserMessage { text, target_agent, file_context } => {
    // Build context prefix from file contents
    let mut context_prefix = String::new();
    for fc in &file_context {
        context_prefix.push_str(&format!(
            "<file path=\"{}\">\n{}\n</file>\n\n",
            fc.path, fc.content
        ));
    }
    let full_message = if context_prefix.is_empty() {
        text
    } else {
        format!("{context_prefix}{text}")
    };

    // If target_agent is set, look up the agent def and use its system prompt + model
    let (system_prompt, model_override) = if let Some(agent_name) = &target_agent {
        // Agent registry lookup — for now, pass agent_registry into the loop
        // or resolve at the TUI layer and pass the overrides in the message
        // ...
    } else {
        (default_system_prompt.clone(), None)
    };

    // Use model_override if present, otherwise use the session's active model
    // ...
}
```

The exact wiring depends on whether we pass the `AgentRegistry` into the agent loop or resolve everything at the TUI layer. **Recommended:** resolve at the TUI layer — the `AgentMessage::UserMessage` carries the resolved system prompt and model override, not the agent name. This keeps the agent loop simple.

**Step 2: Run tests**

Run: `cargo test -p ucode-agent`
Expected: PASS

**Step 3: Commit**

```
feat(agent): handle file context and agent routing in agent loop
```

---

### Task D2: Initialize agent registry in CLI startup

**Files:**
- Modify: `crates/ucode-cli/src/main.rs`
- Modify: `crates/ucode-tui/src/lib.rs`

**Step 1: Implement**

In CLI startup:

```rust
// Build agent registry
let mut agent_registry = AgentRegistry::new();
let agents_dir = ucode_core::logging::default_config_home().join("agents");
agent_registry.discover_user_agents(&agents_dir);
agent_registry.apply_overrides(&app_config.agent_overrides);

// Pass to TUI
```

Add `agent_registry` to whatever struct is passed into the TUI `run()` function.

In `AppState::new()`, accept and store the `AgentRegistry`.

**Step 2: Run full build**

Run: `cargo build --bin ucode`
Expected: compiles

**Step 3: Commit**

```
feat(cli): initialize agent registry from built-ins + user agents + config overrides
```

---

## Summary

| Task | What | Crate |
|------|------|-------|
| A1 | `AgentDef`, `ToolPermissions`, `AgentSource` types | ucode-core |
| A2 | Parse agent markdown frontmatter | ucode-core |
| B1 | 4 built-in agent definitions | ucode-core |
| B2 | `AgentRegistry` with discovery + search | ucode-core |
| B3 | `[agents]` section in `ucode.toml` | ucode-agent, ucode-providers |
| C1 | `mention_completions()` in AppState | ucode-tui |
| C2 | Wire @ autocomplete into event loop | ucode-tui |
| C3 | Resolve @mentions on send (files + agents) | ucode-tui, ucode-agent |
| D1 | Agent loop handles routing + file context | ucode-agent |
| D2 | Initialize registry in CLI startup | ucode-cli |
