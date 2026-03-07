use crate::keybinds::Action;

// ---------------------------------------------------------------------------
// CommandCategory
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandCategory {
    Recent,
    Session,
    Tools,
    Plugins,
}

impl CommandCategory {
    pub fn label(self) -> &'static str {
        match self {
            Self::Recent => "recent",
            Self::Session => "session",
            Self::Tools => "tools",
            Self::Plugins => "plugins",
        }
    }
}

// ---------------------------------------------------------------------------
// CommandSource
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandSource {
    Builtin,
    User,
    Project,
    Plugin(String),
}

impl CommandSource {
    pub fn badge(&self) -> &str {
        match self {
            Self::Builtin => "[builtin]",
            Self::User => "[user]",
            Self::Project => "[project]",
            Self::Plugin(_) => "[plugin]",
        }
    }
}

// ---------------------------------------------------------------------------
// CommandDef
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct CommandDef {
    pub name: String,
    pub description: String,
    pub category: CommandCategory,
    pub source: CommandSource,
    /// Optional argument hint shown in the palette, e.g. `"<name>"`.
    pub args_hint: Option<String>,
    /// TUI action to dispatch when this command is selected, if any.
    pub action: Option<Action>,
}

// ---------------------------------------------------------------------------
// CommandRegistry
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct CommandRegistry {
    commands: Vec<CommandDef>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self {
            commands: Vec::new(),
        }
    }

    pub fn with_builtins() -> Self {
        let mut reg = Self::new();

        reg.commands.push(CommandDef {
            name: "/connect".to_owned(),
            description: "Connect provider or auth method".to_owned(),
            category: CommandCategory::Tools,
            source: CommandSource::Builtin,
            args_hint: None,
            action: Some(Action::OpenConnect),
        });

        let tools: &[(&str, &str)] = &[
            ("/skills", "Browse and activate skills"),
            ("/models", "Switch model or model group"),
            ("/tools", "View available tools"),
            ("/checkpoint", "Create workspace checkpoint"),
            ("/rollback", "Restore prior checkpoint"),
            ("/jobs", "View background jobs"),
        ];
        for (name, desc) in tools {
            reg.commands.push(CommandDef {
                name: (*name).to_owned(),
                description: (*desc).to_owned(),
                category: CommandCategory::Tools,
                source: CommandSource::Builtin,
                args_hint: None,
                action: None,
            });
        }

        let session: &[(&str, &str, Option<&str>)] = &[
            ("/session list", "List all sessions", None),
            ("/session fork", "Fork current session", None),
            ("/session rename", "Rename current session", Some("<name>")),
        ];
        for (name, desc, hint) in session {
            reg.commands.push(CommandDef {
                name: (*name).to_owned(),
                description: (*desc).to_owned(),
                category: CommandCategory::Session,
                source: CommandSource::Builtin,
                args_hint: hint.map(str::to_owned),
                action: None,
            });
        }

        reg
    }

    pub fn register(&mut self, cmd: CommandDef) {
        self.commands.push(cmd);
    }

    /// Exact match by name (case-sensitive).
    pub fn resolve(&self, name: &str) -> Option<&CommandDef> {
        self.commands.iter().find(|c| c.name == name)
    }

    /// Substring match on name and description (case-insensitive).
    /// An empty query returns all commands.
    pub fn search(&self, query: &str) -> Vec<&CommandDef> {
        if query.is_empty() {
            return self.commands.iter().collect();
        }
        let needle = query.to_lowercase();
        self.commands
            .iter()
            .filter(|c| {
                c.name.to_lowercase().contains(&needle)
                    || c.description.to_lowercase().contains(&needle)
            })
            .collect()
    }

    pub fn list(&self) -> &[CommandDef] {
        &self.commands
    }

    /// Remove all commands whose source is `Plugin(source_name)`.
    pub fn remove_by_source_name(&mut self, source_name: &str) {
        self.commands
            .retain(|c| !matches!(&c.source, CommandSource::Plugin(n) if n == source_name));
    }

    /// Return commands whose name contains `name` as a substring (case-insensitive).
    /// Used to suggest alternatives when a command is not found.
    pub fn suggest(&self, name: &str) -> Vec<&CommandDef> {
        let needle = name.to_lowercase();
        self.commands
            .iter()
            .filter(|c| {
                let lower = c.name.to_lowercase();
                // Strip leading "/" for prefix comparison so "conect" matches "/connect".
                let bare = lower.trim_start_matches('/');
                lower.contains(&needle)
                    || needle.contains(lower.as_str())
                    || shared_prefix_len(bare, &needle) >= 3
            })
            .collect()
    }
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Length of the longest common prefix between two strings.
fn shared_prefix_len(a: &str, b: &str) -> usize {
    a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_with_builtins() {
        let reg = CommandRegistry::with_builtins();
        assert_eq!(reg.list().len(), 10);
    }

    #[test]
    fn test_resolve_exact_match() {
        let reg = CommandRegistry::with_builtins();
        let cmd = reg.resolve("/connect").unwrap();
        assert_eq!(cmd.name, "/connect");
        assert_eq!(cmd.description, "Connect provider or auth method");
    }

    #[test]
    fn test_resolve_not_found() {
        let reg = CommandRegistry::with_builtins();
        assert!(reg.resolve("/nonexistent").is_none());
    }

    #[test]
    fn test_search_by_name() {
        let reg = CommandRegistry::with_builtins();
        let results = reg.search("session");
        assert_eq!(results.len(), 3);
        for cmd in &results {
            assert!(
                cmd.name.contains("session") || cmd.description.to_lowercase().contains("session"),
                "unexpected: {}",
                cmd.name
            );
        }
    }

    #[test]
    fn test_search_by_description() {
        let reg = CommandRegistry::with_builtins();
        let results = reg.search("checkpoint");
        assert!(!results.is_empty());
        assert!(results.iter().any(|c| c.name == "/checkpoint"));
    }

    #[test]
    fn test_search_empty_query() {
        let reg = CommandRegistry::with_builtins();
        let results = reg.search("");
        assert_eq!(results.len(), 10);
    }

    #[test]
    fn test_search_case_insensitive() {
        let reg = CommandRegistry::with_builtins();
        let results = reg.search("CONNECT");
        assert!(!results.is_empty());
        assert!(results.iter().any(|c| c.name == "/connect"));
    }

    #[test]
    fn test_register_custom_command() {
        let mut reg = CommandRegistry::with_builtins();
        reg.register(CommandDef {
            name: "/custom".to_owned(),
            description: "A custom command".to_owned(),
            category: CommandCategory::Tools,
            source: CommandSource::User,
            args_hint: None,
            action: None,
        });
        let cmd = reg.resolve("/custom").unwrap();
        assert_eq!(cmd.name, "/custom");
        assert_eq!(reg.list().len(), 11);
    }

    #[test]
    fn test_suggest_similar() {
        let reg = CommandRegistry::with_builtins();
        // "conect" shares prefix "con" with "/connect"
        let suggestions = reg.suggest("conect");
        assert!(
            suggestions.iter().any(|c| c.name == "/connect"),
            "expected /connect in suggestions for 'conect'"
        );
    }

    #[test]
    fn test_command_source_badges() {
        assert_eq!(CommandSource::Builtin.badge(), "[builtin]");
        assert_eq!(CommandSource::User.badge(), "[user]");
        assert_eq!(CommandSource::Project.badge(), "[project]");
        assert_eq!(
            CommandSource::Plugin("my-plugin".to_owned()).badge(),
            "[plugin]"
        );
    }

    #[test]
    fn test_palette_uses_registry_commands() {
        use crate::overlays::palette::PaletteState;
        let reg = CommandRegistry::with_builtins();
        let state = PaletteState::from_registry(&reg);
        assert_eq!(state.commands.len(), 10);
    }

    #[test]
    fn remove_by_source_name_removes_matching_plugin_commands() {
        let mut reg = CommandRegistry::with_builtins();
        reg.register(CommandDef {
            name: "/analyze".to_owned(),
            description: "Run analysis".to_owned(),
            category: CommandCategory::Plugins,
            source: CommandSource::Plugin("code-analyzer".to_owned()),
            args_hint: None,
            action: None,
        });
        reg.register(CommandDef {
            name: "/lint".to_owned(),
            description: "Run linter".to_owned(),
            category: CommandCategory::Plugins,
            source: CommandSource::Plugin("linter".to_owned()),
            args_hint: None,
            action: None,
        });
        assert_eq!(reg.list().len(), 12);
        reg.remove_by_source_name("code-analyzer");
        assert_eq!(reg.list().len(), 11);
        assert!(reg.resolve("/analyze").is_none());
        assert!(reg.resolve("/lint").is_some());
    }

    #[test]
    fn remove_by_source_name_leaves_builtins_intact() {
        let mut reg = CommandRegistry::with_builtins();
        reg.register(CommandDef {
            name: "/plugin-cmd".to_owned(),
            description: "Plugin command".to_owned(),
            category: CommandCategory::Plugins,
            source: CommandSource::Plugin("my-plugin".to_owned()),
            args_hint: None,
            action: None,
        });
        reg.remove_by_source_name("my-plugin");
        // All 10 builtins should remain.
        assert_eq!(reg.list().len(), 10);
        assert!(reg.resolve("/connect").is_some());
    }
}
