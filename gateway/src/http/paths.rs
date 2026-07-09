//! # Paths Endpoint
//!
//! `GET /api/paths` — exposes the daemon's vault directory and key
//! sub-directories so the UI can render accurate copy. The vault root is
//! resolved at process start via `dirs::document_dir().or(dirs::home_dir())`
//! so it varies across hosts (e.g., `~/Documents/zbot/` on a desktop with
//! `~/Documents/`, `~/zbot/` on a Pi without it). Hardcoding `~/Documents/...`
//! in the UI misled users on those hosts.

use crate::services::VaultPaths;
use crate::state::AppState;
use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Response body for `GET /api/paths`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathsResponse {
    pub vault_dir: String,
    pub config_dir: String,
    pub logs_dir: String,
    pub plugins_dir: String,
    pub agents_dir: String,
    /// Display variants: `$HOME` prefix replaced with `~`. UI-only sugar.
    pub vault_dir_display: String,
    pub config_dir_display: String,
    pub logs_dir_display: String,
    pub plugins_dir_display: String,
}

/// Replace a `$HOME` prefix on `path` with `~`. Falls back to the absolute
/// path when `home` is `None` or `path` doesn't start with `home`.
fn pretty(path: &Path, home: Option<&Path>) -> String {
    let abs = path.to_string_lossy().into_owned();
    if let Some(h) = home {
        let h = h.to_string_lossy();
        if let Some(rest) = abs.strip_prefix(h.as_ref()) {
            return format!("~{}", rest);
        }
    }
    abs
}

fn build_response(paths: &VaultPaths, home: Option<&Path>) -> PathsResponse {
    let vault_dir = paths.vault_dir().clone();
    let config_dir = paths.config_dir();
    let logs_dir = paths.logs_dir();
    let plugins_dir = paths.plugins_dir();
    let agents_dir = paths.agents_dir();

    PathsResponse {
        vault_dir_display: pretty(&vault_dir, home),
        config_dir_display: pretty(&config_dir, home),
        logs_dir_display: pretty(&logs_dir, home),
        plugins_dir_display: pretty(&plugins_dir, home),
        vault_dir: vault_dir.to_string_lossy().into_owned(),
        config_dir: config_dir.to_string_lossy().into_owned(),
        logs_dir: logs_dir.to_string_lossy().into_owned(),
        plugins_dir: plugins_dir.to_string_lossy().into_owned(),
        agents_dir: agents_dir.to_string_lossy().into_owned(),
    }
}

/// `GET /api/paths` — vault path snapshot for the UI.
pub async fn get_paths(State(state): State<AppState>) -> Json<PathsResponse> {
    let home = dirs::home_dir();
    Json(build_response(&state.paths, home.as_deref()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::VaultPaths;
    use std::path::PathBuf;

    #[test]
    fn paths_response_includes_all_dirs() {
        let vault = PathBuf::from("/tmp/test-vault");
        let paths = VaultPaths::new(vault.clone());

        let response = build_response(&paths, None);

        assert_eq!(response.vault_dir, "/tmp/test-vault");
        assert_eq!(response.config_dir, "/tmp/test-vault/config");
        assert_eq!(response.logs_dir, "/tmp/test-vault/logs");
        assert_eq!(response.plugins_dir, "/tmp/test-vault/plugins");
        assert_eq!(response.agents_dir, "/tmp/test-vault/agents");
    }

    #[test]
    fn display_variant_replaces_home_prefix() {
        let home = PathBuf::from("/tmp/test-home");
        let vault = home.join("zbot");
        let paths = VaultPaths::new(vault);

        let response = build_response(&paths, Some(&home));

        assert_eq!(response.vault_dir_display, "~/zbot");
        assert_eq!(response.config_dir_display, "~/zbot/config");
        assert_eq!(response.logs_dir_display, "~/zbot/logs");
        assert_eq!(response.plugins_dir_display, "~/zbot/plugins");
    }

    #[test]
    fn display_variant_passes_through_when_home_not_prefix() {
        let home = PathBuf::from("/home/alice");
        let vault = PathBuf::from("/var/data/zbot");
        let paths = VaultPaths::new(vault);

        let response = build_response(&paths, Some(&home));

        assert_eq!(response.vault_dir_display, "/var/data/zbot");
        assert_eq!(response.config_dir_display, "/var/data/zbot/config");
    }

    #[test]
    fn pretty_handles_missing_home() {
        let path = PathBuf::from("/tmp/foo/zbot");
        assert_eq!(pretty(&path, None), "/tmp/foo/zbot");
    }
}
