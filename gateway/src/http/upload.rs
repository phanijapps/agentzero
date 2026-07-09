//! # Upload Endpoint
//!
//! HTTP API for uploading file attachments to chat messages.

use crate::state::AppState;
use axum::{
    extract::{Multipart, State},
    http::StatusCode,
    Json,
};
use serde::Serialize;
use std::path::PathBuf;

/// Maximum file size: 50 MB.
const MAX_FILE_SIZE: u64 = 50 * 1024 * 1024;

// ============================================================================
// RESPONSE TYPES
// ============================================================================

/// Successful upload response.
#[derive(Debug, Serialize)]
pub struct UploadResponse {
    /// Unique identifier for the uploaded file.
    pub id: String,
    /// Original filename as provided by the client.
    pub filename: String,
    /// MIME type of the uploaded file.
    pub mime_type: String,
    /// Size in bytes.
    pub size: u64,
    /// Absolute path on the server filesystem.
    pub path: String,
}

/// Error response for upload failures.
#[derive(Debug, Serialize)]
pub struct UploadError {
    pub error: String,
}

// ============================================================================
// HANDLER
// ============================================================================

/// POST /api/upload
///
/// Accept a multipart file upload and persist it to `{vault}/temp/attachments/`.
/// Returns metadata including an absolute path suitable for referencing in messages.
pub async fn upload_file(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<UploadResponse>, (StatusCode, Json<UploadError>)> {
    // Extract the first file field from the multipart body.
    let field = match multipart.next_field().await {
        Ok(Some(f)) => f,
        Ok(None) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(UploadError {
                    error: "No file field in multipart body".to_string(),
                }),
            ));
        }
        Err(e) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(UploadError {
                    error: format!("Failed to read multipart field: {}", e),
                }),
            ));
        }
    };

    // Capture metadata before consuming the field body.
    let original_filename = field.file_name().unwrap_or("unnamed").to_string();
    let content_type = field
        .content_type()
        .unwrap_or("application/octet-stream")
        .to_string();

    // Read file bytes (consuming the field).
    let data = match field.bytes().await {
        Ok(b) => b,
        Err(e) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(UploadError {
                    error: format!("Failed to read file data: {}", e),
                }),
            ));
        }
    };

    let size = data.len() as u64;

    // Enforce size limit.
    if size > MAX_FILE_SIZE {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(UploadError {
                error: format!(
                    "File too large ({} bytes). Maximum allowed: {} bytes",
                    size, MAX_FILE_SIZE
                ),
            }),
        ));
    }

    // Generate a unique ID and derive the stored filename.
    let id = uuid::Uuid::new_v4().to_string();
    let extension = extension_from_filename(&original_filename)
        .or_else(|| extension_from_mime(&content_type))
        .unwrap_or_default();

    let stored_name = if extension.is_empty() {
        id.clone()
    } else {
        format!("{}.{}", id, extension)
    };

    // Build the attachments directory under the vault temp dir.
    let uploads_dir: PathBuf = state.paths.vault_dir().join("temp").join("attachments");
    if let Err(e) = std::fs::create_dir_all(&uploads_dir) {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(UploadError {
                error: format!("Failed to create uploads directory: {}", e),
            }),
        ));
    }

    let dest = uploads_dir.join(&stored_name);

    // Persist to disk.
    if let Err(e) = tokio::fs::write(&dest, &data).await {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(UploadError {
                error: format!("Failed to write file: {}", e),
            }),
        ));
    }

    // Return the absolute path so agents can read the file directly.
    let abs_path = dest.to_string_lossy().to_string();

    Ok(Json(UploadResponse {
        id,
        filename: original_filename,
        mime_type: content_type,
        size,
        path: abs_path,
    }))
}

// ============================================================================
// HELPERS
// ============================================================================

/// Extract the file extension from a filename (e.g., "report.xlsx" → "xlsx").
fn extension_from_filename(filename: &str) -> Option<String> {
    std::path::Path::new(filename)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|s| s.to_lowercase())
}

/// Derive a common file extension from a MIME type.
fn extension_from_mime(mime: &str) -> Option<String> {
    match mime {
        "image/png" => Some("png".to_string()),
        "image/jpeg" => Some("jpg".to_string()),
        "image/gif" => Some("gif".to_string()),
        "image/webp" => Some("webp".to_string()),
        "image/svg+xml" => Some("svg".to_string()),
        "application/pdf" => Some("pdf".to_string()),
        "text/plain" => Some("txt".to_string()),
        "text/csv" => Some("csv".to_string()),
        "application/json" => Some("json".to_string()),
        "application/zip" => Some("zip".to_string()),
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => {
            Some("xlsx".to_string())
        }
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
            Some("docx".to_string())
        }
        _ => None,
    }
}
