//! # Seeded defaults registry
//!
//! Records which bundled-default IDs we have ever seeded for this vault, so
//! deletions stick across daemon restarts. Without this, every restart sees
//! "this default ID is not on disk" and re-creates it (issue: cron jobs the
//! user removed kept reappearing).
//!
//! The registry is a single JSON file at
//! `<vault>/config/seeded-defaults.json`, append-only in spirit (we never
//! remove entries). It is keyed by category so the same mechanism can later
//! cover agents, MCPs, or anything else seeded from a bundled template.

use agent_primitives::vault_paths::SharedVaultPaths;
use gateway_cron::{CreateCronJobRequest, CronService};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::PathBuf;

#[cfg(test)]
const REGISTRY_FILE: &str = "seeded-defaults.json";

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub(crate) struct SeededDefaultsRegistry {
    #[serde(default)]
    pub cron: BTreeSet<String>,
}

impl SeededDefaultsRegistry {
    fn path(paths: &SharedVaultPaths) -> PathBuf {
        paths.seeded_defaults()
    }

    /// Load the registry from disk; returns `Default` on missing or
    /// malformed file (we never want a bad registry to block boot).
    pub(crate) async fn load(paths: &SharedVaultPaths) -> Self {
        let path = Self::path(paths);
        if !path.exists() {
            return Self::default();
        }
        match tokio::fs::read_to_string(&path).await {
            Ok(content) => serde_json::from_str(&content).unwrap_or_else(|e| {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "seeded_defaults: failed to parse registry; treating as empty"
                );
                Self::default()
            }),
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "seeded_defaults: failed to read registry; treating as empty"
                );
                Self::default()
            }
        }
    }

    pub(crate) async fn save(&self, paths: &SharedVaultPaths) -> std::io::Result<()> {
        let path = Self::path(paths);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        tokio::fs::write(&path, json).await
    }
}

/// Outcome of seeding a single bundled cron job request.
#[derive(Debug, PartialEq, Eq)]
enum SeedOutcome {
    /// New job written to disk.
    Created,
    /// Already on disk before the registry was introduced; recorded.
    Migrated,
    /// Already in the registry; nothing to do.
    AlreadySeeded,
    /// Create failed; left the registry untouched.
    Failed,
}

/// Apply the registry-aware seed sequence for a list of bundled cron
/// requests. Returns the number of jobs newly written to disk.
///
/// Each ID is processed at most once per vault: present in the registry →
/// skip; on disk but missing from registry → migrate (record but don't
/// recreate); otherwise → create and record. The registry is saved once at
/// the end if anything changed.
pub(crate) async fn seed_cron_with_registry(
    paths: &SharedVaultPaths,
    cron_service: &CronService,
    requests: Vec<CreateCronJobRequest>,
) -> usize {
    if requests.is_empty() {
        return 0;
    }

    let mut registry = SeededDefaultsRegistry::load(paths).await;
    let mut seeded = 0usize;
    let mut registry_changed = false;

    for request in requests {
        let outcome = process_request(cron_service, &mut registry, request).await;
        match outcome {
            SeedOutcome::Created => {
                seeded += 1;
                registry_changed = true;
            }
            SeedOutcome::Migrated => registry_changed = true,
            SeedOutcome::AlreadySeeded | SeedOutcome::Failed => {}
        }
    }

    if registry_changed {
        if let Err(e) = registry.save(paths).await {
            tracing::warn!(error = %e, "seed_default_cron: failed to save registry");
        }
    }

    seeded
}

async fn process_request(
    cron_service: &CronService,
    registry: &mut SeededDefaultsRegistry,
    request: CreateCronJobRequest,
) -> SeedOutcome {
    let job_id = request.id.clone();

    if registry.cron.contains(&job_id) {
        tracing::debug!(
            job_id = %job_id,
            "seed_default_cron: already seeded once, skipping"
        );
        return SeedOutcome::AlreadySeeded;
    }

    if cron_service.get(&job_id).await.is_ok() {
        tracing::debug!(
            job_id = %job_id,
            "seed_default_cron: pre-existing on disk, migrating into registry"
        );
        registry.cron.insert(job_id);
        return SeedOutcome::Migrated;
    }

    match cron_service.create(request).await {
        Ok(_) => {
            registry.cron.insert(job_id.clone());
            tracing::info!(job_id = %job_id, "Seeded default cron job");
            SeedOutcome::Created
        }
        Err(e) => {
            tracing::warn!(
                job_id = %job_id,
                error = %e,
                "seed_default_cron: failed to create job"
            );
            SeedOutcome::Failed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_primitives::vault_paths::VaultPaths;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn paths_for(temp: &TempDir) -> SharedVaultPaths {
        Arc::new(VaultPaths::new(temp.path().to_path_buf()))
    }

    #[tokio::test]
    async fn load_returns_default_when_file_absent() {
        let temp = TempDir::new().unwrap();
        let paths = paths_for(&temp);

        let registry = SeededDefaultsRegistry::load(&paths).await;
        assert!(registry.cron.is_empty());
    }

    #[tokio::test]
    async fn save_then_load_round_trips() {
        let temp = TempDir::new().unwrap();
        let paths = paths_for(&temp);

        let mut registry = SeededDefaultsRegistry::default();
        registry.cron.insert("default-cleanup".to_string());
        registry.cron.insert("daily-report".to_string());
        registry.save(&paths).await.unwrap();

        let loaded = SeededDefaultsRegistry::load(&paths).await;
        assert!(loaded.cron.contains("default-cleanup"));
        assert!(loaded.cron.contains("daily-report"));
        assert_eq!(loaded.cron.len(), 2);
    }

    #[tokio::test]
    async fn load_recovers_from_malformed_json() {
        let temp = TempDir::new().unwrap();
        let paths = paths_for(&temp);

        let registry_path = temp.path().join("config").join(REGISTRY_FILE);
        tokio::fs::create_dir_all(registry_path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&registry_path, "{not valid json")
            .await
            .unwrap();

        let registry = SeededDefaultsRegistry::load(&paths).await;
        assert!(registry.cron.is_empty());
    }

    #[tokio::test]
    async fn save_creates_config_dir_if_missing() {
        let temp = TempDir::new().unwrap();
        let paths = paths_for(&temp);

        let registry = SeededDefaultsRegistry::default();
        registry.save(&paths).await.unwrap();

        let registry_path = temp.path().join("config").join(REGISTRY_FILE);
        assert!(registry_path.exists());
    }

    fn sample_request(id: &str) -> CreateCronJobRequest {
        CreateCronJobRequest {
            id: id.to_string(),
            name: format!("{id} display"),
            schedule: "0 0 */4 * * *".to_string(),
            kind: Default::default(),
            agent_id: "general-purpose".to_string(),
            message: "noop".to_string(),
            http: None,
            respond_to: vec![],
            enabled: true,
            timezone: None,
            metadata: None,
        }
    }

    #[tokio::test]
    async fn first_seed_creates_jobs_and_records_them() {
        let temp = TempDir::new().unwrap();
        let paths = paths_for(&temp);
        let cron_service = CronService::new(paths.clone());

        let seeded = seed_cron_with_registry(
            &paths,
            &cron_service,
            vec![sample_request("default-cleanup")],
        )
        .await;

        assert_eq!(seeded, 1);
        assert!(cron_service.get("default-cleanup").await.is_ok());

        let registry = SeededDefaultsRegistry::load(&paths).await;
        assert!(registry.cron.contains("default-cleanup"));
    }

    #[tokio::test]
    async fn delete_then_reseed_does_not_recreate() {
        // Repro of the original bug: user deletes a seeded default, then we
        // run the seeder again (simulating restart). Job must stay deleted.
        let temp = TempDir::new().unwrap();
        let paths = paths_for(&temp);
        let cron_service = CronService::new(paths.clone());

        let seeded_first = seed_cron_with_registry(
            &paths,
            &cron_service,
            vec![sample_request("default-cleanup")],
        )
        .await;
        assert_eq!(seeded_first, 1);

        cron_service.delete("default-cleanup").await.unwrap();
        assert!(cron_service.get("default-cleanup").await.is_err());

        let seeded_second = seed_cron_with_registry(
            &paths,
            &cron_service,
            vec![sample_request("default-cleanup")],
        )
        .await;

        assert_eq!(seeded_second, 0);
        assert!(
            cron_service.get("default-cleanup").await.is_err(),
            "deleted default reappeared on reseed"
        );
    }

    #[tokio::test]
    async fn preexisting_job_without_registry_is_migrated_then_skipped() {
        // Vaults that ran a pre-fix daemon already have the seeded job on
        // disk but no registry. The first reseed should record the ID
        // without recreating, then subsequent deletes must stick.
        let temp = TempDir::new().unwrap();
        let paths = paths_for(&temp);
        let cron_service = CronService::new(paths.clone());

        cron_service
            .create(sample_request("default-cleanup"))
            .await
            .unwrap();

        let seeded_first = seed_cron_with_registry(
            &paths,
            &cron_service,
            vec![sample_request("default-cleanup")],
        )
        .await;
        assert_eq!(seeded_first, 0, "migration must not double-create");

        let registry = SeededDefaultsRegistry::load(&paths).await;
        assert!(registry.cron.contains("default-cleanup"));

        cron_service.delete("default-cleanup").await.unwrap();
        let seeded_second = seed_cron_with_registry(
            &paths,
            &cron_service,
            vec![sample_request("default-cleanup")],
        )
        .await;
        assert_eq!(seeded_second, 0);
        assert!(cron_service.get("default-cleanup").await.is_err());
    }

    #[tokio::test]
    async fn newly_shipped_default_is_seeded_after_existing_ones() {
        // Future-proof: a vault that has already migrated `default-cleanup`
        // should still pick up a brand-new bundled default we ship later.
        let temp = TempDir::new().unwrap();
        let paths = paths_for(&temp);
        let cron_service = CronService::new(paths.clone());

        seed_cron_with_registry(
            &paths,
            &cron_service,
            vec![sample_request("default-cleanup")],
        )
        .await;

        let seeded = seed_cron_with_registry(
            &paths,
            &cron_service,
            vec![
                sample_request("default-cleanup"),
                sample_request("default-summary"),
            ],
        )
        .await;

        assert_eq!(seeded, 1);
        assert!(cron_service.get("default-summary").await.is_ok());

        let registry = SeededDefaultsRegistry::load(&paths).await;
        assert!(registry.cron.contains("default-cleanup"));
        assert!(registry.cron.contains("default-summary"));
    }

    #[tokio::test]
    async fn empty_request_list_is_a_noop() {
        let temp = TempDir::new().unwrap();
        let paths = paths_for(&temp);
        let cron_service = CronService::new(paths.clone());

        let seeded = seed_cron_with_registry(&paths, &cron_service, vec![]).await;

        assert_eq!(seeded, 0);
        let registry_path = temp.path().join("config").join(REGISTRY_FILE);
        assert!(
            !registry_path.exists(),
            "empty seed should not touch the registry file"
        );
    }
}
