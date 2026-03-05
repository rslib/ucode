use clap::Subcommand;
use ucode_auth::ProviderId;

#[derive(Debug, Subcommand)]
pub enum AuthCommand {
    /// Show credential status for all providers.
    Status,

    /// Store an API key for a provider (reads from stdin).
    SetKey {
        /// Provider to configure.
        provider: ProviderId,
    },

    /// Delete stored credentials for a provider.
    Logout {
        /// Provider to log out from.
        provider: ProviderId,
    },

    /// Initiate a login flow for a provider (stub).
    Login {
        /// Provider to log in to.
        provider: ProviderId,

        /// Use device-code flow.
        #[arg(long)]
        device: bool,

        /// Use subscription-based login.
        #[arg(long)]
        subscription: bool,
    },
}
