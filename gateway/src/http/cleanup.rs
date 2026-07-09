//! # Cleanup Endpoints
//!
//! Bounded admin operations that clear vault-owned ephemeral storage.
//! `POST /api/cleanup/vault-temp` wipes `<vault>/temp/` either entirely
//! (no body / `olderThanHours: null`) or just files older than N hours
//! (`olderThanHours: N`). Recurses into subdirectories and removes them
//! once their contents are gone.
//!
//! Path is hardcoded to [`VaultPaths::temp_dir`] — no caller-supplied
//! paths, no path traversal risk. Symlinks are deleted as links (never
//! followed into) so a malicious link can't escape the temp tree.

use crate::state::AppState;
use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};
use tracing::{info, warn};

/// Body for `POST /api/cleanup/vault-temp`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupTempRequest {
    /// Delete only files modified more than this many hours ago. When
    /// `None` (or absent), wipe everything in the temp tree.
    #[serde(default)]
    pub older_than_hours: Option<u64>,
}

/// Response shape for `POST /api/cleanup/vault-temp`.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupTempResponse {
    pub deleted_files: usize,
    pub total_bytes: u64,
    pub removed_directories: usize,
    pub errors: usize,
}

/// `POST /api/cleanup/vault-temp` — clean the vault temp directory.
pub async fn cleanup_vault_temp(
    State(state): State<AppState>,
    body: Option<Json<CleanupTempRequest>>,
) -> Json<CleanupTempResponse> {
    let request = body.map(|Json(b)| b).unwrap_or_default();
    let temp_dir = state.paths.temp_dir();
    let threshold = request
        .older_than_hours
        .map(|h| SystemTime::now() - Duration::from_secs(h.saturating_mul(3600)));

    info!(
        target_dir = %temp_dir.display(),
        older_than_hours = ?request.older_than_hours,
        "Vault temp cleanup requested"
    );

    let mut stats = CleanupTempResponse::default();
    if temp_dir.exists() {
        clean_recursive(&temp_dir, threshold, &mut stats);
    } else {
        info!("Vault temp dir does not exist; nothing to clean");
    }

    info!(
        deleted_files = stats.deleted_files,
        total_bytes = stats.total_bytes,
        removed_directories = stats.removed_directories,
        errors = stats.errors,
        "Vault temp cleanup completed"
    );

    Json(stats)
}

/// Walk `dir` depth-first, deleting files that match `threshold` (or
/// all files when `threshold` is `None`). After a directory's contents
/// are removed, the directory itself is removed if empty. Symlinks are
/// deleted as links (never followed). Errors are counted, never raised.
fn clean_recursive(dir: &Path, threshold: Option<SystemTime>, stats: &mut CleanupTempResponse) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            warn!(dir = %dir.display(), error = %e, "Failed to read directory");
            stats.errors += 1;
            return;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let meta = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => {
                stats.errors += 1;
                continue;
            }
        };

        if meta.file_type().is_symlink() {
            // Delete the link itself; never follow.
            if eligible_for_delete(&meta, threshold) {
                match fs::remove_file(&path) {
                    Ok(_) => stats.deleted_files += 1,
                    Err(_) => stats.errors += 1,
                }
            }
            continue;
        }

        if meta.is_dir() {
            clean_recursive(&path, threshold, stats);
            // Remove the directory if it's now empty. Skips silently
            // when the dir still has un-deletable contents.
            if is_dir_empty(&path) && fs::remove_dir(&path).is_ok() {
                stats.removed_directories += 1;
            }
            continue;
        }

        if meta.is_file() && eligible_for_delete(&meta, threshold) {
            let size = meta.len();
            match fs::remove_file(&path) {
                Ok(_) => {
                    stats.deleted_files += 1;
                    stats.total_bytes += size;
                }
                Err(_) => stats.errors += 1,
            }
        }
    }
}

fn eligible_for_delete(meta: &fs::Metadata, threshold: Option<SystemTime>) -> bool {
    eligible_by_mtime(meta.modified().ok(), threshold)
}

/// The pure decision: given a file's mtime (or `None` if unknown) and
/// an optional threshold, decide whether to delete. No threshold = yes.
/// Threshold + known mtime = yes iff mtime is older. Threshold +
/// unknown mtime = no (safe default).
fn eligible_by_mtime(mtime: Option<SystemTime>, threshold: Option<SystemTime>) -> bool {
    let Some(threshold) = threshold else {
        return true;
    };
    match mtime {
        Some(m) => m < threshold,
        None => false,
    }
}

fn is_dir_empty(dir: &Path) -> bool {
    fs::read_dir(dir)
        .map(|mut entries| entries.next().is_none())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use std::time::SystemTime;
    use tempfile::tempdir;

    fn touch(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut f = File::create(path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    #[test]
    fn empty_temp_dir_is_a_noop() {
        let tmp = tempdir().unwrap();
        let mut stats = CleanupTempResponse::default();
        clean_recursive(tmp.path(), None, &mut stats);
        assert_eq!(stats.deleted_files, 0);
        assert_eq!(stats.total_bytes, 0);
        assert_eq!(stats.removed_directories, 0);
        assert_eq!(stats.errors, 0);
    }

    #[test]
    fn wipes_everything_when_threshold_is_none() {
        let tmp = tempdir().unwrap();
        touch(&tmp.path().join("a.txt"), "hello");
        touch(&tmp.path().join("nested/b.log"), "world");
        touch(&tmp.path().join("nested/deep/c.json"), "{}");

        let mut stats = CleanupTempResponse::default();
        clean_recursive(tmp.path(), None, &mut stats);

        assert_eq!(stats.deleted_files, 3);
        assert!(stats.total_bytes > 0);
        // Both nested dirs should be removed (deep first, then nested).
        assert_eq!(stats.removed_directories, 2);
        // The temp root itself is preserved (it's the bound of the operation).
        assert!(tmp.path().exists());
        assert!(is_dir_empty(tmp.path()));
    }

    #[test]
    fn threshold_keeps_fresh_files_when_no_old_files_exist() {
        // All files just created → mtime is "now" → with a 24h
        // threshold, none qualify. Tests that fresh files survive.
        let tmp = tempdir().unwrap();
        touch(&tmp.path().join("fresh1.txt"), "a");
        touch(&tmp.path().join("fresh2.txt"), "b");
        touch(&tmp.path().join("sub/fresh3.txt"), "c");

        let mut stats = CleanupTempResponse::default();
        let threshold = Some(SystemTime::now() - Duration::from_secs(24 * 3600));
        clean_recursive(tmp.path(), threshold, &mut stats);

        assert_eq!(stats.deleted_files, 0);
        // sub/ stays because its child is still there.
        assert_eq!(stats.removed_directories, 0);
        assert!(tmp.path().join("fresh1.txt").exists());
        assert!(tmp.path().join("sub/fresh3.txt").exists());
    }

    #[test]
    fn empty_subdirectory_is_removed_even_with_threshold() {
        // A pre-existing empty subdir gets removed regardless of
        // threshold (no files to gate on). Important: this is what the
        // recursive sweep does after past cleanups have emptied dirs.
        let tmp = tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("empty/inner")).unwrap();

        let mut stats = CleanupTempResponse::default();
        let threshold = Some(SystemTime::now() - Duration::from_secs(24 * 3600));
        clean_recursive(tmp.path(), threshold, &mut stats);

        assert_eq!(stats.deleted_files, 0);
        assert_eq!(stats.removed_directories, 2);
        assert!(!tmp.path().join("empty").exists());
    }

    // ---- Pure predicate tests for the mtime decision logic ----

    #[test]
    fn eligible_by_mtime_no_threshold_means_always_yes() {
        assert!(eligible_by_mtime(Some(SystemTime::now()), None));
        assert!(eligible_by_mtime(None, None));
    }

    #[test]
    fn eligible_by_mtime_old_file_qualifies() {
        let now = SystemTime::now();
        let old_mtime = now - Duration::from_secs(48 * 3600);
        let threshold = now - Duration::from_secs(24 * 3600);
        assert!(eligible_by_mtime(Some(old_mtime), Some(threshold)));
    }

    #[test]
    fn eligible_by_mtime_fresh_file_does_not_qualify() {
        let now = SystemTime::now();
        let fresh_mtime = now - Duration::from_secs(60); // 1 min ago
        let threshold = now - Duration::from_secs(24 * 3600);
        assert!(!eligible_by_mtime(Some(fresh_mtime), Some(threshold)));
    }

    #[test]
    fn eligible_by_mtime_unknown_with_threshold_is_no() {
        // Safer default: if we can't read mtime, leave it alone.
        let threshold = Some(SystemTime::now() - Duration::from_secs(24 * 3600));
        assert!(!eligible_by_mtime(None, threshold));
    }
}
