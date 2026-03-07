use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum AuthCommand {
    /// Show credential status for all providers.
    Status,

    /// Store an API key for a provider (reads from stdin).
    SetKey {
        /// Provider name (e.g., "openai", "anthropic", "my-custom-proxy").
        provider: String,
    },

    /// Delete stored credentials for a provider.
    Logout {
        /// Provider name.
        provider: String,
    },

    /// Initiate a login flow for a provider.
    Login {
        /// Provider name or URL for well-known auth.
        provider: String,

        /// Use device-code flow.
        #[arg(long)]
        device: bool,

        /// Use subscription-based login (browser OAuth).
        #[arg(long)]
        subscription: bool,

        /// Enterprise URL (for GitHub Copilot Enterprise).
        #[arg(long)]
        url: Option<String>,
    },
}
