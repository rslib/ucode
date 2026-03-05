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
}
