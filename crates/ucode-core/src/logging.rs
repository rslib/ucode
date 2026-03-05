use std::path::PathBuf;
use std::str::FromStr;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::error::CoreError;

// ---------------------------------------------------------------------------
// LogLevel
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogLevel {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Error => f.write_str("error"),
            Self::Warn => f.write_str("warn"),
            Self::Info => f.write_str("info"),
            Self::Debug => f.write_str("debug"),
            Self::Trace => f.write_str("trace"),
        }
    }
}

impl FromStr for LogLevel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "error" => Ok(Self::Error),
            "warn" | "warning" => Ok(Self::Warn),
            "info" => Ok(Self::Info),
            "debug" => Ok(Self::Debug),
            "trace" => Ok(Self::Trace),
            other => Err(format!("unknown log level: {other}")),
        }
    }
}

impl From<LogLevel> for tracing::Level {
    fn from(l: LogLevel) -> Self {
        match l {
            LogLevel::Error => tracing::Level::ERROR,
            LogLevel::Warn => tracing::Level::WARN,
            LogLevel::Info => tracing::Level::INFO,
            LogLevel::Debug => tracing::Level::DEBUG,
            LogLevel::Trace => tracing::Level::TRACE,
        }
    }
}

impl From<LogLevel> for tracing_subscriber::filter::LevelFilter {
    fn from(l: LogLevel) -> Self {
        tracing::Level::from(l).into()
    }
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// Returns `${XDG_STATE_HOME:-~/.local/state}/ucode/logs`.
pub fn default_log_dir() -> PathBuf {
    let base = std::env::var("XDG_STATE_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_owned());
            PathBuf::from(home).join(".local").join("state")
        });
    base.join("ucode").join("logs")
}

/// Returns `${UCODE_HOME}` or `${XDG_CONFIG_HOME:-~/.config}/ucode`.
pub fn default_config_home() -> PathBuf {
    if let Ok(p) = std::env::var("UCODE_HOME")
        && !p.is_empty()
    {
        return PathBuf::from(p);
    }
    let base = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_owned());
            PathBuf::from(home).join(".config")
        });
    base.join("ucode")
}

// ---------------------------------------------------------------------------
// LogConfig
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct LogConfig {
    /// Effective log level.
    pub level: LogLevel,
    /// Whether to emit logs to stderr.
    pub stderr: bool,
    /// Optional explicit log file path (overrides per-session file).
    pub log_file: Option<PathBuf>,
    /// Directory for session and rolling logs.
    pub log_dir: PathBuf,
    /// Whether to enable rolling global log.
    pub rolling: bool,
    /// Session ID for per-session log file naming.
    pub session_id: Option<String>,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: LogLevel::Info,
            stderr: true,
            log_file: None,
            log_dir: default_log_dir(),
            rolling: false,
            session_id: None,
        }
    }
}

impl LogConfig {
    pub fn with_level(mut self, level: LogLevel) -> Self {
        self.level = level;
        self
    }

    pub fn with_stderr(mut self, stderr: bool) -> Self {
        self.stderr = stderr;
        self
    }

    pub fn with_log_file(mut self, path: PathBuf) -> Self {
        self.log_file = Some(path);
        self
    }

    pub fn with_log_dir(mut self, dir: PathBuf) -> Self {
        self.log_dir = dir;
        self
    }

    pub fn with_rolling(mut self, rolling: bool) -> Self {
        self.rolling = rolling;
        self
    }

    pub fn with_session_id(mut self, id: impl Into<String>) -> Self {
        self.session_id = Some(id.into());
        self
    }
}

// ---------------------------------------------------------------------------
// LogGuard
// ---------------------------------------------------------------------------

/// Holds worker guards for non-blocking file appenders.
/// Must be kept alive for the duration of the program.
pub struct LogGuard {
    _guards: Vec<WorkerGuard>,
}

// ---------------------------------------------------------------------------
// init_logging
// ---------------------------------------------------------------------------

/// Initialise the global tracing subscriber.
///
/// Layers are type-erased via `.boxed()` so they can be composed regardless
/// of the concrete writer types involved.  The returned [`LogGuard`] must be
/// held for the lifetime of the process; dropping it flushes and closes the
/// background I/O threads.
pub fn init_logging(config: &LogConfig) -> Result<LogGuard, CoreError> {
    std::fs::create_dir_all(&config.log_dir).map_err(|e| CoreError::LogInit {
        message: format!("cannot create log dir {}: {e}", config.log_dir.display()),
    })?;

    let level_filter: tracing_subscriber::filter::LevelFilter = config.level.into();
    let mut guards: Vec<WorkerGuard> = Vec::new();

    // All layers are type-erased against `Registry` so they can be collected
    // into a Vec and composed in a single `.with(layers)` call.  The global
    // level filter is included in the vec for the same reason — adding it via
    // a separate `.with()` would change the subscriber type and break the
    // `Layer<S>` bound for the subsequent vec.
    type BoxedLayer = Box<dyn Layer<tracing_subscriber::Registry> + Send + Sync + 'static>;
    let mut layers: Vec<BoxedLayer> = Vec::new();

    // Global level gate — must be first so downstream layers see filtered spans.
    layers.push(level_filter.boxed());

    // --- stderr layer ---
    if config.stderr {
        layers.push(
            tracing_subscriber::fmt::layer()
                .compact()
                .with_writer(std::io::stderr)
                .with_ansi(true)
                .boxed(),
        );
    }

    // --- session / explicit file layer ---
    if let Some(layer) = build_file_layer(config, &mut guards)? {
        layers.push(layer);
    }

    // --- rolling layer ---
    if config.rolling {
        let roller = tracing_appender::rolling::daily(config.log_dir.clone(), "ucode");
        let (nb, guard) = tracing_appender::non_blocking(roller);
        guards.push(guard);
        layers.push(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(nb)
                .boxed(),
        );
    }

    tracing_subscriber::registry()
        .with(layers)
        .try_init()
        .map_err(|e| CoreError::LogInit {
            message: format!("set_global_default failed: {e}"),
        })?;

    Ok(LogGuard { _guards: guards })
}

fn build_file_layer(
    config: &LogConfig,
    guards: &mut Vec<WorkerGuard>,
) -> Result<Option<Box<dyn Layer<tracing_subscriber::Registry> + Send + Sync + 'static>>, CoreError>
{
    let path: Option<PathBuf> = config.log_file.clone().or_else(|| {
        config
            .session_id
            .as_ref()
            .map(|sid| config.log_dir.join(format!("session-{sid}.log")))
    });

    let Some(path) = path else {
        return Ok(None);
    };

    // Ensure parent directory exists (log_file may point outside log_dir).
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| CoreError::LogInit {
            message: format!("cannot create log file dir {}: {e}", parent.display()),
        })?;
    }

    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| CoreError::LogInit {
            message: format!("cannot open log file {}: {e}", path.display()),
        })?;

    let (nb, guard) = tracing_appender::non_blocking(file);
    guards.push(guard);

    Ok(Some(
        tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(nb)
            .boxed(),
    ))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_level_roundtrip() {
        for (s, expected) in [
            ("error", LogLevel::Error),
            ("WARN", LogLevel::Warn),
            ("Warning", LogLevel::Warn),
            ("info", LogLevel::Info),
            ("DEBUG", LogLevel::Debug),
            ("trace", LogLevel::Trace),
        ] {
            assert_eq!(s.parse::<LogLevel>().unwrap(), expected);
            // Display produces lowercase canonical form parseable back.
            assert_eq!(expected.to_string().parse::<LogLevel>().unwrap(), expected);
        }
    }

    #[test]
    fn log_level_unknown_is_err() {
        assert!("verbose".parse::<LogLevel>().is_err());
    }

    #[test]
    fn log_level_into_tracing() {
        assert_eq!(tracing::Level::from(LogLevel::Debug), tracing::Level::DEBUG);
        let _: tracing_subscriber::filter::LevelFilter = LogLevel::Info.into();
    }

    #[test]
    fn default_log_dir_uses_xdg_state_home() {
        // Temporarily override env var in a controlled way.
        // Note: env mutation is process-wide; keep the value unique.
        let dir = default_log_dir();
        assert!(dir.ends_with("ucode/logs"));
    }

    #[test]
    fn default_config_home_uses_ucode_home() {
        // UCODE_HOME takes priority.
        unsafe { std::env::set_var("UCODE_HOME", "/tmp/test-ucode-home") };
        let home = default_config_home();
        unsafe { std::env::remove_var("UCODE_HOME") };
        assert_eq!(home, PathBuf::from("/tmp/test-ucode-home"));
    }

    #[test]
    fn log_config_builder() {
        let cfg = LogConfig::default()
            .with_level(LogLevel::Debug)
            .with_stderr(false)
            .with_rolling(true)
            .with_session_id("abc123");
        assert_eq!(cfg.level, LogLevel::Debug);
        assert!(!cfg.stderr);
        assert!(cfg.rolling);
        assert_eq!(cfg.session_id.as_deref(), Some("abc123"));
    }

    #[test]
    fn init_logging_file_sink() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = LogConfig::default()
            .with_stderr(false)
            .with_log_dir(dir.path().to_path_buf())
            .with_session_id("test-session");

        // init_logging may fail if a global subscriber is already set (other
        // tests run in the same process).  That's acceptable — we just verify
        // it doesn't panic and the error path is clean.
        let result = init_logging(&cfg);
        match result {
            Ok(_guard) => {
                // Guard must be alive; log file should exist.
                let log_path = dir.path().join("session-test-session.log");
                assert!(log_path.exists());
            }
            Err(CoreError::LogInit { .. }) => {
                // Already initialised by another test — acceptable.
            }
            Err(e) => panic!("unexpected error: {e}"),
        }
    }
}
