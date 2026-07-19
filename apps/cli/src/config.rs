//! Daemon URL + auth resolution.
//!
//! Precedence (high to low):
//! 1. `--url <url>` CLI flag (resolved in `main.rs`, passed into `Config::with_url`)
//! 2. `ZBOT_URL` env var
//! 3. `~/.config/zbot/cli.toml` (`daemon_url = "..."`)
//! 4. Default: `http://localhost:18791`

use anyhow::Result;
use serde::Deserialize;
use std::path::PathBuf;

pub const DEFAULT_DAEMON_URL: &str = "http://localhost:18791";

#[derive(Debug, Clone)]
pub struct Config {
    pub daemon_url: String,
}

#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    #[serde(default)]
    daemon_url: Option<String>,
}

impl Config {
    /// Resolve config from precedence chain. `cli_override` is the `--url` flag value.
    pub fn resolve(cli_override: Option<String>) -> Result<Self> {
        // 1. CLI flag wins
        if let Some(url) = cli_override.and_then(non_empty) {
            return Ok(Self {
                daemon_url: normalise(url),
            });
        }

        // 2. Env var
        if let Some(url) = std::env::var("ZBOT_URL").ok().and_then(non_empty) {
            return Ok(Self {
                daemon_url: normalise(url),
            });
        }

        // 3. Config file
        if let Some(url) = file_config_url()? {
            return Ok(Self {
                daemon_url: normalise(url),
            });
        }

        // 4. Default
        Ok(Self {
            daemon_url: DEFAULT_DAEMON_URL.to_string(),
        })
    }

    /// Derive the WebSocket URL from the HTTP URL by swapping the scheme.
    /// Used by Phase 2 to open the `/ws` upgrade.
    #[allow(dead_code)]
    pub fn websocket_url(&self) -> String {
        let base = self.daemon_url.trim_end_matches('/');
        if let Some(rest) = base.strip_prefix("https://") {
            format!("wss://{}/ws", rest)
        } else if let Some(rest) = base.strip_prefix("http://") {
            format!("ws://{}/ws", rest)
        } else {
            format!("ws://{}/ws", base)
        }
    }
}

fn file_config_url() -> Result<Option<String>> {
    let path = config_file_path();
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path)?;
    let parsed: FileConfig = toml::from_str(&raw)?;
    Ok(parsed.daemon_url.and_then(non_empty))
}

fn config_file_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("zbot")
        .join("cli.toml")
}

fn non_empty(s: String) -> Option<String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn normalise(url: String) -> String {
    url.trim_end_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_url_when_nothing_provided() {
        // Ensure env var doesn't leak from host
        std::env::remove_var("ZBOT_URL");
        // We can't easily isolate the file lookup here without touching the user's
        // real config dir, so this test runs in environments where the file is absent.
        // In CI / dev boxes without ~/.config/zbot/cli.toml it confirms the default.
        if !config_file_path().exists() {
            let c = Config::resolve(None).unwrap();
            assert_eq!(c.daemon_url, DEFAULT_DAEMON_URL);
        }
    }

    #[test]
    fn cli_override_wins() {
        let c = Config::resolve(Some("http://example.com:9000/".into())).unwrap();
        assert_eq!(c.daemon_url, "http://example.com:9000");
    }

    #[test]
    fn ws_url_from_http() {
        let c = Config {
            daemon_url: "http://localhost:18791".into(),
        };
        assert_eq!(c.websocket_url(), "ws://localhost:18791/ws");
    }

    #[test]
    fn ws_url_from_https() {
        let c = Config {
            daemon_url: "https://zbot.example".into(),
        };
        assert_eq!(c.websocket_url(), "wss://zbot.example/ws");
    }
}
