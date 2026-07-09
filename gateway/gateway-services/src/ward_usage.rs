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
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
        }
    }
}

pub type WardUsageMap = BTreeMap<String, WardRecord>;

/// Service that owns the `wards/.usage.json` sidecar.
pub struct WardUsage {
    wards_dir: PathBuf,
    lock: Mutex<()>,
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
        match std::fs::read_to_string(self.sidecar_path()) {
            Ok(raw) if raw.trim().is_empty() => BTreeMap::new(),
            Ok(raw) => serde_json::from_str(&raw).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "ward .usage.json malformed; treating as empty");
                BTreeMap::new()
            }),
            Err(_) => BTreeMap::new(),
        }
    }

    fn save_inner(&self, map: &WardUsageMap) -> Result<(), String> {
        std::fs::create_dir_all(&self.wards_dir).map_err(|e| e.to_string())?;
        let dest = self.sidecar_path();
        let tmp = dest.with_extension("json.tmp");
        let raw = serde_json::to_string_pretty(map).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, raw).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &dest).map_err(|e| e.to_string())?;
        Ok(())
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
        self.mutate(|map| {
            let now = Utc::now();
            let entry = map
                .entry(ward.to_string())
                .or_insert_with(|| WardRecord::new_at(now, created_by));
            entry.created_by = created_by;
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
