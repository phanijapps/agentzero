//! # z-Bot Daemon
//!
//! Standalone server for the agent runtime.
//!
//! ## Usage
//!
//! ```bash
//! # Start with defaults
//! zbotd
//!
//! # Start with custom ports
//! zbotd --ws-port 19000 --http-port 19001
//!
//! # Start with custom data directory
//! zbotd --data-dir /path/to/zbot
//!
//! # Start with config file
//! zbotd --config /path/to/daemon.yaml
//!
//! # Enable file logging via CLI
//! zbotd --log-dir /var/log/zbot --log-max-files 14
//!
//! # Serve web dashboard from static files
//! zbotd --static-dir /path/to/dashboard/dist
//!
//! # Disable web dashboard
//! zbotd --no-dashboard
//! ```
//!
//! ## Logging Configuration
//!
//! Logging can be configured via:
//! 1. `settings.json` in the data directory (persistent)
//! 2. CLI arguments (override settings.json)
//!
//! Example `settings.json`:
//! ```json
//! {
//!   "logs": {
//!     "enabled": true,
//!     "level": "info",
//!     "rotation": "daily",
//!     "maxFiles": 7,
//!     "suppressStdout": false
//!   }
//! }
//! ```

#[cfg(unix)]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

use anyhow::Result;
use clap::Parser;
use gateway::{GatewayConfig, GatewayServer};
use gateway_services::{AppSettings, LogSettings};
use std::fs;
use std::path::PathBuf;
use tracing::{info, Level};
use tracing_appender::rolling::RollingFileAppender;
use tracing_appender::rolling::Rotation;
use tracing_subscriber::{
    fmt, fmt::writer::MakeWriterExt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter,
};

/// z-Bot Daemon - AI agent runtime server
#[derive(Parser, Debug)]
#[command(name = "zbotd")]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Legacy standalone WebSocket port.
    ///
    /// Off by default — the gateway now serves WebSocket traffic on the
    /// HTTP port at `/ws`. This value is only used when
    /// `--legacy-ws-port-enabled` is set.
    #[arg(long, default_value_t = gateway::DEFAULT_WS_PORT)]
    ws_port: u16,

    /// HTTP port. Also hosts the unified WebSocket upgrade at `/ws`.
    #[arg(long, default_value_t = gateway::DEFAULT_HTTP_PORT)]
    http_port: u16,

    /// Bind the legacy standalone WebSocket port in addition to the
    /// unified `/ws` endpoint. For external integrations that haven't
    /// migrated from `ws://host:18790` yet. Slated for removal.
    #[arg(long)]
    legacy_ws_port_enabled: bool,

    /// Host address to bind to. Default binds to all interfaces so the
    /// daemon is reachable from other devices on the LAN. Override with
    /// `--host 127.0.0.1` to restrict to localhost.
    #[arg(long, default_value = "0.0.0.0")]
    host: String,

    /// Path to z-Bot data directory (default: ~/Documents/zbot)
    #[arg(long)]
    data_dir: Option<PathBuf>,

    /// Path to daemon configuration file
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Log level (trace, debug, info, warn, error)
    /// Overrides settings.json
    #[arg(long)]
    log_level: Option<String>,

    /// Directory for log files (enables file logging when set)
    /// Overrides settings.json
    #[arg(long)]
    log_dir: Option<PathBuf>,

    /// Log rotation strategy: daily, hourly, minutely, or never
    /// Overrides settings.json
    #[arg(long)]
    log_rotation: Option<String>,

    /// Maximum number of log files to keep (0 = unlimited)
    /// Overrides settings.json
    #[arg(long)]
    log_max_files: Option<usize>,

    /// Disable logging to stdout (only log to file)
    /// Overrides settings.json
    #[arg(long)]
    log_no_stdout: bool,

    /// Path to static files for web dashboard
    #[arg(long)]
    static_dir: Option<PathBuf>,

    /// Disable serving the web dashboard
    #[arg(long)]
    no_dashboard: bool,
}

/// Merged logging configuration from settings.json and CLI args.
///
/// CLI arguments take precedence over settings.json values.
#[derive(Debug, Clone)]
struct LogConfig {
    /// Enable file logging
    enabled: bool,
    /// Log directory (None = default {data_dir}/logs)
    directory: Option<PathBuf>,
    /// Log level
    level: String,
    /// Rotation strategy
    rotation: String,
    /// Max files to keep (0 = unlimited)
    max_files: usize,
    /// Suppress stdout output
    suppress_stdout: bool,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            directory: None,
            level: "info".to_string(),
            rotation: "daily".to_string(),
            max_files: 7,
            suppress_stdout: false,
        }
    }
}

impl From<LogSettings> for LogConfig {
    fn from(settings: LogSettings) -> Self {
        Self {
            enabled: settings.enabled,
            directory: settings.directory,
            level: settings.level,
            rotation: settings.rotation,
            max_files: settings.max_files,
            suppress_stdout: settings.suppress_stdout,
        }
    }
}

/// Load settings.json from the data directory.
///
/// Returns default settings if the file doesn't exist or can't be parsed.
fn load_settings(data_dir: &std::path::Path) -> AppSettings {
    // Try new path first (config/settings.json), fall back to legacy path (settings.json)
    let new_path = data_dir.join("config").join("settings.json");
    let legacy_path = data_dir.join("settings.json");

    let settings_path = if new_path.exists() {
        new_path
    } else if legacy_path.exists() {
        legacy_path
    } else {
        return AppSettings::default();
    };

    match fs::read_to_string(&settings_path) {
        Ok(content) => match serde_json::from_str(&content) {
            Ok(settings) => {
                eprintln!("[startup] Loaded settings from {:?}", settings_path);
                settings
            }
            Err(e) => {
                eprintln!("[startup] Warning: Failed to parse settings.json: {}", e);
                AppSettings::default()
            }
        },
        Err(e) => {
            eprintln!("[startup] Warning: Failed to read settings.json: {}", e);
            AppSettings::default()
        }
    }
}

/// Resolve logging configuration by merging settings.json with CLI args.
///
/// Precedence: CLI args > settings.json > defaults
fn resolve_log_config(args: &Args, data_dir: &std::path::Path) -> LogConfig {
    // Load settings from file
    let file_settings = load_settings(data_dir);
    let mut config = LogConfig::from(file_settings.logs);

    // CLI args override file settings
    if args.log_dir.is_some() {
        config.directory = args.log_dir.clone();
        config.enabled = true; // Setting log-dir enables file logging
    }

    if let Some(ref level) = args.log_level {
        config.level = level.clone();
    }

    if let Some(ref rotation) = args.log_rotation {
        config.rotation = rotation.clone();
    }

    if let Some(max_files) = args.log_max_files {
        config.max_files = max_files;
    }

    if args.log_no_stdout {
        config.suppress_stdout = true;
    }

    config
}

/// Parse rotation string into Rotation enum.
fn parse_rotation(rotation: &str) -> Rotation {
    match rotation.to_lowercase().as_str() {
        "hourly" => Rotation::HOURLY,
        "minutely" => Rotation::MINUTELY,
        "never" => Rotation::NEVER,
        _ => Rotation::DAILY,
    }
}

/// Parse log level string into Level enum.
fn parse_level(level: &str) -> Level {
    match level.to_lowercase().as_str() {
        "trace" => Level::TRACE,
        "debug" => Level::DEBUG,
        "info" => Level::INFO,
        "warn" => Level::WARN,
        "error" => Level::ERROR,
        _ => Level::INFO,
    }
}

/// Setup logging with optional file output.
///
/// Returns a guard that must be held for the lifetime of the program
/// to ensure log files are properly flushed.
fn setup_logging(
    config: &LogConfig,
    data_dir: &std::path::Path,
) -> Result<Option<tracing_appender::non_blocking::WorkerGuard>> {
    let level = parse_level(&config.level);
    let env_filter = EnvFilter::from_default_env().add_directive(level.into());

    // Check if file logging is enabled
    if config.enabled {
        // Determine log directory (default: {data_dir}/logs)
        let log_dir = config
            .directory
            .clone()
            .unwrap_or_else(|| data_dir.join("logs"));

        // Ensure log directory exists
        if !log_dir.exists() {
            fs::create_dir_all(&log_dir)?;
        }

        let rotation = parse_rotation(&config.rotation);

        // Use builder API for max_log_files support
        let file_appender = if config.max_files > 0 {
            RollingFileAppender::builder()
                .rotation(rotation)
                .filename_prefix("zbotd")
                .filename_suffix("log")
                .max_log_files(config.max_files)
                .build(&log_dir)?
        } else {
            RollingFileAppender::builder()
                .rotation(rotation)
                .filename_prefix("zbotd")
                .filename_suffix("log")
                .build(&log_dir)?
        };

        let (file_writer, file_guard) = tracing_appender::non_blocking(file_appender);

        if config.suppress_stdout {
            // Only file logging
            tracing_subscriber::registry()
                .with(env_filter)
                .with(
                    fmt::layer()
                        .with_writer(file_writer)
                        .with_ansi(false)
                        .with_target(true)
                        .with_thread_ids(true)
                        .with_file(true)
                        .with_line_number(true),
                )
                .init();
        } else {
            // Both stdout and file logging using combined writer
            let combined_writer = file_writer.and(std::io::stdout);

            tracing_subscriber::registry()
                .with(env_filter)
                .with(
                    fmt::layer()
                        .with_writer(combined_writer)
                        .with_ansi(false) // Disable ANSI for file compatibility
                        .with_target(true)
                        .with_thread_ids(false)
                        .with_file(false)
                        .with_line_number(false),
                )
                .init();
        }

        info!("File logging enabled: {:?}", log_dir);
        info!(
            "Log rotation: {}, max files: {}",
            config.rotation, config.max_files
        );

        Ok(Some(file_guard))
    } else {
        // Only stdout logging (default behavior)
        tracing_subscriber::registry()
            .with(env_filter)
            .with(
                fmt::layer()
                    .with_target(true)
                    .with_thread_ids(false)
                    .with_file(false)
                    .with_line_number(false),
            )
            .init();

        Ok(None)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Determine data directory FIRST (needed to load settings)
    let data_dir = args.data_dir.clone().unwrap_or_else(|| {
        dirs::document_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("zbot")
    });

    // Ensure data directory exists
    if !data_dir.exists() {
        fs::create_dir_all(&data_dir)?;
    }

    // Resolve logging configuration (merge settings.json + CLI args)
    let log_config = resolve_log_config(&args, &data_dir);

    // Setup logging based on merged configuration
    let _guard = setup_logging(&log_config, &data_dir)?;

    // `option_env!("BUILD_VERSION")` resolves to e.g. `2026.5.3.develop`
    // when `apps/daemon/build.rs` ran with `ZBOT_INSTALL=1` (set by
    // `make install` / `scripts/install.sh`). Otherwise it's None and
    // the bare `CARGO_PKG_VERSION` is used.
    const VERSION: &str = match option_env!("BUILD_VERSION") {
        Some(v) => v,
        None => env!("CARGO_PKG_VERSION"),
    };
    info!("z-Bot Daemon v{}", VERSION);
    info!("Data directory: {:?}", data_dir);

    // Load gateway configuration
    let mut gateway_config: GatewayConfig = if let Some(config_path) = args.config {
        info!("Loading configuration from {:?}", config_path);
        let content = fs::read_to_string(&config_path)?;
        serde_yaml::from_str(&content)?
    } else {
        GatewayConfig {
            host: args.host.parse()?,
            websocket_port: args.ws_port,
            http_port: args.http_port,
            legacy_ws_port_enabled: args.legacy_ws_port_enabled,
            ..Default::default()
        }
    };

    // Override with CLI args
    if let Some(static_dir) = args.static_dir {
        gateway_config.static_dir = Some(static_dir.to_string_lossy().to_string());
        info!("Static directory: {:?}", static_dir);
    }
    if args.no_dashboard {
        gateway_config.serve_dashboard = false;
    }

    // Create and start server
    let mut server = GatewayServer::new(gateway_config, data_dir);
    server.start().await?;

    info!("Daemon started. Press Ctrl+C to stop.");

    // Wait for shutdown signal
    tokio::signal::ctrl_c().await?;

    info!("Shutting down...");
    server.shutdown().await;

    info!("Daemon stopped.");
    Ok(())
}
