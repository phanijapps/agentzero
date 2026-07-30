//! # WardUsage — per-ward telemetry sidecar
//!
//! Persistent record of how each ward is used, written to
//! `<vault>/wards/.usage.json`. Feeds the ward curator (heuristic cleanup
//! and LLM consolidation) defined in
//! `docs/architecture/future-state/2026-05-23-ward-curator-spec.md`.
//!
//! Operations are serialised through an internal `std::sync::Mutex` so
//! concurrent bumps from within the same daemon never lose updates. Writes
//! are atomic at the filesystem layer (temp file + `rename(2)`). Cross-
//! process safety is not guaranteed in this version — a single daemon owns
//! the sidecar at any moment.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use agent_primitives::WardArchetypeId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

const MAX_WARD_USAGE_BYTES: usize = 4 * 1024 * 1024;

/// How a ward came into existence. Drives whether the curator may act on it.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WardProvenance {
    /// Seeded at boot by code (`scratch`, `wiki`). Never curator-touched.
    Bundled,
    /// Authored by the user (manual file creation). Never curator-touched.
    /// Default for unknown wards — the conservative safe option.
    #[default]
    User,
    /// Scaffolded by the cold-path planner → builder flow. Curator-eligible.
    Agent,
}

/// Lifecycle state of a ward in the curator's eyes.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WardState {
    #[default]
    Active,
    Stale,
    Archived,
}

/// One row in `.usage.json`, keyed externally by ward name.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WardRecord {
    #[serde(default)]
    pub use_count: u64,
    #[serde(default)]
    pub patch_count: u64,
    #[serde(default)]
    pub last_used_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_patched_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub created_by: WardProvenance,
    #[serde(default)]
    pub state: WardState,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub archived_at: Option<DateTime<Utc>>,
    /// Creation-time provenance only. The copied `ward-conf.yaml` snapshot
    /// remains the structural authority if this metadata is absent or stale.
    #[serde(default)]
    pub archetype: Option<WardArchetypeId>,
}

impl WardRecord {
    fn new_at(now: DateTime<Utc>, created_by: WardProvenance) -> Self {
        Self {
            use_count: 0,
            patch_count: 0,
            last_used_at: None,
            last_patched_at: None,
            created_at: now,
            created_by,
            state: WardState::Active,
            pinned: false,
            archived_at: None,
            archetype: None,
        }
    }
}

pub type WardUsageMap = BTreeMap<String, WardRecord>;

/// Service that owns the `wards/.usage.json` sidecar.
pub struct WardUsage {
    wards_dir: PathBuf,
    lock: Mutex<()>,
}

#[cfg(target_os = "linux")]
fn save_sidecar_linux(wards_dir: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;

    let directory = open_verified_wards_directory_linux(wards_dir)?;

    let temporary = format!(".usage.{}.tmp", uuid::Uuid::new_v4());
    let mut file = create_sidecar_temp_at(&directory, &temporary).map_err(|e| e.to_string())?;
    let result = (|| {
        let metadata = file.metadata().map_err(|error| error.to_string())?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.nlink() != 1 {
            return Err("ward usage temporary file is unsafe".into());
        }
        file.write_all(bytes).map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        rename_sidecar_at(&directory, &temporary, ".usage.json")
            .map_err(|error| error.to_string())?;
        directory.sync_all().map_err(|error| error.to_string())
    })();
    if result.is_err() {
        let _ = unlink_sidecar_at(&directory, &temporary);
    }
    result
}

#[cfg(target_os = "linux")]
fn open_verified_wards_directory_linux(wards_dir: &Path) -> Result<File, String> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    const O_DIRECTORY: i32 = 0o200000;
    const O_NOFOLLOW: i32 = 0o400000;
    const O_CLOEXEC: i32 = 0o2000000;
    let before = std::fs::symlink_metadata(wards_dir).map_err(|error| error.to_string())?;
    if before.file_type().is_symlink() || !before.is_dir() {
        return Err("wards directory is unsafe".into());
    }
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
        .open(wards_dir)
        .map_err(|error| error.to_string())?;
    let opened = directory.metadata().map_err(|error| error.to_string())?;
    if before.dev() != opened.dev() || before.ino() != opened.ino() || !opened.is_dir() {
        return Err("wards directory changed while opening".into());
    }
    Ok(directory)
}

#[cfg(target_os = "linux")]
fn create_sidecar_temp_at(parent: &File, name: &str) -> std::io::Result<File> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};

    unsafe extern "C" {
        fn openat(dirfd: i32, pathname: *const std::ffi::c_char, flags: i32, ...) -> i32;
    }
    let name = CString::new(name).map_err(|_| std::io::Error::other("invalid temp name"))?;
    let flags = 0o1 | 0o100 | 0o200 | 0o400000 | 0o2000000;
    // SAFETY: `name` is NUL-terminated, `parent` is a live directory fd,
    // and a mode is supplied because O_CREAT is set.
    let fd = unsafe { openat(parent.as_raw_fd(), name.as_ptr(), flags, 0o600) };
    if fd == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        // SAFETY: `openat` returned a fresh descriptor now owned by `File`.
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

#[cfg(target_os = "linux")]
fn rename_sidecar_at(parent: &File, old_name: &str, new_name: &str) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::fd::AsRawFd;

    unsafe extern "C" {
        fn renameat(
            olddirfd: i32,
            oldpath: *const std::ffi::c_char,
            newdirfd: i32,
            newpath: *const std::ffi::c_char,
        ) -> i32;
    }
    let old_name =
        CString::new(old_name).map_err(|_| std::io::Error::other("invalid temp name"))?;
    let new_name =
        CString::new(new_name).map_err(|_| std::io::Error::other("invalid sidecar name"))?;
    // SAFETY: both names are NUL-terminated and `parent` is a live directory fd.
    if unsafe {
        renameat(
            parent.as_raw_fd(),
            old_name.as_ptr(),
            parent.as_raw_fd(),
            new_name.as_ptr(),
        )
    } == -1
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn unlink_sidecar_at(parent: &File, name: &str) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::fd::AsRawFd;

    unsafe extern "C" {
        fn unlinkat(dirfd: i32, pathname: *const std::ffi::c_char, flags: i32) -> i32;
    }
    let name = CString::new(name).map_err(|_| std::io::Error::other("invalid temp name"))?;
    // SAFETY: `name` is NUL-terminated and `parent` is a live directory fd.
    if unsafe { unlinkat(parent.as_raw_fd(), name.as_ptr(), 0) } == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn load_sidecar_linux(wards_dir: &Path) -> Result<Option<String>, String> {
    use std::os::unix::fs::MetadataExt;

    let directory = open_verified_wards_directory_linux(wards_dir)?;
    let mut file = match open_sidecar_at(&directory, ".usage.json") {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.nlink() != 1 {
        return Err("ward usage sidecar is unsafe".into());
    }
    if metadata.len() > MAX_WARD_USAGE_BYTES as u64 {
        return Err("ward usage sidecar exceeds the byte limit".into());
    }
    let mut bytes = Vec::with_capacity((metadata.len() as usize).min(MAX_WARD_USAGE_BYTES));
    Read::by_ref(&mut file)
        .take((MAX_WARD_USAGE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > MAX_WARD_USAGE_BYTES {
        return Err("ward usage sidecar exceeds the byte limit".into());
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| "ward usage sidecar must be valid UTF-8".into())
}

#[cfg(target_os = "linux")]
fn open_sidecar_at(parent: &File, name: &str) -> std::io::Result<File> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};

    unsafe extern "C" {
        fn openat(dirfd: i32, pathname: *const std::ffi::c_char, flags: i32, ...) -> i32;
    }
    let name = CString::new(name).map_err(|_| std::io::Error::other("invalid sidecar name"))?;
    let flags = 0o4000 | 0o400000 | 0o2000000;
    // SAFETY: `name` is NUL-terminated and `parent` is a live directory fd.
    let fd = unsafe { openat(parent.as_raw_fd(), name.as_ptr(), flags) };
    if fd == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        // SAFETY: `openat` returned a fresh descriptor now owned by `File`.
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

#[cfg(not(target_os = "linux"))]
fn save_sidecar_portable(wards_dir: &Path, bytes: &[u8]) -> Result<(), String> {
    let root = std::fs::symlink_metadata(wards_dir).map_err(|error| error.to_string())?;
    if root.file_type().is_symlink() || !root.is_dir() {
        return Err("wards directory is unsafe".into());
    }
    let canonical_root = std::fs::canonicalize(wards_dir).map_err(|error| error.to_string())?;
    let temporary = wards_dir.join(format!(".usage.{}.tmp", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    let result = (|| {
        let metadata = file.metadata().map_err(|error| error.to_string())?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || !portable_opened_file_has_single_link(&file)
        {
            return Err("ward usage temporary file is unsafe".into());
        }
        if !std::fs::canonicalize(&temporary)
            .map_err(|error| error.to_string())?
            .starts_with(&canonical_root)
        {
            return Err("ward usage temporary file escaped the wards directory".into());
        }
        file.write_all(bytes).map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        replace_sidecar_portable(&temporary, &wards_dir.join(".usage.json"))
            .map_err(|error| error.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(all(not(target_os = "linux"), not(windows)))]
fn replace_sidecar_portable(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::rename(source, destination)
}

#[cfg(windows)]
fn replace_sidecar_portable(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(
            existing_file_name: *const u16,
            new_file_name: *const u16,
            flags: u32,
        ) -> i32;
    }
    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    // SAFETY: both paths are NUL-terminated UTF-16 buffers alive for the call.
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(target_os = "linux"))]
fn load_sidecar_portable(wards_dir: &Path) -> Result<Option<String>, String> {
    let path = wards_dir.join(".usage.json");
    let before = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    if !before.is_file()
        || before.file_type().is_symlink()
        || !portable_metadata_has_single_link(&before)
        || before.len() > MAX_WARD_USAGE_BYTES as u64
    {
        return Err("ward usage sidecar is unsafe or oversized".into());
    }
    let canonical_root = std::fs::canonicalize(wards_dir).map_err(|error| error.to_string())?;
    let canonical_path = std::fs::canonicalize(&path).map_err(|error| error.to_string())?;
    if !canonical_path.starts_with(canonical_root) {
        return Err("ward usage sidecar escaped the wards directory".into());
    }
    let mut file = OpenOptions::new()
        .read(true)
        .open(&path)
        .map_err(|error| error.to_string())?;
    let opened = file.metadata().map_err(|error| error.to_string())?;
    if !opened.is_file()
        || opened.file_type().is_symlink()
        || !portable_opened_file_has_single_link(&file)
        || opened.len() > MAX_WARD_USAGE_BYTES as u64
    {
        return Err("ward usage sidecar is unsafe or oversized".into());
    }
    let canonical_after = std::fs::canonicalize(&path).map_err(|error| error.to_string())?;
    if canonical_after != canonical_path || !canonical_after.starts_with(&canonical_root) {
        return Err("ward usage sidecar changed or escaped while opening".into());
    }
    let after = std::fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
    if !after.is_file()
        || after.file_type().is_symlink()
        || !portable_metadata_has_single_link(&after)
        || after.len() > MAX_WARD_USAGE_BYTES as u64
        || !portable_opened_file_matches_path(&path, &file, &before, &opened, &after)?
    {
        return Err("ward usage sidecar changed or became unsafe while opening".into());
    }
    let mut bytes = Vec::with_capacity((opened.len() as usize).min(MAX_WARD_USAGE_BYTES));
    Read::by_ref(&mut file)
        .take((MAX_WARD_USAGE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > MAX_WARD_USAGE_BYTES {
        return Err("ward usage sidecar exceeds the byte limit".into());
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| "ward usage sidecar must be valid UTF-8".into())
}

#[cfg(not(target_os = "linux"))]
fn portable_metadata_has_single_link(metadata: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        return metadata.nlink() == 1;
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        true
    }
}

#[cfg(not(target_os = "linux"))]
fn portable_opened_file_has_single_link(file: &File) -> bool {
    #[cfg(windows)]
    {
        return crate::windows_file::has_single_link(file).unwrap_or(false);
    }
    #[cfg(not(windows))]
    {
        file.metadata()
            .is_ok_and(|metadata| portable_metadata_has_single_link(&metadata))
    }
}

#[cfg(all(unix, any(test, not(target_os = "linux"))))]
fn portable_opened_file_matches_path(
    _path: &Path,
    _file: &File,
    before: &std::fs::Metadata,
    opened: &std::fs::Metadata,
    after: &std::fs::Metadata,
) -> Result<bool, String> {
    use std::os::unix::fs::MetadataExt;
    Ok(before.dev() == opened.dev()
        && before.ino() == opened.ino()
        && after.dev() == opened.dev()
        && after.ino() == opened.ino())
}

#[cfg(windows)]
fn portable_opened_file_matches_path(
    path: &Path,
    file: &File,
    _before: &std::fs::Metadata,
    _opened: &std::fs::Metadata,
    _after: &std::fs::Metadata,
) -> Result<bool, String> {
    let reopened = OpenOptions::new()
        .read(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    let metadata = reopened.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || !portable_opened_file_has_single_link(&reopened)
    {
        return Ok(false);
    }
    crate::windows_file::same_file(file, &reopened).map_err(|error| error.to_string())
}

#[cfg(all(not(target_os = "linux"), not(any(unix, windows))))]
fn portable_opened_file_matches_path(
    _path: &Path,
    _file: &File,
    _before: &std::fs::Metadata,
    _opened: &std::fs::Metadata,
    _after: &std::fs::Metadata,
) -> Result<bool, String> {
    Ok(false)
}

impl WardUsage {
    /// Bind to a wards directory. The sidecar lives at `<wards_dir>/.usage.json`.
    pub fn new(wards_dir: impl Into<PathBuf>) -> Self {
        Self {
            wards_dir: wards_dir.into(),
            lock: Mutex::new(()),
        }
    }

    fn sidecar_path(&self) -> PathBuf {
        self.wards_dir.join(".usage.json")
    }

    fn load_inner(&self) -> WardUsageMap {
        #[cfg(target_os = "linux")]
        let loaded = load_sidecar_linux(&self.wards_dir);
        #[cfg(not(target_os = "linux"))]
        let loaded = load_sidecar_portable(&self.wards_dir);
        match loaded {
            Ok(None) => BTreeMap::new(),
            Ok(Some(raw)) if raw.trim().is_empty() => BTreeMap::new(),
            Ok(Some(raw)) => serde_json::from_str(&raw).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "ward .usage.json malformed; treating as empty");
                BTreeMap::new()
            }),
            Err(error) => {
                tracing::warn!(error = %error, "ward .usage.json unsafe or unreadable; treating as empty");
                BTreeMap::new()
            }
        }
    }

    fn save_inner(&self, map: &WardUsageMap) -> Result<(), String> {
        std::fs::create_dir_all(&self.wards_dir).map_err(|e| e.to_string())?;
        let raw = serde_json::to_string_pretty(map).map_err(|e| e.to_string())?;
        #[cfg(target_os = "linux")]
        {
            save_sidecar_linux(&self.wards_dir, raw.as_bytes())
        }
        #[cfg(not(target_os = "linux"))]
        {
            save_sidecar_portable(&self.wards_dir, raw.as_bytes())
        }
    }

    /// Read the whole sidecar. Returns an empty map when the file is
    /// missing or malformed (and logs a warning in the malformed case).
    pub fn load(&self) -> WardUsageMap {
        let _guard = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        self.load_inner()
    }

    /// Atomically overwrite the sidecar with `map`.
    pub fn save(&self, map: &WardUsageMap) -> Result<(), String> {
        let _guard = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        self.save_inner(map)
    }

    fn mutate<F>(&self, f: F) -> Result<(), String>
    where
        F: FnOnce(&mut WardUsageMap),
    {
        let _guard = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut map = self.load_inner();
        f(&mut map);
        self.save_inner(&map)
    }

    /// Increment `use_count` and stamp `last_used_at`. Lazy-inserts an
    /// unknown ward with `created_by = User` — the conservative default
    /// when something bumps a ward whose creation wasn't captured.
    pub fn bump_use(&self, ward: &str) -> Result<(), String> {
        self.mutate(|map| {
            let now = Utc::now();
            let entry = map
                .entry(ward.to_string())
                .or_insert_with(|| WardRecord::new_at(now, WardProvenance::default()));
            entry.use_count += 1;
            entry.last_used_at = Some(now);
        })
    }

    /// Increment `patch_count` and stamp `last_patched_at`.
    pub fn bump_patch(&self, ward: &str) -> Result<(), String> {
        self.mutate(|map| {
            let now = Utc::now();
            let entry = map
                .entry(ward.to_string())
                .or_insert_with(|| WardRecord::new_at(now, WardProvenance::default()));
            entry.patch_count += 1;
            entry.last_patched_at = Some(now);
        })
    }

    /// Record a freshly-created ward with explicit provenance. Idempotent:
    /// re-marking an existing ward keeps its counters but updates
    /// `created_by` so a previously-unknown record can be corrected once
    /// the real provenance is known.
    pub fn mark_created(&self, ward: &str, created_by: WardProvenance) -> Result<(), String> {
        self.mark_created_with_archetype(ward, created_by, None)
    }

    /// Record creation provenance and the selected archetype in one sidecar
    /// mutation. Passing `None` preserves an existing archetype value.
    pub fn mark_created_with_archetype(
        &self,
        ward: &str,
        created_by: WardProvenance,
        archetype: Option<WardArchetypeId>,
    ) -> Result<(), String> {
        self.mutate(|map| {
            let now = Utc::now();
            let entry = map
                .entry(ward.to_string())
                .or_insert_with(|| WardRecord::new_at(now, created_by));
            entry.created_by = created_by;
            if archetype.is_some() {
                entry.archetype = archetype;
            }
        })
    }

    /// Set the lifecycle state. Transitioning into `Archived` stamps
    /// `archived_at`; any other transition leaves `archived_at` alone.
    pub fn set_state(&self, ward: &str, state: WardState) -> Result<(), String> {
        self.mutate(|map| {
            if let Some(entry) = map.get_mut(ward) {
                entry.state = state;
                if matches!(state, WardState::Archived) {
                    entry.archived_at = Some(Utc::now());
                }
            }
        })
    }

    /// Toggle the curator opt-out flag.
    pub fn set_pinned(&self, ward: &str, pinned: bool) -> Result<(), String> {
        self.mutate(|map| {
            if let Some(entry) = map.get_mut(ward) {
                entry.pinned = pinned;
            }
        })
    }

    /// Read a single ward's record without holding the lock across other work.
    pub fn get(&self, ward: &str) -> Option<WardRecord> {
        self.load().get(ward).cloned()
    }
}

impl WardUsage {
    /// Sidecar path — exposed for tests and the curator's audit log writer.
    pub fn path(&self) -> PathBuf {
        self.sidecar_path()
    }

    /// Wards directory — exposed for the curator (it needs to walk siblings).
    pub fn wards_dir(&self) -> &Path {
        &self.wards_dir
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    fn make_temp() -> (tempfile::TempDir, WardUsage) {
        let dir = tempfile::tempdir().unwrap();
        let usage = WardUsage::new(dir.path().to_path_buf());
        (dir, usage)
    }

    #[test]
    fn load_returns_empty_when_sidecar_missing() {
        let (_dir, usage) = make_temp();
        assert!(usage.load().is_empty());
    }

    #[test]
    fn save_then_load_roundtrip() {
        let (_dir, usage) = make_temp();
        let mut map = WardUsageMap::new();
        map.insert(
            "alpha".to_string(),
            WardRecord::new_at(Utc::now(), WardProvenance::Agent),
        );
        usage.save(&map).unwrap();
        let reread = usage.load();
        assert_eq!(reread.len(), 1);
        assert_eq!(reread["alpha"].created_by, WardProvenance::Agent);
    }

    #[cfg(unix)]
    #[test]
    fn save_replaces_sidecar_symlink_without_writing_its_target() {
        use std::os::unix::fs::symlink;

        let (dir, usage) = make_temp();
        let victim = dir.path().join("victim");
        std::fs::write(&victim, "unchanged").unwrap();
        symlink(&victim, usage.path()).unwrap();
        let mut map = WardUsageMap::new();
        map.insert(
            "alpha".to_string(),
            WardRecord::new_at(Utc::now(), WardProvenance::Agent),
        );

        usage.save(&map).unwrap();

        assert_eq!(std::fs::read_to_string(victim).unwrap(), "unchanged");
        assert!(!std::fs::symlink_metadata(usage.path())
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(
            usage.get("alpha").unwrap().created_by,
            WardProvenance::Agent
        );
        assert!(!std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().ends_with(".tmp")));
    }

    #[cfg(unix)]
    #[test]
    fn unsafe_and_oversized_sidecars_are_bounded_and_treated_as_empty() {
        use std::os::unix::fs::symlink;

        let (dir, usage) = make_temp();
        let victim = dir.path().join("victim");
        std::fs::write(&victim, r#"{"injected":{}}"#).unwrap();
        symlink(&victim, usage.path()).unwrap();
        assert!(usage.load().is_empty());

        std::fs::remove_file(usage.path()).unwrap();
        std::fs::write(usage.path(), vec![b'x'; MAX_WARD_USAGE_BYTES + 1]).unwrap();
        assert!(usage.load().is_empty());

        std::fs::remove_file(usage.path()).unwrap();
        assert!(std::process::Command::new("mkfifo")
            .arg(usage.path())
            .status()
            .unwrap()
            .success());
        assert!(usage.load().is_empty());
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn portable_ward_usage_updates_existing_sidecar() {
        let (_dir, usage) = make_temp();
        usage.bump_use("alpha").unwrap();
        usage.bump_use("alpha").unwrap();
        assert_eq!(usage.get("alpha").unwrap().use_count, 2);
    }

    #[cfg(unix)]
    #[test]
    fn portable_sidecar_identity_rejects_replaced_path() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(".usage.json");
        let moved = directory.path().join(".usage.original.json");
        std::fs::write(&path, "{}").unwrap();

        let before = std::fs::symlink_metadata(&path).unwrap();
        let file = File::open(&path).unwrap();
        let opened = file.metadata().unwrap();
        std::fs::rename(&path, moved).unwrap();
        std::fs::write(&path, "{}").unwrap();
        let after = std::fs::symlink_metadata(&path).unwrap();

        assert!(
            !portable_opened_file_matches_path(&path, &file, &before, &opened, &after).unwrap()
        );
    }

    #[test]
    fn bump_use_increments_and_stamps() {
        let (_dir, usage) = make_temp();
        usage.bump_use("alpha").unwrap();
        usage.bump_use("alpha").unwrap();
        usage.bump_use("alpha").unwrap();
        let rec = usage.get("alpha").expect("record");
        assert_eq!(rec.use_count, 3);
        assert!(rec.last_used_at.is_some());
        // Unknown wards default to user provenance — conservative.
        assert_eq!(rec.created_by, WardProvenance::User);
    }

    #[test]
    fn bump_patch_increments_and_stamps() {
        let (_dir, usage) = make_temp();
        usage.bump_patch("alpha").unwrap();
        usage.bump_patch("alpha").unwrap();
        let rec = usage.get("alpha").expect("record");
        assert_eq!(rec.patch_count, 2);
        assert!(rec.last_patched_at.is_some());
    }

    #[test]
    fn mark_created_records_provenance() {
        let (_dir, usage) = make_temp();
        usage.mark_created("alpha", WardProvenance::Agent).unwrap();
        let rec = usage.get("alpha").expect("record");
        assert_eq!(rec.created_by, WardProvenance::Agent);
        assert_eq!(rec.use_count, 0);

        // Re-marking with a different provenance updates the field but
        // leaves counters intact — useful when a ward was lazy-inserted
        // before its real provenance was known.
        usage.bump_use("alpha").unwrap();
        usage
            .mark_created("alpha", WardProvenance::Bundled)
            .unwrap();
        let rec = usage.get("alpha").expect("record");
        assert_eq!(rec.created_by, WardProvenance::Bundled);
        assert_eq!(rec.use_count, 1);
    }

    #[test]
    fn creation_archetype_roundtrips_and_legacy_records_default_to_absent() {
        let (_dir, usage) = make_temp();
        usage
            .mark_created_with_archetype(
                "compiler",
                WardProvenance::Agent,
                Some(WardArchetypeId::Coding),
            )
            .unwrap();
        assert_eq!(
            usage.get("compiler").unwrap().archetype,
            Some(WardArchetypeId::Coding)
        );

        let legacy = r#"{
          "legacy": {
            "created_at": "2026-01-01T00:00:00Z",
            "created_by": "agent",
            "state": "active"
          }
        }"#;
        std::fs::write(usage.path(), legacy).unwrap();
        assert_eq!(usage.get("legacy").unwrap().archetype, None);
    }

    #[test]
    fn set_state_archived_stamps_archived_at() {
        let (_dir, usage) = make_temp();
        usage.mark_created("alpha", WardProvenance::Agent).unwrap();
        usage.set_state("alpha", WardState::Stale).unwrap();
        let rec = usage.get("alpha").expect("record");
        assert_eq!(rec.state, WardState::Stale);
        assert!(rec.archived_at.is_none());

        usage.set_state("alpha", WardState::Archived).unwrap();
        let rec = usage.get("alpha").expect("record");
        assert_eq!(rec.state, WardState::Archived);
        assert!(rec.archived_at.is_some());
    }

    #[test]
    fn set_pinned_toggles_flag() {
        let (_dir, usage) = make_temp();
        usage.mark_created("alpha", WardProvenance::Agent).unwrap();
        usage.set_pinned("alpha", true).unwrap();
        assert!(usage.get("alpha").unwrap().pinned);
        usage.set_pinned("alpha", false).unwrap();
        assert!(!usage.get("alpha").unwrap().pinned);
    }

    #[test]
    fn malformed_sidecar_is_treated_as_empty() {
        let (dir, usage) = make_temp();
        std::fs::write(dir.path().join(".usage.json"), "not json {{{").unwrap();
        assert!(usage.load().is_empty());
        // ...and we can still bump on top of it; the bad content gets replaced.
        usage.bump_use("alpha").unwrap();
        assert_eq!(usage.get("alpha").unwrap().use_count, 1);
    }

    #[test]
    fn concurrent_bumps_do_not_lose_updates() {
        let (_dir, usage) = make_temp();
        let usage = Arc::new(usage);
        let mut handles = Vec::new();
        for _ in 0..50 {
            let u = usage.clone();
            handles.push(thread::spawn(move || {
                for _ in 0..4 {
                    u.bump_use("alpha").unwrap();
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        // 50 threads × 4 bumps = 200 expected — the internal Mutex must
        // serialise read-modify-write so none are lost.
        assert_eq!(usage.get("alpha").unwrap().use_count, 200);
    }
}
