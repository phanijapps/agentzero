//! # Ward curator — Layer-1 heuristic cleanup
//!
//! Spec: `docs/architecture/future-state/2026-05-23-ward-curator-spec.md` §2.
//!
//! Operates on agent-created, non-pinned wards. Inactive longer than
//! `archive_days` → archived (directory moved to `_archive/`); inactive
//! longer than `stale_days` → marked stale (sidecar only); recently active
//! again → reactivated. Bundled / user-authored wards are never touched.
//!
//! Backups are written as `.tar.gz` via the system `tar` command before any
//! mutation. Per-run audit logs land under `<data_dir>/curator_logs/<ts>/`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::ward_usage::{WardProvenance, WardRecord, WardState, WardUsage, WardUsageMap};

pub const DEFAULT_STALE_DAYS: i64 = 30;
pub const DEFAULT_ARCHIVE_DAYS: i64 = 90;
pub const DEFAULT_BACKUP_KEEP: usize = 5;

/// `POST /api/curator/cleanup` body. All fields optional — defaults match
/// Hermes (`stale=30d`, `archive=90d`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CleanupRequest {
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub stale_days: Option<i64>,
    #[serde(default)]
    pub archive_days: Option<i64>,
}

/// One state transition produced by a cleanup pass.
#[derive(Debug, Clone, Serialize)]
pub struct Transition {
    pub ward: String,
    pub from: WardState,
    pub to: WardState,
    pub anchor: DateTime<Utc>,
    pub age_days: i64,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archive_path: Option<PathBuf>,
}

/// JSON returned by the cleanup endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct CleanupReport {
    pub ok: bool,
    pub ran_at: DateTime<Utc>,
    pub dry_run: bool,
    pub scanned: usize,
    pub skipped_pinned: usize,
    pub skipped_non_agent: usize,
    pub transitions: Vec<Transition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report_path: Option<PathBuf>,
}

/// `POST /api/curator/restore` body.
#[derive(Debug, Clone, Deserialize)]
pub struct RestoreRequest {
    pub backup: String,
}

/// JSON returned by the restore endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct RestoreReport {
    pub ok: bool,
    pub restored_at: DateTime<Utc>,
    pub backup: String,
}

/// Layer-1 ward curator. Stateless across calls — every request reads the
/// current sidecar and walks the filesystem fresh.
pub struct WardCurator {
    wards_dir: PathBuf,
    audit_dir: PathBuf,
}

impl WardCurator {
    /// `wards_dir` is `<vault>/wards/`; `data_dir` is `<vault>/data/`. The
    /// audit log lives at `<data_dir>/curator_logs/<ts>/`.
    pub fn new(wards_dir: impl Into<PathBuf>, data_dir: impl AsRef<Path>) -> Self {
        let audit_dir = data_dir.as_ref().join("curator_logs");
        Self {
            wards_dir: wards_dir.into(),
            audit_dir,
        }
    }

    /// Execute a cleanup pass. Returns the run report; mutations are
    /// skipped entirely when `req.dry_run` is true or there are no
    /// pending transitions.
    pub fn cleanup(&self, req: &CleanupRequest) -> Result<CleanupReport, String> {
        let ran_at = Utc::now();
        let stale_days = req.stale_days.unwrap_or(DEFAULT_STALE_DAYS);
        let archive_days = req.archive_days.unwrap_or(DEFAULT_ARCHIVE_DAYS);

        let usage = WardUsage::new(&self.wards_dir);
        let map = usage.load();

        let mut scanned = 0usize;
        let mut skipped_pinned = 0usize;
        let mut skipped_non_agent = 0usize;
        let mut planned: Vec<Transition> = Vec::new();

        for (name, rec) in &map {
            scanned += 1;
            if rec.created_by != WardProvenance::Agent {
                skipped_non_agent += 1;
                continue;
            }
            if rec.pinned {
                skipped_pinned += 1;
                continue;
            }
            let anchor = max_anchor(rec);
            let age_days = (ran_at - anchor).num_days();
            let next = compute_transition(rec.state, age_days, stale_days, archive_days);
            if next == rec.state {
                continue;
            }
            planned.push(Transition {
                ward: name.clone(),
                from: rec.state,
                to: next,
                anchor,
                age_days,
                reason: format!("no activity in {age_days}d"),
                archive_path: None,
            });
        }

        // Dry-run or no-op — return without touching anything.
        if req.dry_run || planned.is_empty() {
            return Ok(CleanupReport {
                ok: true,
                ran_at,
                dry_run: req.dry_run,
                scanned,
                skipped_pinned,
                skipped_non_agent,
                transitions: planned,
                backup_path: None,
                report_path: None,
            });
        }

        // Backup before any mutation, then prune older backups.
        let backup_path = self.write_backup(ran_at)?;
        self.prune_backups();

        // Apply transitions.
        let mut map_mut = map;
        let mut applied: Vec<Transition> = Vec::with_capacity(planned.len());
        for mut t in planned {
            if let Some(rec) = map_mut.get_mut(&t.ward) {
                rec.state = t.to;
                if t.to == WardState::Archived {
                    rec.archived_at = Some(ran_at);
                    let from = self.wards_dir.join(&t.ward);
                    let archive_root = self.wards_dir.join("_archive");
                    if let Err(e) = std::fs::create_dir_all(&archive_root) {
                        tracing::warn!(error = %e, "create _archive dir failed");
                    }
                    let to = archive_root.join(&t.ward);
                    match std::fs::rename(&from, &to) {
                        Ok(()) => {
                            t.archive_path = Some(to);
                        }
                        Err(e) => {
                            // The sidecar state change is kept — the user
                            // can re-archive from the next pass or restore
                            // from the backup if this was unexpected.
                            tracing::warn!(
                                ward = %t.ward,
                                error = %e,
                                "archive rename failed; sidecar state still updated"
                            );
                        }
                    }
                }
            }
            applied.push(t);
        }
        usage.save(&map_mut)?;

        let mut report = CleanupReport {
            ok: true,
            ran_at,
            dry_run: false,
            scanned,
            skipped_pinned,
            skipped_non_agent,
            transitions: applied,
            backup_path: Some(backup_path),
            report_path: None,
        };
        report.report_path = Some(self.write_audit(&report)?);
        Ok(report)
    }

    /// Restore the `wards/` tree from a named backup. The timestamp `backup`
    /// must match a file under `_curator_backups/<backup>.tar.gz`.
    pub fn restore(&self, backup: &str) -> Result<RestoreReport, String> {
        if backup.contains('/') || backup.contains("..") {
            return Err("invalid backup name".to_string());
        }
        let archive = self
            .wards_dir
            .join("_curator_backups")
            .join(format!("{backup}.tar.gz"));
        if !archive.exists() {
            return Err(format!("backup not found: {}", archive.display()));
        }
        let parent = self
            .wards_dir
            .parent()
            .ok_or_else(|| "wards_dir has no parent".to_string())?;
        let status = std::process::Command::new("tar")
            .arg("-xzf")
            .arg(&archive)
            .arg("-C")
            .arg(parent)
            .status()
            .map_err(|e| format!("tar spawn: {e}"))?;
        if !status.success() {
            return Err(format!("tar exited with status {status}"));
        }
        Ok(RestoreReport {
            ok: true,
            restored_at: Utc::now(),
            backup: backup.to_string(),
        })
    }

    fn write_backup(&self, ts: DateTime<Utc>) -> Result<PathBuf, String> {
        let backup_root = self.wards_dir.join("_curator_backups");
        std::fs::create_dir_all(&backup_root).map_err(|e| e.to_string())?;
        let stamp = ts.format("%Y%m%dT%H%M%SZ").to_string();
        let dest = backup_root.join(format!("{stamp}.tar.gz"));

        let parent = self
            .wards_dir
            .parent()
            .ok_or_else(|| "wards_dir has no parent".to_string())?;
        let name = self
            .wards_dir
            .file_name()
            .ok_or_else(|| "wards_dir has no name".to_string())?;

        // Exclude the backup dir itself (would recursively grow) and the
        // archive dir (already-archived wards are recoverable from prior
        // snapshots — including them doubles disk use per pass).
        let status = std::process::Command::new("tar")
            .arg("--exclude=_curator_backups")
            .arg("--exclude=_archive")
            .arg("-czf")
            .arg(&dest)
            .arg("-C")
            .arg(parent)
            .arg(name)
            .status()
            .map_err(|e| format!("tar spawn: {e}"))?;
        if !status.success() {
            return Err(format!("tar exited with status {status}"));
        }
        Ok(dest)
    }

    fn prune_backups(&self) {
        let backup_root = self.wards_dir.join("_curator_backups");
        let Ok(entries) = std::fs::read_dir(&backup_root) else {
            return;
        };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.ends_with(".tar.gz"))
            })
            .collect();
        // Timestamps sort lexically; oldest first.
        files.sort();
        let drop_count = files.len().saturating_sub(DEFAULT_BACKUP_KEEP);
        for old in &files[..drop_count] {
            if let Err(e) = std::fs::remove_file(old) {
                tracing::warn!(path = %old.display(), error = %e, "prune backup failed");
            }
        }
    }

    fn write_audit(&self, report: &CleanupReport) -> Result<PathBuf, String> {
        let stamp = report.ran_at.format("%Y%m%dT%H%M%SZ").to_string();
        let dir = self.audit_dir.join(&stamp);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let json = serde_json::to_string_pretty(report).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("run.json"), json).map_err(|e| e.to_string())?;
        let md = render_report_md(report);
        let md_path = dir.join("REPORT.md");
        std::fs::write(&md_path, md).map_err(|e| e.to_string())?;
        Ok(md_path)
    }
}

fn max_anchor(rec: &WardRecord) -> DateTime<Utc> {
    let mut anchor = rec.created_at;
    if let Some(t) = rec.last_used_at {
        if t > anchor {
            anchor = t;
        }
    }
    if let Some(t) = rec.last_patched_at {
        if t > anchor {
            anchor = t;
        }
    }
    anchor
}

fn compute_transition(
    state: WardState,
    age_days: i64,
    stale_days: i64,
    archive_days: i64,
) -> WardState {
    if age_days > archive_days {
        WardState::Archived
    } else if age_days > stale_days && state == WardState::Active {
        WardState::Stale
    } else if age_days <= stale_days && state == WardState::Stale {
        WardState::Active
    } else {
        state
    }
}

fn render_report_md(report: &CleanupReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "# Ward curator run — {}\n\n",
        report.ran_at.to_rfc3339()
    ));
    out.push_str(&format!("- dry_run: {}\n", report.dry_run));
    out.push_str(&format!("- scanned: {}\n", report.scanned));
    out.push_str(&format!("- skipped_pinned: {}\n", report.skipped_pinned));
    out.push_str(&format!(
        "- skipped_non_agent: {}\n",
        report.skipped_non_agent
    ));
    if let Some(p) = &report.backup_path {
        out.push_str(&format!("- backup_path: `{}`\n", p.display()));
    }
    out.push_str("\n## Transitions\n\n");
    if report.transitions.is_empty() {
        out.push_str("_None._\n");
    } else {
        out.push_str("| ward | from | to | age (days) | archive_path |\n");
        out.push_str("|---|---|---|---|---|\n");
        for t in &report.transitions {
            let from = format!("{:?}", t.from).to_lowercase();
            let to = format!("{:?}", t.to).to_lowercase();
            let ap = t
                .archive_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                t.ward, from, to, t.age_days, ap
            ));
        }
    }
    out
}

// ============================================================================
// PHASE C — LLM CONSOLIDATION CURATOR
// ============================================================================

/// One row in the candidate table the LLM consolidates over.
#[derive(Debug, Clone, Serialize)]
pub struct WardCandidate {
    pub name: String,
    /// First ~200 chars of the ward's `## Purpose / Scope` section, collapsed.
    /// Empty when the doctrine has no Purpose section.
    pub purpose: String,
    pub use_count: u64,
    pub state: WardState,
    pub last_used_at: Option<DateTime<Utc>>,
    /// `(now - max(last_used_at, last_patched_at, created_at)).num_days()`.
    pub age_days: i64,
}

/// A single decision the curator-agent (LLM) returned.
/// `POST /api/curator/consolidate` body. All fields optional — `dry_run`
/// defaults to **true** (the LLM consolidation is heavier and rarer than
/// cleanup, so mutation is opt-in). Passing `plan` short-circuits the LLM
/// call entirely, which the curator-agent flow uses to keep "decide" and
/// "apply" cleanly separable.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ConsolidateRequest {
    #[serde(default = "default_consolidate_dry_run")]
    pub dry_run: bool,
    #[serde(default = "default_max_consolidations")]
    pub max_consolidations: usize,
    /// If present, skip the LLM and apply this plan directly. Useful for
    /// tests, replays, and dry-run-then-commit workflows.
    #[serde(default)]
    pub plan: Option<ConsolidationPlan>,
}

fn default_consolidate_dry_run() -> bool {
    true
}

fn default_max_consolidations() -> usize {
    5
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "lowercase")]
pub enum ConsolidationAction {
    /// Combine `from` wards into a new umbrella `into`. The umbrella is created
    /// with a fresh `AGENTS.md` whose Purpose/Scope is `purpose`.
    Merge {
        from: Vec<String>,
        into: String,
        purpose: String,
        reason: String,
    },
    /// Move sibling content from `from` into an existing umbrella `into`;
    /// archive each sibling. Doctrine of `into` is untouched.
    Absorb {
        from: Vec<String>,
        into: String,
        reason: String,
    },
    /// Standalone archive (same as Phase B archive) for a one-off ward.
    Archive { ward: String, reason: String },
}

/// LLM-emitted consolidation plan, accepted directly by the apply endpoint
/// so tests / dry-runs can supply a hand-crafted plan without an LLM.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConsolidationPlan {
    #[serde(default)]
    pub consolidations: Vec<ConsolidationAction>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ApplyStatus {
    /// Action applied successfully.
    Ok,
    /// Action was rejected (validation failed) or skipped on dry-run.
    Skipped,
    /// Action partially applied or hit a filesystem/sidecar error.
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppliedAction {
    pub action: ConsolidationAction,
    pub status: ApplyStatus,
    pub details: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConsolidationReport {
    pub ok: bool,
    pub ran_at: DateTime<Utc>,
    pub dry_run: bool,
    pub plan: ConsolidationPlan,
    pub applied: Vec<AppliedAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report_path: Option<PathBuf>,
}

impl WardCurator {
    /// Walk the sidecar and build the candidate table for the LLM. Includes
    /// **all** entries — the curator-agent's prompt filters bundled / user /
    /// pinned itself, but having the full table makes the LLM aware of what
    /// it must avoid.
    pub fn build_candidates(&self) -> Vec<WardCandidate> {
        let now = Utc::now();
        let usage = WardUsage::new(&self.wards_dir);
        let map = usage.load();
        let mut out: Vec<WardCandidate> = map
            .into_iter()
            .map(|(name, rec)| {
                let anchor = max_anchor(&rec);
                let purpose = ward_purpose_for(&self.wards_dir, &name).unwrap_or_default();
                WardCandidate {
                    name,
                    purpose,
                    use_count: rec.use_count,
                    state: rec.state,
                    last_used_at: rec.last_used_at,
                    age_days: (now - anchor).num_days(),
                }
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Apply a consolidation plan. On `dry_run`, returns the plan with each
    /// action marked `Skipped`; no filesystem mutation, no backup. Otherwise
    /// writes a pre-run tar.gz, attempts each action in order, persists the
    /// sidecar, and writes the per-run audit log.
    ///
    /// Procedure re-keying is **the caller's responsibility** — this method
    /// stays focused on filesystem + sidecar so `gateway-services` doesn't
    /// pick up a `zbot-stores-traits` dep.
    pub fn apply_consolidation(
        &self,
        plan: &ConsolidationPlan,
        dry_run: bool,
    ) -> Result<ConsolidationReport, String> {
        let ran_at = Utc::now();
        let usage = WardUsage::new(&self.wards_dir);
        let map = usage.load();

        if dry_run {
            // Pre-validate so the user gets meaningful feedback before
            // committing to a real run.
            let mut applied = Vec::with_capacity(plan.consolidations.len());
            for action in &plan.consolidations {
                let (status, details) = match validate_action(action, &map, &self.wards_dir) {
                    Ok(()) => (ApplyStatus::Skipped, Some("dry-run".to_string())),
                    Err(e) => (ApplyStatus::Skipped, Some(format!("would fail: {e}"))),
                };
                applied.push(AppliedAction {
                    action: action.clone(),
                    status,
                    details,
                });
            }
            return Ok(ConsolidationReport {
                ok: true,
                ran_at,
                dry_run: true,
                plan: plan.clone(),
                applied,
                backup_path: None,
                report_path: None,
            });
        }

        if plan.consolidations.is_empty() {
            return Ok(ConsolidationReport {
                ok: true,
                ran_at,
                dry_run: false,
                plan: plan.clone(),
                applied: Vec::new(),
                backup_path: None,
                report_path: None,
            });
        }

        let backup_path = self.write_backup(ran_at)?;
        self.prune_backups();

        let mut map_mut = map;
        let mut applied = Vec::with_capacity(plan.consolidations.len());
        for action in &plan.consolidations {
            let result = match action {
                ConsolidationAction::Merge {
                    from,
                    into,
                    purpose,
                    ..
                } => self.apply_merge(from, into, purpose, &mut map_mut, ran_at),
                ConsolidationAction::Absorb { from, into, .. } => {
                    self.apply_absorb(from, into, &mut map_mut, ran_at)
                }
                ConsolidationAction::Archive { ward, .. } => {
                    self.apply_archive_single(ward, &mut map_mut, ran_at)
                }
            };
            applied.push(match result {
                Ok(details) => AppliedAction {
                    action: action.clone(),
                    status: ApplyStatus::Ok,
                    details: Some(details),
                },
                Err(e) => AppliedAction {
                    action: action.clone(),
                    status: ApplyStatus::Failed,
                    details: Some(e),
                },
            });
        }
        usage.save(&map_mut)?;

        let mut report = ConsolidationReport {
            ok: true,
            ran_at,
            dry_run: false,
            plan: plan.clone(),
            applied,
            backup_path: Some(backup_path),
            report_path: None,
        };
        report.report_path = Some(self.write_consolidation_audit(&report)?);
        Ok(report)
    }

    fn apply_merge(
        &self,
        from: &[String],
        into: &str,
        purpose: &str,
        map: &mut WardUsageMap,
        ran_at: DateTime<Utc>,
    ) -> Result<String, String> {
        validate_action(
            &ConsolidationAction::Merge {
                from: from.to_vec(),
                into: into.to_string(),
                purpose: purpose.to_string(),
                reason: String::new(),
            },
            map,
            &self.wards_dir,
        )?;

        let into_dir = self.wards_dir.join(into);
        std::fs::create_dir_all(&into_dir).map_err(|e| e.to_string())?;
        write_umbrella_agents_md(&into_dir, into, purpose)?;

        let mb_dir = into_dir.join("memory-bank");
        let specs_dir = into_dir.join("specs");
        std::fs::create_dir_all(&mb_dir).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&specs_dir).map_err(|e| e.to_string())?;

        for name in from {
            let from_dir = self.wards_dir.join(name);
            // memory-bank: flatten with `<from>__` prefix so files from each
            // source ward don't clobber each other.
            copy_dir_into_with_prefix(
                &from_dir.join("memory-bank"),
                &mb_dir,
                &format!("{name}__"),
            )?;
            // specs: each ward gets its own subdir under the umbrella's specs.
            copy_dir_into_subdir(&from_dir.join("specs"), &specs_dir, name)?;
        }

        // Archive each `from` and update its sidecar entry.
        let archive_root = self.wards_dir.join("_archive");
        std::fs::create_dir_all(&archive_root).map_err(|e| e.to_string())?;
        for name in from {
            let f = self.wards_dir.join(name);
            let to = archive_root.join(name);
            std::fs::rename(&f, &to).map_err(|e| format!("archive {name}: {e}"))?;
            if let Some(rec) = map.get_mut(name) {
                rec.state = WardState::Archived;
                rec.archived_at = Some(ran_at);
            }
        }

        // Insert (or refresh) the umbrella's sidecar entry as agent-authored.
        let rec = WardRecord {
            use_count: 0,
            patch_count: 0,
            last_used_at: None,
            last_patched_at: None,
            created_at: ran_at,
            created_by: WardProvenance::Agent,
            state: WardState::Active,
            pinned: false,
            archived_at: None,
        };
        map.insert(into.to_string(), rec);

        Ok(format!("merged [{}] into '{into}'", from.join(", ")))
    }

    fn apply_absorb(
        &self,
        from: &[String],
        into: &str,
        map: &mut WardUsageMap,
        ran_at: DateTime<Utc>,
    ) -> Result<String, String> {
        validate_action(
            &ConsolidationAction::Absorb {
                from: from.to_vec(),
                into: into.to_string(),
                reason: String::new(),
            },
            map,
            &self.wards_dir,
        )?;

        let into_dir = self.wards_dir.join(into);
        let mb_dir = into_dir.join("memory-bank");
        let specs_dir = into_dir.join("specs");
        std::fs::create_dir_all(&mb_dir).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&specs_dir).map_err(|e| e.to_string())?;

        for name in from {
            let from_dir = self.wards_dir.join(name);
            copy_dir_into_with_prefix(
                &from_dir.join("memory-bank"),
                &mb_dir,
                &format!("{name}__"),
            )?;
            copy_dir_into_subdir(&from_dir.join("specs"), &specs_dir, name)?;
        }

        let archive_root = self.wards_dir.join("_archive");
        std::fs::create_dir_all(&archive_root).map_err(|e| e.to_string())?;
        for name in from {
            let f = self.wards_dir.join(name);
            let to = archive_root.join(name);
            std::fs::rename(&f, &to).map_err(|e| format!("archive {name}: {e}"))?;
            if let Some(rec) = map.get_mut(name) {
                rec.state = WardState::Archived;
                rec.archived_at = Some(ran_at);
            }
        }

        Ok(format!("absorbed [{}] into '{into}'", from.join(", ")))
    }

    fn apply_archive_single(
        &self,
        ward: &str,
        map: &mut WardUsageMap,
        ran_at: DateTime<Utc>,
    ) -> Result<String, String> {
        let rec = map
            .get(ward)
            .ok_or_else(|| format!("ward '{ward}' has no usage record"))?;
        if rec.created_by != WardProvenance::Agent {
            return Err(format!("ward '{ward}' is not agent-created"));
        }
        if rec.pinned {
            return Err(format!("ward '{ward}' is pinned"));
        }

        let f = self.wards_dir.join(ward);
        let archive_root = self.wards_dir.join("_archive");
        std::fs::create_dir_all(&archive_root).map_err(|e| e.to_string())?;
        let to = archive_root.join(ward);
        std::fs::rename(&f, &to).map_err(|e| format!("archive {ward}: {e}"))?;

        if let Some(rec) = map.get_mut(ward) {
            rec.state = WardState::Archived;
            rec.archived_at = Some(ran_at);
        }
        Ok(format!("archived '{ward}'"))
    }

    fn write_consolidation_audit(&self, report: &ConsolidationReport) -> Result<PathBuf, String> {
        let stamp = report.ran_at.format("%Y%m%dT%H%M%SZ").to_string();
        let dir = self.audit_dir.join(&stamp);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let json = serde_json::to_string_pretty(report).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("consolidation.json"), json).map_err(|e| e.to_string())?;
        let md = render_consolidation_md(report);
        let md_path = dir.join("CONSOLIDATION.md");
        std::fs::write(&md_path, md).map_err(|e| e.to_string())?;
        Ok(md_path)
    }
}

fn validate_action(
    action: &ConsolidationAction,
    map: &WardUsageMap,
    wards_dir: &Path,
) -> Result<(), String> {
    let check_from = |name: &str| -> Result<(), String> {
        let rec = map
            .get(name)
            .ok_or_else(|| format!("from-ward '{name}' has no usage record"))?;
        if rec.created_by != WardProvenance::Agent {
            return Err(format!("from-ward '{name}' is not agent-created"));
        }
        if rec.pinned {
            return Err(format!("from-ward '{name}' is pinned"));
        }
        if !wards_dir.join(name).is_dir() {
            return Err(format!("from-ward '{name}' has no directory"));
        }
        Ok(())
    };
    match action {
        ConsolidationAction::Merge { from, into, .. } => {
            if from.is_empty() {
                return Err("merge requires at least one `from` ward".into());
            }
            for name in from {
                check_from(name)?;
            }
            if wards_dir.join(into).exists() {
                return Err(format!(
                    "umbrella '{into}' already exists; use absorb instead"
                ));
            }
            if from.iter().any(|n| n == into) {
                return Err("`into` cannot also appear in `from`".into());
            }
            Ok(())
        }
        ConsolidationAction::Absorb { from, into, .. } => {
            if from.is_empty() {
                return Err("absorb requires at least one `from` ward".into());
            }
            for name in from {
                check_from(name)?;
            }
            if !wards_dir.join(into).is_dir() {
                return Err(format!(
                    "absorb target '{into}' does not exist; use merge instead"
                ));
            }
            if from.iter().any(|n| n == into) {
                return Err("`into` cannot also appear in `from`".into());
            }
            Ok(())
        }
        ConsolidationAction::Archive { ward, .. } => {
            let rec = map
                .get(ward)
                .ok_or_else(|| format!("ward '{ward}' has no usage record"))?;
            if rec.created_by != WardProvenance::Agent {
                return Err(format!("ward '{ward}' is not agent-created"));
            }
            if rec.pinned {
                return Err(format!("ward '{ward}' is pinned"));
            }
            Ok(())
        }
    }
}

/// Extract a one-line scope blurb from a ward's `AGENTS.md` `## Purpose`
/// section. Returns `None` when the file is missing or has no Purpose.
fn ward_purpose_for(wards_dir: &Path, ward: &str) -> Option<String> {
    let path = wards_dir.join(ward).join("AGENTS.md");
    let raw = std::fs::read_to_string(&path).ok()?;
    let mut lines = raw.lines();
    lines
        .by_ref()
        .find(|l| l.trim_start().starts_with("## Purpose"))?;
    let mut blurb = String::new();
    for line in lines {
        if line.trim_start().starts_with("## ") {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !blurb.is_empty() {
            blurb.push(' ');
        }
        blurb.push_str(trimmed);
        if blurb.len() >= 200 {
            blurb.truncate(200);
            break;
        }
    }
    if blurb.is_empty() {
        None
    } else {
        Some(blurb)
    }
}

fn write_umbrella_agents_md(into_dir: &Path, name: &str, purpose: &str) -> Result<(), String> {
    let doc = format!(
        "# {name}\n\n## Purpose / Scope\n{purpose}\n\n## Provenance\nAuto-created by the ward curator's consolidation pass.\n"
    );
    std::fs::write(into_dir.join("AGENTS.md"), doc).map_err(|e| e.to_string())
}

/// Recursively copy files from `src` into `dest`, prefixing each top-level
/// file's name with `prefix`. Subdirectories are recursed in (their internal
/// paths are not prefixed).
fn copy_dir_into_with_prefix(src: &Path, dest: &Path, prefix: &str) -> Result<(), String> {
    if !src.is_dir() {
        return Ok(());
    }
    let entries = std::fs::read_dir(src).map_err(|e| e.to_string())?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let from = entry.path();
        let leaf = name.to_string_lossy();
        let to = dest.join(format!("{prefix}{leaf}"));
        if from.is_dir() {
            std::fs::create_dir_all(&to).map_err(|e| e.to_string())?;
            copy_dir_recursive(&from, &to)?;
        } else {
            std::fs::copy(&from, &to).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Copy `src` (a directory) into `dest/subdir/` recursively.
fn copy_dir_into_subdir(src: &Path, dest: &Path, subdir: &str) -> Result<(), String> {
    if !src.is_dir() {
        return Ok(());
    }
    let dest_sub = dest.join(subdir);
    std::fs::create_dir_all(&dest_sub).map_err(|e| e.to_string())?;
    copy_dir_recursive(src, &dest_sub)
}

fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<(), String> {
    if !src.is_dir() {
        return Ok(());
    }
    let entries = std::fs::read_dir(src).map_err(|e| e.to_string())?;
    for entry in entries.flatten() {
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if from.is_dir() {
            std::fs::create_dir_all(&to).map_err(|e| e.to_string())?;
            copy_dir_recursive(&from, &to)?;
        } else {
            std::fs::copy(&from, &to).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn render_consolidation_md(report: &ConsolidationReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "# Ward curator consolidation — {}\n\n",
        report.ran_at.to_rfc3339()
    ));
    out.push_str(&format!("- dry_run: {}\n", report.dry_run));
    if let Some(p) = &report.backup_path {
        out.push_str(&format!("- backup_path: `{}`\n", p.display()));
    }
    out.push_str(&format!(
        "- actions planned: {}\n",
        report.plan.consolidations.len()
    ));
    let ok = report
        .applied
        .iter()
        .filter(|a| a.status == ApplyStatus::Ok)
        .count();
    let failed = report
        .applied
        .iter()
        .filter(|a| a.status == ApplyStatus::Failed)
        .count();
    out.push_str(&format!("- applied ok: {ok}, failed: {failed}\n\n"));

    out.push_str("## Actions\n\n");
    if report.applied.is_empty() {
        out.push_str("_None._\n");
    } else {
        for a in &report.applied {
            let kind = match &a.action {
                ConsolidationAction::Merge { from, into, .. } => {
                    format!("merge [{}] → {}", from.join(", "), into)
                }
                ConsolidationAction::Absorb { from, into, .. } => {
                    format!("absorb [{}] → {}", from.join(", "), into)
                }
                ConsolidationAction::Archive { ward, .. } => format!("archive {ward}"),
            };
            out.push_str(&format!(
                "- **{:?}** — {} {}\n",
                a.status,
                kind,
                a.details.as_deref().unwrap_or("")
            ));
        }
    }
    out
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ward_usage::WardUsage;
    use chrono::Duration;

    fn write_record(
        usage: &WardUsage,
        ward: &str,
        provenance: WardProvenance,
        state: WardState,
        last_used: Option<DateTime<Utc>>,
        pinned: bool,
    ) {
        usage.mark_created(ward, provenance).unwrap();
        usage.set_state(ward, state).unwrap();
        if pinned {
            usage.set_pinned(ward, true).unwrap();
        }
        // Back-date both `created_at` and `last_used_at` so the anchor
        // (max of the three) matches the simulated age — without this,
        // `mark_created`'s `created_at = now` dominates and ages out to 0.
        let mut map = usage.load();
        if let Some(rec) = map.get_mut(ward) {
            rec.last_used_at = last_used;
            if let Some(t) = last_used {
                rec.created_at = t;
            }
        }
        usage.save(&map).unwrap();
    }

    #[test]
    fn compute_transition_table() {
        // Active + within stale window → stay active.
        assert_eq!(
            compute_transition(WardState::Active, 10, 30, 90),
            WardState::Active
        );
        // Active + past stale, within archive → stale.
        assert_eq!(
            compute_transition(WardState::Active, 31, 30, 90),
            WardState::Stale
        );
        // Active + past archive → archived (skips stale).
        assert_eq!(
            compute_transition(WardState::Active, 91, 30, 90),
            WardState::Archived
        );
        // Stale + past archive → archived.
        assert_eq!(
            compute_transition(WardState::Stale, 91, 30, 90),
            WardState::Archived
        );
        // Stale + back within stale window → reactivate.
        assert_eq!(
            compute_transition(WardState::Stale, 5, 30, 90),
            WardState::Active
        );
        // Archived → stays archived.
        assert_eq!(
            compute_transition(WardState::Archived, 5, 30, 90),
            WardState::Archived
        );
    }

    #[test]
    fn dry_run_returns_plan_without_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let wards_dir = dir.path().join("wards");
        std::fs::create_dir_all(&wards_dir).unwrap();
        let usage = WardUsage::new(&wards_dir);
        write_record(
            &usage,
            "old-agent",
            WardProvenance::Agent,
            WardState::Active,
            Some(Utc::now() - Duration::days(120)),
            false,
        );
        let curator = WardCurator::new(&wards_dir, dir.path().join("data"));
        let report = curator
            .cleanup(&CleanupRequest {
                dry_run: true,
                ..Default::default()
            })
            .unwrap();
        assert!(report.dry_run);
        assert_eq!(report.transitions.len(), 1);
        assert_eq!(report.transitions[0].to, WardState::Archived);
        // Sidecar unchanged.
        assert_eq!(
            usage.get("old-agent").unwrap().state,
            WardState::Active,
            "dry-run must not mutate"
        );
        // No backup written.
        assert!(report.backup_path.is_none());
        assert!(!wards_dir.join("_curator_backups").exists());
    }

    #[test]
    fn live_run_archives_old_agent_ward() {
        let dir = tempfile::tempdir().unwrap();
        let wards_dir = dir.path().join("wards");
        std::fs::create_dir_all(wards_dir.join("old-agent")).unwrap();
        std::fs::write(wards_dir.join("old-agent/AGENTS.md"), "# old-agent").unwrap();
        let usage = WardUsage::new(&wards_dir);
        write_record(
            &usage,
            "old-agent",
            WardProvenance::Agent,
            WardState::Active,
            Some(Utc::now() - Duration::days(200)),
            false,
        );
        let curator = WardCurator::new(&wards_dir, dir.path().join("data"));
        let report = curator.cleanup(&CleanupRequest::default()).unwrap();
        assert_eq!(report.transitions.len(), 1);
        let t = &report.transitions[0];
        assert_eq!(t.from, WardState::Active);
        assert_eq!(t.to, WardState::Archived);
        // Directory moved.
        assert!(!wards_dir.join("old-agent").exists());
        assert!(wards_dir.join("_archive/old-agent/AGENTS.md").exists());
        // Sidecar updated.
        let rec = usage.get("old-agent").unwrap();
        assert_eq!(rec.state, WardState::Archived);
        assert!(rec.archived_at.is_some());
        // Backup + audit log written.
        assert!(report.backup_path.as_ref().unwrap().exists());
        assert!(report.report_path.as_ref().unwrap().exists());
    }

    #[test]
    fn live_run_marks_stale_without_moving_dir() {
        let dir = tempfile::tempdir().unwrap();
        let wards_dir = dir.path().join("wards");
        std::fs::create_dir_all(wards_dir.join("getting-stale")).unwrap();
        let usage = WardUsage::new(&wards_dir);
        write_record(
            &usage,
            "getting-stale",
            WardProvenance::Agent,
            WardState::Active,
            Some(Utc::now() - Duration::days(45)),
            false,
        );
        let curator = WardCurator::new(&wards_dir, dir.path().join("data"));
        let report = curator.cleanup(&CleanupRequest::default()).unwrap();
        assert_eq!(report.transitions.len(), 1);
        assert_eq!(report.transitions[0].to, WardState::Stale);
        // Dir not moved.
        assert!(wards_dir.join("getting-stale").exists());
        assert!(usage.get("getting-stale").unwrap().archived_at.is_none());
    }

    #[test]
    fn skips_pinned_and_non_agent_wards() {
        let dir = tempfile::tempdir().unwrap();
        let wards_dir = dir.path().join("wards");
        std::fs::create_dir_all(&wards_dir).unwrap();
        let usage = WardUsage::new(&wards_dir);
        // Bundled — never touched.
        write_record(
            &usage,
            "wiki",
            WardProvenance::Bundled,
            WardState::Active,
            Some(Utc::now() - Duration::days(500)),
            false,
        );
        // User-authored — never touched.
        write_record(
            &usage,
            "my-notes",
            WardProvenance::User,
            WardState::Active,
            Some(Utc::now() - Duration::days(500)),
            false,
        );
        // Pinned agent ward — never touched even though it's eligible.
        write_record(
            &usage,
            "pinned-agent",
            WardProvenance::Agent,
            WardState::Active,
            Some(Utc::now() - Duration::days(500)),
            true,
        );
        let curator = WardCurator::new(&wards_dir, dir.path().join("data"));
        let report = curator
            .cleanup(&CleanupRequest {
                dry_run: true,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(report.transitions.len(), 0);
        assert_eq!(report.scanned, 3);
        assert_eq!(report.skipped_pinned, 1);
        assert_eq!(report.skipped_non_agent, 2);
    }

    #[test]
    fn reactivates_stale_after_recent_use() {
        let dir = tempfile::tempdir().unwrap();
        let wards_dir = dir.path().join("wards");
        std::fs::create_dir_all(&wards_dir).unwrap();
        let usage = WardUsage::new(&wards_dir);
        write_record(
            &usage,
            "back-from-dead",
            WardProvenance::Agent,
            WardState::Stale,
            Some(Utc::now() - Duration::days(2)),
            false,
        );
        let curator = WardCurator::new(&wards_dir, dir.path().join("data"));
        let report = curator.cleanup(&CleanupRequest::default()).unwrap();
        assert_eq!(report.transitions.len(), 1);
        assert_eq!(report.transitions[0].from, WardState::Stale);
        assert_eq!(report.transitions[0].to, WardState::Active);
    }

    #[test]
    fn no_op_run_writes_no_backup() {
        let dir = tempfile::tempdir().unwrap();
        let wards_dir = dir.path().join("wards");
        std::fs::create_dir_all(&wards_dir).unwrap();
        let usage = WardUsage::new(&wards_dir);
        write_record(
            &usage,
            "fresh",
            WardProvenance::Agent,
            WardState::Active,
            Some(Utc::now()),
            false,
        );
        let curator = WardCurator::new(&wards_dir, dir.path().join("data"));
        let report = curator.cleanup(&CleanupRequest::default()).unwrap();
        assert!(report.transitions.is_empty());
        assert!(report.backup_path.is_none());
        assert!(!wards_dir.join("_curator_backups").exists());
    }

    // ========================================================================
    // PHASE C — CONSOLIDATION TESTS
    // ========================================================================

    /// Set up two agent-authored wards with content so a merge has something
    /// to copy. Returns (TempDir, wards_dir, WardCurator).
    fn make_mergeable_pair() -> (tempfile::TempDir, PathBuf, WardCurator) {
        let dir = tempfile::tempdir().unwrap();
        let wards_dir = dir.path().join("wards");
        std::fs::create_dir_all(&wards_dir).unwrap();

        for name in ["travel-rome", "travel-paris"] {
            let wd = wards_dir.join(name);
            std::fs::create_dir_all(wd.join("memory-bank")).unwrap();
            std::fs::create_dir_all(wd.join("specs")).unwrap();
            std::fs::write(
                wd.join("AGENTS.md"),
                format!("# {name}\n\n## Purpose\nTrip planning for {name}.\n"),
            )
            .unwrap();
            std::fs::write(
                wd.join("memory-bank/ward.md"),
                format!("notes about {name}"),
            )
            .unwrap();
            std::fs::write(
                wd.join("specs/itinerary.md"),
                format!("itinerary for {name}"),
            )
            .unwrap();
        }

        let usage = WardUsage::new(&wards_dir);
        for name in ["travel-rome", "travel-paris"] {
            write_record(
                &usage,
                name,
                WardProvenance::Agent,
                WardState::Active,
                Some(Utc::now() - Duration::days(5)),
                false,
            );
        }

        let curator = WardCurator::new(&wards_dir, dir.path().join("data"));
        (dir, wards_dir, curator)
    }

    #[test]
    fn build_candidates_orders_and_summarises() {
        let (_dir, _, curator) = make_mergeable_pair();
        let cands = curator.build_candidates();
        assert_eq!(cands.len(), 2);
        // Stable alphabetical ordering.
        assert_eq!(cands[0].name, "travel-paris");
        assert_eq!(cands[1].name, "travel-rome");
        // Purpose blurb extracted from the doctrine.
        assert!(cands[0].purpose.contains("Trip planning"));
    }

    #[test]
    fn merge_creates_umbrella_copies_content_archives_sources() {
        let (_dir, wards_dir, curator) = make_mergeable_pair();
        let plan = ConsolidationPlan {
            consolidations: vec![ConsolidationAction::Merge {
                from: vec!["travel-rome".to_string(), "travel-paris".to_string()],
                into: "travel-planning".to_string(),
                purpose: "Trip planning across all destinations.".to_string(),
                reason: "both target city itineraries".to_string(),
            }],
        };
        let report = curator.apply_consolidation(&plan, false).unwrap();
        assert_eq!(report.applied.len(), 1);
        assert_eq!(report.applied[0].status, ApplyStatus::Ok);

        // Umbrella exists with merged content.
        assert!(wards_dir.join("travel-planning/AGENTS.md").exists());
        let doctrine =
            std::fs::read_to_string(wards_dir.join("travel-planning/AGENTS.md")).unwrap();
        assert!(doctrine.contains("Trip planning across all destinations"));
        // Memory-bank merged with per-source prefix to avoid clobbering.
        assert!(wards_dir
            .join("travel-planning/memory-bank/travel-rome__ward.md")
            .exists());
        assert!(wards_dir
            .join("travel-planning/memory-bank/travel-paris__ward.md")
            .exists());
        // Specs copied into per-source subdirectories.
        assert!(wards_dir
            .join("travel-planning/specs/travel-rome/itinerary.md")
            .exists());

        // Sources archived.
        assert!(!wards_dir.join("travel-rome").exists());
        assert!(wards_dir.join("_archive/travel-rome/AGENTS.md").exists());
        assert!(wards_dir.join("_archive/travel-paris/AGENTS.md").exists());

        // Sidecar reflects the new world.
        let usage = WardUsage::new(&wards_dir);
        let map = usage.load();
        assert_eq!(map["travel-rome"].state, WardState::Archived);
        assert_eq!(map["travel-paris"].state, WardState::Archived);
        assert_eq!(map["travel-planning"].created_by, WardProvenance::Agent);
        assert_eq!(map["travel-planning"].state, WardState::Active);

        // Backup + audit log written.
        assert!(report.backup_path.as_ref().unwrap().exists());
        assert!(report.report_path.as_ref().unwrap().exists());
    }

    #[test]
    fn dry_run_validates_without_mutating() {
        let (_dir, wards_dir, curator) = make_mergeable_pair();
        let plan = ConsolidationPlan {
            consolidations: vec![ConsolidationAction::Merge {
                from: vec!["travel-rome".to_string(), "travel-paris".to_string()],
                into: "travel-planning".to_string(),
                purpose: "Trip planning.".to_string(),
                reason: "test".to_string(),
            }],
        };
        let report = curator.apply_consolidation(&plan, true).unwrap();
        assert!(report.dry_run);
        assert_eq!(report.applied[0].status, ApplyStatus::Skipped);
        assert_eq!(
            report.applied[0].details.as_deref(),
            Some("dry-run"),
            "valid plan should report 'dry-run', not 'would fail'"
        );
        // Untouched.
        assert!(wards_dir.join("travel-rome").exists());
        assert!(!wards_dir.join("travel-planning").exists());
        assert!(report.backup_path.is_none());
    }

    #[test]
    fn dry_run_surfaces_validation_errors() {
        let (_dir, _, curator) = make_mergeable_pair();
        let plan = ConsolidationPlan {
            consolidations: vec![ConsolidationAction::Merge {
                from: vec!["does-not-exist".to_string()],
                into: "travel-planning".to_string(),
                purpose: "...".to_string(),
                reason: "test".to_string(),
            }],
        };
        let report = curator.apply_consolidation(&plan, true).unwrap();
        let details = report.applied[0].details.as_deref().unwrap_or_default();
        assert!(
            details.contains("would fail"),
            "expected validation error, got: {details}"
        );
    }

    #[test]
    fn merge_refuses_to_clobber_existing_into_dir() {
        let (_dir, wards_dir, curator) = make_mergeable_pair();
        // Create a directory at the planned umbrella name first.
        std::fs::create_dir_all(wards_dir.join("travel-planning")).unwrap();
        let plan = ConsolidationPlan {
            consolidations: vec![ConsolidationAction::Merge {
                from: vec!["travel-rome".to_string()],
                into: "travel-planning".to_string(),
                purpose: "...".to_string(),
                reason: "test".to_string(),
            }],
        };
        let report = curator.apply_consolidation(&plan, false).unwrap();
        assert_eq!(report.applied[0].status, ApplyStatus::Failed);
        // Source untouched.
        assert!(wards_dir.join("travel-rome").exists());
    }

    #[test]
    fn absorb_into_existing_ward_merges_and_archives_source() {
        let (_dir, wards_dir, curator) = make_mergeable_pair();
        // Create the umbrella ahead of time so this becomes an absorb.
        std::fs::create_dir_all(wards_dir.join("travel-planning/memory-bank")).unwrap();
        std::fs::write(
            wards_dir.join("travel-planning/AGENTS.md"),
            "# travel-planning\n\n## Purpose\nAll trips.\n",
        )
        .unwrap();
        let usage = WardUsage::new(&wards_dir);
        write_record(
            &usage,
            "travel-planning",
            WardProvenance::Agent,
            WardState::Active,
            Some(Utc::now()),
            false,
        );

        let plan = ConsolidationPlan {
            consolidations: vec![ConsolidationAction::Absorb {
                from: vec!["travel-rome".to_string()],
                into: "travel-planning".to_string(),
                reason: "rome fits inside the existing umbrella".to_string(),
            }],
        };
        let report = curator.apply_consolidation(&plan, false).unwrap();
        assert_eq!(report.applied[0].status, ApplyStatus::Ok);
        // Source archived.
        assert!(!wards_dir.join("travel-rome").exists());
        assert!(wards_dir.join("_archive/travel-rome/AGENTS.md").exists());
        // Content absorbed into existing umbrella.
        assert!(wards_dir
            .join("travel-planning/memory-bank/travel-rome__ward.md")
            .exists());
    }

    #[test]
    fn archive_action_archives_single_ward() {
        let (_dir, wards_dir, curator) = make_mergeable_pair();
        let plan = ConsolidationPlan {
            consolidations: vec![ConsolidationAction::Archive {
                ward: "travel-paris".to_string(),
                reason: "no longer relevant".to_string(),
            }],
        };
        let report = curator.apply_consolidation(&plan, false).unwrap();
        assert_eq!(report.applied[0].status, ApplyStatus::Ok);
        assert!(!wards_dir.join("travel-paris").exists());
        assert!(wards_dir.join("_archive/travel-paris/AGENTS.md").exists());
    }

    #[test]
    fn validation_blocks_pinned_and_non_agent_wards() {
        let (_dir, wards_dir, curator) = make_mergeable_pair();
        // Mark travel-rome pinned and travel-paris as user-authored.
        let usage = WardUsage::new(&wards_dir);
        usage.set_pinned("travel-rome", true).unwrap();
        usage
            .mark_created("travel-paris", WardProvenance::User)
            .unwrap();

        let plan = ConsolidationPlan {
            consolidations: vec![ConsolidationAction::Merge {
                from: vec!["travel-rome".to_string(), "travel-paris".to_string()],
                into: "travel-planning".to_string(),
                purpose: "...".to_string(),
                reason: "...".to_string(),
            }],
        };
        let report = curator.apply_consolidation(&plan, false).unwrap();
        assert_eq!(report.applied[0].status, ApplyStatus::Failed);
        let msg = report.applied[0].details.as_deref().unwrap_or_default();
        assert!(
            msg.contains("pinned") || msg.contains("not agent-created"),
            "unexpected message: {msg}"
        );
        // Untouched.
        assert!(wards_dir.join("travel-rome").exists());
        assert!(wards_dir.join("travel-paris").exists());
    }
}
