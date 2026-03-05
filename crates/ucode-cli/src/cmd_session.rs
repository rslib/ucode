use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum SessionCommand {
    /// List sessions (excludes archived by default).
    List {
        /// Include archived sessions.
        #[arg(long)]
        all: bool,
    },

    /// Show details of a session.
    Show {
        /// Session ID.
        id: String,
    },

    /// Rename a session (locks title from auto-overwrite).
    Rename {
        /// Session ID.
        id: String,
        /// New title.
        title: String,
    },

    /// Archive a session.
    Archive {
        /// Session ID.
        id: String,
    },

    /// Unarchive a session.
    Unarchive {
        /// Session ID.
        id: String,
    },

    /// Fork a session, creating a child with shared transcript history.
    Fork {
        /// Parent session ID to fork from.
        id: String,
        /// Fork at this transcript turn index (default: end of transcript).
        #[arg(long)]
        at_turn: Option<usize>,
    },

    /// Resume a session by ID (print its details for now).
    Resume {
        /// Session ID to resume.
        id: String,
    },

    /// Continue the most recently updated non-archived session.
    Continue,
}
