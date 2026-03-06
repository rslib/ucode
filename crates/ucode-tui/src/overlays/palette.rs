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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandSource {
    Builtin,
    Plugin(String),
}

impl CommandSource {
    pub fn badge(&self) -> String {
        match self {
            Self::Builtin => "[builtin]".to_owned(),
            Self::Plugin(name) => format!("[{name}]"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PaletteCommand {
    pub name: String,
    pub description: String,
    pub category: CommandCategory,
    pub source: CommandSource,
}

pub fn builtin_commands() -> Vec<PaletteCommand> {
    let tools = [
        ("/connect", "Connect provider or auth method"),
        ("/skills", "Browse and activate skills"),
        ("/models", "Switch model or model group"),
        ("/tools", "View available tools"),
        ("/checkpoint", "Create workspace checkpoint"),
        ("/rollback", "Restore prior checkpoint"),
        ("/jobs", "View background jobs"),
    ];
    let session = [
        ("/session list", "List all sessions"),
        ("/session fork", "Fork current session"),
        ("/session rename", "Rename current session"),
    ];

    let mut cmds = Vec::with_capacity(tools.len() + session.len());
    for (name, desc) in tools {
        cmds.push(PaletteCommand {
            name: name.to_owned(),
            description: desc.to_owned(),
            category: CommandCategory::Tools,
            source: CommandSource::Builtin,
        });
    }
    for (name, desc) in session {
        cmds.push(PaletteCommand {
            name: name.to_owned(),
            description: desc.to_owned(),
            category: CommandCategory::Session,
            source: CommandSource::Builtin,
        });
    }
    cmds
}

pub struct PaletteState {
    pub visible: bool,
    pub input: String,
    pub cursor: usize,
    pub commands: Vec<PaletteCommand>,
    pub filtered_indices: Vec<usize>,
    pub selected: usize,
}

impl PaletteState {
    pub fn new() -> Self {
        let commands = builtin_commands();
        let filtered_indices = (0..commands.len()).collect();
        Self {
            visible: false,
            input: String::new(),
            cursor: 0,
            commands,
            filtered_indices,
            selected: 0,
        }
    }

    pub fn open(&mut self) {
        self.visible = true;
        self.input.clear();
        self.cursor = 0;
        self.selected = 0;
        self.update_filter();
    }

    pub fn close(&mut self) {
        self.visible = false;
    }

    pub fn insert_char(&mut self, c: char) {
        self.input.insert(self.cursor, c);
        self.cursor += c.len_utf8();
        self.update_filter();
    }

    pub fn delete_char(&mut self) {
        if self.cursor == 0 {
            return;
        }
        // Find the char boundary before cursor.
        let before = &self.input[..self.cursor];
        let char_start = before
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.input.drain(char_start..self.cursor);
        self.cursor = char_start;
        self.update_filter();
    }

    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn move_down(&mut self) {
        if self.filtered_indices.is_empty() {
            return;
        }
        let max = self.filtered_indices.len() - 1;
        if self.selected < max {
            self.selected += 1;
        }
    }

    pub fn selected_command(&self) -> Option<&PaletteCommand> {
        let idx = *self.filtered_indices.get(self.selected)?;
        self.commands.get(idx)
    }

    pub fn execute_selected(&mut self) -> Option<PaletteCommand> {
        let cmd = self.selected_command()?.clone();
        self.close();
        Some(cmd)
    }

    pub fn update_filter(&mut self) {
        if self.input.is_empty() {
            self.filtered_indices = (0..self.commands.len()).collect();
        } else {
            let needle = self.input.to_lowercase();
            self.filtered_indices = self
                .commands
                .iter()
                .enumerate()
                .filter(|(_, cmd)| {
                    cmd.name.to_lowercase().contains(&needle)
                        || cmd.description.to_lowercase().contains(&needle)
                })
                .map(|(i, _)| i)
                .collect();
        }
        // Clamp selected to valid range.
        if self.filtered_indices.is_empty() {
            self.selected = 0;
        } else {
            let max = self.filtered_indices.len() - 1;
            if self.selected > max {
                self.selected = max;
            }
        }
    }
}

impl Default for PaletteState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_commands_count() {
        assert_eq!(builtin_commands().len(), 10);
    }

    #[test]
    fn category_labels() {
        assert_eq!(CommandCategory::Recent.label(), "recent");
        assert_eq!(CommandCategory::Session.label(), "session");
        assert_eq!(CommandCategory::Tools.label(), "tools");
        assert_eq!(CommandCategory::Plugins.label(), "plugins");
    }

    #[test]
    fn source_badges() {
        assert_eq!(CommandSource::Builtin.badge(), "[builtin]");
        assert_eq!(
            CommandSource::Plugin("my-plugin".to_owned()).badge(),
            "[my-plugin]"
        );
    }

    #[test]
    fn new_state_all_filtered_not_visible() {
        let state = PaletteState::new();
        assert!(!state.visible);
        assert_eq!(state.filtered_indices.len(), state.commands.len());
        assert_eq!(state.selected, 0);
        assert!(state.input.is_empty());
    }

    #[test]
    fn open_sets_visible_clears_input() {
        let mut state = PaletteState::new();
        state.input = "leftover".to_owned();
        state.cursor = 8;
        state.open();
        assert!(state.visible);
        assert!(state.input.is_empty());
        assert_eq!(state.cursor, 0);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn insert_char_filters_session_commands() {
        let mut state = PaletteState::new();
        state.open();
        for c in "session".chars() {
            state.insert_char(c);
        }
        // Only session commands should remain.
        assert!(!state.filtered_indices.is_empty());
        for &idx in &state.filtered_indices {
            let cmd = &state.commands[idx];
            let haystack = format!("{} {}", cmd.name, cmd.description).to_lowercase();
            assert!(
                haystack.contains("session"),
                "unexpected command: {}",
                cmd.name
            );
        }
        // Exactly 3 session commands.
        assert_eq!(state.filtered_indices.len(), 3);
    }

    #[test]
    fn move_up_saturates_at_zero() {
        let mut state = PaletteState::new();
        state.open();
        assert_eq!(state.selected, 0);
        state.move_up();
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn move_down_clamps_at_last() {
        let mut state = PaletteState::new();
        state.open();
        let last = state.filtered_indices.len() - 1;
        for _ in 0..last + 5 {
            state.move_down();
        }
        assert_eq!(state.selected, last);
    }

    #[test]
    fn move_up_down_navigation() {
        let mut state = PaletteState::new();
        state.open();
        state.move_down();
        state.move_down();
        assert_eq!(state.selected, 2);
        state.move_up();
        assert_eq!(state.selected, 1);
    }

    #[test]
    fn selected_command_returns_correct() {
        let mut state = PaletteState::new();
        state.open();
        // First command should be /connect (index 0 in builtin_commands).
        let cmd = state.selected_command().unwrap();
        assert_eq!(cmd.name, "/connect");
    }

    #[test]
    fn execute_selected_returns_command_and_closes() {
        let mut state = PaletteState::new();
        state.open();
        let cmd = state.execute_selected().unwrap();
        assert_eq!(cmd.name, "/connect");
        assert!(!state.visible);
    }

    #[test]
    fn delete_char_refilters() {
        let mut state = PaletteState::new();
        state.open();
        for c in "session".chars() {
            state.insert_char(c);
        }
        assert_eq!(state.filtered_indices.len(), 3);
        // Delete all chars one by one.
        for _ in 0.."session".len() {
            state.delete_char();
        }
        assert!(state.input.is_empty());
        // Should show all commands again.
        assert_eq!(state.filtered_indices.len(), state.commands.len());
    }

    #[test]
    fn empty_filter_shows_all() {
        let mut state = PaletteState::new();
        state.open();
        state.update_filter();
        assert_eq!(state.filtered_indices.len(), state.commands.len());
    }

    #[test]
    fn filter_no_match_gives_empty() {
        let mut state = PaletteState::new();
        state.open();
        for c in "zzznomatch".chars() {
            state.insert_char(c);
        }
        assert!(state.filtered_indices.is_empty());
        // selected_command returns None when nothing matches.
        assert!(state.selected_command().is_none());
    }

    #[test]
    fn selected_clamped_after_filter_narrows() {
        let mut state = PaletteState::new();
        state.open();
        // Move to last item.
        let last = state.filtered_indices.len() - 1;
        for _ in 0..last {
            state.move_down();
        }
        assert_eq!(state.selected, last);
        // Now filter to only 3 items.
        for c in "session".chars() {
            state.insert_char(c);
        }
        // selected must be within [0, 2].
        assert!(state.selected <= 2);
    }
}
