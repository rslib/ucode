mod auth_handler;
mod cmd_auth;
mod cmd_session;
mod headless;
mod session_handler;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use ucode_auth::KeyringStore;
use ucode_core::logging::{LogConfig, LogLevel, default_log_dir, init_logging};

use cmd_auth::AuthCommand;
use cmd_session::SessionCommand;

#[derive(Debug, Parser)]
#[command(name = "ucode", about = "ucode agentic tool")]
struct Cli {
    /// Log level: error, warn, info, debug, trace.
    #[arg(long, value_name = "LEVEL", global = true)]
    log_level: Option<LogLevel>,

    /// Write logs to this file path.
    #[arg(long, value_name = "PATH", global = true)]
    log_file: Option<PathBuf>,

    /// Override the log directory.
    #[arg(long, value_name = "DIR", global = true)]
    log_dir: Option<PathBuf>,

    /// Enable or disable stderr logging. Bare flag means true.
    #[arg(long, num_args = 0..=1, default_missing_value = "true", global = true)]
    log_stderr: Option<bool>,

    /// Shorthand for --log-level trace.
    #[arg(long, global = true)]
    trace: bool,

    /// Run in non-interactive mode (no TUI prompts).
    #[arg(long, global = true)]
    non_interactive: bool,

    /// Output results as JSON (implies --non-interactive).
    #[arg(long, global = true)]
    json_output: bool,

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

    /// Manage sessions.
    Session {
        #[command(subcommand)]
        subcommand: SessionCommand,
    },

    /// Run a prompt non-interactively.
    Run {
        /// The prompt to execute.
        prompt: String,
        /// Resume an existing session.
        #[arg(long)]
        resume_session: Option<String>,
        /// Timeout in seconds.
        #[arg(long, default_value = "300")]
        timeout: u64,
    },
}

/// Build a [`LogConfig`] honouring CLI flags > env vars > defaults.
fn resolve_log_config(cli: &Cli) -> LogConfig {
    // --- level ---
    let level = if cli.trace {
        LogLevel::Trace
    } else if let Some(l) = cli.log_level {
        l
    } else {
        std::env::var("UCODE_LOG_LEVEL")
            .ok()
            .and_then(|s| s.parse::<LogLevel>().ok())
            .unwrap_or_default()
    };

    // --- stderr ---
    let stderr = if let Some(v) = cli.log_stderr {
        v
    } else {
        std::env::var("UCODE_LOG_STDERR")
            .ok()
            .and_then(|s| parse_bool_env(&s))
            .unwrap_or(true)
    };

    // --- log file ---
    let log_file = cli.log_file.clone().or_else(|| {
        std::env::var("UCODE_LOG_FILE")
            .ok()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
    });

    // --- log dir ---
    let log_dir = cli.log_dir.clone().unwrap_or_else(|| {
        std::env::var("UCODE_LOG_DIR")
            .ok()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(default_log_dir)
    });

    // --- rolling (env only, no CLI flag) ---
    let rolling = std::env::var("UCODE_LOG_ROLLING")
        .ok()
        .and_then(|s| parse_bool_env(&s))
        .unwrap_or(false);

    let mut config = LogConfig::default()
        .with_level(level)
        .with_stderr(stderr)
        .with_log_dir(log_dir)
        .with_rolling(rolling);

    if let Some(path) = log_file {
        config = config.with_log_file(path);
    }

    config
}

/// Parse "1"/"true"/"yes" → `Some(true)`, "0"/"false"/"no" → `Some(false)`, else `None`.
fn parse_bool_env(s: &str) -> Option<bool> {
    match s.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" => Some(true),
        "0" | "false" | "no" => Some(false),
        _ => None,
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let config = resolve_log_config(&cli);
    let _log_guard = init_logging(&config)?;
    tracing::debug!("logging initialised");

    let store = KeyringStore::new();

    let session_dir = ucode_core::logging::default_config_home().join("sessions");
    let session_store = ucode_core::SessionStore::new(session_dir)?;

    match cli.command {
        None => {
            println!("ucode v{}", env!("CARGO_PKG_VERSION"));
        }
        Some(Command::Auth { subcommand }) => match subcommand {
            AuthCommand::Status => auth_handler::handle_status(&store)?,
            AuthCommand::SetKey { provider } => auth_handler::handle_set_key(&store, &provider)?,
            AuthCommand::Logout { provider } => auth_handler::handle_logout(&store, &provider)?,
            AuthCommand::Login {
                provider,
                device,
                subscription,
                url,
            } => {
                auth_handler::handle_login(&store, &provider, device, subscription, url.as_deref())
                    .await?
            }
        },
        Some(Command::Session { subcommand }) => match subcommand {
            SessionCommand::List { all } => session_handler::handle_list(&session_store, all)?,
            SessionCommand::Show { id } => session_handler::handle_show(&session_store, &id)?,
            SessionCommand::Rename { id, title } => {
                session_handler::handle_rename(&session_store, &id, title)?
            }
            SessionCommand::Archive { id } => session_handler::handle_archive(&session_store, &id)?,
            SessionCommand::Unarchive { id } => {
                session_handler::handle_unarchive(&session_store, &id)?
            }
            SessionCommand::Fork { id, at_turn } => {
                session_handler::handle_fork(&session_store, &id, at_turn)?
            }
            SessionCommand::Resume { id } => session_handler::handle_resume(&session_store, &id)?,
            SessionCommand::Continue => session_handler::handle_continue(&session_store)?,
        },
        Some(Command::Run {
            prompt,
            resume_session,
            timeout: _,
        }) => {
            let mut runner = headless::HeadlessRunner::new(cli.json_output);
            if let Some(id) = resume_session {
                runner = runner.with_session_id(id);
            }

            if cli.json_output {
                let out = runner.build_output(
                    vec![],
                    headless::HeadlessUsage::default(),
                    headless::ExitCode::Success,
                );
                match runner.format_output(&out) {
                    Ok(json) => println!("{json}"),
                    Err(e) => tracing::error!("failed to serialize headless output: {e}"),
                }
            } else {
                println!("headless mode: would execute prompt: {prompt}");
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Mutex;

    use super::*;

    /// Tests that manipulate environment variables must hold this lock to
    /// prevent races (env vars are process-global, tests run in parallel).
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn test_cli() -> Cli {
        Cli {
            log_level: None,
            log_file: None,
            log_dir: None,
            log_stderr: None,
            trace: false,
            non_interactive: false,
            json_output: false,
            command: None,
        }
    }

    /// Save an env var's current value and remove it, returning a guard that
    /// restores (or removes) it on drop.
    struct EnvGuard {
        key: &'static str,
        saved: Option<String>,
    }

    impl EnvGuard {
        fn remove(key: &'static str) -> Self {
            let saved = std::env::var(key).ok();
            unsafe { std::env::remove_var(key) };
            Self { key, saved }
        }

        fn set(key: &'static str, value: &str) -> Self {
            let saved = std::env::var(key).ok();
            unsafe { std::env::set_var(key, value) };
            Self { key, saved }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.saved {
                Some(v) => unsafe { std::env::set_var(self.key, v) },
                None => unsafe { std::env::remove_var(self.key) },
            }
        }
    }

    // -----------------------------------------------------------------------
    // parse_bool_env
    // -----------------------------------------------------------------------

    #[test]
    fn parse_bool_env_true_values() {
        for s in ["1", "true", "yes", "TRUE", "Yes"] {
            assert_eq!(
                parse_bool_env(s),
                Some(true),
                "expected Some(true) for {s:?}"
            );
        }
    }

    #[test]
    fn parse_bool_env_false_values() {
        for s in ["0", "false", "no", "FALSE", "No"] {
            assert_eq!(
                parse_bool_env(s),
                Some(false),
                "expected Some(false) for {s:?}"
            );
        }
    }

    #[test]
    fn parse_bool_env_invalid() {
        for s in ["maybe", "", "2"] {
            assert_eq!(parse_bool_env(s), None, "expected None for {s:?}");
        }
    }

    // -----------------------------------------------------------------------
    // resolve_log_config
    // -----------------------------------------------------------------------

    #[test]
    fn resolve_defaults() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _l = EnvGuard::remove("UCODE_LOG_LEVEL");
        let _s = EnvGuard::remove("UCODE_LOG_STDERR");
        let _f = EnvGuard::remove("UCODE_LOG_FILE");
        let _d = EnvGuard::remove("UCODE_LOG_DIR");
        let _r = EnvGuard::remove("UCODE_LOG_ROLLING");

        let cfg = resolve_log_config(&test_cli());

        assert_eq!(cfg.level, LogLevel::Info);
        assert!(cfg.stderr);
        assert!(!cfg.rolling);
        assert!(cfg.log_file.is_none());
        assert!(
            cfg.log_dir.ends_with("ucode/logs"),
            "log_dir={}",
            cfg.log_dir.display()
        );
    }

    #[test]
    fn resolve_cli_trace_flag() {
        let _lock = ENV_LOCK.lock().unwrap();
        let cli = Cli {
            trace: true,
            ..test_cli()
        };
        let cfg = resolve_log_config(&cli);
        assert_eq!(cfg.level, LogLevel::Trace);
    }

    #[test]
    fn resolve_cli_log_level_overrides_env() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard::set("UCODE_LOG_LEVEL", "debug");
        let cli = Cli {
            log_level: Some(LogLevel::Error),
            ..test_cli()
        };
        let cfg = resolve_log_config(&cli);
        assert_eq!(cfg.level, LogLevel::Error);
    }

    #[test]
    fn resolve_env_log_level_when_no_cli() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard::set("UCODE_LOG_LEVEL", "debug");
        let cfg = resolve_log_config(&test_cli());
        assert_eq!(cfg.level, LogLevel::Debug);
    }

    #[test]
    fn resolve_trace_beats_log_level() {
        let _lock = ENV_LOCK.lock().unwrap();
        let cli = Cli {
            trace: true,
            log_level: Some(LogLevel::Error),
            ..test_cli()
        };
        let cfg = resolve_log_config(&cli);
        assert_eq!(cfg.level, LogLevel::Trace);
    }

    #[test]
    fn resolve_cli_log_dir() {
        let _lock = ENV_LOCK.lock().unwrap();
        let cli = Cli {
            log_dir: Some(PathBuf::from("/tmp/test-logs")),
            ..test_cli()
        };
        let cfg = resolve_log_config(&cli);
        assert_eq!(cfg.log_dir, PathBuf::from("/tmp/test-logs"));
    }

    #[test]
    fn resolve_env_stderr_false() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard::set("UCODE_LOG_STDERR", "0");
        let cfg = resolve_log_config(&test_cli());
        assert!(!cfg.stderr);
    }

    #[test]
    fn resolve_cli_stderr_overrides_env() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard::set("UCODE_LOG_STDERR", "0");
        let cli = Cli {
            log_stderr: Some(true),
            ..test_cli()
        };
        let cfg = resolve_log_config(&cli);
        assert!(cfg.stderr);
    }
}
