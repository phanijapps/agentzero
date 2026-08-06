//! `zbot` — lightweight streaming Claude-Code-style CLI for the z-Bot daemon.
//!
//! Architecture: this CLI is a thin front-end. The interactive mode uses
//! `rustyline` for input editing + direct stdout streaming for output —
//! no full-screen TUI framework. Every byte printed stays printed; no
//! re-renders, no border math, no layout drift.
//!
//! Modes
//! -----
//! - `zbot`                                — interactive REPL (rustyline + stream)
//! - `zbot "do X"`                         — one-shot, prints + exits
//! - `cat file.md | zbot "summarise"`      — stdin is prepended to message
//! - `cat file.md | zbot`                  — stdin is the whole message
//! - `zbot --url http://desktop:18791`     — connect to a remote daemon

mod client;
mod config;
mod events;
mod oneshot;
mod peers;
mod repl;
mod slash;
mod stream;
mod style;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::{io::IsTerminal, path::PathBuf};

use crate::client::DaemonClient;
use crate::config::Config;
use crate::events::EventStream;

#[derive(Parser, Debug)]
#[command(
    name = "zbot",
    version,
    about = "Streaming chat client for the z-Bot daemon",
    long_about = None,
)]
struct Args {
    /// Daemon base URL (overrides ZBOT_URL and config file).
    #[arg(long, value_name = "URL")]
    url: Option<String>,

    /// Resume a specific session by id (interactive mode only).
    #[arg(long, value_name = "ID")]
    session: Option<String>,

    /// Disable ANSI colors (also auto-disabled if $NO_COLOR is set or
    /// stdout is not a terminal).
    #[arg(long)]
    no_color: bool,

    /// Invoke a catalog-approved surface action through the gateway registry.
    #[arg(long, value_name = "ACTION", requires_all = ["surface_target", "expected_state"])]
    surface_action: Option<String>,

    /// Target id for --surface-action (for example, an autonomy item id).
    #[arg(long, value_name = "ID")]
    surface_target: Option<String>,

    /// Required current target state; prevents stale or replayed mutations.
    #[arg(long, value_name = "STATE")]
    expected_state: Option<String>,

    /// z-Bot data directory for local file commands (default: ~/Documents/zbot).
    #[arg(long, value_name = "DIR")]
    data_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Commands>,

    /// One-shot prompt. When provided, sends and exits on turn completion.
    /// If stdin is not a TTY, its contents are prepended to this message.
    prompt: Option<String>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Manage explicitly trusted A2A peers using the local peer store.
    Peers(peers::PeersArgs),
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    let args = Args::parse();

    if let Some(Commands::Peers(peers_args)) = args.command {
        peers::run(peers_args, resolve_data_dir(args.data_dir)?)
            .await
            .context("manage A2A peers")?;
        return Ok(());
    }

    let cfg = Config::resolve(args.url.clone()).context("resolve daemon URL")?;
    let client = DaemonClient::new(cfg.clone());

    client
        .health()
        .await
        .with_context(|| format!("daemon unreachable at {}", cfg.daemon_url))?;

    if let Some(action_id) = args.surface_action.as_deref() {
        client
            .invoke_surface_action(
                action_id,
                args.surface_target
                    .as_deref()
                    .expect("clap requires target"),
                args.expected_state.as_deref().expect("clap requires state"),
            )
            .await
            .context("invoke surface action")?;
        return Ok(());
    }

    let chat = client
        .init_chat_session()
        .await
        .context("init chat session")?;

    let events = EventStream::connect(&cfg.websocket_url())
        .await
        .with_context(|| format!("ws connect to {}", cfg.websocket_url()))?;

    let color = use_color(args.no_color);

    match pick_mode(&args) {
        Mode::Interactive => {
            crate::repl::run(chat, cfg.daemon_url.clone(), events, client.clone(), color)
                .await
                .context("interactive REPL")?;
        }
        Mode::OneShot => {
            let message = oneshot::compose_message(args.prompt.clone())
                .context("compose user message from args + stdin")?;
            oneshot::run_oneshot(chat, events, message, color)
                .await
                .context("one-shot turn")?;
        }
    }

    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum Mode {
    Interactive,
    OneShot,
}

fn pick_mode(args: &Args) -> Mode {
    if args.prompt.is_some() {
        return Mode::OneShot;
    }
    if !std::io::stdin().is_terminal() {
        return Mode::OneShot;
    }
    if !std::io::stdout().is_terminal() {
        return Mode::OneShot;
    }
    Mode::Interactive
}

fn use_color(no_color_flag: bool) -> bool {
    if no_color_flag {
        return false;
    }
    if std::env::var_os("NO_COLOR")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
    {
        return false;
    }
    std::io::stdout().is_terminal()
}

fn resolve_data_dir(override_dir: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = override_dir {
        return Ok(path);
    }
    Ok(dirs::document_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("zbot"))
}

fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_env("ZBOT_LOG").unwrap_or_else(|_| EnvFilter::new("warn"));
    fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}
