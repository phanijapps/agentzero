//! `POST /api/wards/:ward_id/open` — opens the ward folder in the OS file browser.
//!
//! Desktop-app convenience endpoint: lets the UI launch the native file browser
//! focused on the ward's vault directory. Cross-platform launcher detection is
//! resolved at compile time via `#[cfg(target_os = ...)]` so the binary always
//! picks the right tool for its host.
//!
//! No authorisation — this is a single-user desktop app. The caller picks the
//! ward, the backend resolves the path via `VaultPaths::ward_dir`, and we
//! refuse to spawn a launcher if the directory doesn't exist on disk (404).

use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Serialize;
use std::process::Command;

/// Success payload — echoes back the resolved absolute path so the UI can show
/// a user-friendly confirmation if desired.
#[derive(Debug, Serialize)]
pub struct OpenWardResponse {
    pub path: String,
}

/// Error response shape — matches the convention used by `ward_content.rs`.
#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub error: String,
}

type HandlerError = (StatusCode, Json<ErrorBody>);

/// `POST /api/wards/:ward_id/open`
///
/// Opens the ward directory in the native OS file browser.
/// - Linux:   `xdg-open`
/// - macOS:   `open`
/// - Windows: `explorer.exe`
///
/// Returns 404 when the ward directory doesn't exist on disk.
/// Returns 500 when the OS-specific launcher fails to spawn.
pub async fn open_ward_folder(
    Path(ward_id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<OpenWardResponse>, HandlerError> {
    let path = state.paths.ward_dir(&ward_id);
    if !path.exists() {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorBody {
                error: format!("Ward '{}' has no folder on disk", ward_id),
            }),
        ));
    }

    // Detached spawn — the opener process runs independently so we don't block
    // this HTTP handler waiting for the file manager window to appear/close.
    let launcher = detect_launcher();
    Command::new(launcher).arg(&path).spawn().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody {
                error: format!("Failed to open folder with {}: {}", launcher, e),
            }),
        )
    })?;

    Ok(Json(OpenWardResponse {
        path: path.display().to_string(),
    }))
}

#[cfg(target_os = "linux")]
fn detect_launcher() -> &'static str {
    "xdg-open"
}

#[cfg(target_os = "macos")]
fn detect_launcher() -> &'static str {
    "open"
}

#[cfg(target_os = "windows")]
fn detect_launcher() -> &'static str {
    "explorer.exe"
}

// Fallback for unsupported targets — `Command::new` will fail at runtime with
// a clear message routed through the 500 branch above.
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn detect_launcher() -> &'static str {
    "xdg-open"
}
