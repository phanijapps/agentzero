//! # Gateway Configuration
//!
//! Configuration for the gateway server.

use serde::{Deserialize, Serialize};
use std::net::IpAddr;

/// Gateway server configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayConfig {
    /// Host address to bind to.
    #[serde(default = "default_host")]
    pub host: IpAddr,

    /// HTTP port.
    #[serde(default = "default_http_port")]
    pub http_port: u16,

    /// Enable CORS for development.
    #[serde(default)]
    pub cors_enabled: bool,

    /// Allowed CORS origins (if cors_enabled).
    #[serde(default)]
    pub cors_origins: Vec<String>,

    /// Path to static files for web dashboard (optional).
    #[serde(default)]
    pub static_dir: Option<String>,

    /// Enable serving the web dashboard.
    #[serde(default = "default_serve_dashboard")]
    pub serve_dashboard: bool,

    /// Enable capability-gated agent work-surface events. Enabled by default;
    /// the daemon's `--no-agent-surfaces` flag provides an immediate rollback.
    #[serde(default = "default_agent_surfaces_enabled")]
    pub agent_surfaces_enabled: bool,

    /// Enable the authenticated A2A federation surface. Disabled by default.
    #[serde(default)]
    pub a2a_enabled: bool,

    /// Public origin advertised in the A2A Agent Card.
    #[serde(default)]
    pub a2a_public_base_url: Option<String>,

    /// Exact browser origins allowed to call authenticated A2A routes.
    #[serde(default)]
    pub a2a_allowed_origins: Vec<String>,

    /// Operator-approved public instructions used for inbound remote work.
    #[serde(default = "default_a2a_public_skill_instructions")]
    pub a2a_public_skill_instructions: String,
}

fn default_serve_dashboard() -> bool {
    true
}

fn default_agent_surfaces_enabled() -> bool {
    true
}

fn default_a2a_public_skill_instructions() -> String {
    "Answer the bounded remote text request without taking external actions.".to_string()
}

fn default_host() -> IpAddr {
    "127.0.0.1".parse().unwrap()
}

fn default_http_port() -> u16 {
    crate::DEFAULT_HTTP_PORT
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            http_port: default_http_port(),
            cors_enabled: true,
            cors_origins: vec![
                "http://localhost:1420".to_string(), // Tauri dev
                "http://localhost:3000".to_string(), // Web dev
            ],
            static_dir: None,
            serve_dashboard: true,
            agent_surfaces_enabled: true,
            a2a_enabled: false,
            a2a_public_base_url: None,
            a2a_allowed_origins: Vec::new(),
            a2a_public_skill_instructions: default_a2a_public_skill_instructions(),
        }
    }
}

impl GatewayConfig {
    /// Get the HTTP bind address.
    pub fn http_addr(&self) -> String {
        format!("{}:{}", self.host, self.http_port)
    }
}

/// Resolve the effective bind host from a `DiscoveryConfig`.
///
/// Precedence:
/// 1. `advanced.bind_host` if present and parseable.
/// 2. `0.0.0.0` if `expose_to_lan = true`.
/// 3. `127.0.0.1` otherwise.
///
/// Garbage in `advanced.bind_host` falls back to loopback rather than
/// crashing — surfacing the misconfiguration via logs is better than
/// failing to start. Defensive: when an override is set but invalid,
/// we deliberately do NOT fall through to `expose_to_lan` — a typo
/// must never accidentally publish the daemon on 0.0.0.0.
pub fn resolve_bind_host(cfg: &discovery::DiscoveryConfig) -> std::net::IpAddr {
    use std::net::{IpAddr, Ipv4Addr};
    if let Some(s) = cfg.advanced.bind_host.as_deref() {
        if let Ok(parsed) = s.parse::<IpAddr>() {
            return parsed;
        }
        tracing::warn!(
            target: "discovery",
            "ignoring invalid network.advanced.bindHost={:?}; falling back to loopback for safety",
            s
        );
        return IpAddr::V4(Ipv4Addr::LOCALHOST);
    }
    if cfg.expose_to_lan {
        IpAddr::V4(Ipv4Addr::UNSPECIFIED)
    } else {
        IpAddr::V4(Ipv4Addr::LOCALHOST)
    }
}

#[cfg(test)]
mod resolve_bind_tests {
    use super::*;
    use discovery::{AdvancedConfig, DiscoveryConfig};
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn off_yields_loopback() {
        let cfg = DiscoveryConfig {
            expose_to_lan: false,
            ..Default::default()
        };
        assert_eq!(resolve_bind_host(&cfg), IpAddr::V4(Ipv4Addr::LOCALHOST));
    }

    #[test]
    fn on_yields_unspecified() {
        let cfg = DiscoveryConfig::default();
        assert!(cfg.expose_to_lan);
        assert_eq!(resolve_bind_host(&cfg), IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    }

    #[test]
    fn advanced_override_wins_when_present_and_valid() {
        let cfg = DiscoveryConfig {
            expose_to_lan: false, // would be loopback…
            advanced: AdvancedConfig {
                bind_host: Some("10.1.2.3".into()), // …but override wins
                http_port: 18791,
            },
            ..Default::default()
        };
        assert_eq!(
            resolve_bind_host(&cfg),
            IpAddr::V4(Ipv4Addr::new(10, 1, 2, 3))
        );
    }

    #[test]
    fn advanced_override_with_garbage_falls_back_to_loopback() {
        let cfg = DiscoveryConfig {
            advanced: AdvancedConfig {
                bind_host: Some("not-an-ip".into()),
                http_port: 18791,
            },
            ..Default::default()
        };
        assert_eq!(resolve_bind_host(&cfg), IpAddr::V4(Ipv4Addr::LOCALHOST));
    }
}

#[cfg(test)]
mod gateway_config_tests {
    use super::*;

    #[test]
    fn default_serves_dashboard_with_dev_origins() {
        let cfg = GatewayConfig::default();
        assert!(cfg.serve_dashboard);
        assert!(cfg.cors_enabled);
        assert!(cfg.agent_surfaces_enabled);
        assert!(!cfg.a2a_enabled);
        assert!(cfg
            .cors_origins
            .contains(&"http://localhost:1420".to_string()));
        assert!(cfg
            .cors_origins
            .contains(&"http://localhost:3000".to_string()));
        assert!(cfg.static_dir.is_none());
        assert_eq!(cfg.host, default_host());
    }

    #[test]
    fn http_addr_format() {
        let cfg = GatewayConfig {
            http_port: 22222,
            ..Default::default()
        };
        assert_eq!(cfg.http_addr(), format!("{}:22222", cfg.host));
    }

    #[test]
    fn default_helpers_match_consts() {
        assert_eq!(default_http_port(), crate::DEFAULT_HTTP_PORT);
        assert!(default_serve_dashboard());
    }

    #[test]
    fn default_host_is_loopback() {
        assert!(default_host().is_loopback());
    }

    #[test]
    fn json_round_trip_preserves_fields() {
        let cfg = GatewayConfig {
            http_port: 40002,
            ..Default::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let parsed: GatewayConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.http_port, cfg.http_port);
        assert_eq!(parsed.serve_dashboard, cfg.serve_dashboard);
    }

    #[test]
    fn json_with_missing_fields_uses_serde_defaults() {
        let cfg: GatewayConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(cfg.host, default_host());
        assert_eq!(cfg.http_port, default_http_port());
        assert!(cfg.serve_dashboard);
        assert!(!cfg.cors_enabled);
        assert!(cfg.cors_origins.is_empty());
        assert!(cfg.static_dir.is_none());
        assert!(cfg.agent_surfaces_enabled);
        assert!(!cfg.a2a_enabled);
    }
}
