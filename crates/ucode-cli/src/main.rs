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

fn default_model_for(adapter: &ucode_providers::config::AdapterKind) -> String {
    use ucode_providers::config::AdapterKind;
    match adapter {
        AdapterKind::Anthropic => "claude-sonnet-4-20250514".to_owned(),
        AdapterKind::Openai => "gpt-4o".to_owned(),
        AdapterKind::Gemini => "gemini-2.0-flash".to_owned(),
        AdapterKind::Ollama => "llama3.2".to_owned(),
        AdapterKind::Copilot => "gpt-4o".to_owned(),
    }
}

fn agent_event_to_core_event(ev: &ucode_agent::AgentEvent) -> ucode_core::Event {
    use ucode_agent::AgentEvent;
    match ev {
        AgentEvent::Token(t) => ucode_core::Event::Token(t.clone()),
        AgentEvent::StreamDone => ucode_core::Event::Done,
        AgentEvent::Error(e) => {
            ucode_core::Event::Error(ucode_core::CoreError::Internal { message: e.clone() })
        }
        AgentEvent::SystemMessage(m) => ucode_core::Event::Log(m.clone()),
        AgentEvent::ToolCallStarted { name } => {
            ucode_core::Event::Log(format!("tool call: {name}"))
        }
        AgentEvent::ToolCallCompleted {
            name,
            success,
            duration_ms,
            ..
        } => ucode_core::Event::Log(format!(
            "tool {name} {} in {duration_ms}ms",
            if *success { "succeeded" } else { "failed" }
        )),
    }
}

fn print_agent_event(ev: &ucode_agent::AgentEvent) {
    use ucode_agent::AgentEvent;
    match ev {
        AgentEvent::Token(t) => print!("{t}"),
        AgentEvent::StreamDone => println!(),
        AgentEvent::SystemMessage(m) => eprintln!("[system] {m}"),
        AgentEvent::Error(e) => eprintln!("[error] {e}"),
        AgentEvent::ToolCallStarted { name } => eprintln!("[tool] starting: {name}"),
        AgentEvent::ToolCallCompleted {
            name,
            success,
            duration_ms,
            ..
        } => {
            let status = if *success { "ok" } else { "failed" };
            eprintln!("[tool] {name}: {status} ({duration_ms}ms)");
        }
    }
}

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

    let is_tui_mode = cli.command.is_none() && !cli.json_output;

    let mut config = resolve_log_config(&cli);

    // In TUI mode, logs must go to file only — stderr corrupts the alternate
    // screen.  Unless the user explicitly asked for stderr (--log-stderr),
    // disable it and enable a session log file instead.
    if is_tui_mode && cli.log_stderr.is_none() {
        config = config.with_stderr(false).with_rolling(true);
    }

    let _log_guard = init_logging(&config)?;
    tracing::debug!("logging initialised");

    let store = KeyringStore::new();

    let session_dir = ucode_core::logging::default_config_home().join("sessions");
    let session_store = ucode_core::SessionStore::new(session_dir)?;

    match cli.command {
        None => {
            let mut app_config = ucode_agent::AppConfig::load_default()
                .map_err(|e| anyhow::anyhow!("config error: {e}"))?;
            app_config.discover_from_keyring(&store);

            let (event_tx, event_rx) = ucode_tui::create_event_channel();

            if !app_config.has_providers() {
                // No providers yet — launch TUI with a PendingAgentSetup so
                // the user can `/connect` and spawn an agent mid-session.
                let cred_store: std::sync::Arc<dyn ucode_auth::CredentialStore> =
                    std::sync::Arc::new(store);
                let session = session_store.create(std::env::current_dir().unwrap_or_default())?;
                let session_store_arc = std::sync::Arc::new(session_store);

                let mut tool_registry = ucode_tools::ToolRegistry::new();
                ucode_tools::register_builtins(&mut tool_registry);
                let tool_registry = std::sync::Arc::new(tool_registry);

                let pending = ucode_tui::PendingAgentSetup {
                    credential_store: cred_store,
                    session_store: session_store_arc,
                    session,
                    tool_registry,
                };

                ucode_tui::run(event_tx, event_rx, None, Some(pending))
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
            } else {
                let provider_name = app_config
                    .default_provider()
                    .expect("has_providers was true")
                    .to_owned();
                let provider_config = app_config.providers[&provider_name].clone();
                let model = default_model_for(&provider_config.adapter);

                let cred_store: std::sync::Arc<dyn ucode_auth::CredentialStore> =
                    std::sync::Arc::new(store);

                let session = session_store.create(std::env::current_dir().unwrap_or_default())?;
                let session_store_arc = std::sync::Arc::new(session_store);

                let mut tool_registry = ucode_tools::ToolRegistry::new();
                ucode_tools::register_builtins(&mut tool_registry);
                let tool_registry = std::sync::Arc::new(tool_registry);

                let agent_config = ucode_tui::AgentConfig {
                    loop_config: ucode_agent::AgentLoopConfig {
                        provider_name,
                        provider_config,
                        model,
                        credential_store: Some(cred_store),
                    },
                    session_store: session_store_arc,
                    session,
                    tool_registry,
                    all_providers: app_config.providers,
                };

                ucode_tui::run(event_tx, event_rx, Some(agent_config), None)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
            }
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
            timeout,
        }) => {
            let app_config = ucode_agent::AppConfig::load_default()
                .map_err(|e| anyhow::anyhow!("config error: {e}"))?;

            let provider_name = app_config
                .default_provider()
                .ok_or_else(|| anyhow::anyhow!("No providers configured"))?
                .to_owned();
            let provider_config = app_config.providers[&provider_name].clone();
            let model = default_model_for(&provider_config.adapter);

            let cred_store: std::sync::Arc<dyn ucode_auth::CredentialStore> =
                std::sync::Arc::new(store);

            let session = if let Some(ref id) = resume_session {
                session_store.load(id)?
            } else {
                session_store.create(std::env::current_dir().unwrap_or_default())?
            };
            let session_store = std::sync::Arc::new(session_store);

            let mut tool_registry = ucode_tools::ToolRegistry::new();
            ucode_tools::register_builtins(&mut tool_registry);
            let tool_registry = std::sync::Arc::new(tool_registry);

            let loop_config = ucode_agent::AgentLoopConfig {
                provider_name,
                provider_config,
                model,
                credential_store: Some(cred_store),
            };

            let (msg_tx, msg_rx) = tokio::sync::mpsc::unbounded_channel();
            let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();

            let _ = msg_tx.send(ucode_agent::AgentMessage::UserMessage(prompt));
            drop(msg_tx);

            let agent_handle = tokio::spawn(ucode_agent::run_agent_loop(
                msg_rx,
                event_tx,
                loop_config,
                session_store,
                session,
                tool_registry,
            ));

            let runner = headless::HeadlessRunner::new(cli.json_output)
                .with_session_id(resume_session.unwrap_or_default());
            let mut events = Vec::new();
            let timeout_dur = std::time::Duration::from_secs(timeout);
            let deadline = tokio::time::Instant::now() + timeout_dur;

            loop {
                tokio::select! {
                    ev = event_rx.recv() => {
                        match ev {
                            Some(agent_ev) => {
                                let he = runner.record_event(&agent_event_to_core_event(&agent_ev));
                                events.push(he);
                                if !cli.json_output {
                                    print_agent_event(&agent_ev);
                                }
                            }
                            None => break,
                        }
                    }
                    _ = tokio::time::sleep_until(deadline) => {
                        eprintln!("timeout after {timeout}s");
                        break;
                    }
                }
            }

            agent_handle.await.ok();

            if cli.json_output {
                let exit_code = headless::HeadlessRunner::determine_exit_code(&events);
                let out =
                    runner.build_output(events, headless::HeadlessUsage::default(), exit_code);
                match runner.format_output(&out) {
                    Ok(json) => println!("{json}"),
                    Err(e) => tracing::error!("failed to serialize: {e}"),
                }
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
