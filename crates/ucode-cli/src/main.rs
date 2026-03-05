mod auth_handler;
mod cmd_auth;

use anyhow::Result;
use clap::{Parser, Subcommand};
use ucode_auth::KeyringStore;

use cmd_auth::AuthCommand;

#[derive(Debug, Parser)]
#[command(name = "ucode", about = "ucode agentic tool")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Manage provider credentials.
    Auth {
        #[command(subcommand)]
        subcommand: AuthCommand,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let store = KeyringStore::new();

    match cli.command {
        None => {
            println!("ucode v{}", env!("CARGO_PKG_VERSION"));
        }
        Some(Command::Auth { subcommand }) => match subcommand {
            AuthCommand::Status => auth_handler::handle_status(&store)?,
            AuthCommand::SetKey { provider } => auth_handler::handle_set_key(&store, provider)?,
            AuthCommand::Logout { provider } => auth_handler::handle_logout(&store, provider)?,
            AuthCommand::Login {
                provider,
                device,
                subscription,
            } => auth_handler::handle_login(&store, provider, device, subscription)?,
        },
    }

    Ok(())
}
